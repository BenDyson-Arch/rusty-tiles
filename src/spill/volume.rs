//! Free-space preflight and memory-backed (tmpfs) detection. Not a reservation: other processes
//! can still consume space, so ENOSPC during a spill stays a typed I/O failure.
use super::{InsufficientSpace, SpillError};
use std::{io, path::Path};

/// Which filesystem a preflight checks; carried in the refusal cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpaceRole {
    Scratch,
    Output,
}

impl SpaceRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Scratch => "scratch",
            Self::Output => "output",
        }
    }
}

/// Free space available to this process, and whether the filesystem is RAM (tmpfs/ramfs).
/// Memory-backed scratch is reported, not refused; it sits outside the memory bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Volume {
    pub available_bytes: u64,
    pub memory_backed: bool,
}

pub(crate) fn inspect_volume(path: &Path) -> Result<Volume, SpillError> {
    implementation::inspect(path).map_err(|e| SpillError::io("inspect scratch volume", path, e))
}

/// Refuses before any workspace exists when `available < needed`.
pub(crate) fn preflight(path: &Path, needed: u64, role: SpaceRole) -> Result<Volume, SpillError> {
    preflight_with(path, needed, role, inspect_volume)
}

pub(super) fn preflight_with(
    path: &Path,
    needed: u64,
    role: SpaceRole,
    inspect: impl FnOnce(&Path) -> Result<Volume, SpillError>,
) -> Result<Volume, SpillError> {
    let volume = inspect(path)?;
    if volume.available_bytes < needed {
        return Err(SpillError::InsufficientSpace(InsufficientSpace {
            path: path.to_owned(),
            needed,
            available: volume.available_bytes,
            filesystem: role,
        }));
    }
    Ok(volume)
}

#[cfg(unix)]
fn c_path(path: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

#[cfg(all(unix, not(target_vendor = "apple")))]
mod implementation {
    use super::*;
    // Field widths differ across targets; the casts are no-ops on 64-bit Linux.
    #[allow(clippy::unnecessary_cast)]
    pub(super) fn inspect(path: &Path) -> io::Result<Volume> {
        let path = c_path(path)?;
        // SAFETY: statvfs is a plain C output record, zero initialization is valid.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: the CString is terminated and stat is writable for the full ABI record.
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let unit = if stat.f_frsize != 0 {
            stat.f_frsize as u64
        } else {
            stat.f_bsize as u64
        };
        Ok(Volume {
            available_bytes: (stat.f_bavail as u64).saturating_mul(unit),
            memory_backed: memory_backed(&path)?,
        })
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[allow(clippy::unnecessary_cast)]
    fn memory_backed(path: &std::ffi::CStr) -> io::Result<bool> {
        const TMPFS_MAGIC: u64 = 0x0102_1994;
        const RAMFS_MAGIC: u64 = 0x8584_58f6;
        // SAFETY: statfs is a plain C output record, zero initialization is valid.
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: the CStr is terminated and stat is writable for the full ABI record.
        if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // f_type is signed and 32-bit on some targets; compare the magic's low word.
        let kind = stat.f_type as u64 & 0xffff_ffff;
        Ok(matches!(kind, TMPFS_MAGIC | RAMFS_MAGIC))
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn memory_backed(_: &std::ffi::CStr) -> io::Result<bool> {
        Ok(false)
    }
}

#[cfg(target_vendor = "apple")]
mod implementation {
    use super::*;
    // statvfs block counts are 32-bit on Apple targets; statfs has 64-bit counts.
    #[allow(clippy::unnecessary_cast)]
    pub(super) fn inspect(path: &Path) -> io::Result<Volume> {
        let path = c_path(path)?;
        // SAFETY: statfs is a plain C output record, zero initialization is valid.
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: the CString is terminated and stat is writable for the full ABI record.
        if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Volume {
            available_bytes: (stat.f_bavail as u64).saturating_mul(stat.f_bsize as u64),
            memory_backed: false,
        })
    }
}

#[cfg(windows)]
mod implementation {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            available_to_caller: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
    }
    pub(super) fn inspect(path: &Path) -> io::Result<Volume> {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains NUL",
            ));
        }
        wide.push(0);
        let mut available = 0u64;
        // SAFETY: wide is terminated; available is a writable ULARGE_INTEGER; the optional
        // totals are null, which the API permits.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Volume {
            available_bytes: available,
            memory_backed: false,
        })
    }
}

#[cfg(not(any(unix, windows)))]
mod implementation {
    use super::*;
    pub(super) fn inspect(_: &Path) -> io::Result<Volume> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "free-space inspection is unsupported on this platform",
        ))
    }
}
