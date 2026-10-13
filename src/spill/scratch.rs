//! The owned scratch directory, its file I/O seam and disk accounting.
use super::{volume, PreflightStage, SpaceRole, SpillError, Volume};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};

/// File creation and opening for spill runs. Production uses [`FileIo`]; tests inject faults.
pub(crate) trait SpillIo: Send + Sync {
    fn create(&self, path: &Path) -> io::Result<Box<dyn Write + Send>>;
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>>;
    /// A handle for positioned reads of a completed file (random access to sequences).
    fn open_random(&self, path: &Path) -> io::Result<File> {
        File::open(path)
    }
}

/// Reads exactly `bytes.len()` bytes at `offset` without moving a shared cursor.
pub(crate) fn read_exact_at(file: &File, mut bytes: &mut [u8], mut offset: u64) -> io::Result<()> {
    while !bytes.is_empty() {
        #[cfg(unix)]
        let read = std::os::unix::fs::FileExt::read_at(file, bytes, offset);
        #[cfg(windows)]
        let read = std::os::windows::fs::FileExt::seek_read(file, bytes, offset);
        match read {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Ok(count) => {
                bytes = &mut bytes[count..];
                offset += count as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) struct FileIo;

impl SpillIo for FileIo {
    fn create(&self, path: &Path) -> io::Result<Box<dyn Write + Send>> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        Ok(Box::new(file))
    }
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Ok(Box::new(File::open(path)?))
    }
}

#[derive(Default)]
pub(super) struct Accounting {
    live: AtomicU64,
    peak: AtomicU64,
}

impl Accounting {
    fn add(&self, bytes: u64) {
        let live = self.live.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.peak.fetch_max(live, Ordering::Relaxed);
    }
    fn remove(&self, bytes: u64) {
        self.live.fetch_sub(bytes, Ordering::Relaxed);
    }
}

/// A private scratch directory created after a successful free-space preflight. Dropping it
/// removes every file inside; [`Scratch::close`] does the same and reports failure.
///
/// [`Scratch::create`] preflights and creates the directory at once. [`Scratch::deferred`]
/// does both only when the first spill file is requested, so a consumer whose sorts and
/// sequences stay in memory never touches the filesystem.
pub(crate) struct Scratch {
    parent: PathBuf,
    needed: u64,
    inspect: fn(&Path) -> Result<Volume, SpillError>,
    created: OnceLock<Created>,
    io: Arc<dyn SpillIo>,
    accounting: Arc<Accounting>,
    next_file: AtomicU64,
}

struct Created {
    dir: tempfile::TempDir,
    volume: Volume,
}

impl Scratch {
    /// The `--scratch-dir` default: the system temporary directory.
    pub(crate) fn default_parent() -> PathBuf {
        std::env::temp_dir()
    }

    /// Preflights `needed` bytes on `parent` (default: system temp dir), then creates the
    /// scratch directory inside it. Refusal happens before anything is created. An explicit
    /// `parent` (`--scratch-dir`) must already exist; a missing one is a typed `NotFound`.
    /// Scratch is created after the stage-1 (geometry) preflight, so refusals carry that stage.
    pub(crate) fn create(parent: Option<&Path>, needed: u64) -> Result<Self, SpillError> {
        Self::create_with(parent, needed, Arc::new(FileIo), volume::inspect_volume)
    }

    pub(crate) fn create_with(
        parent: Option<&Path>,
        needed: u64,
        io: Arc<dyn SpillIo>,
        inspect: impl FnOnce(&Path) -> Result<Volume, SpillError>,
    ) -> Result<Self, SpillError> {
        let parent = parent.map_or_else(Self::default_parent, Path::to_path_buf);
        let created = Self::materialize(&parent, needed, inspect)?;
        let scratch = Self::deferred_with(Some(&parent), needed, io, volume::inspect_volume);
        let _ = scratch.created.set(created);
        Ok(scratch)
    }

    /// Records `parent` and the free-space estimate `needed` without touching the filesystem.
    /// The preflight and the directory happen at the first spill file, if any.
    pub(crate) fn deferred(parent: Option<&Path>, needed: u64) -> Self {
        Self::deferred_with(parent, needed, Arc::new(FileIo), volume::inspect_volume)
    }

    pub(crate) fn deferred_with(
        parent: Option<&Path>,
        needed: u64,
        io: Arc<dyn SpillIo>,
        inspect: fn(&Path) -> Result<Volume, SpillError>,
    ) -> Self {
        Self {
            parent: parent.map_or_else(Self::default_parent, Path::to_path_buf),
            needed,
            inspect,
            created: OnceLock::new(),
            io,
            accounting: Arc::default(),
            next_file: AtomicU64::new(0),
        }
    }

    fn materialize(
        parent: &Path,
        needed: u64,
        inspect: impl FnOnce(&Path) -> Result<Volume, SpillError>,
    ) -> Result<Created, SpillError> {
        let volume = volume::preflight_with(
            parent,
            needed,
            SpaceRole::Scratch,
            PreflightStage::Geometry,
            inspect,
        )
        .map_err(|error| match error {
            SpillError::Io { path, source, .. }
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                SpillError::io("scratch directory does not exist", path, source)
            }
            other => other,
        })?;
        let dir = tempfile::Builder::new()
            .prefix("rusty-tiles-spill-")
            .tempdir_in(parent)
            .map_err(|e| SpillError::io("create scratch directory", parent, e))?;
        Ok(Created { dir, volume })
    }

    fn created(&self) -> Result<&Created, SpillError> {
        if let Some(created) = self.created.get() {
            return Ok(created);
        }
        let created = Self::materialize(&self.parent, self.needed, self.inspect)?;
        // A concurrent first spill may have won; the loser's directory is removed on drop.
        let _ = self.created.set(created);
        Ok(self.created.get().expect("scratch was just created"))
    }

    /// Whether the scratch directory exists (always, for [`Scratch::create`]).
    pub(crate) fn is_created(&self) -> bool {
        self.created.get().is_some()
    }

    /// The scratch directory, or its parent while a deferred scratch is not yet created.
    pub(crate) fn path(&self) -> &Path {
        self.created
            .get()
            .map_or(self.parent.as_path(), |created| created.dir.path())
    }

    /// True when scratch is RAM (tmpfs/ramfs); reported as `scratch_memory_backed`.
    /// False while a deferred scratch is not yet created.
    pub(crate) fn memory_backed(&self) -> bool {
        self.created
            .get()
            .is_some_and(|created| created.volume.memory_backed)
    }

    /// Free bytes observed by the preflight (0 before a deferred scratch is created).
    pub(crate) fn available_at_start(&self) -> u64 {
        self.created
            .get()
            .map_or(0, |created| created.volume.available_bytes)
    }

    /// Filesystem fragment/cluster size observed by the preflight (0 before creation).
    pub(crate) fn allocation_unit_bytes(&self) -> u64 {
        self.created
            .get()
            .map_or(0, |created| created.volume.allocation_unit_bytes)
    }

    /// Bytes currently held by live spill files.
    pub(crate) fn live_bytes(&self) -> u64 {
        self.accounting.live.load(Ordering::Relaxed)
    }

    /// Largest number of bytes held by live spill files at once.
    pub(crate) fn peak_bytes(&self) -> u64 {
        self.accounting.peak.load(Ordering::Relaxed)
    }

    /// Removes the directory, reporting failure rather than ignoring it as drop does.
    pub(crate) fn close(self) -> Result<(), SpillError> {
        let Some(created) = self.created.into_inner() else {
            return Ok(());
        };
        let path = created.dir.path().to_owned();
        created
            .dir
            .close()
            .map_err(|e| SpillError::io("remove scratch directory", path, e))
    }

    pub(super) fn io(&self) -> &dyn SpillIo {
        &*self.io
    }

    pub(super) fn io_handle(&self) -> Arc<dyn SpillIo> {
        self.io.clone()
    }

    /// A new file name in the scratch directory, creating a deferred scratch first.
    pub(super) fn new_file(&self, stem: &str) -> Result<SpillFile, SpillError> {
        let dir = self.created()?.dir.path();
        let id = self.next_file.fetch_add(1, Ordering::Relaxed);
        Ok(SpillFile {
            path: dir.join(format!("{stem}-{id}.spill")),
            bytes: 0,
            accounting: self.accounting.clone(),
        })
    }
}

/// Ownership of one scratch file: removed (best effort) when dropped. Holders must drop open
/// handles first so removal also succeeds on Windows.
pub(super) struct SpillFile {
    path: PathBuf,
    bytes: u64,
    accounting: Arc<Accounting>,
}

impl SpillFile {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Records the completed size for scratch accounting.
    pub(super) fn completed(&mut self, bytes: u64) {
        debug_assert!(bytes >= self.bytes);
        self.accounting.add(bytes - self.bytes);
        self.bytes = bytes;
    }
}

impl Drop for SpillFile {
    /// Live bytes drop only when the file is gone; a failed removal stays accounted.
    fn drop(&mut self) {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {}
            _ => self.accounting.remove(self.bytes),
        }
    }
}
