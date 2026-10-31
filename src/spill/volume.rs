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
    /// How `memory_backed` was established.
    pub classification: Classification,
    /// The mount holding the path was found by path prefix in mountinfo order, because the
    /// kernel did not report the path's mount ID (`statx` `STATX_MNT_ID`, Linux 5.8+).
    pub mount_by_prefix: bool,
    /// Filesystem allocation fragment/cluster size. Logical spill byte counts exclude this
    /// per-file rounding and filesystem metadata; the producer reserves those separately.
    pub allocation_unit_bytes: u64,
}

/// Declares storage whose kind cannot be resolved (an overlay whose writable upper layer is
/// outside this mount namespace, as inside most containers) disk-backed: `1` declares it,
/// unset or `0` keeps the fail-safe memory-backed charge. Storage detected as tmpfs or ramfs
/// is never reclassified.
pub(crate) const STORAGE_DISK_ENV: &str = "RUSTY_TILES_STORAGE_DISK";

/// How a volume's `memory_backed` was established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Classification {
    /// The filesystem type, or an overlay's writable upper layer, was inspected.
    Detected,
    /// An overlay whose upper layer could not be resolved or inspected: memory-backed, to
    /// fail safe.
    AssumedMemory,
    /// Such an overlay, declared disk-backed by [`STORAGE_DISK_ENV`].
    DeclaredDisk,
}

/// What inspecting a path's filesystem established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Detection {
    /// tmpfs or ramfs; `huge` when the mount may back file data with transparent huge pages,
    /// so one write can allocate a whole huge page.
    Ram {
        huge: bool,
    },
    Disk,
    /// An overlay whose upper layer could not be resolved or inspected.
    Unresolved,
}

/// `memory_backed` and its classification from a detection and the [`STORAGE_DISK_ENV`]
/// value. Only an unresolved detection consults the declaration.
pub(crate) fn classify(
    detection: Detection,
    declared: Option<&std::ffi::OsStr>,
) -> io::Result<(bool, Classification)> {
    Ok(match detection {
        Detection::Ram { .. } => (true, Classification::Detected),
        Detection::Disk => (false, Classification::Detected),
        Detection::Unresolved => match declared.map(|v| v.to_str()) {
            None | Some(Some("0")) => (true, Classification::AssumedMemory),
            Some(Some("1")) => (false, Classification::DeclaredDisk),
            Some(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{STORAGE_DISK_ENV} must be 1 (declare unresolvable storage disk) or 0"
                    ),
                ))
            }
        },
    })
}

impl Volume {
    /// `disk`, `memory_backed`, `memory_backed_assumed` or `disk_declared`, as reported in
    /// diagnostics.
    pub(crate) fn class(&self) -> &'static str {
        match (self.memory_backed, self.classification) {
            (_, Classification::AssumedMemory) => "memory_backed_assumed",
            (_, Classification::DeclaredDisk) => "disk_declared",
            (false, Classification::Detected) => "disk",
            (true, Classification::Detected) => "memory_backed",
        }
    }
}

/// One `/proc/self/mountinfo` line: mount ID, mount point, filesystem type and super options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mount {
    pub id: u64,
    pub mount_point: PathBuf,
    pub fstype: String,
    pub options: String,
}

impl Mount {
    /// The unescaped value of super option `key` (`upperdir`, `huge`, ...).
    pub(crate) fn option(&self, key: &str) -> Option<String> {
        self.options.split(',').find_map(|option| {
            option
                .strip_prefix(key)
                .and_then(|rest| rest.strip_prefix('='))
                .map(unescape)
        })
    }
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

/// The lines of mountinfo text, in order; malformed lines are skipped.
pub(crate) fn parse_mountinfo(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            // Optional fields end at " - "; then filesystem type, source and super options.
            let (head, tail) = line.split_once(" - ")?;
            let head: Vec<&str> = head.split(' ').collect();
            let mut tail = tail.split(' ');
            Some(Mount {
                id: head.first()?.parse().ok()?,
                mount_point: PathBuf::from(unescape(head.get(4)?)),
                fstype: tail.next()?.to_owned(),
                options: tail.nth(1).unwrap_or("").to_owned(),
            })
        })
        .collect()
}

/// The mount holding the canonical `path`. With the kernel's mount ID for the path it is the
/// line with that ID, exactly. Without one it is the last line, in mountinfo order, whose
/// mount point contains `path`: a later mount at an ancestor covers earlier, deeper ones
/// (which stay listed but hidden), and a later, deeper mount lies inside the visible tree.
/// That order holds for mounts made in place; moved or propagated mounts can break it, which
/// is why the fallback is reported.
pub(crate) fn select_mount<'a>(
    mounts: &'a [Mount],
    id: Option<u64>,
    path: &Path,
) -> Option<&'a Mount> {
    match id {
        Some(id) => mounts.iter().find(|m| m.id == id),
        None => mounts
            .iter()
            .rev()
            .find(|m| path.starts_with(&m.mount_point)),
    }
}

