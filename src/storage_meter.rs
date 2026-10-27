//! D8 meters for memory-backed (tmpfs) storage a run charged to its memory budget.
//!
//! The run registers one [`Meter`] per charged directory. Every file, link and directory
//! created under it through this module admits its allocation before the bytes land: file
//! data in whole allocation units plus [`ENTRY_BYTES`] for each entry. A write that would
//! pass the charge is refused without writing, and the meter keeps that refusal so the run
//! reports `resource_limit` rather than the I/O error its writer saw. Paths outside every
//! registered directory are not metered and behave as plain `std::fs` calls. Charges are
//! cumulative: removing a file does not return its allocation.
use crate::{JobError, JobErrorKind};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};

/// Allocation charged for each file, link or directory besides its data: a tmpfs inode,
/// its dentry and the parent's directory slot.
pub(crate) const ENTRY_BYTES: u64 = 1024;

/// One charged owner: what it may hold and what it has admitted.
#[derive(Debug)]
pub(crate) struct Meter {
    what: &'static str,
    advice: &'static str,
    limit: u64,
    unit: u64,
    used: AtomicU64,
    refusal: OnceLock<String>,
}

impl Meter {
    /// `limit` bytes for `what`; file data is charged in `unit`-byte allocation units
    /// (at least 4 KiB, the memory page).
    pub(crate) fn new(
        what: &'static str,
        advice: &'static str,
        limit: u64,
        unit: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            what,
            advice,
            limit,
            unit: unit.max(4096),
            used: AtomicU64::new(0),
            refusal: OnceLock::new(),
        })
    }

    pub(crate) fn limit(&self) -> u64 {
        self.limit
    }

    pub(crate) fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    /// The allocation charged for a file of `len` bytes, its entry included.
    pub(crate) fn file_bytes(&self, len: u64) -> u64 {
        len.div_ceil(self.unit)
            .saturating_mul(self.unit)
            .saturating_add(ENTRY_BYTES)
    }

    /// Admit `bytes` more, or refuse without admitting anything.
    pub(crate) fn admit(&self, bytes: u64) -> io::Result<()> {
        let mut used = self.used.load(Ordering::Relaxed);
        loop {
            let next = used.saturating_add(bytes);
            if next > self.limit {
                let message = format!(
                    "{} on memory-backed storage needs more than the {} bytes charged to the \
                     memory budget ({used} admitted, {bytes} more requested); {}",
                    self.what, self.limit, self.advice
                );
                let _ = self.refusal.set(message.clone());
                return Err(io::Error::new(io::ErrorKind::StorageFull, message));
            }
            match self
                .used
                .compare_exchange_weak(used, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return Ok(()),
                Err(now) => used = now,
            }
        }
    }

    /// Admit one file's growth to `len` bytes, given the `charged` allocation it holds.
    pub(crate) fn grow(&self, charged: &mut u64, len: u64) -> io::Result<()> {
        let need = self.file_bytes(len);
        if need > *charged {
            self.admit(need - *charged)?;
            *charged = need;
        }
        Ok(())
    }

    /// The first refusal, as the run's `resource_limit` error.
    pub(crate) fn refusal(&self) -> Option<JobError> {
        self.refusal
            .get()
            .map(|message| JobError::new(JobErrorKind::ResourceLimit, message.clone()))
    }
}

/// Registered roots, most recent last.
static REGISTRY: Mutex<Vec<(PathBuf, Arc<Meter>)>> = Mutex::new(Vec::new());

/// A registered root; dropping it unregisters the meter.
#[derive(Debug)]
pub(crate) struct Registration {
    root: PathBuf,
    meter: Arc<Meter>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut registry = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        registry.retain(|(root, meter)| !(root == &self.root && Arc::ptr_eq(meter, &self.meter)));
    }
}

/// Meter every entry created under `root` through this module against `meter`.
pub(crate) fn register(root: &Path, meter: Arc<Meter>) -> Registration {
    let mut registry = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    registry.push((root.to_owned(), meter.clone()));
    Registration {
        root: root.to_owned(),
        meter,
    }
}

