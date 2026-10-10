//! Consumer-owned, finite local input capture. Decoders and codecs receive bytes.
//! Stable inputs during preparation remain a precondition; this is not an
//! atomic multi-file snapshot or a hostile-filesystem openat sandbox.
use super::source;
use crate::{JobError, JobErrorKind};
use same_file::Handle;
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_JSON_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_REQUESTS: usize = 64;
const MAX_URI_BYTES: usize = 4096;
const MAX_COMPONENTS: usize = 128;
type Result<T> = std::result::Result<T, JobError>;

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidInput, message)
}
fn unsupported(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::Unsupported, message)
}
fn changed(path: &Path) -> JobError {
    invalid(format!(
        "mesh source changed during capture: {}",
        path.display()
    ))
}

pub(super) struct Snapshot {
    pub output: PathBuf,
    pub document: source::Document,
    root: Arc<Vec<u8>>,
    resources: Vec<Arc<Vec<u8>>>,
    pub source_bytes: u64,
    pub external_files: u64,
    pub external_bytes: u64,
}
impl Snapshot {
    fn resource(&self, index: usize) -> &[u8] {
        &self.resources[index]
    }
    pub fn buffers(&self) -> Vec<&[u8]> {
        self.document
            .buffer_sources()
            .iter()
            .map(|source| match source {
                source::BufferSource::Embedded(range) => &self.root[range.clone()],
                source::BufferSource::External(index) => self.resource(*index),
            })
            .collect()
    }
    pub fn images(&self) -> Vec<Option<&[u8]>> {
        self.document
            .image_sources()
            .iter()
            .map(|source| match source {
                source::ImageSource::BufferView(_) => None,
                source::ImageSource::External(index) => Some(self.resource(*index)),
            })
            .collect()
    }
}

// Parse the URI once, segment by literal slash, and only then decode each
// segment. Encoded separators never become filesystem path syntax.
fn relative_uri(uri: &str) -> Result<PathBuf> {
    if uri.is_empty() {
        return Err(invalid("resource URI must not be empty"));
    }
    let component_count = uri.split('/').count();
    if uri.len() > MAX_URI_BYTES || component_count > MAX_COMPONENTS {
        return Err(unsupported("resource URI exceeds length/component ceiling"));
    }
    if uri.starts_with('/') || uri.contains([':', '?', '#']) {
        return Err(unsupported(
            "resource URI must be a relative path without scheme, query or fragment",
        ));
    }
    if uri.ends_with('/') {
        return Err(invalid("resource URI must name a file"));
    }
    let mut parts = Vec::new();
    for (index, raw) in uri.split('/').enumerate() {
        let mut decoded = Vec::with_capacity(raw.len());
        let bytes = raw.as_bytes();
        let mut cursor = 0;
        while cursor < bytes.len() {
            let byte = bytes[cursor];
            if byte == b'%' {
                let pair = bytes
                    .get(cursor + 1..cursor + 3)
                    .ok_or_else(|| invalid("truncated resource URI percent escape"))?;
                let digit = |b: u8| match b {
                    b'0'..=b'9' => Some(b - b'0'),
                    b'a'..=b'f' => Some(b - b'a' + 10),
                    b'A'..=b'F' => Some(b - b'A' + 10),
                    _ => None,
                };
                let (Some(high), Some(low)) = (digit(pair[0]), digit(pair[1])) else {
                    return Err(invalid("invalid resource URI percent escape"));
                };
                decoded.push(high * 16 + low);
                cursor += 3;
            } else {
                if byte.is_ascii() && !(byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)) {
                    return Err(invalid("invalid unescaped resource URI character"));
                }
                decoded.push(byte);
                cursor += 1;
            }
        }
        let part = String::from_utf8(decoded)
            .map_err(|_| invalid("resource URI decoded path must be UTF-8"))?;
        if part.chars().any(char::is_control) {
            return Err(invalid("resource URI contains a control character"));
        }
        if part.contains(['/', '\\']) {
            return Err(unsupported("resource URI contains an encoded separator"));
        }
        if index + 1 == component_count && matches!(part.as_str(), "." | "..") {
            return Err(invalid(
                "resource URI terminal dot component denotes a directory",
            ));
        }
        match part.as_str() {
            "" | "." => continue,
            ".." => {
                if parts.pop().is_none() {
                    return Err(unsupported("resource URI escapes the document directory"));
                }
                continue;
            }
            _ => {}
        }
        if part.contains(['<', '>', ':', '"', '|', '?', '*']) || part.ends_with(['.', ' ']) {
            return Err(unsupported(
                "resource URI filename outside portable path profile",
            ));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(invalid("resource URI does not name a file"));
    }
    Ok(parts.into_iter().collect())
}

