//! Free-space preflight and memory-backed (tmpfs) detection. Not a reservation: other processes
//! can still consume space, so ENOSPC during a spill stays a typed I/O failure.
use super::{InsufficientSpace, SpillError};
use std::{
    io,
    path::{Path, PathBuf},
};

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
    /// `memory_backed` was assumed to fail safe: an overlay whose writable upper layer could
    /// not be resolved or inspected (for example a container's upper directory, which lives
    /// outside this mount namespace).
    pub memory_backed_assumed: bool,
    /// Filesystem allocation fragment/cluster size. Logical spill byte counts exclude this
    /// per-file rounding and filesystem metadata; the producer reserves those separately.
    pub allocation_unit_bytes: u64,
}

impl Volume {
    /// `disk`, `memory_backed` or `memory_backed_assumed`, as reported in diagnostics.
    pub(crate) fn class(&self) -> &'static str {
        match (self.memory_backed, self.memory_backed_assumed) {
            (false, _) => "disk",
            (true, false) => "memory_backed",
            (true, true) => "memory_backed_assumed",
        }
    }
}

/// What `/proc/self/mountinfo` says about the mount holding a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MountUpper {
    /// The mount is not an overlay.
    NotOverlay,
    /// An overlay whose writable upper directory is this path.
    Upper(PathBuf),
    /// An overlay without a readable `upperdir` super option, or no mount holds the path.
    Unresolved,
}

