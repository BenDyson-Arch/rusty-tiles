//! Captured regular-file identity. Namespace races after recheck are outside Replace's domain.
use super::{checked_sum, resource, CHUNK_BYTES};
use crate::{runtime::Attempt, JobError, JobErrorKind};
use same_file::Handle;
use std::{
    fs::{self, Metadata, OpenOptions, Permissions},
    io::{self, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};

pub(super) struct Captured {
    pub(super) spelling: PathBuf,
    pub(super) canonical: PathBuf,
    pub(super) parent: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) peak_bytes: usize,
    held: Handle,
    snapshot: Snapshot,
}

struct Snapshot {
    length: u64,
    modified: SystemTime,
    permissions: Permissions,
}

impl Snapshot {
    fn new(metadata: &Metadata, path: &Path) -> Result<Self, JobError> {
        if !metadata.is_file() {
            return Err(changed(path));
        }
        Ok(Self {
            length: metadata.len(),
            modified: metadata
                .modified()
                .map_err(|e| JobError::io("inspect source modification time", path, e))?,
            permissions: metadata.permissions(),
        })
    }
    fn matches(&self, metadata: &Metadata, path: &Path) -> Result<bool, JobError> {
        Ok(metadata.is_file()
            && metadata.len() == self.length
            && metadata
                .modified()
                .map_err(|e| JobError::io("inspect source modification time", path, e))?
                == self.modified
            && same_permissions(&metadata.permissions(), &self.permissions))
    }
}