fn open_file(path: &Path) -> Result<File> {
    open_file_with(path, false)
}
fn open_file_with(path: &Path, recheck: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options.open(path).map_err(|error| {
        if recheck
            && matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            )
        {
            return changed(path);
        }
        #[cfg(unix)]
        if error.raw_os_error() == Some(libc::ELOOP) {
            return changed(path);
        }
        JobError::io("open mesh source", path, error)
    })
}
fn identity(file: &File, path: &Path) -> Result<Handle> {
    let clone = file
        .try_clone()
        .map_err(|error| JobError::io("clone mesh source handle", path, error))?;
    Handle::from_file(clone)
        .map_err(|error| JobError::io("inspect mesh source identity", path, error))
}
fn same_metadata(left: &Metadata, right: &Metadata) -> bool {
    let same = left.is_file()
        && right.is_file()
        && left.len() == right.len()
        && left.modified().ok() == right.modified().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same && left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        same
    }
}
fn metadata(path: &Path) -> Result<Metadata> {
    fs::symlink_metadata(path)
        .map_err(|error| JobError::io("inspect mesh source entry", path, error))
}

fn canonical_path(path: &Path, recheck: bool) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|error| {
        if recheck
            && matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            )
        {
            changed(path)
        } else {
            JobError::io("resolve mesh source path", path, error)
        }
    })
}

fn dependency_path(base: &Path, relative: &Path, recheck: bool) -> Result<(PathBuf, Metadata)> {
    let mut path = base.to_owned();
    let count = relative.components().count();
    let mut leaf = None;
    for (index, component) in relative.components().enumerate() {
        path.push(component.as_os_str());
        let entry = if recheck {
            metadata_again(&path)?
        } else {
            metadata(&path)?
        };
        if entry.file_type().is_symlink() {
            if recheck {
                return Err(changed(&path));
            }
            return Err(unsupported(format!(
                "mesh resource symlink is outside profile: {}",
                path.display()
            )));
        }
        if index + 1 < count {
            if !entry.is_dir() {
                if recheck {
                    return Err(changed(&path));
                }
                return Err(invalid(
                    "mesh resource intermediate component must be a directory",
                ));
            }
        } else if !entry.is_file() {
            if recheck {
                return Err(changed(&path));
            }
            return Err(invalid("mesh resource must be a regular file"));
        } else {
            leaf = Some(entry);
        }
    }
    let canonical = canonical_path(&path, recheck)?;
    if !canonical.starts_with(base) {
        if recheck {
            return Err(changed(&path));
        }
        return Err(unsupported(
            "mesh resource resolves outside the document directory",
        ));
    }
    Ok((canonical, leaf.expect("nonempty normalized URI")))
}

