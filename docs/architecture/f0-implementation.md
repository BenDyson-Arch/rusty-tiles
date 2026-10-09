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

Member names are UTF-8 relative paths using `/`, without empty, `.` or `..`
components, backslashes, NUL or absolute/drive-qualified paths. Duplicate names
and the generated `@3dtilesIndex1@` name are rejected. Directory traversal rejects
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

Staging is in the destination filesystem. A writable candidate is consumed into
a closed, synced sealed candidate; that candidate is consumed by one publication
attempt. Pinned `tempfile` 3.27 primitives provide `persist_noclobber` for
create-new and `persist` for file replacement. There is no copy fallback and no
predelete of the old destination. Namespace installation establishes commitment;
subsequent cleanup failure produces diagnostics and retained paths, not failure
of the committed operation. Atomic installation does not imply atomic cleanup,
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