impl Captured {
    pub(super) fn capture(
        input: PathBuf,
        source_limit: usize,
        working_limit: usize,
        fixed_storage: usize,
        attempt: &Attempt,
    ) -> Result<Self, JobError> {
        attempt.check()?;
        let mut peak_bytes = checked_sum(&[input.capacity(), fixed_storage])?;
        if peak_bytes > working_limit {
            return Err(resource("request path capacity exceeds working byte limit"));
        }
        let spelling = std::path::absolute(&input)
            .map_err(|e| JobError::io("resolve source spelling", &input, e))?;
        peak_bytes = peak_bytes.max(checked_sum(&[
            input.capacity(),
            spelling.capacity(),
            fixed_storage,
        ])?);
        if peak_bytes > working_limit {
            return Err(resource(
                "absolute source spelling exceeds working byte limit",
            ));
        }
        drop(input);
        let leaf = fs::symlink_metadata(&spelling)
            .map_err(|e| JobError::io("inspect source leaf", &spelling, e))?;
        if !leaf.is_file() || leaf.file_type().is_symlink() {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                "vector compression requires a regular non-symlink leaf",
            ));
        }
        let canonical = fs::canonicalize(&spelling)
            .map_err(|e| JobError::io("resolve source", &spelling, e))?;
        let parent = canonical
            .parent()
            .ok_or_else(|| {
                JobError::new(
                    JobErrorKind::InvalidRequest,
                    "source has no parent directory",
                )
            })?
            .to_path_buf();
        let held = open(&spelling, false)?;
        let metadata = held
            .as_file()
            .metadata()
            .map_err(|e| JobError::io("inspect opened source", &spelling, e))?;
        let snapshot = Snapshot::new(&metadata, &spelling)?;
        #[cfg(windows)]
        if snapshot.permissions.readonly() {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                "readonly Windows sources cannot be replaced",
            ));
        }
        let length = usize::try_from(snapshot.length)
            .map_err(|_| resource("source length exceeds host addressing"))?;
        if length > source_limit || length > isize::MAX as usize {
            return Err(resource("source byte limit exceeded"));
        }
        let paths = checked_sum(&[
            spelling.capacity(),
            canonical.capacity(),
            canonical.capacity(),
            parent.capacity(),
        ])?;
        // Reserve the later runtime name/path construction phase, including
        // old/new allocation overlap; the retained TempPath is only boxed bytes.
        let stage_path = staging_storage(&parent)?;
        let capture_work = checked_sum(&[length, paths, stage_path, fixed_storage])?;
        if capture_work > working_limit {
            return Err(resource("source capture working byte limit exceeded"));
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|e| {
            JobError::with_cause(JobErrorKind::ResourceLimit, "reserve captured source", e)
        })?;
        // The allocator's returned capacity, rather than just the requested length,
        // enters the later shared codec ledger.
        peak_bytes = peak_bytes.max(checked_sum(&[
            bytes.capacity(),
            paths,
            stage_path,
            fixed_storage,
        ])?);
        if peak_bytes > working_limit {
            return Err(resource(
                "source capture capacity exceeds working byte limit",
            ));
        }
        bytes.resize(length, 0);
        let mut captured = Self {
            spelling,
            canonical,
            parent,
            bytes,
            peak_bytes,
            held,
            snapshot,
        };
        captured.check_path()?;
        let mut reader = captured.held.as_file();
        let mut offset = 0;
        while offset < length {
            attempt.check()?;
            let end = offset.saturating_add(CHUNK_BYTES).min(length);
            match reader.read(&mut captured.bytes[offset..end]) {
                Ok(0) => return Err(changed(&captured.spelling)),
                Ok(count) => offset += count,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(JobError::io("capture source bytes", &captured.spelling, e)),
            }
        }
        check_eof(&mut reader, &captured.spelling, attempt)?;
        captured.check_path()?;
        attempt.check()?;
        Ok(captured)
    }

    pub(super) fn persistent_storage(&self) -> Result<usize, JobError> {
        checked_sum(&[
            self.bytes.capacity(),
            self.spelling.capacity(),
            self.canonical.capacity(),
            self.canonical.capacity(),
            self.parent.capacity(),
            staging_storage(&self.parent)?,
        ])
    }

    fn check_path(&self) -> Result<Handle, JobError> {
        let leaf = inspect_leaf(&self.spelling)?;
        if !leaf.is_file()
            || leaf.file_type().is_symlink()
            || !self.snapshot.matches(&leaf, &self.spelling)?
        {
            return Err(changed(&self.spelling));
        }
        let canonical = resolve_again(&self.spelling)?;
        if canonical != self.canonical || canonical.parent() != Some(self.parent.as_path()) {
            return Err(changed(&self.spelling));
        }
        if canonical.capacity() > self.canonical.capacity() {
            return Err(resource("recheck path capacity exceeds admitted storage"));
        }
        let current = open(&self.spelling, true)?;
        if current != self.held {
            return Err(changed(&self.spelling));
        }
        let metadata = current
            .as_file()
            .metadata()
            .map_err(|e| JobError::io("inspect selected source", &self.spelling, e))?;
        let held_metadata = self
            .held
            .as_file()
            .metadata()
            .map_err(|e| JobError::io("inspect held source", &self.spelling, e))?;
        if !self.snapshot.matches(&metadata, &self.spelling)?
            || !self.snapshot.matches(&held_metadata, &self.spelling)?
        {
            return Err(changed(&self.spelling));
        }
        Ok(current)
    }

    pub(super) fn verify(
        &self,
        chunk: &mut [u8; CHUNK_BYTES],
        attempt: &Attempt,
    ) -> Result<(), JobError> {
        attempt.check()?;
        let current = self.check_path()?;
        let mut reader = current.as_file();
        let mut offset = 0;
        while offset < self.bytes.len() {
            attempt.check()?;
            let wanted = (self.bytes.len() - offset).min(chunk.len());
            match reader.read(&mut chunk[..wanted]) {
                Ok(0) => return Err(changed(&self.spelling)),
                Ok(count) => {
                    if chunk[..count] != self.bytes[offset..offset + count] {
                        return Err(changed(&self.spelling));
                    }
                    offset += count;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    return Err(JobError::io(
                        "compare selected source bytes",
                        &self.spelling,
                        e,
                    ))
                }
            }
        }
        check_eof(&mut reader, &self.spelling, attempt)?;
        // Reopen the original spelling after the comparison, so a path replacement
        // during the read cannot pass merely because the first reader stayed valid.
        self.check_path()?;
        let metadata = current
            .as_file()
            .metadata()
            .map_err(|e| JobError::io("inspect compared source", &self.spelling, e))?;
        if !self.snapshot.matches(&metadata, &self.spelling)? {
            return Err(changed(&self.spelling));
        }
        attempt.check()
    }

    /// Moving the fixed output spelling out drops the source bytes and held
    /// handle before the caller can grant publication permission.
    pub(super) fn into_output(self) -> PathBuf {
        self.canonical
    }

    pub(super) fn candidate_permissions(&self) -> Permissions {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Permissions::from_mode(self.snapshot.permissions.mode() & 0o777)
        }
        #[cfg(not(unix))]
        {
            self.snapshot.permissions.clone()
        }
    }
}

