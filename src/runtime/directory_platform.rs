//! Conservative local-filesystem support for exclusive directory publication.
//! Parent and mount topology must remain stable throughout the attempt.
use super::{JobError, JobErrorKind};
use std::{io, path::Path};

fn unsupported(message: &str) -> JobError {
    JobError::new(JobErrorKind::Unsupported, message)
}

pub(super) fn preflight(parent: &Path) -> Result<(), JobError> {
    implementation::preflight(parent)
}

pub(super) fn install(source: &Path, destination: &Path) -> io::Result<()> {
    implementation::install(source, destination)
}

/// Actual supported filesystem fixture; never silently skips publication tests.
#[cfg(test)]
pub(crate) fn test_directory() -> tempfile::TempDir {
    let candidate = tempfile::tempdir().expect("create directory test fixture");
    match preflight(candidate.path()) {
        Ok(()) => candidate,
        Err(default_error) => {
            #[cfg(target_os = "linux")]
            {
                let fallback = tempfile::tempdir_in("/dev/shm").expect(
                    "directory publication tests need supported storage; /dev/shm unavailable",
                );
                preflight(fallback.path()).unwrap_or_else(|error| panic!(
                    "directory publication tests require supported storage: default={default_error:?}; /dev/shm={error:?}"
                ));
                fallback
            }
            #[cfg(not(target_os = "linux"))]
            panic!(
                "directory publication tests require supported local storage: {default_error:?}"
            );
        }
    }
}

#[cfg(unix)]
fn c_path(path: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

#[cfg(target_os = "linux")]
mod implementation {
    use super::*;
    pub(super) fn preflight(parent: &Path) -> Result<(), JobError> {
        let path =
            c_path(parent).map_err(|e| JobError::io("inspect directory volume", parent, e))?;
        // SAFETY: statfs is a plain C output record, zero initialization is valid.
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: the CString is terminated and stat is writable for the full ABI record.
        if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
            return Err(JobError::io(
                "inspect directory volume",
                parent,
                io::Error::last_os_error(),
            ));
        }
        // ext2/ext3 share ext4's magic; all gained exclusive rename by 4.9.
        if !matches!(
            stat.f_type as u64,
            0xef53 | 0x9123683e | 0x01021994 | 0x58465342
        ) {
            return Err(unsupported("directory publication requires a supported local Linux filesystem (ext, btrfs, tmpfs, xfs)"));
        }
        // SAFETY: utsname is a plain C output record.
        let mut name: libc::utsname = unsafe { std::mem::zeroed() };
        // SAFETY: name points to a writable full utsname record.
        if unsafe { libc::uname(&mut name) } != 0 {
            return Err(JobError::io(
                "inspect Linux version",
                parent,
                io::Error::last_os_error(),
            ));
        }
        // SAFETY: successful uname terminates its fixed-size release string.
        let release = unsafe { std::ffi::CStr::from_ptr(name.release.as_ptr()) }.to_string_lossy();
        let mut parts = release.split('.');
        let version = parts
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .zip(parts.next().and_then(|v| v.parse::<u32>().ok()));
        if version.is_none_or(|v| v < (4, 9)) {
            return Err(unsupported(
                "directory publication requires Linux 4.9 or newer",
            ));
        }
        // Empty source cannot name an object. This checks syscall admission,
        // not filesystem support, which the conservative allowlist establishes.
        let empty = c"";
        // SAFETY: path pointers are terminated and remain live; syscall arguments
        // match the platform renameat2 ABI. No userspace buffer is written.
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                empty.as_ptr(),
                libc::AT_FDCWD,
                empty.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != -1 || io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT) {
            return Err(unsupported(
                "exclusive directory rename syscall is unavailable",
            ));
        }
        Ok(())
    }
    pub(super) fn install(source: &Path, destination: &Path) -> io::Result<()> {
        let source = c_path(source)?;
        let destination = c_path(destination)?;
        // SAFETY: path pointers are terminated and remain live; syscall arguments
        // match the platform renameat2 ABI. No userspace buffer is written.
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(target_os = "macos")]
mod implementation {
    use super::*;
    #[repr(C)]
    struct Capabilities {
        length: u32,
        capabilities: [u32; 4],
        valid: [u32; 4],
    }
    pub(super) fn preflight(parent: &Path) -> Result<(), JobError> {
        let path =
            c_path(parent).map_err(|e| JobError::io("inspect directory volume", parent, e))?;
        // SAFETY: statfs is a plain C output record, zero initialization is valid.
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: the CString is terminated and stat is writable for the full ABI record.
        if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
            return Err(JobError::io(
                "inspect directory volume",
                parent,
                io::Error::last_os_error(),
            ));
        }
        // SAFETY: successful statfs terminates its filesystem name.
        let kind = unsafe { std::ffi::CStr::from_ptr(stat.f_fstypename.as_ptr()) }.to_bytes();
        if stat.f_flags & libc::MNT_LOCAL as u32 == 0 || !matches!(kind, b"apfs" | b"hfs") {
            return Err(unsupported(
                "directory publication requires local APFS or HFS+",
            ));
        }
        // SAFETY: attrlist contains only integers; zero initializes unused groups.
        let mut attrs: libc::attrlist = unsafe { std::mem::zeroed() };
        attrs.bitmapcount = libc::ATTR_BIT_MAP_COUNT as u16;
        attrs.volattr = libc::ATTR_VOL_INFO | libc::ATTR_VOL_CAPABILITIES;
        // SAFETY: capabilities contains only u32 values. Its repr(C) layout matches
        // the 4-byte-aligned getattrlist length followed by vol_capabilities_attr_t.
        let mut caps: Capabilities = unsafe { std::mem::zeroed() };
        // SAFETY: statfs supplied a terminated volume-root path; attrs is complete,
        // and caps is writable for the advertised size. The returned length is checked.
        let result = unsafe {
            libc::getattrlist(
                stat.f_mntonname.as_ptr(),
                &mut attrs,
                (&mut caps as *mut Capabilities).cast(),
                std::mem::size_of::<Capabilities>(),
                0,
            )
        };
        if result != 0
            || caps.length as usize != std::mem::size_of::<Capabilities>()
            || caps.valid[1] & libc::VOL_CAP_INT_RENAME_EXCL == 0
            || caps.capabilities[1] & libc::VOL_CAP_INT_RENAME_EXCL == 0
        {
            return Err(unsupported(
                "directory volume does not advertise exclusive rename support",
            ));
        }
        Ok(())
    }
    pub(super) fn install(source: &Path, destination: &Path) -> io::Result<()> {
        let source = c_path(source)?;
        let destination = c_path(destination)?;
        // SAFETY: both CStrings remain live and contain terminated native paths.
        if unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) }
            == 0
        {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(windows)]