struct BoundPath {
    path: PathBuf,
    canonical: PathBuf,
    // Some for dependencies, so the final sweep repeats component checks.
    relative: Option<PathBuf>,
    capture: usize,
}
struct Captured {
    identity: Handle,
    metadata: Metadata,
    bytes: Arc<Vec<u8>>,
}
struct Output {
    path: PathBuf,
    identity: Option<Handle>,
}
impl Output {
    fn bind(path: &Path) -> Result<Self> {
        // Some platforms report a missing descendant of a regular file as
        // NotFound rather than NotADirectory. Inspect the nearest existing
        // parent so those requests receive the same classification everywhere.
        let absolute = std::path::absolute(path)
            .map_err(|error| JobError::io("resolve absolute mesh output", path, error))?;
        let mut parent = absolute.parent().ok_or_else(|| {
            JobError::new(
                JobErrorKind::InvalidRequest,
                "mesh output needs a parent directory",
            )
        })?;
        loop {
            match fs::metadata(parent) {
                Ok(entry) if entry.is_dir() => break,
                Ok(_) => {
                    return Err(JobError::new(
                        JobErrorKind::InvalidRequest,
                        "mesh output parent must be a directory",
                    ))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    parent = parent
                        .parent()
                        .ok_or_else(|| JobError::io("inspect mesh output parent", parent, error))?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotADirectory => {
                    return Err(JobError::new(
                        JobErrorKind::InvalidRequest,
                        "mesh output parent must be a directory",
                    ));
                }
                Err(error) => {
                    return Err(JobError::io("inspect mesh output parent", parent, error))
                }
            }
        }
        let existing = match fs::symlink_metadata(path) {
            Ok(entry) => Some(entry),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) if error.kind() == io::ErrorKind::NotADirectory => {
                return Err(JobError::new(
                    JobErrorKind::InvalidRequest,
                    "mesh output parent must be a directory",
                ));
            }
            Err(error) => return Err(JobError::io("inspect mesh output", path, error)),
        };
        if existing.as_ref().is_some_and(|entry| !entry.is_file()) {
            return Err(invalid(
                "mesh output must be a regular file without symlinks",
            ));
        }
        let resolved = crate::output_path::resolve(path)?;
        let output_identity = if let Some(entry) = existing {
            let file = open_file(&resolved)?;
            let opened = file
                .metadata()
                .map_err(|error| JobError::io("inspect mesh output handle", &resolved, error))?;
            if !same_metadata(&entry, &opened) {
                return Err(invalid("mesh output changed during binding"));
            }
            Some(identity(&file, &resolved)?)
        } else {
            None
        };
        Ok(Self {
            path: resolved,
            identity: output_identity,
        })
    }
    fn exclude(&self, path: &Path, source: &Handle) -> Result<()> {
        if self.path == path
            || self.path.starts_with(path)
            || self
                .identity
                .as_ref()
                .is_some_and(|output| output == source)
        {
            return Err(JobError::new(
                JobErrorKind::InvalidRequest,
                "mesh output overlaps or aliases a source file",
            ));
        }
        Ok(())
    }
}

