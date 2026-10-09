# F0 implementation decisions (#115)

This bounded slice implements opaque 3TZ packaging and completed-file publication.
It does not certify scene resources, migrate other converters, replace directories,
or establish crash durability. The current implementation earns retention only
against the contracts below and independent tests.

## Package contract

Rust, CLI and Python call the same package operation. The default output policy
is create-new; explicit replacement permits replacing an existing file.
The directory adapter accepts a directory or its `tileset.json` path. Named
members provide explicit archive names and source paths. Accepted inventory must
contain exactly one `tileset.json`; its bytes are opaque and need not parse as JSON.

Member names fit the ZIP header’s 65,535-byte filename field and are UTF-8
relative paths using `/`, without empty, `.` or `..`
components, backslashes, NUL or absolute/drive-qualified paths. Duplicate names
and the generated `@3dtilesIndex1@` name are rejected. Names containing `.3tz`
or `.3dtiles.zip` are rejected; output names end in `.3tz` or `.3dtiles.zip`.
Each source member and the generated index must be smaller than `u32::MAX`
bytes, so local headers contain actual 32-bit sizes rather than ZIP64 sentinels.
These container restrictions follow the [Maxar 3TZ v1.4 specification](https://github.com/Maxar-Public/3tz-specification/blob/main/Specification.md).
Archive-level ZIP64 offsets remain possible. Semantic tileset validity is outside
this opaque packaging contract. Directory traversal rejects
symlink entries, including a symlink input root, and nonregular members; it does
not silently omit them. Existing symlinks in ancestors of the supplied root are
resolved for overlap checks. Output may not lie within the source tree or alias
any selected member, including through hardlinks. These are core domain rules.

Accepted bytes, including `conversion.json`, are copied unchanged. The caller
must keep sources stable for the operation; this is not a filesystem snapshot.
Resolution is read-only and precedes output-parent/scratch creation. Copying uses
bounded chunks and one open source at a time; inventory and index memory grow
with member count. Archive serialization owns ordering and index construction,
with no job or runtime dependencies.

The typed receipt contains `member_count` (excluding the generated index),
`source_bytes` (sum of accepted member bytes), and `archive_bytes` (completed
archive size). Result output and cleanup diagnostics are separate from receipt.
No synthetic conversion report is embedded or rewritten.

## File lifecycle and platform contract

One single-use run owns first-cause arbitration and publication permission.
Every accepted fatal producer/observer error or cancellation before permission
prevents permission and installation. Publisher failure after permission is a
separate failure point. Synchronous callbacks finish before sealing; no user
callback runs under the gate lock or after sealing. Nested calls use independent
runs. Reuse fails without aborting a concurrent original operation.

Staging is in the destination filesystem. On Unix, new F0 files retain
`tempfile` mode 0600 (subject to umask) after publication; replacement installs these permissions
instead of preserving the old file mode. Legacy converter publication is unchanged. A writable candidate is consumed into
a closed, synced sealed candidate; that candidate is consumed by one publication
attempt. Pinned `tempfile` 3.27 primitives provide `persist_noclobber` for
create-new and `persist` for file replacement. There is no copy fallback and no
predelete of the old destination. Namespace installation establishes commitment;
subsequent cleanup failure produces diagnostics and retained paths, not failure
of the committed operation. If successful persistence leaves a temporary name,
retain and report it: success returns no ownership token proving that the name
has not been reoccupied, so an unconditional unlink would be unsafe. Atomic installation does not imply atomic cleanup,
filesystem snapshotting, or power-loss durability. Platform validation must be
reported separately for the operating systems actually exercised.

## Evidence and fault boundaries

Private test seams cover candidate write/finalize, namespace installation and
cleanup. They must not become public configuration flags. Runtime tests exercise
first-cause order, producer failure, callback cancellation/reentry, event drain,
permission races, run reuse and consumed publication ownership. Filesystem tests
exercise competing create-new destinations, replacement preservation and retained
cleanup paths. Injected errors are identified as injected, not real disk failures.

Independent archive evidence reads ZIP records/index offsets and member bytes
without relying on the production writer's own validator. Frontend evidence
includes equivalent Rust/CLI requests and an installed Python wheel, including
callback exception identity and destination preservation. Existing converter tests
remain regressions, not proof of this contract or acceptance of unexamined code.

The independent model monitor was corrected in response to the review on #113:
producer failure now establishes the same pre-permission prohibition as callback
failure and cancellation, including rejection at permission grant. The new negative
control deliberately revives an encoder-failed candidate and must be detected.
This model remains design evidence, not a production runtime proof.

## Migration and implementation disposition

Use `rusty_tiles::package::{package, PackageRequest, PackageMember}` with a
fresh root-exported `RunControl` per operation. `PackageRequest::directory`
and `PackageRequest::members` share validation and the same publication path;
`.with_policy(OutputPolicy::Replace)` opts into file replacement.
`PackageResult` carries the receipt and cleanup diagnostics. The old Rust pack
entry points delegate to this path, wrap failures in `Error::Job`, and retain
their legacy return signatures; callers needing receipts/cleanup details must
use the new result. `convert_to_3tz_reported` no longer interprets or synthesizes
a conversion report.

Python `convert_to_3tz` now returns `PackageResult`, with a typed `receipt`
instead of `ConversionResult.report`, and accepts `callback`. CLI `convert`
returns `packageReceipt` and `cleanupDiagnostics`; selected `conversion.json`
does not become the command's interpreted conversion report. Error kinds and
exit mappings are documented in the adapter guides. Destination-inside-source,
symlink, stale index, archive filename/size, and private Unix permission rules
are intentional migrations, not cases to preserve via compatibility flags.

The F0 serializer is replaced with a narrow stored-ZIP implementation following
[PKWARE APPNOTE 6.3.9](https://pkware.cachefly.net/webdocs/APPNOTE/APPNOTE-6.3.9.TXT).
It computes CRCs while streaming, fills local headers, writes the MD5 index,
and emits central-directory ZIP64 records when offsets/counts require them.
No destructor retries failed encoding. The old `ZipWriter` failed retention
because its persistent-failure destructor writes directly to stderr. Its legacy
converter entry point remains explicitly unproven under #113; no mode flag
selects between writers in the new package operation.

The migrated dependency direction is adapters -> package -> archive codec and
runtime. Runtime imports neither package nor codec. Legacy inventory selection
and temporary-file policy remain with the old output adapter; archive code owns
only encoding/reading, and the new package path never calls the legacy job.
