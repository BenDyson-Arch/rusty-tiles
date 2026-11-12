//! The temporary paths running jobs own (spill scratch, private work directories and
//! output staging), so a process that must exit before a job finishes unwinding can still
//! remove them. Each owner registers its path when it creates it and unregisters it after
//! removing it ([`Registered`] is dropped after the owner's directory guard). Paths a job
//! creates inside a registered directory need no entry of their own. Output a job has
//! already been permitted to publish is never listed: staging is unregistered before it is
//! installed, and the previous output held during a replace is never registered.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static PATHS: Mutex<Vec<(u64, PathBuf)>> = Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(0);

/// One registered path; dropping it removes the entry (not the path).
#[derive(Debug)]
pub(crate) struct Registered(u64);

impl Drop for Registered {
    fn drop(&mut self) {
        let mut paths = PATHS.lock().unwrap_or_else(|e| e.into_inner());
        paths.retain(|(id, _)| *id != self.0);
    }
}

/// A private temporary directory registered for its whole life: the registration ends
/// only after the directory is removed.
#[derive(Debug)]
pub(crate) struct RegisteredDir {
    dir: tempfile::TempDir,
    registered: Registered,
}

impl RegisteredDir {
    pub(crate) fn new(dir: tempfile::TempDir) -> Self {
        let registered = register(dir.path());
        Self { dir, registered }
    }

    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Remove the directory now, reporting failure.
    pub(crate) fn close(self) -> std::io::Result<()> {
        let Self { dir, registered } = self;
        let closed = dir.close();
        drop(registered);
        closed
    }

    /// Stop removing on drop; the caller removes the path while holding the registration.
    pub(crate) fn keep(self) -> (PathBuf, Registered) {
        (self.dir.keep(), self.registered)
    }
}

/// Register `path` until the returned guard is dropped.
pub(crate) fn register(path: &Path) -> Registered {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut paths = PATHS.lock().unwrap_or_else(|e| e.into_inner());
    paths.push((id, path.to_owned()));
    Registered(id)
}

/// Remove every registered path now: directories recursively (symlinks inside are removed,
/// never followed), files directly. For a CLI that is about to exit while a stopped job is
/// still blocked; returns the paths it could not remove. Entries stay registered, so the
/// owners' own cleanup, if it still runs, finds nothing left to do.
#[doc(hidden)]
pub fn remove_registered_now() -> Vec<PathBuf> {
    remove(snapshot())
}

fn snapshot() -> Vec<PathBuf> {
    PATHS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(_, path)| path.clone())
        .collect()
}

fn remove(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|path| {
            let removed = match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
                Ok(_) => std::fs::remove_file(path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            };
            removed.is_err() && path.exists()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Other tests in this process register their own scratch, so these tests never call
    // `remove_registered_now`.
    #[test]
    fn registration_lasts_until_the_guard_drops() {
        let root = tempfile::tempdir().unwrap();
        let (kept, gone) = (root.path().join("kept"), root.path().join("gone"));
        let guard = register(&kept);
        drop(register(&gone));
        let listed = snapshot();
        assert!(listed.contains(&kept));
        assert!(!listed.contains(&gone));
        drop(guard);
        assert!(!snapshot().contains(&kept));
    }

    #[test]
    fn removal_takes_directories_and_files_without_following_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("precious"), b"x").unwrap();
        let dir = root.path().join("dir");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested").join("f"), b"x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), dir.join("link")).unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        let missing = root.path().join("missing");
        assert!(remove(vec![dir.clone(), file.clone(), missing]).is_empty());
        assert!(!dir.exists() && !file.exists());
        assert!(outside.path().join("precious").exists());
    }
}