fn read_bounded(
    file: &mut File,
    path: &Path,
    before: &Metadata,
    root: bool,
    remaining: u64,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<u8>> {
    let mut prefix = [0; 4];
    let mut prefix_len = 0;
    if root {
        while prefix_len < prefix.len() {
            check()?;
            let read = file
                .read(&mut prefix[prefix_len..])
                .map_err(|error| JobError::io("read mesh source prefix", path, error))?;
            if read == 0 {
                break;
            }
            prefix_len += read;
        }
    }
    let ceiling = if root && (prefix_len < 4 || &prefix != b"glTF") {
        MAX_JSON_BYTES
    } else {
        MAX_FILE_BYTES
    };
    let ceiling = ceiling.min(remaining);
    if before.len() > ceiling {
        return Err(unsupported(
            "mesh source exceeds file or aggregate byte ceiling",
        ));
    }
    if prefix_len as u64 > before.len() {
        return Err(changed(path));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(before.len() as usize)
        .map_err(|_| unsupported("bounded mesh source allocation unavailable"))?;
    bytes.extend_from_slice(&prefix[..prefix_len]);
    let mut portion = [0; 64 * 1024];
    loop {
        check()?;
        let allowed = (before.len() + 1 - bytes.len() as u64).min(portion.len() as u64) as usize;
        let read = file
            .read(&mut portion[..allowed])
            .map_err(|error| JobError::io("read mesh source bytes", path, error))?;
        if read == 0 {
            break;
        }
        if bytes.len() as u64 + read as u64 > before.len() {
            return Err(changed(path));
        }
        bytes
            .try_reserve_exact(read)
            .map_err(|_| unsupported("bounded mesh source allocation unavailable"))?;
        bytes.extend_from_slice(&portion[..read]);
    }
    let after = file
        .metadata()
        .map_err(|error| JobError::io("inspect captured mesh source", path, error))?;
    if bytes.len() as u64 != before.len() || !same_metadata(before, &after) {
        return Err(changed(path));
    }
    check()?;
    Ok(bytes)
}

fn metadata_again(path: &Path) -> Result<Metadata> {
    fs::symlink_metadata(path).map_err(|error| {
        if matches!(
            error.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
        ) {
            changed(path)
        } else {
            JobError::io("reinspect mesh source entry", path, error)
        }
    })
}

fn verify_path(path: &BoundPath, base: &Path, capture: &Captured) -> Result<()> {
    let (canonical, current) = if let Some(relative) = &path.relative {
        dependency_path(base, relative, true)?
    } else {
        let current = metadata_again(&path.path)?;
        if !current.is_file() {
            return Err(changed(&path.path));
        }
        let canonical = canonical_path(&path.path, true)?;
        (canonical, current)
    };
    if canonical != path.canonical || !same_metadata(&capture.metadata, &current) {
        return Err(changed(&path.path));
    }
    let file = open_file_with(&canonical, true)?;
    let opened = file
        .metadata()
        .map_err(|error| JobError::io("reinspect mesh source handle", &canonical, error))?;
    if !same_metadata(&capture.metadata, &opened)
        || identity(&file, &canonical)? != capture.identity
    {
        return Err(changed(&path.path));
    }
    let held = capture
        .identity
        .as_file()
        .metadata()
        .map_err(|error| JobError::io("reinspect held mesh source", &canonical, error))?;
    if !same_metadata(&capture.metadata, &held) {
        return Err(changed(&path.path));
    }
    Ok(())
}

pub(super) fn load(
    input: &Path,
    output: &Path,
    mut check: impl FnMut() -> Result<()>,
) -> Result<Snapshot> {
    check()?;
    let original = std::path::absolute(input)
        .map_err(|error| JobError::io("resolve absolute mesh source", input, error))?;
    let admitted = metadata(&original)?;
    if !admitted.is_file() {
        return Err(invalid(
            "mesh source must be a regular file without a symlink leaf",
        ));
    }
    if admitted.len() > MAX_FILE_BYTES {
        return Err(unsupported("mesh source exceeds 32 MiB file ceiling"));
    }
    let input = canonical_path(&original, false)?;
    let base = input
        .parent()
        .expect("canonical source file has parent")
        .to_owned();
    let output = Output::bind(output)?;
    let mut file = open_file(&input)?;
    let before = file
        .metadata()
        .map_err(|error| JobError::io("inspect mesh source handle", &input, error))?;
    if !same_metadata(&admitted, &before) {
        return Err(changed(&original));
    }
    let root_identity = identity(&file, &input)?;
    output.exclude(&input, &root_identity)?;
    let root_path = BoundPath {
        path: original,
        canonical: input.clone(),
        relative: None,
        capture: 0,
    };
    // Check the pathname against the opened object before reading, and retain
    // that identity until the entire dependency set has been captured.
    let mut captures = vec![Captured {
        identity: root_identity,
        metadata: before,
        bytes: Arc::new(Vec::new()),
    }];
    verify_path(&root_path, &base, &captures[0])?;
    let root = Arc::new(read_bounded(
        &mut file,
        &input,
        &captures[0].metadata,
        true,
        MAX_TOTAL_BYTES,
        &mut check,
    )?);
    captures[0].bytes = root.clone();
    drop(file);
    verify_path(&root_path, &base, &captures[0])?;
    let document = source::Document::parse(&root, &mut check)?;
    let requests = document.resources();
    if requests.len() > MAX_REQUESTS {
        return Err(unsupported("mesh dependency requests exceed 64"));
    }
    // Complete syntax/escape admission before opening the first dependency.
    let mut relatives = Vec::with_capacity(requests.len());
    for request in requests {
        check()?;
        relatives.push(relative_uri(&request.uri)?);
    }
    let mut paths = vec![root_path];
    let mut resources = Vec::with_capacity(relatives.len());
    let mut unique_bytes = root.len() as u64;
    let mut external_files = 0;
    let mut external_bytes = 0;
    for relative in relatives {
        check()?;
        let (path, admitted) = dependency_path(&base, &relative, false)?;
        if admitted.len() > MAX_FILE_BYTES {
            return Err(unsupported("mesh resource exceeds 32 MiB file ceiling"));
        }
        let mut file = open_file(&path)?;
        let before = file
            .metadata()
            .map_err(|error| JobError::io("inspect mesh resource handle", &path, error))?;
        if !same_metadata(&admitted, &before) {
            return Err(changed(&path));
        }
        let opened_identity = identity(&file, &path)?;
        output.exclude(&path, &opened_identity)?;
        let existing = captures
            .iter()
            .position(|capture| capture.identity == opened_identity);
        if existing == Some(0) {
            return Err(unsupported("mesh resource aliases its source document"));
        }
        let index = if let Some(index) = existing {
            if !same_metadata(&captures[index].metadata, &before) {
                return Err(changed(&path));
            }
            index
        } else {
            if captures.len() > MAX_REQUESTS {
                return Err(unsupported("mesh unique source files exceed 65"));
            }
            if before.len() > MAX_TOTAL_BYTES - unique_bytes {
                return Err(unsupported("mesh captured inputs exceed 64 MiB"));
            }
            let index = captures.len();
            captures.push(Captured {
                identity: opened_identity,
                metadata: before,
                bytes: Arc::new(Vec::new()),
            });
            let bound = BoundPath {
                path: base.join(&relative),
                canonical: path.clone(),
                relative: Some(relative.clone()),
                capture: index,
            };
            verify_path(&bound, &base, &captures[index])?;
            let bytes = Arc::new(read_bounded(
                &mut file,
                &path,
                &captures[index].metadata,
                false,
                MAX_TOTAL_BYTES - unique_bytes,
                &mut check,
            )?);
            unique_bytes += bytes.len() as u64;
            external_bytes += bytes.len() as u64;
            external_files += 1;
            captures[index].bytes = bytes;
            index
        };
        let bound = BoundPath {
            path: base.join(&relative),
            canonical: path,
            relative: Some(relative),
            capture: index,
        };
        verify_path(&bound, &base, &captures[index])?;
        resources.push(captures[index].bytes.clone());
        paths.push(bound);
    }
    // Include every alias spelling in the final consistency sweep. Handles
    // remain alive throughout comparisons, then close before any callback.
    for path in &paths {
        check()?;
        verify_path(path, &base, &captures[path.capture])?;
    }
    drop(captures);
    check()?;
    Ok(Snapshot {
        output: output.path,
        document,
        source_bytes: root.len() as u64,
        root,
        resources,
        external_files,
        external_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document(uris: &[&str]) -> Vec<u8> {
        let buffers: Vec<_> = uris
            .iter()
            .map(|uri| json!({"byteLength":36,"uri":uri}))
            .collect();
        serde_json::to_vec(&json!({
            "asset":{"version":"2.0"},"buffers":buffers,
            "bufferViews":[{"buffer":0,"byteLength":36,"target":34962}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
            "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0
        })).unwrap()
    }

    fn setup(uris: &[&str]) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input.gltf");
        let output = directory.path().join("output.3tz");
        fs::write(&input, document(uris)).unwrap();
        (directory, input, output)
    }

    #[test]
    fn every_uri_is_admitted_before_missing_dependencies_are_opened() {
        let (_directory, input, output) =
            setup(&["missing.bin", "https://example.invalid/blocked.bin"]);
        let error = load(&input, &output, || Ok(())).err().unwrap();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
        assert!(!output.exists());
    }

    #[test]
    fn terminal_dot_components_cannot_be_normalized_into_file_names() {
        for uri in ["data.bin/.", "folder/..", "data.bin/%2e", "folder/%2e%2e"] {
            let error = relative_uri(uri).unwrap_err();
            assert_eq!(error.kind(), JobErrorKind::InvalidInput, "{uri}");
        }
        assert_eq!(
            relative_uri("./folder/../data.bin").unwrap(),
            PathBuf::from("data.bin")
        );
    }

    #[test]
    fn reserved_filename_characters_require_one_percent_decoding_pass() {
        for uri in [
            "a!b.bin", "a$b.bin", "a&b.bin", "a'b.bin", "a(b.bin", "a)b.bin", "a*b.bin", "a+b.bin",
            "a,b.bin", "a;b.bin", "a=b.bin", "a@b.bin",
        ] {
            let error = relative_uri(uri).unwrap_err();
            assert_eq!(error.kind(), JobErrorKind::InvalidInput, "{uri}");
        }
        assert_eq!(relative_uri("a%2Bb.bin").unwrap(), PathBuf::from("a+b.bin"));
        assert_eq!(
            relative_uri("a%252fb.bin").unwrap(),
            PathBuf::from("a%2fb.bin")
        );
        assert_eq!(relative_uri("a%23b.bin").unwrap(), PathBuf::from("a#b.bin"));
        assert_eq!(
            relative_uri("a%2fb.bin").unwrap_err().kind(),
            JobErrorKind::Unsupported
        );
    }

    #[test]
    fn encoded_aliases_capture_one_immutable_resource() {
        let (directory, input, output) = setup(&["data.bin", "%64ata.bin"]);
        fs::write(directory.path().join("data.bin"), [0; 36]).unwrap();
        let snapshot = load(&input, &output, || Ok(())).unwrap();
        assert_eq!(snapshot.external_files, 1);
        assert_eq!(snapshot.external_bytes, 36);
        assert!(Arc::ptr_eq(&snapshot.resources[0], &snapshot.resources[1]));
        fs::write(directory.path().join("data.bin"), [1; 36]).unwrap();
        assert_eq!(snapshot.resource(0), [0; 36]);
        assert_eq!(snapshot.resource(1), [0; 36]);
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_deduplicate_and_unused_resource_output_aliases_are_rejected() {
        let (directory, input, output) = setup(&["data.bin", "alias.bin"]);
        fs::write(directory.path().join("data.bin"), [0; 36]).unwrap();
        fs::hard_link(
            directory.path().join("data.bin"),
            directory.path().join("alias.bin"),
        )
        .unwrap();
        let snapshot = load(&input, &output, || Ok(())).unwrap();
        assert_eq!((snapshot.external_files, snapshot.external_bytes), (1, 36));
        fs::hard_link(directory.path().join("alias.bin"), &output).unwrap();
        let error = load(&input, &output, || Ok(())).err().unwrap();
        assert_eq!(error.kind(), JobErrorKind::InvalidRequest);
        assert_eq!(fs::read(output).unwrap(), [0; 36]);
    }

    #[test]
    fn output_descendants_of_regular_source_files_are_invalid_requests() {
        let (directory, input, _) = setup(&["data.bin"]);
        let dependency = directory.path().join("data.bin");
        fs::write(&dependency, [0; 36]).unwrap();
        let original = fs::read(&input).unwrap();
        let unrelated = directory.path().join("unrelated-file");
        fs::write(&unrelated, b"preserve").unwrap();
        for output in [
            input.join("out.3tz"),
            dependency.join("out.3tz"),
            unrelated.join("nested/out.3tz"),
        ] {
            let error = load(&input, &output, || Ok(())).err().unwrap();
            assert_eq!(error.kind(), JobErrorKind::InvalidRequest);
            assert!(!output.exists());
            assert_eq!(fs::read(&input).unwrap(), original);
            assert_eq!(fs::read(&dependency).unwrap(), [0; 36]);
            assert_eq!(fs::read(&unrelated).unwrap(), b"preserve");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_fifo_resources_are_refused_without_reading_them() {
        use std::{
            ffi::CString,
            os::unix::{ffi::OsStrExt, fs::symlink},
        };
        let (directory, input, output) = setup(&["link.bin"]);
        fs::write(directory.path().join("real.bin"), [0; 36]).unwrap();
        symlink("real.bin", directory.path().join("link.bin")).unwrap();
        let error = load(&input, &output, || Ok(())).err().unwrap();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
        fs::create_dir(directory.path().join("real-dir")).unwrap();
        fs::write(directory.path().join("real-dir/data.bin"), [0; 36]).unwrap();
        symlink("real-dir", directory.path().join("link-dir")).unwrap();
        fs::write(&input, document(&["link-dir/data.bin"])).unwrap();
        let error = load(&input, &output, || Ok(())).err().unwrap();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
        let fifo = directory.path().join("pipe.bin");
        let native = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // The FIFO has no writer. An accidental blocking open would hang.
        assert_eq!(unsafe { libc::mkfifo(native.as_ptr(), 0o600) }, 0);
        fs::write(&input, document(&["pipe.bin"])).unwrap();
        let error = load(&input, &output, || Ok(())).err().unwrap();
        assert_eq!(error.kind(), JobErrorKind::InvalidInput);
        assert!(!output.exists());
    }

    // Linux receipts can observe actual held descriptors without coupling
    // mutation timing to an implementation's checkpoint count. Mutations fire
    // when a later dependency is open and earlier snapshots already exist.
    #[cfg(target_os = "linux")]
    fn is_open(path: &Path) -> bool {
        fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| fs::read_link(entry.path()).ok())
            .any(|target| target == path)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn root_growth_after_admission_is_invalid_input() {
        let (directory, input, output) = setup(&["data.bin"]);
        fs::write(directory.path().join("data.bin"), [0; 36]).unwrap();
        fs::write(&output, b"prior").unwrap();
        let mut grew = false;
        let error = load(&input, &output, || {
            if !grew && is_open(&input) {
                OpenOptions::new()
                    .write(true)
                    .open(&input)
                    .unwrap()
                    .set_len(MAX_JSON_BYTES + 1)
                    .unwrap();
                grew = true;
            }
            Ok(())
        })
        .err()
        .unwrap();
        assert!(grew, "growth must occur after the root handle is admitted");
        assert_eq!(error.kind(), JobErrorKind::InvalidInput);
        assert_eq!(fs::read(&output).unwrap(), b"prior");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dependency_growth_after_admission_is_invalid_input() {
        let (directory, input, output) = setup(&["data.bin"]);
        let dependency = directory.path().join("data.bin");
        fs::write(&dependency, [0; 36]).unwrap();
        fs::write(&output, b"prior").unwrap();
        let mut grew = false;
        let error = load(&input, &output, || {
            if !grew && is_open(&dependency) {
                OpenOptions::new()
                    .write(true)
                    .open(&dependency)
                    .unwrap()
                    .set_len(MAX_FILE_BYTES + 1)
                    .unwrap();
                grew = true;
            }
            Ok(())
        })
        .err()
        .unwrap();
        assert!(
            grew,
            "growth must occur after the dependency handle is admitted"
        );
        assert_eq!(error.kind(), JobErrorKind::InvalidInput);
        assert_eq!(fs::read(&output).unwrap(), b"prior");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn zero_and_short_root_prefix_growth_is_detected_before_reader_arithmetic() {
        for initial_length in 0..4 {
            let (directory, input, output) = setup(&["data.bin"]);
            fs::write(directory.path().join("data.bin"), [0; 36]).unwrap();
            let complete = fs::read(&input).unwrap();
            fs::write(&input, &complete[..initial_length]).unwrap();
            let mut grew = false;
            let error = load(&input, &output, || {
                if !grew && is_open(&input) {
                    fs::write(&input, &complete).unwrap();
                    grew = true;
                }
                Ok(())
            })
            .err()
            .unwrap();
            assert!(
                grew,
                "prefix growth must execute for length {initial_length}"
            );
            assert_eq!(error.kind(), JobErrorKind::InvalidInput);
            assert!(error.message().contains("source changed during capture"));
            assert!(!output.exists());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn final_sweep_detects_earlier_resource_changes_during_later_capture() {
        let (directory, input, output) = setup(&["early.bin", "later.bin"]);
        let early = directory.path().join("early.bin");
        let later = directory.path().join("later.bin");
        fs::write(&early, [0; 36]).unwrap();
        fs::write(&later, [0; 36]).unwrap();
        fs::write(&output, b"prior").unwrap();
        let mut mutated = false;
        let error = load(&input, &output, || {
            if !mutated && is_open(&later) {
                fs::write(&early, [7; 37]).unwrap();
                mutated = true;
            }
            Ok(())
        })
        .err()
        .unwrap();
        assert!(mutated, "mutation must execute during capture");
        assert_eq!(error.kind(), JobErrorKind::InvalidInput);
        assert_eq!(fs::read(&output).unwrap(), b"prior");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn final_sweep_detects_root_pathname_rebinding() {
        let (directory, input, output) = setup(&["data.bin"]);
        let dependency = directory.path().join("data.bin");
        fs::write(&dependency, [0; 36]).unwrap();
        let original = fs::read(&input).unwrap();
        let mut mutated = false;
        let error = load(&input, &output, || {
            if !mutated && is_open(&dependency) {
                fs::rename(&input, directory.path().join("held-original.gltf")).unwrap();
                fs::write(&input, &original).unwrap();
                mutated = true;
            }
            Ok(())
        })
        .err()
        .unwrap();
        assert!(mutated);
        assert_eq!(error.kind(), JobErrorKind::InvalidInput);
        assert!(!output.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn final_sweep_checks_each_alias_pathname_against_its_capture() {
        let (directory, input, output) = setup(&["early.bin", "alias.bin", "later.bin"]);
        let early = directory.path().join("early.bin");
        let alias = directory.path().join("alias.bin");
        let later = directory.path().join("later.bin");
        fs::write(&early, [0; 36]).unwrap();
        fs::hard_link(&early, &alias).unwrap();
        fs::write(&later, [0; 36]).unwrap();
        let mut mutated = false;
        let error = load(&input, &output, || {
            if !mutated && is_open(&later) {
                fs::remove_file(&alias).unwrap();
                fs::write(&alias, [0; 36]).unwrap();
                mutated = true;
            }
            Ok(())
        })
        .err()
        .unwrap();
        assert!(mutated);
        assert_eq!(error.kind(), JobErrorKind::InvalidInput);
        assert!(!output.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cancellation_error_precedes_mutation_diagnostics() {
        let (directory, input, output) = setup(&["data.bin"]);
        let dependency = directory.path().join("data.bin");
        fs::write(&dependency, [0; 36]).unwrap();
        fs::write(&output, b"prior").unwrap();
        let mut cancelled = false;
        let error = load(&input, &output, || {
            if is_open(&dependency) {
                fs::remove_file(&input).unwrap();
                cancelled = true;
                return Err(JobError::new(
                    JobErrorKind::Cancelled,
                    "capture cancellation",
                ));
            }
            Ok(())
        })
        .err()
        .unwrap();
        assert!(cancelled);
        assert_eq!(error.kind(), JobErrorKind::Cancelled);
        assert_eq!(error.message(), "capture cancellation");
        assert_eq!(fs::read(&output).unwrap(), b"prior");
    }
}
