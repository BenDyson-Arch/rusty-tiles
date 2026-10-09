# D1 completed-directory publication

D1 extends the accepted F0 Attempt to a completed directory with CreateNew only.
One attempt owns event admission, first failure, cancellation and the install
permission decision. The producer owns its formats, all member writers and
finalization. There is no converter identity in the runtime, generic artifact
registry, or nested public conversion call.

## Destination and support

The output parent must already exist. Preparation resolves that parent once,
rejects any existing destination (including symlinks), and checks the platform
and filesystem before creating private output. Source and destination paths are
bound before callbacks. The parent namespace and mount topology must remain
stable; this contract does not defend against unrelated actors renaming parent
components or modifying the private tree.

Supported combinations are Linux >=4.9 on ext/btrfs/tmpfs/xfs, macOS local
APFS/HFS+ advertising exclusive rename, and Windows fixed local NTFS. The exact
primitives and primary-source evidence are in
[platforms.md](../../bench/architecture_audit/directory/platforms.md).
Other platforms/filesystems return Unsupported before staging. Failures of a
previously admitted primitive remain failures; no ordinary rename or copy/delete
fallback weakens CreateNew. Filesystem admission is conservative and does not
claim to detect every policy restriction or later mount change.

## Ownership and commit

A prepared target creates one uniquely named private directory next to the
output. The tree is inaccessible through the final output name during
production. The producer closes all writers and native handles, finishes its
report and required observations, and closes event admission before consuming
the stage into a sealed directory. Sealed directories expose no writing API.

Publication obtains F0 permission immediately before the exclusive namespace
rename. Cancellation winning that decision prevents installation; after
permission, the installer owns the outcome. Success is the commit point: the
complete tree occupies the final name. A competing empty directory, nonempty
directory, file or symlink must not be replaced. Two concurrent CreateNew
attempts may yield at most one success.

This is a namespace visibility guarantee, not a crash durability or filesystem
snapshot promise. Files are flushed and synchronized by the pilot; D1 does not
claim a recursive directory fsync protocol. Default permissions come from
private staging (0700 subject to platform behavior on Unix), not copied source
permissions. Publication does not broaden them.

On precommit failure, explicitly remove the owned private tree. Failed cleanup
returns its retained path and a secondary diagnostic; Drop must not retry that
reported deletion. After successful rename, never delete the former staging
name, because a competitor may now own it. There is no required postcommit
producer work or success callback.

## Proof and stop condition

Exercise actual platform primitives with collision controls, a deterministic
competitor inserted immediately before installation, cancellation before and
after permission, concurrent attempts, and explicit install/cleanup failures.
The [RGB raster pilot](d1-raster-contract.md) must independently prove decoded
samples, placement, inventory and report facts, with malformed/unsupported
source, observer, write/finalizer and callback-CWD controls. Platform tests must
execute on admitted filesystems; unavailable support is not a passing proof.

D1 completes once those obligations pass for this profile. D2 replacement,
hold-old/install/restore states and typed recovery remain open in #118. The
broader legacy raster/terrain routes remain unaccepted under #124; this slice
does not silently migrate or remove their additional behavior. #82 feasibility
informs the reader boundary, but no GDAL replacement is part of D1.
