# D2 directory replacement and recovery

D2 extends the completed-tree publisher and real raster consumer from D1 with
`OutputPolicy::Replace`. The producer still closes all writers, native handles,
reports and required observations before sealing. One F0 Attempt grants
publication permission before any existing output is moved. Cancellation before
permission prevents replacement; after permission, publication owns the outcome.

## Admission and permissions

CreateNew retains the D1 contract. Replace uses the same supported local
filesystem matrix and exclusive rename primitives described in
[platforms.md](../../bench/architecture_audit/directory/platforms.md). The parent
must exist and its namespace and mount topology must remain stable. Private
staging and recovery containers must not be modified by unrelated actors.

Replace authorizes retiring whichever final-path entry exists at the hold
operation: a directory, regular file or symlink itself. It does not follow a
symlink to replace or delete its referent. This is not a comparison against the
entry inspected during preparation. The new output is always the completed
directory, with the private staging permissions described by D1.

## State transitions and visibility

The publisher allocates a private recovery container beside the destination.
Before holding any old output, it disables automatic cleanup of that container.
Its `previous` entry is a recovery asset until a new output commits.

1. Hold the current final-path entry at `previous`, using exclusive rename.
   An absent source means there is no previous output. Other hold failures stop
   publication before candidate installation.
2. Install the sealed candidate at the final path with exclusive rename. This
   successful rename is the sole commit point.
3. If installation fails after holding old output, restore `previous` to the
   final path with exclusive rename. Never overwrite a competitor to restore.

Readers may observe the final name absent between hold and install, or between
hold and successful restoration. No uninterrupted visibility, crash atomicity,
recursive durability or automatic recovery after process termination is claimed.
There is no ordinary-rename or copy/delete fallback.

## Failure and cleanup ownership

Failed installation remains the primary error. Successful restoration recovers
the exact held entry, including its inventory, bytes and permissions. Failed
restoration adds the restore error as a secondary diagnostic and returns typed
`DirectoryRecovery { output, previous_output }`. The previous output remains
at that recovery path. It is never treated as scratch or deleted through Drop.
The caller can inspect both paths and explicitly recover after resolving the
destination conflict; the library does not retry destructively.

Candidate scratch cleanup failures retain their paths and secondary diagnostics
as in D1. After successful installation, old-output removal is cleanup only:
failure returns a successful result with an actionable cleanup diagnostic.
Recursive cleanup may have removed part of the backup before failing; after
commit, a complete old copy is no longer guaranteed.
No required producer work follows commit. Former staging names are never
removed after a successful rename because another actor may have reused them.

## Concurrent writers

There is no process mutex or cross-process serialization promise. Every move is
exclusive at its destination. A competitor inserted after hold blocks both
installation and restoration and remains untouched; the held old output is
retained for recovery. A later Replace may hold a completed output from an
earlier attempt and replace it. Consequently, a successful result establishes
publication at its commit point, not perpetual ownership of the final name.

The required deterministic two-attempt control is: A holds old output; B sees
the absent target and commits; A cannot install or restore over B, and retains
its old output. Independent CreateNew and external writers have the same
destination protection when they insert an entry after A's hold.

## Consumer and proof

The existing raster request accepts `with_policy(OutputPolicy::Replace)`;
CLI `--force` and Python `force=True` map to that same publisher. Failure
recovery paths are machine-readable in Rust, CLI JSON and Python exceptions.
CLI recovery includes readable `output` and `previousOutput` strings plus
`nativePaths`: an encoding name (`unix-bytes` or `windows-utf16`) and the exact
native unit arrays for both paths. Recovery tools must use those arrays when
display strings cannot represent the original filename. Rust and Python retain
native path values directly.
Default policy remains CreateNew. The raster decoding profile is unchanged.

Acceptance requires separate hold/install/restore fault controls, exact
restoration inventory, blocked restoration with old output retained, committed
success despite failed backup removal, cancellation on both sides of permission,
file/symlink replacement without following referents, and actual platform tests.
The independent CLI oracle must exercise the real consumer, output inventory,
refusal preservation and replacement. D2 does not accept the broader legacy
raster/terrain routes in #124 or complete GDAL replacement in #82.
