//! SQLite's temporary files inside the validation scratch directory (#342 review P2-2).
//!
//! SQLite opens its sorter runs, temporary databases and statement journals with no name, and
//! the platform VFS then names them in its own temporary directory (`SQLITE_TMPDIR`, `TMPDIR`,
//! `/var/tmp`, ... on Unix), outside the scratch volume the validator preflights and outside
//! `--scratch-dir`. This VFS delegates every call to the platform default, except that an
//! anonymous open is given a fresh name inside the scratch directory first. The platform VFS
//! still applies its own delete-on-close handling (Unix unlinks the file as soon as it is
//! open), so nothing outlives its handle, and every file lands on the preflighted volume.
//!
//! The copy keeps the default's `pAppData`, `szOsFile` and `mxPathname`, so delegated methods
//! that read them see the default's values; `xOpen` itself is called with the default's own
//! pointer. No SQL, operation count or result changes: only where the bytes are written.
use rusqlite::ffi;
use std::{
    ffi::{c_char, c_int, CString},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

/// A registered VFS; unregistered and freed when dropped, after every connection using it is
/// closed. It holds the allocation as a raw pointer from [`Box::into_raw`], so the pointer
/// SQLite keeps, and the `Registered` it is cast back to in `open`, carry the provenance of the
/// whole allocation rather than of its first field (review N2).
pub(super) struct ScratchVfs(*mut Registered);

// The first field: SQLite's pointer to the VFS is a pointer to `Registered`.
#[repr(C)]
struct Registered {
    vfs: ffi::sqlite3_vfs,
    base: *mut ffi::sqlite3_vfs,
    name: CString,
    /// The scratch directory with a trailing separator, as the platform's path bytes (any bytes
    /// on Unix, UTF-8 elsewhere, as SQLite expects there).
    dir: Vec<u8>,
    /// Anonymous files opened through this VFS.
    opened: AtomicU64,
    /// Names handed to the platform VFS, which may keep the pointer while the file is open.
    /// One per anonymous open, which SQLite makes a few of per sort or temporary table; they
    /// are freed with the VFS, after its connection closes.
    names: Mutex<Vec<Box<[u8]>>>,
}

// SAFETY: `base` points to the process-wide default VFS, which SQLite never frees; the
// allocation behind the pointer is owned by this value and its mutable state is synchronized.
unsafe impl Send for ScratchVfs {}
unsafe impl Sync for ScratchVfs {}

/// VFS names and temporary-file names: process-wide, so two VFSs over the same directory
/// never pick the same file name (review N3).
static SERIAL: AtomicU64 = AtomicU64::new(0);

/// `dir` + `sqlite-N.tmp`, terminated by two NULs. SQLite requires every name it passes to a
/// VFS `xOpen` to be followed by its URI parameters, here none: the name's NUL and the empty
/// list's NUL. `sqlite3_uri_parameter` and the `sqlite3_filename_*` accessors walk past the
/// first NUL, so a single-NUL name could be over-read by one byte (review N1).
fn temp_name(dir: &[u8], serial: u64) -> Box<[u8]> {
    let mut name = dir.to_vec();
    name.extend_from_slice(format!("sqlite-{serial}.tmp").as_bytes());
    name.extend_from_slice(&[0, 0]);
    name.into_boxed_slice()
}

unsafe extern "C" fn open(
    vfs: *mut ffi::sqlite3_vfs,
    name: ffi::sqlite3_filename,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    // SAFETY: SQLite passes back the pointer it was registered with, a `Registered` that
    // lives until it is unregistered.
    let this = unsafe { &*vfs.cast::<Registered>() };
    let base = this.base;
    // SAFETY: the default VFS has an `xOpen`.
    let base_open = unsafe { (*base).xOpen }.expect("the default VFS opens files");
    let mut name = name;
    if name.is_null() && !this.dir.contains(&0) {
        this.opened.fetch_add(1, Ordering::Relaxed);
        let path = temp_name(&this.dir, SERIAL.fetch_add(1, Ordering::Relaxed));
        let mut names = this.names.lock().unwrap_or_else(|e| e.into_inner());
        // The heap buffer does not move when its box moves into the vector.
        name = path.as_ptr().cast::<c_char>();
        names.push(path);
    }
    // SAFETY: delegated with the default's own pointer and SQLite's arguments.
    unsafe { base_open(base, name, file, flags, out_flags) }
}

/// The directory's path bytes with a trailing separator; `None` only off Unix for a path
/// that is not UTF-8.
fn dir_bytes(dir: &Path) -> Option<Vec<u8>> {
    #[cfg(unix)]
    let mut bytes = std::os::unix::ffi::OsStrExt::as_bytes(dir.as_os_str()).to_vec();
    #[cfg(not(unix))]
    let mut bytes = dir.to_str()?.as_bytes().to_vec();
    if !bytes.ends_with(std::path::MAIN_SEPARATOR_STR.as_bytes()) {
        bytes.extend_from_slice(std::path::MAIN_SEPARATOR_STR.as_bytes());
    }
    Some(bytes)
}

impl ScratchVfs {
    /// A VFS placing anonymous files in `dir`; `None` when there is no default VFS, the
    /// registration fails, or (off Unix) the directory is not UTF-8. The caller then keeps
    /// SQLite's own temporary directory and says so ([`ScratchVfs::fallback_reason`]).
    pub(super) fn new(dir: &Path) -> Option<Self> {
        let dir = dir_bytes(dir)?;
        // SAFETY: a null name finds the default VFS; it is never freed.
        let base = unsafe { ffi::sqlite3_vfs_find(std::ptr::null()) };
        if base.is_null() {
            return None;
        }
        let name = CString::new(format!(
            "rusty-tiles-scratch-{}",
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ))
        .ok()?;
        // SAFETY: a plain copy of the default's descriptor; its pointers stay valid.
        let mut vfs = unsafe { std::ptr::read(base) };
        vfs.zName = name.as_ptr();
        vfs.pNext = std::ptr::null_mut();
        vfs.xOpen = Some(open);
        let registered = Box::into_raw(Box::new(Registered {
            vfs,
            base,
            name,
            dir,
            opened: AtomicU64::new(0),
            names: Mutex::new(Vec::new()),
        }));
        // SAFETY: the allocation lives until `Drop` unregisters it and only then frees it.
        let rc = unsafe { ffi::sqlite3_vfs_register(registered.cast(), 0) };
        if rc != ffi::SQLITE_OK {
            // SAFETY: never registered; reclaimed once.
            drop(unsafe { Box::from_raw(registered) });
            return None;
        }
        Some(Self(registered))
    }

    /// Why SQLite's temporary files cannot be placed in `dir` (review N4).
    pub(super) fn fallback_reason(dir: &Path) -> String {
        format!(
            "SQLite temporary files stay in SQLite's own temporary directory, not the scratch \
             directory {}: {}",
            dir.display(),
            if dir_bytes(dir).is_none() {
                "the path is not UTF-8"
            } else {
                "the SQLite VFS could not be registered"
            }
        )
    }

    fn registered(&self) -> &Registered {
        // SAFETY: owned and alive until `Drop`.
        unsafe { &*self.0 }
    }

    pub(super) fn name(&self) -> &str {
        self.registered().name.to_str().expect("an ASCII name")
    }

    /// Anonymous files opened so far (tests).
    #[cfg(test)]
    pub(super) fn opened(&self) -> u64 {
        self.registered().opened.load(Ordering::Relaxed)
    }
}

impl Drop for ScratchVfs {
    fn drop(&mut self) {
        // SAFETY: registered in `new`; no connection uses it any more (field order of `Store`).
        // Unregistered before the allocation is freed.
        unsafe {
            ffi::sqlite3_vfs_unregister(self.0.cast());
            drop(Box::from_raw(self.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review N1: generated names end with the name's NUL and the empty parameter list's.
    #[test]
    fn temp_names_are_double_nul_terminated() {
        let name = temp_name(b"/scratch/", 7);
        assert_eq!(&*name, b"/scratch/sqlite-7.tmp\0\0");
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_directories_are_used_on_unix() {
        use std::os::unix::ffi::OsStrExt;
        let dir = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
        assert_eq!(dir_bytes(dir).unwrap(), b"/tmp/\xff/");
    }
}