/// The meter of the deepest registered root containing `path`.
pub(crate) fn meter_for(path: &Path) -> Option<Arc<Meter>> {
    let registry = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    registry
        .iter()
        .filter(|(root, _)| path.starts_with(root) && path != root)
        .max_by_key(|(root, _)| root.components().count())
        .map(|(_, meter)| meter.clone())
}

/// A writer that admits each write's growth before forwarding it (appends only).
#[derive(Debug)]
pub(crate) struct Metered<W> {
    inner: W,
    meter: Option<Arc<Meter>>,
    len: u64,
    charged: u64,
}

impl<W: Write> Metered<W> {
    /// `inner`, a new empty file whose entry was admitted as `charged`.
    fn new(inner: W, meter: Option<Arc<Meter>>, charged: u64) -> Self {
        Self {
            inner,
            meter,
            len: 0,
            charged,
        }
    }
}

impl Metered<File> {
    pub(crate) fn sync_all(&self) -> io::Result<()> {
        self.inner.sync_all()
    }
}

impl<W: Write> Write for Metered<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(meter) = &self.meter {
            meter.grow(&mut self.charged, self.len.saturating_add(buf.len() as u64))?;
        }
        let n = self.inner.write(buf)?;
        self.len += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Admit a new entry under `path`'s meter; returns the entry's charge (0 when unmetered).
fn entry(path: &Path) -> io::Result<(Option<Arc<Meter>>, u64)> {
    let meter = meter_for(path);
    let mut charged = 0;
    if let Some(meter) = &meter {
        meter.grow(&mut charged, 0)?;
    }
    Ok((meter, charged))
}

/// [`File::create`], metered.
pub(crate) fn create(path: &Path) -> io::Result<Metered<File>> {
    let (meter, charged) = entry(path)?;
    Ok(Metered::new(File::create(path)?, meter, charged))
}

/// A new file that must not exist yet, metered.
pub(crate) fn create_new(path: &Path) -> io::Result<Metered<File>> {
    let (meter, charged) = entry(path)?;
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    Ok(Metered::new(file, meter, charged))
}

/// [`std::fs::write`], metered: the whole file is admitted before it is created.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(meter) = meter_for(path) {
        meter.admit(meter.file_bytes(bytes.len() as u64))?;
    }
    std::fs::write(path, bytes)
}

/// [`std::fs::create_dir`], metered.
pub(crate) fn create_dir(path: &Path) -> io::Result<()> {
    entry(path)?;
    std::fs::create_dir(path)
}

/// [`std::fs::create_dir_all`], metered: one entry per directory it creates.
pub(crate) fn create_dir_all(path: &Path) -> io::Result<()> {
    let mut missing = Vec::new();
    let mut dir = path.to_owned();
    while !dir.as_os_str().is_empty() && !dir.exists() {
        missing.push(dir.clone());
        if !dir.pop() {
            break;
        }
    }
    for dir in missing.into_iter().rev() {
        entry(&dir)?;
        match std::fs::create_dir(&dir) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

/// [`std::fs::hard_link`], metered (an entry; the data is the source's).
pub(crate) fn hard_link(source: &Path, link: &Path) -> io::Result<()> {
    entry(link)?;
    std::fs::hard_link(source, link)
}

/// [`std::fs::copy`], metered: the copy is admitted before it is made.
pub(crate) fn copy(source: &Path, target: &Path) -> io::Result<u64> {
    if let Some(meter) = meter_for(target) {
        meter.admit(meter.file_bytes(std::fs::metadata(source)?.len()))?;
    }
    std::fs::copy(source, target)
}

/// A private temporary directory in `parent`, metered.
pub(crate) fn tempdir_in(parent: &Path, prefix: &str) -> io::Result<tempfile::TempDir> {
    entry(&parent.join(prefix))?;
    tempfile::Builder::new().prefix(prefix).tempdir_in(parent)
}

/// The metered model of what is stored under `dir`: per file the larger of
/// [`Meter::file_bytes`] and its allocated blocks plus [`ENTRY_BYTES`], and [`ENTRY_BYTES`]
/// per directory, a hard-linked file inside `links` counted as an entry
/// only (its data is counted where it was written). Depth-first; one open directory per
/// level. A metered run's stored model never exceeds what its meter admitted, so a larger
/// figure names a write that bypassed the meter.
pub(crate) fn stored_bytes(meter: &Meter, dir: &Path, links: &Path) -> io::Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        let bytes = if metadata.is_dir() {
            ENTRY_BYTES.saturating_add(stored_bytes(meter, &path, links)?)
        } else if linked(&metadata) && path.starts_with(links) {
            ENTRY_BYTES
        } else {
            // What the filesystem actually allocated, when that exceeds the length model
            // (huge pages, preallocation): the cross-check must see real allocation.
            meter
                .file_bytes(metadata.len())
                .max(allocated(&metadata).saturating_add(ENTRY_BYTES))
        };
        total = total.saturating_add(bytes);
    }
    Ok(total)
}

