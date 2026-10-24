//! Directory durability for published output. File contents are synced when a
//! member is sealed; a rename, create or unlink lives in the parent directory's
//! entry table, so it needs its own fsync to survive a crash.
//!
//! Every publisher goes through these helpers: the single-file runtime
//! publisher and the directory publisher (which also covers the terrain and
//! raster prune of staged directories, because the whole staged tree is synced
//! before it is installed).
//!
//! `File::sync_all` is the crate's file sync, so directories use it too. On
//! macOS std implements it with `F_FULLFSYNC`, the same strength as file syncs.
//! Windows cannot open a directory through std without
//! `FILE_FLAG_BACKUP_SEMANTICS` and has no directory fsync; NTFS journals
//! namespace changes itself, so these helpers are documented no-ops there.
use std::{
    io,
    path::{Path, PathBuf},
};

/// Make earlier creates, renames and unlinks inside `directory` durable.
pub(crate) fn sync_directory(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        match std::fs::File::open(directory).and_then(|handle| handle.sync_all()) {
            // Some network and FUSE filesystems cannot sync a directory handle.
            // That is a property of the volume, not a failed publication.
            Err(e) if directory_sync_unsupported(&e) => Ok(()),
            result => result,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Ok(())
    }
}

#[cfg(unix)]
fn directory_sync_unsupported(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code) if code == libc::EINVAL || code == libc::ENOTSUP || code == libc::ENOTTY
    )
}

/// The directory that holds `path`'s entry.
pub(crate) fn parent_of(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_owned()
}

/// Sync the directory holding `path`'s entry, for the entry's own rename or unlink.
pub(crate) fn sync_parent(path: &Path) -> io::Result<()> {
    sync_directory(&parent_of(path))
}

/// Sync `root` and every directory below it, without following symlinks.
/// Only directory publication, which needs native support, stages trees.
#[cfg_attr(not(feature = "native-geospatial"), allow(dead_code))]
pub(crate) fn sync_tree(root: &Path) -> io::Result<()> {
    sync_tree_with(root, sync_directory)
}

fn sync_tree_with(root: &Path, mut sync: impl FnMut(&Path) -> io::Result<()>) -> io::Result<()> {
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            }
        }
        sync(&directory)?;
    }
    Ok(())
}

/// `create_dir_all` that also makes each directory it created durable: the
/// parent of every new directory is synced, from the deepest new one up to the
/// first ancestor that already existed.
pub(crate) fn create_dir_all_durable(directory: &Path) -> io::Result<()> {
    create_dir_all_with(directory, sync_directory)
}

fn create_dir_all_with(
    directory: &Path,
    mut sync: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let missing: Vec<&Path> = directory
        .ancestors()
        .take_while(|ancestor| !ancestor.as_os_str().is_empty() && !ancestor.exists())
        .collect();
    std::fs::create_dir_all(directory)?;
    // `ancestors` yields the deepest path first, which is the sync order.
    for created in missing {
        sync(&parent_of(created))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syncs_a_directory_and_a_nested_tree() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("a/b")).unwrap();
        std::fs::write(root.path().join("a/b/file"), b"x").unwrap();
        sync_directory(root.path()).unwrap();
        sync_tree(root.path()).unwrap();
        sync_parent(&root.path().join("a")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_sync_that_does_nothing_is_detected_by_missing_paths() {
        // Only a real open-and-fsync can fail on an absent directory.
        let root = tempfile::tempdir().unwrap();
        assert!(sync_directory(&root.path().join("missing")).is_err());
        assert!(sync_parent(&root.path().join("missing/child")).is_err());
        assert!(sync_tree(&root.path().join("missing")).is_err());
    }

    #[test]
    fn tree_sync_visits_the_root_and_every_nested_directory_once() {
        let root = tempfile::tempdir().unwrap();
        for dir in ["a/b/c", "a/d", "e"] {
            std::fs::create_dir_all(root.path().join(dir)).unwrap();
        }
        std::fs::write(root.path().join("a/b/file"), b"x").unwrap();
        let mut seen = Vec::new();
        sync_tree_with(root.path(), |dir| {
            seen.push(dir.strip_prefix(root.path()).unwrap().to_owned());
            Ok(())
        })
        .unwrap();
        seen.sort();
        let expected: Vec<PathBuf> = ["", "a", "a/b", "a/b/c", "a/d", "e"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(seen, expected);
        // A failing sync stops the walk with its error.
        let error = sync_tree_with(root.path(), |_| Err(io::Error::other("injected"))).unwrap_err();
        assert_eq!(error.to_string(), "injected");
    }

    #[test]
    fn create_dir_all_syncs_the_parent_of_each_new_directory_deepest_first() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("existing")).unwrap();
        let target = root.path().join("existing/x/y/z");
        let mut synced = Vec::new();
        create_dir_all_with(&target, |dir| {
            synced.push((dir.to_owned(), dir.is_dir()));
            Ok(())
        })
        .unwrap();
        assert!(target.is_dir());
        let base = root.path().join("existing");
        assert_eq!(
            synced,
            [
                (base.join("x/y"), true),
                (base.join("x"), true),
                (base, true)
            ]
        );
        // Nothing new, nothing to sync.
        create_dir_all_with(&target, |_| panic!("no directory was created")).unwrap();
        create_dir_all_durable(&root.path().join("existing/other/deep")).unwrap();
    }

    #[test]
    fn a_bare_name_syncs_the_current_directory() {
        assert_eq!(parent_of(Path::new("output")), Path::new("."));
        assert_eq!(parent_of(Path::new("/tmp/output")), Path::new("/tmp"));
    }
}