/// Undo the kernel's octal escaping of mountinfo fields (`\040` for a space and so on).
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 4 <= bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let value =
                (bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0');
            out.push(value);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The upper layer of the mount holding the absolute, canonical `path`, from mountinfo text:
/// the deepest mount point containing it (a later line wins, as later mounts cover earlier
/// ones), and for an overlay its `upperdir` super option.
pub(crate) fn overlay_upper(mountinfo: &str, path: &Path) -> MountUpper {
    let mut best: Option<(usize, &str)> = None;
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let Some(mount_point) = fields.get(4) else {
            continue;
        };
        let mount_point = PathBuf::from(unescape(mount_point));
        if !path.starts_with(&mount_point) {
            continue;
        }
        let depth = mount_point.components().count();
        if best.is_none_or(|(d, _)| depth >= d) {
            best = Some((depth, line));
        }
    }
    let Some((_, line)) = best else {
        return MountUpper::Unresolved;
    };
    // Optional fields end at " - "; then filesystem type, source and super options.
    let Some((_, tail)) = line.split_once(" - ") else {
        return MountUpper::Unresolved;
    };
    let mut tail = tail.split(' ');
    if tail.next() != Some("overlay") {
        return MountUpper::NotOverlay;
    }
    let options = tail.nth(1).unwrap_or("");
    options
        .split(',')
        .find_map(|option| option.strip_prefix("upperdir="))
        .filter(|upper| !upper.is_empty())
        .map_or(MountUpper::Unresolved, |upper| {
            MountUpper::Upper(PathBuf::from(unescape(upper)))
        })
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
        let (memory_backed, memory_backed_assumed) = memory_backed(&path)?;
        Ok(Volume {
            available_bytes: (stat.f_bavail as u64).saturating_mul(unit),
            memory_backed,
            memory_backed_assumed,
            allocation_unit_bytes: unit,
        })
    }

    /// Whether the filesystem holding `path` is RAM, and whether that was assumed. An
    /// overlay is classified by its writable upper layer (D8: a tmpfs upper layer is RAM, a
    /// disk upper layer is disk); an upper layer that cannot be resolved or inspected is
    /// assumed to be RAM, failing safe.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn memory_backed(path: &std::ffi::CStr) -> io::Result<(bool, bool)> {
        use std::os::unix::ffi::OsStrExt;
        let mut kind = fs_type(path)?;
        let mut path = PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes()));
        // An overlay's upper layer may itself be an overlay; a few levels suffice.
        for _ in 0..4 {
            match kind {
                FsType::Ram => return Ok((true, false)),
                FsType::Other => return Ok((false, false)),
                FsType::Overlay => {}
            }
            let upper = std::fs::canonicalize(&path).ok().and_then(|canonical| {
                let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
                match overlay_upper(&mountinfo, &canonical) {
                    MountUpper::Upper(upper) => Some(upper),
                    _ => None,
                }
            });
            let Some(upper) = upper else {
                return Ok((true, true));
            };
            kind = match c_path(&upper).and_then(|c| fs_type(&c)) {
                Ok(kind) => kind,
                Err(_) => return Ok((true, true)),
            };
            path = upper;
        }
        Ok((true, true))
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    enum FsType {
        Ram,
        Overlay,
        Other,
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[allow(clippy::unnecessary_cast)]
    fn fs_type(path: &std::ffi::CStr) -> io::Result<FsType> {
        const TMPFS_MAGIC: u64 = 0x0102_1994;
        const RAMFS_MAGIC: u64 = 0x8584_58f6;
        const OVERLAYFS_MAGIC: u64 = 0x794c_7630;
        // SAFETY: statfs is a plain C output record, zero initialization is valid.
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: the CStr is terminated and stat is writable for the full ABI record.
        if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // f_type is signed and 32-bit on some targets; compare the magic's low word.
        Ok(match stat.f_type as u64 & 0xffff_ffff {
            TMPFS_MAGIC | RAMFS_MAGIC => FsType::Ram,
            OVERLAYFS_MAGIC => FsType::Overlay,
            _ => FsType::Other,
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn memory_backed(_: &std::ffi::CStr) -> io::Result<(bool, bool)> {
        Ok((false, false))
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
            memory_backed_assumed: false,
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
            memory_backed_assumed: false,
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

#[cfg(test)]
mod mountinfo_tests {
    use super::*;

    const ROOT: &str = "21 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw";
    const TMP: &str = "30 21 0:27 / /tmp rw,nosuid,nodev shared:5 - tmpfs tmpfs rw,size=32G";
    /// An overlay whose upper and work directories are on the tmpfs above.
    const ON_TMPFS: &str = "41 21 0:40 / /mnt/merged rw,relatime shared:9 - overlay overlay \
rw,lowerdir=/srv/lower,upperdir=/tmp/ov/upper,workdir=/tmp/ov/work";
    /// Docker's root overlay with a disk-backed upper directory.
    const ON_EXT4: &str =
        "52 21 0:51 / /var/lib/docker/overlay2/abc/merged rw,relatime - overlay overlay \
rw,lowerdir=/var/lib/docker/overlay2/l/X:/var/lib/docker/overlay2/l/Y,\
upperdir=/var/lib/docker/overlay2/abc/diff,workdir=/var/lib/docker/overlay2/abc/work";
    /// A read-only overlay (lower layers only): there is no upper layer to inspect.
    const NO_UPPER: &str = "60 21 0:60 / /ro rw - overlay overlay ro,lowerdir=/a:/b";

    fn info(lines: &[&str]) -> String {
        lines.join("\n") + "\n"
    }

    #[test]
    fn overlay_upper_layers_are_resolved_from_mountinfo() {
        let all = info(&[ROOT, TMP, ON_TMPFS, ON_EXT4, NO_UPPER]);
        assert_eq!(
            overlay_upper(&all, Path::new("/mnt/merged/out/work")),
            MountUpper::Upper("/tmp/ov/upper".into())
        );
        assert_eq!(
            overlay_upper(&all, Path::new("/var/lib/docker/overlay2/abc/merged/tmp/x")),
            MountUpper::Upper("/var/lib/docker/overlay2/abc/diff".into())
        );
        assert_eq!(
            overlay_upper(&all, Path::new("/ro/x")),
            MountUpper::Unresolved
        );
        // The deepest mount wins; plain filesystems are not overlays.
        assert_eq!(
            overlay_upper(&all, Path::new("/tmp/x")),
            MountUpper::NotOverlay
        );
        assert_eq!(
            overlay_upper(&all, Path::new("/home/x")),
            MountUpper::NotOverlay
        );
        // A mount point that only shares a name prefix does not hold the path.
        assert_eq!(
            overlay_upper(&all, Path::new("/mnt/merged2/x")),
            MountUpper::NotOverlay
        );
        // No mountinfo line holds the path: unresolved.
        assert_eq!(
            overlay_upper(&info(&[TMP]), Path::new("/home/x")),
            MountUpper::Unresolved
        );
        // A later mount over the same point covers the earlier one.
        let covered = info(&[
            ROOT,
            ON_TMPFS,
            "70 41 0:70 / /mnt/merged rw - ext4 /dev/sdb1 rw",
        ]);
        assert_eq!(
            overlay_upper(&covered, Path::new("/mnt/merged/x")),
            MountUpper::NotOverlay
        );
        // Octal escapes in mount points and options.
        let spaced = info(&[
            ROOT,
            "80 21 0:80 / /mnt/my\\040disk rw - overlay overlay rw,lowerdir=/l,upperdir=/tmp/up\\054per,workdir=/tmp/w",
        ]);
        assert_eq!(
            overlay_upper(&spaced, Path::new("/mnt/my disk/f")),
            MountUpper::Upper("/tmp/up,per".into())
        );
    }

    #[test]
    fn volume_classes_name_assumed_memory() {
        let volume = |memory_backed, memory_backed_assumed| Volume {
            available_bytes: 0,
            memory_backed,
            memory_backed_assumed,
            allocation_unit_bytes: 4096,
        };
        assert_eq!(volume(false, false).class(), "disk");
        assert_eq!(volume(true, false).class(), "memory_backed");
        assert_eq!(volume(true, true).class(), "memory_backed_assumed");
    }
}