/// Bytes the filesystem allocated to a file (`st_blocks` × 512 on Unix).
#[cfg(unix)]
pub(crate) fn allocated(metadata: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::blocks(metadata).saturating_mul(512)
}
#[cfg(not(unix))]
pub(crate) fn allocated(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

#[cfg(unix)]
fn linked(metadata: &std::fs::Metadata) -> bool {
    std::os::unix::fs::MetadataExt::nlink(metadata) > 1
}
#[cfg(not(unix))]
fn linked(_: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A write past the charge is refused before its bytes land: after the refusal the
    /// directory holds no more than the meter admitted, on whatever filesystem the test
    /// directory is (on tmpfs, its allocated blocks are within the admitted figure too).
    #[test]
    fn writes_are_admitted_before_they_land_and_refused_past_the_charge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        std::fs::create_dir(&root).unwrap();
        let limit = 64 * 1024;
        let meter = Meter::new("the work directory", "test", limit, 4096);
        let registration = register(&root, meter.clone());
        assert!(meter_for(&root.join("a")).is_some());
        assert!(meter_for(&root).is_none() && meter_for(dir.path()).is_none());
        create_dir_all(&root.join("x/y")).unwrap();
        write(&root.join("x/y/a.bin"), &[1; 5000]).unwrap();
        hard_link(&root.join("x/y/a.bin"), &root.join("a.link")).unwrap();
        let mut file = std::io::BufWriter::new(create(&root.join("stream.bin")).unwrap());
        let mut written = 0u64;
        let error = loop {
            match file.write_all(&[7; 3000]).and_then(|_| file.flush()) {
                Ok(()) => written += 3000,
                Err(error) => break error,
            }
        };
        drop(file);
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        let refusal = meter.refusal().unwrap();
        assert_eq!(refusal.kind(), JobErrorKind::ResourceLimit);
        assert!(refusal.to_string().contains("memory-backed"), "{refusal}");
        assert!(meter.used() <= limit && written > 0);
        let stored = stored_bytes(&meter, &root, &root.join("nowhere")).unwrap();
        // The hard link counts its data twice here (it is outside `links`), so subtract it.
        assert!(
            stored - meter.file_bytes(5000) + ENTRY_BYTES <= meter.used(),
            "{stored}"
        );
        #[cfg(unix)]
        if crate::spill::inspect_volume(&root).unwrap().memory_backed {
            let blocks = |p: &str| {
                std::os::unix::fs::MetadataExt::blocks(&std::fs::metadata(root.join(p)).unwrap())
                    * 512
            };
            assert!(blocks("stream.bin") + blocks("x/y/a.bin") <= limit);
        }
        // Whole-file writes and copies are refused before anything is created.
        assert!(write(&root.join("big.bin"), &vec![0; limit as usize]).is_err());
        assert!(!root.join("big.bin").exists());
        drop(registration);
        assert!(meter_for(&root.join("a")).is_none());
        // Unregistered paths are plain filesystem calls.
        write(&root.join("free.bin"), &vec![0; 2 * limit as usize]).unwrap();
    }
}
