//! Confined local input admission and bounded accessor windows. Sources remain stable until
//! capture ends; change detection is observational, rather than an atomic snapshot.
use crate::gltf_source::{
    binding,
    source::{self, AccessorView, BinChunk, BufferSource, Document, Limits},
};
use crate::{JobError, JobErrorKind};
use std::{
    collections::{HashMap, HashSet},
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

/// A file identity stores no open descriptor. Windows uses the same FILE_ID_INFO boundary as
/// the raster source; Unix uses the identity of the opened handle, never a pathname alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FileIdentity([u64; 3]);
fn file_identity(file: &File, path: &Path, metadata: &Metadata) -> Result<FileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = (file, path);
        Ok(FileIdentity([metadata.dev(), metadata.ino(), 0]))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        struct Info {
            volume: u64,
            id: [u64; 2],
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandleEx(
                handle: *mut std::ffi::c_void,
                class: i32,
                info: *mut std::ffi::c_void,
                size: u32,
            ) -> i32;
        }
        let _ = metadata;
        let mut info = Info {
            volume: 0,
            id: [0; 2],
        };
        // SAFETY: the live File supplies a valid handle, and Info matches FILE_ID_INFO.
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle().cast(),
                18,
                (&mut info as *mut Info).cast(),
                std::mem::size_of::<Info>() as u32,
            )
        } == 0
        {
            return Err(JobError::io(
                "inspect mesh source identity",
                path,
                io::Error::last_os_error(),
            ));
        }
        Ok(FileIdentity([info.volume, info.id[0], info.id[1]]))
    }
}