/// Whether a tmpfs mount may back file data written with `write(2)` by transparent huge
/// pages: its `huge=` option (`always` and `within_size` may; `never`, `advise`, which needs
/// an `madvise`d mapping, and an absent option do not), unless the global
/// `/sys/kernel/mm/transparent_hugepage/shmem_enabled` selection is `force` (every mount may)
/// or `deny` (none may). An unrecognized option value may, to fail safe.
pub(crate) fn huge_pages(mount_huge: Option<&str>, global: Option<&str>) -> bool {
    let selected = global.and_then(|text| {
        text.split_whitespace()
            .find_map(|word| word.strip_prefix('[')?.strip_suffix(']'))
    });
    match selected {
        Some("force") => true,
        Some("deny") => false,
        _ => !matches!(mount_huge, None | Some("never") | Some("advise")),
    }
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
        let (detection, mount_by_prefix) = detect(&path)?;
        let (memory_backed, classification) =
            classify(detection, std::env::var_os(STORAGE_DISK_ENV).as_deref())?;
        // A huge-page tmpfs can allocate a whole huge page for one written byte.
        let allocation_unit_bytes = match detection {
            Detection::Ram { huge: true } => unit.max(huge_page_bytes()),
            _ => unit,
        };
        Ok(Volume {
            available_bytes: (stat.f_bavail as u64).saturating_mul(unit),
            memory_backed,
            classification,
            mount_by_prefix,
            allocation_unit_bytes,
        })
    }

    /// The transparent huge page size (PMD), 2 MiB when the kernel does not report it.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn huge_page_bytes() -> u64 {
        std::fs::read_to_string("/sys/kernel/mm/transparent_hugepage/hpage_pmd_size")
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .filter(|&bytes: &u64| bytes > 0)
            .unwrap_or(2 << 20)
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn huge_page_bytes() -> u64 {
        2 << 20
    }

    /// What the filesystem holding `path` is, and whether a mount was found by path prefix.
    /// tmpfs is classified with its huge-page policy, read from its exact mount (by `statx`
    /// mount ID; mountinfo order when unavailable); a tmpfs whose mount cannot be found may
    /// use huge pages, to fail safe. An overlay is classified by its writable upper layer
    /// (D8: a tmpfs upper layer is RAM, a disk upper layer is disk); an upper layer that
    /// cannot be resolved or inspected is [`Detection::Unresolved`].
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn detect(path: &std::ffi::CStr) -> io::Result<(Detection, bool)> {
        use std::os::unix::ffi::OsStrExt;
        let mut path = PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes()));
        let mut kind = fs_type(&c_path(&path)?)?;
        let mut by_prefix = false;
        let mut mounts: Option<Vec<Mount>> = None;
        // An overlay's upper layer may itself be an overlay; a few levels suffice.
        for _ in 0..4 {
            match kind {
                FsType::Ramfs => return Ok((Detection::Ram { huge: false }, by_prefix)),
                FsType::Other => return Ok((Detection::Disk, by_prefix)),
                FsType::Tmpfs | FsType::Overlay => {}
            }
            let mount = locate(&path, &mut mounts, &mut by_prefix);
            if let FsType::Tmpfs = kind {
                let global =
                    std::fs::read_to_string("/sys/kernel/mm/transparent_hugepage/shmem_enabled")
                        .ok();
                let huge = match mount {
                    Some(mount) if mount.fstype == "tmpfs" => {
                        huge_pages(mount.option("huge").as_deref(), global.as_deref())
                    }
                    _ => true,
                };
                return Ok((Detection::Ram { huge }, by_prefix));
            }
            let upper = mount
                .filter(|mount| mount.fstype == "overlay")
                .and_then(|mount| mount.option("upperdir"))
                .filter(|upper| !upper.is_empty());
            let Some(upper) = upper else {
                return Ok((Detection::Unresolved, by_prefix));
            };
            let upper = PathBuf::from(upper);
            kind = match c_path(&upper).and_then(|c| fs_type(&c)) {
                Ok(kind) => kind,
                Err(_) => return Ok((Detection::Unresolved, by_prefix)),
            };
            path = upper;
        }
        Ok((Detection::Unresolved, by_prefix))
    }

    /// The mount holding `path`, from one read of mountinfo per inspection.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn locate(path: &Path, mounts: &mut Option<Vec<Mount>>, by_prefix: &mut bool) -> Option<Mount> {
        let canonical = std::fs::canonicalize(path).ok()?;
        let id = mount_id(&canonical);
        *by_prefix |= id.is_none();
        if mounts.is_none() {
            *mounts = Some(parse_mountinfo(
                &std::fs::read_to_string("/proc/self/mountinfo").ok()?,
            ));
        }
        select_mount(mounts.as_deref()?, id, &canonical).cloned()
    }

    /// The kernel's mount ID for `path` (`statx` `STATX_MNT_ID`), when it reports one.
    #[cfg(any(target_env = "gnu", target_os = "android"))]
    fn mount_id(path: &Path) -> Option<u64> {
        let c = c_path(path).ok()?;
        // SAFETY: statx is a plain C output record, zero initialization is valid.
        let mut stat: libc::statx = unsafe { std::mem::zeroed() };
        // SAFETY: the CString is terminated and stat is writable for the full ABI record.
        let status =
            unsafe { libc::statx(libc::AT_FDCWD, c.as_ptr(), 0, libc::STATX_MNT_ID, &mut stat) };
        (status == 0 && stat.stx_mask & libc::STATX_MNT_ID != 0).then_some(stat.stx_mnt_id)
    }

    #[cfg(all(
        target_os = "linux",
        not(any(target_env = "gnu", target_os = "android"))
    ))]
    fn mount_id(_: &Path) -> Option<u64> {
        None
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    enum FsType {
        Tmpfs,
        Ramfs,
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
            TMPFS_MAGIC => FsType::Tmpfs,
            RAMFS_MAGIC => FsType::Ramfs,
            OVERLAYFS_MAGIC => FsType::Overlay,
            _ => FsType::Other,
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn detect(_: &std::ffi::CStr) -> io::Result<(Detection, bool)> {
        Ok((Detection::Disk, false))
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
            classification: Classification::Detected,
            mount_by_prefix: false,
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
            classification: Classification::Detected,
            mount_by_prefix: false,
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

    /// The upper layer an overlay path resolves to, through the production selection.
    fn upper(text: &str, id: Option<u64>, path: &str) -> Option<String> {
        let mounts = parse_mountinfo(text);
        let mount = select_mount(&mounts, id, Path::new(path))?;
        (mount.fstype == "overlay")
            .then(|| mount.option("upperdir"))
            .flatten()
    }

    #[test]
    fn overlay_upper_layers_are_resolved_from_mountinfo() {
        let all = info(&[ROOT, TMP, ON_TMPFS, ON_EXT4, NO_UPPER]);
        assert_eq!(
            upper(&all, Some(41), "/mnt/merged/out/work").as_deref(),
            Some("/tmp/ov/upper")
        );
        assert_eq!(
            upper(&all, None, "/mnt/merged/out/work").as_deref(),
            Some("/tmp/ov/upper")
        );
        assert_eq!(
            upper(&all, None, "/var/lib/docker/overlay2/abc/merged/tmp/x").as_deref(),
            Some("/var/lib/docker/overlay2/abc/diff")
        );
        assert_eq!(upper(&all, Some(60), "/ro/x"), None, "no upper layer");
        let mounts = parse_mountinfo(&all);
        let fstype =
            |id, path| select_mount(&mounts, id, Path::new(path)).map(|m| m.fstype.clone());
        assert_eq!(fstype(None, "/tmp/x").as_deref(), Some("tmpfs"));
        assert_eq!(fstype(None, "/home/x").as_deref(), Some("ext4"));
        // A mount point that only shares a name prefix does not hold the path.
        assert_eq!(fstype(None, "/mnt/merged2/x").as_deref(), Some("ext4"));
        // An ID the table does not hold resolves nothing (never a guess).
        assert_eq!(fstype(Some(999), "/tmp/x"), None);
        // Octal escapes in mount points and options.
        let spaced = info(&[
            ROOT,
            "80 21 0:80 / /mnt/my\\040disk rw - overlay overlay rw,lowerdir=/l,upperdir=/tmp/up\\054per,workdir=/tmp/w",
        ]);
        assert_eq!(
            upper(&spaced, None, "/mnt/my disk/f").as_deref(),
            Some("/tmp/up,per")
        );
    }

    /// Review r2: a disk-upper overlay mounted at `merged/deep`, then a tmpfs-upper overlay
    /// mounted over `merged`, which hides the deeper mount (it stays listed). Writes through
    /// `merged/deep` land in the tmpfs upper layer. The kernel's mount ID for that path names
    /// the covering mount; without IDs, mountinfo order does too. Textual depth alone picked
    /// the hidden disk mount and classified the path as disk.
    #[test]
    fn a_covering_mount_is_selected_over_a_hidden_deeper_one() {
        let covered = info(&[
            ROOT,
            "500 21 0:90 / /dev/shm/p/merged/deep rw - overlay overlay \
rw,lowerdir=/dev/shm/p/lower,upperdir=/home/u/disk-upper/upper,workdir=/home/u/disk-upper/work",
            "501 21 0:91 / /dev/shm/p/merged rw - overlay overlay \
rw,lowerdir=/dev/shm/p/new-lower,upperdir=/dev/shm/p/upper,workdir=/dev/shm/p/work",
        ]);
        let path = "/dev/shm/p/merged/deep/file";
        assert_eq!(
            upper(&covered, Some(501), path).as_deref(),
            Some("/dev/shm/p/upper")
        );
        assert_eq!(
            upper(&covered, None, path).as_deref(),
            Some("/dev/shm/p/upper")
        );
        // The hidden mount is still found by its own ID (e.g. through a held descriptor).
        assert_eq!(
            upper(&covered, Some(500), path).as_deref(),
            Some("/home/u/disk-upper/upper")
        );
        // A deeper mount made after its ancestor is visible and selected.
        let nested = info(&[ROOT, TMP, "600 30 0:95 / /tmp/deep rw - ext4 /dev/sdb1 rw"]);
        let mounts = parse_mountinfo(&nested);
        assert_eq!(
            select_mount(&mounts, None, Path::new("/tmp/deep/x"))
                .unwrap()
                .id,
            600
        );
        assert_eq!(
            select_mount(&mounts, None, Path::new("/tmp/x")).unwrap().id,
            30
        );
    }

    /// Huge-page policy: per-mount `huge=`, overridden by the global force/deny selection.
    #[test]
    fn huge_page_policy_follows_the_mount_and_the_global_override() {
        let advise = Some("always within_size [advise] never deny force");
        let force = Some("always within_size advise never deny [force]");
        let deny = Some("always within_size advise never [deny] force");
        for (mount, expected) in [
            (None, false),
            (Some("never"), false),
            (Some("advise"), false),
            (Some("always"), true),
            (Some("within_size"), true),
            (Some("something_new"), true),
        ] {
            assert_eq!(huge_pages(mount, advise), expected, "{mount:?}");
            assert_eq!(huge_pages(mount, None), expected, "{mount:?} without sysfs");
            assert!(huge_pages(mount, force), "{mount:?} forced");
            assert!(!huge_pages(mount, deny), "{mount:?} denied");
        }
        let tmpfs = parse_mountinfo(&info(&[
            "30 21 0:27 / /tmp rw - tmpfs tmpfs rw,inode64,huge=within_size",
            "31 21 0:28 / /run rw - tmpfs tmpfs rw,inode64",
        ]));
        assert_eq!(tmpfs[0].option("huge").as_deref(), Some("within_size"));
        assert_eq!(tmpfs[1].option("huge"), None);
    }

    /// The fail-safe charge and its declared override: only unresolved storage consults
    /// the declaration, so detected tmpfs/ramfs stays memory-backed whatever it says.
    #[test]
    fn only_unresolved_storage_can_be_declared_disk() {
        use std::ffi::OsStr;
        let class = |detection, declared: Option<&str>| {
            classify(detection, declared.map(OsStr::new)).map(|(memory_backed, classification)| {
                Volume {
                    available_bytes: 0,
                    memory_backed,
                    classification,
                    mount_by_prefix: false,
                    allocation_unit_bytes: 4096,
                }
                .class()
            })
        };
        for declared in [None, Some("0"), Some("1")] {
            assert_eq!(
                class(Detection::Ram { huge: false }, declared).unwrap(),
                "memory_backed"
            );
            assert_eq!(class(Detection::Disk, declared).unwrap(), "disk");
        }
        assert_eq!(
            class(Detection::Unresolved, None).unwrap(),
            "memory_backed_assumed"
        );
        assert_eq!(
            class(Detection::Unresolved, Some("0")).unwrap(),
            "memory_backed_assumed"
        );
        let declared = classify(Detection::Unresolved, Some(OsStr::new("1"))).unwrap();
        assert_eq!(declared, (false, Classification::DeclaredDisk));
        assert_eq!(
            class(Detection::Unresolved, Some("1")).unwrap(),
            "disk_declared"
        );
        for bad in ["yes", "true", "", "2"] {
            let error = class(Detection::Unresolved, Some(bad)).unwrap_err();
            assert!(error.to_string().contains(STORAGE_DISK_ENV), "{error}");
        }
    }
}
