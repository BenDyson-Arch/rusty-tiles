# D1 exclusive directory publication platform evidence

The D1 contract requires an existing canonical output parent and stable parent
and mount namespace for the attempt. Staging and output reside in that parent.
Publication is a single namespace rename of a complete private tree; it never
uses an existence check followed by ordinary rename, copy, target removal, or
replacement. This does not promise crash durability or defend against unrelated
replacement of the parent or mounts. Existing files, empty/nonempty directories,
and symbolic links occupy the destination name and must be preserved.

Unsupported platform, filesystem, and missing exclusive-rename capabilities are
rejected before staging creation. The publication syscall can still fail due to
permissions, resource exhaustion, or changes after preflight. Its failure never
triggers an ordinary-rename fallback.

## Linux

Linux 4.9 or later, local ext family, btrfs, tmpfs and xfs. `statfs` checks the
filesystem magic. ext2/ext3/ext4 share a magic number, so the floor is 4.9 rather
than 4.0. A renameat2 call with an empty source checks syscall admission without
creating anything; ENOENT is expected. This is explicitly not a filesystem
capability probe. Publication calls `renameat2(RENAME_NOREPLACE)` through syscall.

[Linux rename manual](https://man7.org/linux/man-pages/man2/rename.2.html)
documents kernel/filesystem availability, EEXIST for occupied targets and EINVAL
for unsupported flags. It also documents ambiguous NFS failure after server
crashes, which motivates rejecting network and unknown filesystems.
[Official kernel VFS documentation](https://www.kernel.org/doc/html/latest/filesystems/vfs.html)
describes the VFS target check for local filesystems and mandates rejecting
unsupported flags. Overlay and FUSE are not supported by this conservative list.

## macOS

Only local APFS and HFS+ volumes. `statfs` identifies local supported formats and
the volume root; `getattrlist(ATTR_VOL_CAPABILITIES)` checks both the valid and
supported interface bits for `VOL_CAP_INT_RENAME_EXCL`. Unknown, unadvertised,
remote and unsupported volumes fail before staging. Publication uses
`renamex_np(RENAME_EXCL)`.

[Apple's official rename manual source](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/man/man2/rename.2)
documents EEXIST for an occupied destination and ENOTSUP for unsupported
filesystem flags. [Apple's getattrlist manual source](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/man/man2/getattrlist.2)
documents the exclusive rename capability and volume-root requirement.

## Windows

Only fixed local NTFS volumes. `GetVolumePathNameW`, `GetDriveTypeW` and
`GetVolumeInformationW` reject remote, removable and unknown filesystems before
staging. The directory is opened with DELETE access and BACKUP_SEMANTICS, and
`SetFileInformationByHandle(FileRenameInfo)` installs its full canonical target
name with `ReplaceIfExists=FALSE`. No copy or cross-volume fallback exists.

[Microsoft FILE_RENAME_INFO documentation](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info)
explicitly states that FALSE rejects an existing target.
[Microsoft filesystem rename specification](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/87f86c9b-6c2a-4803-84b7-131a74a434fa)
details directory rename and collision behavior.
[Microsoft GetVolumeInformationW documentation](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getvolumeinformationw)
provides filesystem identification. The namespace visibility claim relies on
local NTFS rename semantics; no claim extends to SMB, other remote providers,
ReFS or removable filesystems. Native Windows and macOS test execution remains
a required release-platform validation beyond Linux execution.
