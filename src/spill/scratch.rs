//! The owned scratch directory, its file I/O seam and disk accounting.
use super::{volume, PreflightStage, SpaceRole, SpillError, Volume};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

/// File creation and opening for spill runs. Production uses [`FileIo`]; tests inject faults.
pub(crate) trait SpillIo: Send + Sync {
    fn create(&self, path: &Path) -> io::Result<Box<dyn Write + Send>>;
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>>;
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
pub(crate) struct Scratch {
    dir: tempfile::TempDir,
    volume: Volume,
    io: Arc<dyn SpillIo>,
    accounting: Arc<Accounting>,
    next_file: AtomicU64,
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
        let volume = volume::preflight_with(
            &parent,
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
            .tempdir_in(&parent)
            .map_err(|e| SpillError::io("create scratch directory", &parent, e))?;
        Ok(Self {
            dir,
            volume,
            io,
            accounting: Arc::default(),
            next_file: AtomicU64::new(0),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }

    /// True when scratch is RAM (tmpfs/ramfs); reported as `scratch_memory_backed`.
    pub(crate) fn memory_backed(&self) -> bool {
        self.volume.memory_backed
    }

    /// Free bytes observed by the preflight.
    pub(crate) fn available_at_start(&self) -> u64 {
        self.volume.available_bytes
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
        let path = self.dir.path().to_owned();
        self.dir
            .close()
            .map_err(|e| SpillError::io("remove scratch directory", path, e))
    }

    pub(super) fn io(&self) -> &dyn SpillIo {
        &*self.io
    }

    pub(super) fn io_handle(&self) -> Arc<dyn SpillIo> {
        self.io.clone()
    }

    pub(super) fn new_file(&self, stem: &str) -> SpillFile {
        let id = self.next_file.fetch_add(1, Ordering::Relaxed);
        SpillFile {
            path: self.dir.path().join(format!("{stem}-{id}.spill")),
            bytes: 0,
            accounting: self.accounting.clone(),
        }
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
        self.bytes = bytes;
        self.accounting.add(bytes);
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
