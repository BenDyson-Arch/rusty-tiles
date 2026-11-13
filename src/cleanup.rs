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
use std::time::{Duration, Instant};

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
///
/// The registry is process-wide: this removes every path any job in the process has
/// registered. That is right only for a process that runs one job, as the CLI does; a
/// library caller running several jobs at once must not call it.
///
/// Worker threads may still be unwinding and writing, so a file created during removal
/// (`ENOTEMPTY`) or a directory recreated after it would survive a single pass. Removal is
/// repeated every [`SETTLE_INTERVAL`] until one round finds nothing left to remove, for at
/// most [`SETTLE_WINDOW`].
#[doc(hidden)]
pub fn remove_registered_now() -> Vec<PathBuf> {
    remove_settled(&snapshot(), SETTLE_WINDOW, SETTLE_INTERVAL)
}

/// How long [`remove_registered_now`] keeps removing paths that reappear.
const SETTLE_WINDOW: Duration = Duration::from_millis(1500);
/// The pause between its rounds.
const SETTLE_INTERVAL: Duration = Duration::from_millis(100);

/// Remove `paths` in rounds until a round finds none of them present (the previous round's
/// removal held for one interval) or `window` has passed; returns those still present.
fn remove_settled(paths: &[PathBuf], window: Duration, interval: Duration) -> Vec<PathBuf> {
    let deadline = Instant::now() + window;
    loop {
        let present: Vec<PathBuf> = paths
            .iter()
            .filter(|path| std::fs::symlink_metadata(path).is_ok())
            .cloned()
            .collect();
        if present.is_empty() {
            return present;
        }
        let left = remove(present);
        if Instant::now() >= deadline {
            return left;
        }
        std::thread::sleep(interval);
    }
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

    /// A worker still writing into a registered directory (recreating it, as
    /// `create_dir_all` does) while the backstop removes it: rounds repeat until the writes
    /// stop and one round finds nothing left.
    #[test]
    fn removal_repeats_while_a_worker_keeps_writing() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        // Far longer than any pause of a writer in a tight loop, so a round finds the
        // directory gone only once the writer has stopped, unless the writer itself was
        // descheduled for about that long.
        let interval = Duration::from_millis(200);
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("spill");
        std::fs::create_dir_all(&dir).unwrap();
        let writing = Arc::new(AtomicBool::new(true));
        let worker = {
            let (dir, writing) = (dir.clone(), writing.clone());
            std::thread::spawn(move || {
                let mut n = 0u64;
                let mut writes = 0u64;
                // The longest gap between two iterations: how long the writer was paused.
                let (mut last, mut longest_pause) = (Instant::now(), Duration::ZERO);
                while writing.load(Ordering::Relaxed) {
                    let _ = std::fs::create_dir_all(dir.join("nested"));
                    if std::fs::write(dir.join("nested").join(format!("{n}.spill")), b"x").is_ok() {
                        writes += 1;
                    }
                    n += 1;
                    let now = Instant::now();
                    longest_pause = longest_pause.max(now - last);
                    last = now;
                }
                (writes, longest_pause)
            })
        };
        let stopper = {
            let writing = writing.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(600));
                writing.store(false, Ordering::Relaxed);
            })
        };
        let left = remove_settled(
            std::slice::from_ref(&dir),
            Duration::from_secs(10),
            interval,
        );
        // Judge the outcome only once the writer has stopped: nothing can recreate the
        // directory after these joins.
        stopper.join().unwrap();
        let (writes, longest_pause) = worker.join().unwrap();
        assert!(writes > 0, "the worker never wrote");
        assert!(left.is_empty(), "{left:?}");
        // Removal settles once a round finds the directory gone. While the writer runs that
        // happens only if it paused for about an interval (a loaded CI runner can deschedule
        // it), and it may then recreate the directory after removal returned. Without such a
        // pause the directory must be gone, or removal stopped repeating while it was written.
        assert!(
            !dir.exists() || longest_pause >= interval / 2,
            "the directory survived its removal, and the worker never paused (longest pause {longest_pause:?})"
        );
    }
}
