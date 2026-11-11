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
    ffi::{c_int, CString},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

/// A registered VFS; unregistered when dropped, after every connection using it is closed.
pub(super) struct ScratchVfs(Box<Registered>);

// The first field: SQLite's pointer to the VFS is a pointer to `Registered`.
#[repr(C)]
struct Registered {
    vfs: ffi::sqlite3_vfs,
    base: *mut ffi::sqlite3_vfs,
    name: CString,
    /// The scratch directory with a trailing separator, as UTF-8.
    dir: String,
    next: AtomicU64,
    /// Names handed to the platform VFS, which may keep the pointer while the file is open.
    names: Mutex<Vec<CString>>,
}

// SAFETY: `base` points to the process-wide default VFS, which SQLite never frees; every other
// field is owned and synchronized.
unsafe impl Send for ScratchVfs {}
unsafe impl Sync for ScratchVfs {}

static SERIAL: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" fn open(
    vfs: *mut ffi::sqlite3_vfs,
    name: ffi::sqlite3_filename,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    // SAFETY: SQLite passes back the pointer it was registered with, a `Registered`.
    let this = unsafe { &*(vfs as *const Registered) };
    let base = this.base;
    // SAFETY: the default VFS has an `xOpen`.
    let base_open = unsafe { (*base).xOpen }.expect("the default VFS opens files");
    let mut name = name;
    if name.is_null() {
        let serial = this.next.fetch_add(1, Ordering::Relaxed);
        if let Ok(path) = CString::new(format!("{}sqlite-{serial}.tmp", this.dir)) {
            let mut names = this.names.lock().unwrap_or_else(|e| e.into_inner());
            // The heap buffer does not move when its owner moves into the vector.
            name = path.as_ptr();
            names.push(path);
        }
    }
    // SAFETY: delegated with the default's own pointer and SQLite's arguments.
    unsafe { base_open(base, name, file, flags, out_flags) }
}

impl ScratchVfs {
    /// A VFS placing anonymous files in `dir`; `None` when the directory is not UTF-8 or there
    /// is no default VFS, and SQLite then keeps its own temporary directory.
    pub(super) fn new(dir: &Path) -> Option<Self> {
        let mut dir = dir.to_str()?.to_owned();
        if !dir.ends_with(std::path::MAIN_SEPARATOR) {
            dir.push(std::path::MAIN_SEPARATOR);
        }
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
        let mut registered = Box::new(Registered {
            vfs,
            base,
            name,
            dir,
            next: AtomicU64::new(0),
            names: Mutex::new(Vec::new()),
        });
        // SAFETY: the box outlives the registration (`Drop` unregisters first).
        let rc = unsafe { ffi::sqlite3_vfs_register(&mut registered.vfs, 0) };
        (rc == ffi::SQLITE_OK).then_some(Self(registered))
    }

    pub(super) fn name(&self) -> &str {
        self.0.name.to_str().expect("an ASCII name")
    }

    /// Anonymous files opened so far (tests).
    #[cfg(test)]
    pub(super) fn opened(&self) -> u64 {
        self.0.next.load(Ordering::Relaxed)
    }
}

impl Drop for ScratchVfs {
    fn drop(&mut self) {
        // SAFETY: registered in `new`; no connection uses it any more (field order of `Store`).
        unsafe {
            ffi::sqlite3_vfs_unregister(&mut self.0.vfs);
        }
    }
}
