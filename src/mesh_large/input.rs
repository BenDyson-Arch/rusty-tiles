//! Streamed input binding: the root document and its confined dependencies are admitted with
//! the validated F1a parser and binding rules under the large-mesh limits, but payloads stay on
//! disk. Buffer bytes are read through positioned, bounded buffers; the bound files are checked
//! for change after the last payload read (design §2.3: change detection in pass 1).
use crate::mesh_archive::{
    binding,
    source::{self, AccessorView, BinChunk, BufferSource, Document, Limits},
};
use crate::{JobError, JobErrorKind};
use same_file::Handle;
use std::{
    fs::{File, Metadata},
    io,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, JobError>;

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidInput, message)
}

fn unsupported(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::Unsupported, message)
}

/// Positioned read without a shared file cursor.
fn read_at(file: &File, offset: u64, out: &mut [u8]) -> io::Result<usize> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_at(file, out, offset)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::FileExt::seek_read(file, out, offset)
    }
}

/// Fills `out` from `offset`; a short file means it changed after admission.
fn read_exact_at(file: &File, path: &Path, mut offset: u64, mut out: &mut [u8]) -> Result<()> {
    while !out.is_empty() {
        match read_at(file, offset, out) {
            Ok(0) => return Err(binding::changed(path)),
            Ok(n) => {
                offset += n as u64;
                out = &mut out[n..];
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(JobError::io("read mesh source", path, e)),
        }
    }
    Ok(())
}

struct BoundFile {
    /// The path as admitted (for the rebinding check) and its canonical form.
    path: PathBuf,
    file: File,
    identity: Handle,
    metadata: Metadata,
}

impl BoundFile {
    fn open(path: PathBuf, admitted: &Metadata) -> Result<Self> {
        let file = binding::open_file(&path)?;
        let metadata = file
            .metadata()
            .map_err(|e| JobError::io("inspect mesh source handle", &path, e))?;
        if !binding::same_metadata(admitted, &metadata) {
            return Err(binding::changed(&path));
        }
        let identity = binding::identity(&file, &path)?;
        Ok(Self {
            path,
            file,
            identity,
            metadata,
        })
    }

    fn verify(&self) -> Result<()> {
        let handle = self
            .file
            .metadata()
            .map_err(|e| JobError::io("inspect mesh source handle", &self.path, e))?;
        let named = std::fs::symlink_metadata(&self.path).map_err(|_| binding::changed(&self.path))?;
        if !binding::same_metadata(&self.metadata, &handle)
            || !binding::same_metadata(&self.metadata, &named)
        {
            return Err(binding::changed(&self.path));
        }
        Ok(())
    }
}

/// Where one glTF buffer's bytes live: a bound file and the buffer's offset in it.
#[derive(Clone, Copy)]
struct BufferLocation {
    file: usize,
    offset: u64,
}

/// An admitted scene whose payloads are read on demand.
pub(crate) struct StreamSource {
    pub document: Document,
    files: Vec<BoundFile>,
    buffers: Vec<BufferLocation>,
    pub json_bytes: u64,
    pub source_bytes: u64,
    pub external_files: u64,
    pub external_bytes: u64,
}

impl StreamSource {
    /// Admits `input` and every resource it names: root JSON within `J`, the explicit count
    /// caps, the F1a semantic profile, and URIs confined to the document directory without
    /// symlinks. Image resources are admitted (confined, regular files) but not read here.
    pub(crate) fn open(input: &Path, check: &mut dyn FnMut() -> Result<()>) -> Result<Self> {
        check()?;
        let original = std::path::absolute(input)
            .map_err(|error| JobError::io("resolve absolute mesh source", input, error))?;
        let admitted = binding::metadata(&original)?;
        if !admitted.is_file() {
            return Err(invalid(
                "mesh source must be a regular file without a symlink leaf",
            ));
        }
        let canonical = binding::canonical_path(&original, false)?;
        let base = canonical
            .parent()
            .expect("canonical source file has parent")
            .to_owned();
        let root = BoundFile::open(original, &admitted)?;
        let length = root.metadata.len();
        let limits = Limits::LARGE;
        let mut magic = [0u8; 4];
        let glb = length >= 4 && {
            read_exact_at(&root.file, &root.path, 0, &mut magic)?;
            &magic == b"glTF"
        };
        let (json, bin) = if glb {
            let layout = source::glb_layout(length, &limits, &mut |at, out| {
                if at + out.len() as u64 > length {
                    return Err(invalid("truncated GLB envelope"));
                }
                read_exact_at(&root.file, &root.path, at, out)
            })?;
            let mut json = vec![0; (layout.json.end - layout.json.start) as usize];
            read_exact_at(&root.file, &root.path, layout.json.start, &mut json)?;
            let bin = match layout.bin {
                Some(range) => {
                    let tail_len = (range.end - range.start).min(3);
                    let mut tail = vec![0; tail_len as usize];
                    read_exact_at(&root.file, &root.path, range.end - tail_len, &mut tail)?;
                    let range = usize::try_from(range.start)
                        .ok()
                        .zip(usize::try_from(range.end).ok())
                        .ok_or_else(|| unsupported("GLB BIN outside the host address range"))?;
                    Some(BinChunk {
                        range: range.0..range.1,
                        tail,
                    })
                }
                None => None,
            };
            (json, bin)
        } else {
            if length > limits.json_bytes as u64 {
                return Err(unsupported(format!(
                    "glTF JSON exceeds {} MiB",
                    limits.json_bytes >> 20
                )));
            }
            let mut json = vec![0; length as usize];
            read_exact_at(&root.file, &root.path, 0, &mut json)?;
            (json, None)
        };
        let json_bytes = json.len() as u64;
        let document = Document::parse_parts(&json, glb, bin, &limits, &mut *check)?;
        drop(json);

        let mut relatives = Vec::with_capacity(document.resources().len());
        for request in document.resources() {
            check()?;
            relatives.push(binding::relative_uri(&request.uri)?);
        }
        let buffer_requests: Vec<Option<usize>> = document
            .buffer_sources()
            .iter()
            .map(|source| match source {
                BufferSource::External(request) => Some(*request),
                BufferSource::Embedded(_) => None,
            })
            .collect();
        let root_identity = root.identity.clone();
        let mut files = vec![root];
        let mut request_files = vec![None; relatives.len()];
        let mut external_bytes = 0;
        for (request, relative) in relatives.iter().enumerate() {
            check()?;
            let (path, admitted) = binding::dependency_path(&base, relative, false)?;
            if !buffer_requests.contains(&Some(request)) {
                // Image resource: admitted here, captured by the texture pass.
                continue;
            }
            let bound = BoundFile::open(path, &admitted)?;
            if bound.identity == root_identity {
                return Err(unsupported("mesh resource aliases its source document"));
            }
            let index = match files.iter().position(|f| f.identity == bound.identity) {
                Some(existing) => {
                    if !binding::same_metadata(&files[existing].metadata, &bound.metadata) {
                        return Err(binding::changed(&bound.path));
                    }
                    existing
                }
                None => {
                    external_bytes += bound.metadata.len();
                    files.push(bound);
                    files.len() - 1
                }
            };
            request_files[request] = Some(index);
        }
        let mut buffers = Vec::with_capacity(document.buffer_sources().len());
        for (source, &declared) in document
            .buffer_sources()
            .iter()
            .zip(document.buffer_lengths())
        {
            let location = match source {
                BufferSource::Embedded(range) => BufferLocation {
                    file: 0,
                    offset: range.start as u64,
                },
                BufferSource::External(request) => {
                    let file = request_files[*request].expect("buffer request is bound");
                    if files[file].metadata.len() < declared as u64 {
                        return Err(invalid(
                            "actual buffer bytes shorter than declared byteLength",
                        ));
                    }
                    BufferLocation { file, offset: 0 }
                }
            };
            buffers.push(location);
        }
        Ok(Self {
            document,
            source_bytes: length,
            external_files: files.len() as u64 - 1,
            external_bytes,
            files,
            buffers,
            json_bytes,
        })
    }

    /// A positioned reader over one accessor's elements, with a `buffer_bytes` window.
    pub(crate) fn reader<'a>(
        &'a self,
        view: &AccessorView<'a>,
        buffer_bytes: usize,
    ) -> ElementReader<'a> {
        let location = self.buffers[view.buffer];
        let file = &self.files[location.file];
        let element = view.element_bytes();
        ElementReader {
            file: &file.file,
            path: &file.path,
            base: location.offset + view.start as u64,
            stride: view.stride as u64,
            element,
            component: view.component,
            capacity: buffer_bytes.max(element),
            window: Vec::new(),
            window_start: 0,
        }
    }

    /// Fails with "mesh source changed during capture" when a bound file's length, time or
    /// identity changed, or its path was rebound, since admission.
    pub(crate) fn verify_unchanged(&self) -> Result<()> {
        self.files.iter().try_for_each(BoundFile::verify)
    }
}