fn same_permissions(left: &Permissions, right: &Permissions) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        left.mode() == right.mode()
    }
    #[cfg(not(unix))]
    {
        left.readonly() == right.readonly()
    }
}

fn open(path: &Path, recheck: bool) -> Result<Handle, JobError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|e| changed_or_io("open selected source", path, e, recheck))?;
    Handle::from_file(file).map_err(|e| JobError::io("inspect source identity", path, e))
}
fn inspect_leaf(path: &Path) -> Result<Metadata, JobError> {
    fs::symlink_metadata(path).map_err(|e| changed_or_io("recheck source leaf", path, e, true))
}
fn resolve_again(path: &Path) -> Result<PathBuf, JobError> {
    fs::canonicalize(path).map_err(|e| changed_or_io("recheck source path", path, e, true))
}
fn changed_or_io(operation: &str, path: &Path, error: io::Error, recheck: bool) -> JobError {
    let missing = matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    );
    #[cfg(unix)]
    let symlink = error.raw_os_error() == Some(libc::ELOOP);
    #[cfg(not(unix))]
    let symlink = false;
    if recheck && (missing || symlink) {
        changed(path)
    } else {
        JobError::io(operation, path, error)
    }
}
fn check_eof(reader: &mut &std::fs::File, path: &Path, attempt: &Attempt) -> Result<(), JobError> {
    let mut excess = [0u8; 1];
    loop {
        attempt.check()?;
        match reader.read(&mut excess) {
            Ok(0) => return Ok(()),
            Ok(_) => return Err(changed(path)),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(JobError::io("check source extent", path, e)),
        }
    }
}
fn changed(path: &Path) -> JobError {
    // Conflict is a domain event, rather than a fabricated OS error.
    JobError::new(
        JobErrorKind::Conflict,
        format!("selected source changed: {}", path.display()),
    )
}

/// Source-backed conservative staging-construction reserve for the matched
/// tempfile/std algorithms. Pinned Builder name is 13 + 6 bytes; one separator
/// makes P+20 an upper byte length. Windows verbatim join collects components;
/// its old/new Vec capacities are covered by six times max(N+1,4). Four path
/// lengths cover the original clone and old/new rebuilt-string/shrink overlap.
/// This counts requested Rust storage, excluding OS/allocator bookkeeping.
pub(super) fn staging_storage(parent: &Path) -> Result<usize, JobError> {
    let path_bytes = checked_sum(&[parent.as_os_str().as_encoded_bytes().len(), 20])?;
    let components = parent
        .components()
        .count()
        .checked_add(1)
        .ok_or_else(|| resource("staging component count overflow"))?
        .max(4);
    let component_bytes = components
        .checked_mul(std::mem::size_of::<std::path::Component<'_>>())
        .ok_or_else(|| resource("staging component storage overflow"))?;
    checked_sum(&[
        path_bytes,
        path_bytes,
        path_bytes,
        path_bytes,
        component_bytes,
        component_bytes,
        component_bytes,
        component_bytes,
        component_bytes,
        component_bytes,
        19,
    ])
}
