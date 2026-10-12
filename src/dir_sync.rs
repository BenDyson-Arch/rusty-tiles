//! Directory durability for published output. File contents are synced when a
//! member is sealed; a rename, create or unlink lives in the parent directory's
//! entry table, so it needs its own fsync to survive a crash.
//!
//! Every publisher goes through these helpers: the single-file runtime
//! publisher, the legacy 3TZ publisher and the directory publisher (which also
//! covers the terrain and raster prune of staged directories, because the whole
//! staged tree is synced before it is installed).
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
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            }
        }
        sync_directory(&directory)?;
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

    #[test]
    fn a_missing_directory_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        assert!(sync_directory(&root.path().join("missing")).is_err());
        assert!(sync_tree(&root.path().join("missing")).is_err());
    }

    #[test]
    fn a_bare_name_syncs_the_current_directory() {
        assert_eq!(parent_of(Path::new("output")), Path::new("."));
        assert_eq!(parent_of(Path::new("/tmp/output")), Path::new("/tmp"));
    }
}