mod implementation {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    };
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumePathNameW(path: *const u16, root: *mut u16, size: u32) -> i32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_size: u32,
            serial: *mut u32,
            component: *mut u32,
            flags: *mut u32,
            filesystem: *mut u16,
            filesystem_size: u32,
        ) -> i32;
        fn SetFileInformationByHandle(
            handle: *mut c_void,
            class: i32,
            information: *const c_void,
            size: u32,
        ) -> i32;
    }
    fn wide(path: &Path) -> io::Result<Vec<u16>> {
        let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains NUL",
            ));
        }
        value.push(0);
        Ok(value)
    }
    pub(super) fn preflight(parent: &Path) -> Result<(), JobError> {
        let path = wide(parent).map_err(|e| JobError::io("inspect directory volume", parent, e))?;
        let mut root = vec![0u16; 32768];
        // SAFETY: path is terminated; root is writable for the passed element count.
        if unsafe { GetVolumePathNameW(path.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
            return Err(JobError::io(
                "inspect directory volume",
                parent,
                io::Error::last_os_error(),
            ));
        }
        // Fixed local volumes only; exclude remote providers and removable media.
        // SAFETY: successful GetVolumePathNameW supplied a terminated root string.
        if unsafe { GetDriveTypeW(root.as_ptr()) } != 3 {
            return Err(unsupported(
                "directory publication requires a fixed local NTFS volume",
            ));
        }
        let mut filesystem = [0u16; 32];
        // SAFETY: root is terminated, optional outputs are NULL, and filesystem
        // is writable for its advertised UTF-16 element count.
        if unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                filesystem.as_mut_ptr(),
                filesystem.len() as u32,
            )
        } == 0
        {
            return Err(JobError::io(
                "inspect directory filesystem",
                parent,
                io::Error::last_os_error(),
            ));
        }
        let len = filesystem
            .iter()
            .position(|&v| v == 0)
            .unwrap_or(filesystem.len());
        if String::from_utf16_lossy(&filesystem[..len]) != "NTFS" {
            return Err(unsupported("directory publication requires NTFS"));
        }
        Ok(())
    }
    #[repr(C)]
    struct RenameInfo {
        replace: u8,
        root: *mut c_void,
        length: u32,
        name: [u16; 1],
    }
    pub(super) fn install(source: &Path, destination: &Path) -> io::Result<()> {
        let file = std::fs::OpenOptions::new()
            .access_mode(0x00010000)
            .share_mode(7)
            .custom_flags(0x02000000 | 0x00200000)
            .open(source)?;
        let name = wide(destination)?;
        let offset = std::mem::offset_of!(RenameInfo, name);
        let size = offset
            .checked_add((name.len() - 1) * 2)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename path too long"))?;
        let allocation = size.max(std::mem::size_of::<RenameInfo>());
        let mut storage = vec![0usize; allocation.div_ceil(std::mem::size_of::<usize>())];
        let info = storage.as_mut_ptr().cast::<RenameInfo>();
        // SAFETY: repr(C) reproduces FILE_RENAME_INFO (BOOLEAN, pointer-aligned
        // HANDLE, DWORD, UTF-16 flexible array). usize storage guarantees pointer
        // alignment and covers both the struct and complete filename. All padding
        // starts zeroed. Filename copy is in bounds and does not overlap. The file
        // owns a valid directory handle for the synchronous call; storage and name
        // remain live. Class 3 is FileRenameInfo, so replace FALSE means no clobber.
        unsafe {
            (*info).replace = 0;
            (*info).root = std::ptr::null_mut();
            (*info).length = ((name.len() - 1) * 2) as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                storage.as_mut_ptr().cast::<u8>().add(offset).cast::<u16>(),
                name.len() - 1,
            );
            if SetFileInformationByHandle(
                file.as_raw_handle(),
                3,
                storage.as_ptr().cast(),
                size as u32,
            ) != 0
            {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod implementation {
    use super::*;
    pub(super) fn preflight(_: &Path) -> Result<(), JobError> {
        Err(unsupported(
            "exclusive directory publication is unsupported on this platform",
        ))
    }
    pub(super) fn install(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "exclusive directory publication is unsupported",
        ))
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn unsupported_procfs_is_rejected() {
        let parent = Path::new("/proc");
        let failure = preflight(parent).unwrap_err();
        assert_eq!(failure.kind(), JobErrorKind::Unsupported);
    }
    #[test]
    fn exclusive_install_preserves_existing_targets() {
        let parent = test_directory();
        for nonempty in [false, true] {
            let source = parent.path().join("candidate");
            let target = parent.path().join("target");
            std::fs::create_dir(&source).unwrap();
            std::fs::write(source.join("new"), b"complete").unwrap();
            std::fs::create_dir(&target).unwrap();
            if nonempty {
                std::fs::write(target.join("existing"), b"preserve").unwrap();
            }
            assert!(install(&source, &target).is_err());
            assert!(source.join("new").is_file());
            assert!(!target.join("new").exists());
            if nonempty {
                assert_eq!(std::fs::read(target.join("existing")).unwrap(), b"preserve");
            }
            std::fs::remove_dir_all(source).unwrap();
            std::fs::remove_dir_all(target).unwrap();
        }
    }
    #[test]
    fn exclusive_install_exposes_complete_tree() {
        let parent = test_directory();
        let source = parent.path().join("candidate");
        let target = parent.path().join("target");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("complete"), b"payload").unwrap();
        install(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(std::fs::read(target.join("complete")).unwrap(), b"payload");
    }
    #[test]
    fn exclusive_install_preserves_file_created_after_preflight() {
        let parent = test_directory();
        let source = parent.path().join("candidate");
        let target = parent.path().join("target");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(&target, b"other writer").unwrap();
        assert!(install(&source, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"other writer");
        assert!(source.is_dir());
    }
    #[cfg(unix)]
    #[test]
    fn exclusive_install_preserves_dangling_symlink() {
        let parent = test_directory();
        let source = parent.path().join("candidate");
        let target = parent.path().join("target");
        std::fs::create_dir(&source).unwrap();
        std::os::unix::fs::symlink("missing", &target).unwrap();
        assert!(install(&source, &target).is_err());
        assert_eq!(std::fs::read_link(&target).unwrap(), Path::new("missing"));
        assert!(source.is_dir());
    }
}