/// Every admitted pathname is retained, including aliases and image dependencies. Rechecking
/// a dependency repeats every URI component check, so ancestor symlink swaps are detected.
struct BoundPath {
    path: PathBuf,
    canonical: PathBuf,
    relative: Option<PathBuf>,
    metadata: Metadata,
    identity: FileIdentity,
}
impl BoundPath {
    fn admit(
        path: PathBuf,
        canonical: PathBuf,
        relative: Option<PathBuf>,
        admitted: &Metadata,
    ) -> Result<(Self, File)> {
        let file = binding::open_file(&canonical)?;
        let metadata = file
            .metadata()
            .map_err(|e| JobError::io("inspect mesh source handle", &path, e))?;
        if !binding::same_metadata(admitted, &metadata) {
            return Err(binding::changed(&path));
        }
        let identity = file_identity(&file, &path, &metadata)?;
        Ok((
            Self {
                path,
                canonical,
                relative,
                metadata,
                identity,
            },
            file,
        ))
    }
    fn open_checked(&self, base: &Path) -> Result<File> {
        let (canonical, named) = if let Some(relative) = &self.relative {
            binding::dependency_path(base, relative, true)?
        } else {
            let named =
                std::fs::symlink_metadata(&self.path).map_err(|_| binding::changed(&self.path))?;
            (binding::canonical_path(&self.path, true)?, named)
        };
        if canonical != self.canonical || !binding::same_metadata(&self.metadata, &named) {
            return Err(binding::changed(&self.path));
        }
        let file = binding::reopen_file(&canonical)?;
        self.check_handle(&file)?;
        Ok(file)
    }
    fn check_handle(&self, file: &File) -> Result<()> {
        let held = file
            .metadata()
            .map_err(|e| JobError::io("inspect mesh source handle", &self.path, e))?;
        if !binding::same_metadata(&self.metadata, &held)
            || file_identity(file, &self.path, &held)? != self.identity
        {
            return Err(binding::changed(&self.path));
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct BufferLocation {
    path: usize,
    offset: u64,
}

pub(crate) struct StreamSource {
    pub document: Document,
    base: PathBuf,
    paths: Vec<BoundPath>,
    buffers: Vec<BufferLocation>,
    pub json_bytes: u64,
    pub source_bytes: u64,
    pub external_files: u64,
    pub external_bytes: u64,
}
impl StreamSource {
    pub(crate) fn open(input: &Path, check: &mut dyn FnMut() -> Result<()>) -> Result<Self> {
        check()?;
        let original = std::path::absolute(input)
            .map_err(|e| JobError::io("resolve absolute mesh source", input, e))?;
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
        let (root, file) = BoundPath::admit(original, canonical, None, &admitted)?;
        let length = root.metadata.len();
        let limits = Limits::LARGE;
        let mut magic = [0; 4];
        let glb = length >= 4 && {
            read_exact_at(&file, &root.path, 0, &mut magic)?;
            &magic == b"glTF"
        };
        let (json, bin) = if glb {
            let layout = source::glb_layout(length, &limits, &mut |at, out| {
                if at
                    .checked_add(out.len() as u64)
                    .is_none_or(|end| end > length)
                {
                    return Err(invalid("truncated GLB envelope"));
                }
                read_exact_at(&file, &root.path, at, out)
            })?;
            let mut json = vec![0; (layout.json.end - layout.json.start) as usize];
            read_exact_at(&file, &root.path, layout.json.start, &mut json)?;
            let bin = layout
                .bin
                .map(|range| -> Result<_> {
                    let tail_len = (range.end - range.start).min(3);
                    let mut tail = vec![0; tail_len as usize];
                    read_exact_at(&file, &root.path, range.end - tail_len, &mut tail)?;
                    let start = usize::try_from(range.start)
                        .map_err(|_| unsupported("GLB BIN outside the host address range"))?;
                    let end = usize::try_from(range.end)
                        .map_err(|_| unsupported("GLB BIN outside the host address range"))?;
                    Ok(BinChunk {
                        range: start..end,
                        tail,
                    })
                })
                .transpose()?;
            (json, bin)
        } else {
            if length > limits.json_bytes as u64 {
                return Err(unsupported(format!(
                    "glTF JSON exceeds {} MiB",
                    limits.json_bytes >> 20
                )));
            }
            let mut json = vec![0; length as usize];
            read_exact_at(&file, &root.path, 0, &mut json)?;
            (json, None)
        };
        root.check_handle(&file)?;
        drop(file);
        let json_bytes = json.len() as u64;
        let document = Document::parse_parts(&json, glb, bin, &limits, &mut *check)?;
        drop(json);
        let mut paths = vec![root];
        let mut identities = HashMap::from([(paths[0].identity, paths[0].metadata.len())]);
        let mut external_bytes = 0u64;
        for request in document.resources() {
            check()?;
            let relative = binding::relative_uri(&request.uri)?;
            let (canonical, admitted) = binding::dependency_path(&base, &relative, false)?;
            let path = base.join(&relative);
            let (bound, file) = BoundPath::admit(path, canonical, Some(relative), &admitted)?;
            if bound.identity == paths[0].identity {
                return Err(unsupported("mesh resource aliases its source document"));
            }
            if identities
                .insert(bound.identity, bound.metadata.len())
                .is_none()
            {
                external_bytes = external_bytes
                    .checked_add(bound.metadata.len())
                    .ok_or_else(|| unsupported("mesh source byte sum overflow"))?;
            }
            bound.check_handle(&file)?;
            drop(file);
            paths.push(bound);
        }
        let mut buffers = Vec::with_capacity(document.buffer_sources().len());
        for (source, &declared) in document
            .buffer_sources()
            .iter()
            .zip(document.buffer_lengths())
        {
            let location = match source {
                BufferSource::Embedded(range) => BufferLocation {
                    path: 0,
                    offset: range.start as u64,
                },
                BufferSource::External(request) => {
                    let path = request + 1;
                    if paths[path].metadata.len() < declared as u64 {
                        return Err(invalid(
                            "actual buffer bytes shorter than declared byteLength",
                        ));
                    }
                    BufferLocation { path, offset: 0 }
                }
            };
            buffers.push(location);
        }
        let source = Self {
            document,
            base,
            paths,
            buffers,
            json_bytes,
            source_bytes: length,
            external_files: identities.len() as u64 - 1,
            external_bytes,
        };
        source.verify_unchanged()?;
        Ok(source)
    }
    pub(crate) fn reader(&self, view: &AccessorView<'_>, buffer_bytes: usize) -> ElementReader<'_> {
        let location = self.buffers[view.buffer];
        let element = view.element_bytes();
        let base = location.offset + view.start as u64;
        ElementReader {
            source: self,
            path: location.path,
            base,
            stride: view.stride as u64,
            count: view.count,
            element,
            width: view.width,
            component: view.component,
            normalized: view.normalized,
            capacity: buffer_bytes.max(element),
            window: Vec::new(),
            window_start: 0,
        }
    }
    pub(crate) fn verify_unchanged(&self) -> Result<()> {
        for path in &self.paths {
            drop(path.open_checked(&self.base)?);
        }
        Ok(())
    }
    /// Retain F1a's declared-accessor and authored-primitive payload checks even for metadata
    /// in an unselected mesh. Images are confined dependencies, with decoding deferred.
    pub(crate) fn validate_payload(
        &self,
        buffer_bytes: usize,
        check: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        for view in self.document.accessor_views() {
            let mut reader = self.reader(&view, buffer_bytes);
            let mut extrema = source::Extrema::new();
            for i in 0..view.count {
                if i.is_multiple_of(1024) {
                    check()?;
                }
                let values = reader.raw_values(i)?;
                for (c, value) in values[..view.width].iter().enumerate() {
                    extrema.observe(c, *value)?;
                }
            }
            extrema.verify(&view)?;
        }
        let mut attributes = HashSet::new();
        let mut indices = HashSet::new();
        for p in self.document.payload_primitives() {
            check()?;
            for (role, view) in p.attributes() {
                let cached_role = match role {
                    source::VertexRole::Texcoord(_) => source::VertexRole::Texcoord(0),
                    role => role,
                };
                if !attributes.insert((cached_role, view.index)) {
                    continue;
                }
                let mut reader = self.reader(&view, buffer_bytes);
                for i in 0..view.count {
                    if i.is_multiple_of(1024) {
                        check()?;
                    }
                    let values = reader.values(i)?;
                    role.check_payload(&values[..view.width])?;
                }
            }
            if let Some(view) = p
                .indices
                .filter(|v| indices.insert((v.index, p.position.count)))
            {
                let mut reader = self.reader(&view, buffer_bytes);
                for i in 0..view.count {
                    if i.is_multiple_of(1024) {
                        check()?;
                    }
                    source::check_index(
                        reader.scalar(i)? as usize,
                        p.position.count,
                        view.component,
                    )?;
                }
            }
            if let Some(tangent) = p.tangent {
                let mut tangent = self.reader(&tangent, buffer_bytes);
                let mut indices = p.indices.map(|v| self.reader(&v, buffer_bytes));
                for start in (0..p.count).step_by(3) {
                    if start.is_multiple_of(3072) {
                        check()?;
                    }
                    let mut signs = [0.; 3];
                    for (corner, sign) in signs.iter_mut().enumerate() {
                        let vertex = if let Some(indices) = &mut indices {
                            indices.scalar(start + corner)? as usize
                        } else {
                            start + corner
                        };
                        *sign = tangent.raw_values(vertex)?[3];
                    }
                    if signs[1..].iter().any(|s| *s != signs[0]) {
                        return Err(unsupported(
                            "mixed triangle TANGENT W has undefined tangent space",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Readers retain bytes only. A refill opens, validates, reads and closes its source; even
/// multiple live readers together hold at most one payload descriptor during a synchronous
/// capture. The window is clipped to the admitted accessor, including its final element.
pub(crate) struct ElementReader<'a> {
    source: &'a StreamSource,
    path: usize,
    base: u64,
    stride: u64,
    count: usize,
    element: usize,
    width: usize,
    component: usize,
    normalized: bool,
    capacity: usize,
    window: Vec<u8>,
    window_start: u64,
}
impl ElementReader<'_> {
    fn element(&mut self, index: usize) -> Result<&[u8]> {
        if index >= self.count {
            return Err(invalid("mesh accessor index outside admitted range"));
        }
        let start = self.base + index as u64 * self.stride;
        let end = start + self.element as u64;
        if start < self.window_start || end > self.window_start + self.window.len() as u64 {
            let accessor_end =
                self.base + (self.count as u64 - 1) * self.stride + self.element as u64;
            let bytes = (accessor_end - start).min(self.capacity as u64) as usize;
            self.window.resize(bytes, 0);
            let bound = &self.source.paths[self.path];
            let file = bound.open_checked(&self.source.base)?;
            read_exact_at(&file, &bound.path, start, &mut self.window)?;
            bound.check_handle(&file)?;
            drop(file);
            self.window_start = start;
        }
        let at = (start - self.window_start) as usize;
        Ok(&self.window[at..at + self.element])
    }
    pub(crate) fn raw_values(&mut self, index: usize) -> Result<[f64; 4]> {
        let (component, width) = (self.component, self.width);
        let size = source::component_size(component);
        let bytes = self.element(index)?;
        Ok(std::array::from_fn(|c| {
            if c < width {
                source::component_scalar(component, &bytes[c * size..])
            } else {
                0.
            }
        }))
    }
    pub(crate) fn values(&mut self, index: usize) -> Result<[f64; 4]> {
        let mut values = self.raw_values(index)?;
        if self.normalized {
            let divisor = match self.component {
                5121 => u8::MAX as f64,
                5123 => u16::MAX as f64,
                _ => unreachable!("admitted normalized component"),
            };
            for value in &mut values[..self.width] {
                *value /= divisor;
            }
        }
        Ok(values)
    }
    pub(crate) fn scalar(&mut self, index: usize) -> Result<f64> {
        Ok(self.raw_values(index)?[0])
    }
    pub(crate) fn vec3(&mut self, index: usize) -> Result<[f64; 3]> {
        let values = self.raw_values(index)?;
        Ok([values[0], values[1], values[2]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.gltf");
        let doc = json!({
            "asset":{"version":"2.0"},"buffers":[{"uri":"data.bin","byteLength":36}],
            "bufferViews":[{"buffer":0,"byteLength":36}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
            "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}]
        });
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let positions = [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let bytes: Vec<_> = positions
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        fs::write(dir.path().join("data.bin"), bytes).unwrap();
        (dir, input)
    }

    #[test]
    fn streaming_keeps_payload_extrema_checks() {
        let (dir, input) = fixture();
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        source.validate_payload(16, &mut || Ok(())).unwrap();
        drop(source);
        let mut bytes = fs::read(dir.path().join("data.bin")).unwrap();
        bytes[12..16].copy_from_slice(&0.5f32.to_le_bytes());
        fs::write(dir.path().join("data.bin"), bytes).unwrap();
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let error = source.validate_payload(16, &mut || Ok(())).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::InvalidInput);
        assert!(error.message().contains("extrema"));
    }

    #[cfg(unix)]
    #[test]
    fn final_sweep_checks_alias_names_and_ancestor_components() {
        use std::os::unix::fs::symlink;
        let (dir, input) = fixture();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::hard_link(
            dir.path().join("data.bin"),
            dir.path().join("nested/alias.bin"),
        )
        .unwrap();
        let mut doc: serde_json::Value =
            serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
        doc["buffers"]
            .as_array_mut()
            .unwrap()
            .push(json!({"uri":"nested/alias.bin","byteLength":36}));
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        assert_eq!(source.external_files, 1);
        fs::rename(dir.path().join("nested"), dir.path().join("moved")).unwrap();
        symlink("moved", dir.path().join("nested")).unwrap();
        let error = source.verify_unchanged().unwrap_err();
        assert!(error.message().contains("source changed during capture"));
    }

    #[test]
    fn reader_window_stops_at_admitted_accessor_and_refuses_extra_index() {
        let (_dir, input) = fixture();
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let view = source.document.scene_primitives().next().unwrap().position;
        let mut reader = source.reader(&view, 64);
        assert_eq!(reader.vec3(2).unwrap(), [0., 1., 0.]);
        assert_eq!(reader.window.len(), 12);
        let error = reader.vec3(3).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::InvalidInput);
    }
}