/// Reads accessor elements by index through a bounded window. Forward access within the
/// window is served from memory; any other index refills it with one positioned read.
pub(crate) struct ElementReader<'a> {
    file: &'a File,
    path: &'a Path,
    base: u64,
    stride: u64,
    element: usize,
    component: usize,
    capacity: usize,
    window: Vec<u8>,
    window_start: u64,
}

impl ElementReader<'_> {
    fn element(&mut self, index: usize) -> Result<&[u8]> {
        let start = self.base + index as u64 * self.stride;
        let end = start + self.element as u64;
        if start < self.window_start || end > self.window_start + self.window.len() as u64 {
            self.window.resize(self.capacity, 0);
            let mut filled = 0;
            while filled < self.element {
                match read_at(self.file, start + filled as u64, &mut self.window[filled..]) {
                    Ok(0) => return Err(binding::changed(self.path)),
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(JobError::io("read mesh source", self.path, e)),
                }
            }
            self.window.truncate(filled);
            self.window_start = start;
        }
        let at = (start - self.window_start) as usize;
        Ok(&self.window[at..at + self.element])
    }

    pub(crate) fn scalar(&mut self, index: usize) -> Result<f64> {
        let component = self.component;
        Ok(source::component_scalar(component, self.element(index)?))
    }

    pub(crate) fn vec3(&mut self, index: usize) -> Result<[f64; 3]> {
        let component = self.component;
        let size = source::component_size(component);
        let bytes = self.element(index)?;
        Ok(std::array::from_fn(|c| {
            source::component_scalar(component, &bytes[c * size..])
        }))
    }
}
