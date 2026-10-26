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

/// Design §2.7 preflight stage: `Geometry` (stage 1, passes 0–5, before the workspace and
/// scratch exist) or `Texture` (stage 2, passes 6–8, after pass 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreflightStage {
    Geometry,
    Texture,
    /// HLOD coarse levels: interface incidence, source index and coarse payloads.
    Lod,
}

impl PreflightStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Geometry => "geometry",
            Self::Texture => "texture",
            Self::Lod => "lod",
        }
    }
}

/// Free space available to this process, and whether the filesystem is RAM (tmpfs/ramfs).
/// Memory-backed scratch is reported, not refused; it sits outside the memory bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Volume {
    pub available_bytes: u64,
    pub memory_backed: bool,
    /// Filesystem allocation fragment/cluster size. Logical spill byte counts exclude this
    /// per-file rounding and filesystem metadata; the producer reserves those separately.
    pub allocation_unit_bytes: u64,
}

pub(crate) fn inspect_volume(path: &Path) -> Result<Volume, SpillError> {
    implementation::inspect(path)
        .and_then(validate_volume)
        .map_err(|e| SpillError::io("inspect scratch volume", path, e))
}

fn validate_volume(volume: Volume) -> io::Result<Volume> {
    if volume.allocation_unit_bytes == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "filesystem reported a zero allocation unit",
        ));
    }
    Ok(volume)
}

/// Whether two existing paths are on one filesystem, so their space demands add up.
/// Unix compares device ids; elsewhere every pair is conservatively one filesystem.
pub(crate) fn same_filesystem(a: &Path, b: &Path) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(std::fs::metadata(a)?.dev() == std::fs::metadata(b)?.dev())
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        Ok(true)
    }
}

/// Refuses before any workspace exists when `available < needed`.
pub(crate) fn preflight(
    path: &Path,
    needed: u64,
    role: SpaceRole,
    stage: PreflightStage,
) -> Result<Volume, SpillError> {
    preflight_with(path, needed, role, stage, inspect_volume)
}

pub(super) fn preflight_with(
    path: &Path,
    needed: u64,
    role: SpaceRole,
    stage: PreflightStage,
    inspect: impl FnOnce(&Path) -> Result<Volume, SpillError>,
) -> Result<Volume, SpillError> {
    let volume = validate_volume(inspect(path)?)
        .map_err(|e| SpillError::io("inspect scratch volume", path, e))?;
    if volume.available_bytes < needed {
        return Err(SpillError::InsufficientSpace(InsufficientSpace {
            path: path.to_owned(),
            needed,
            available: volume.available_bytes,
            filesystem: role,
            stage,
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
            allocation_unit_bytes: unit,
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
            allocation_unit_bytes: stat.f_bsize as u64,
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
        fn GetVolumePathNameW(
            filename: *const u16,
            volume_path: *mut u16,
            buffer_length: u32,
        ) -> i32;
        fn GetDiskFreeSpaceW(
            root: *const u16,
            sectors_per_cluster: *mut u32,
            bytes_per_sector: *mut u32,
            free_clusters: *mut u32,
            total_clusters: *mut u32,
        ) -> i32;
    }
    pub(super) fn inspect(path: &Path) -> io::Result<Volume> {
        if path.as_os_str().encode_wide().any(|c| c == 0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains NUL",
            ));
        }
        // Resolve relative paths against the actual working directory. GetVolumePathNameW
        // otherwise interprets an unqualified relative path using the boot volume.
        let canonical = std::fs::canonicalize(path)?;
        let mut wide: Vec<u16> = canonical.as_os_str().encode_wide().collect();
        wide.push(0);
        // The mount-point string is no longer than this full path plus its trailing slash.
        // A full-path-sized buffer also avoids GetVolumePathNameW's truncated-root behavior.
        let root_length = wide.len().checked_add(2).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "volume path is too long")
        })?;
        let root_length = u32::try_from(root_length)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "volume path is too long"))?;
        let mut root = vec![0u16; root_length as usize];
        // SAFETY: wide is terminated, root contains root_length writable WCHARs.
        if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root_length) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let (mut sectors, mut sector_bytes, mut free_clusters, mut total_clusters) =
            (0u32, 0u32, 0u32, 0u32);
        // SAFETY: root is the terminated mount point returned by GetVolumePathNameW; every
        // DWORD output points to a writable u32. Only cluster geometry is consumed here.
        if unsafe {
            GetDiskFreeSpaceW(
                root.as_ptr(),
                &mut sectors,
                &mut sector_bytes,
                &mut free_clusters,
                &mut total_clusters,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
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
            allocation_unit_bytes: u64::from(sectors) * u64::from(sector_bytes),
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
