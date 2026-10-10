# Proposed runtime and acceptance contracts

Status: proposed for [#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113).
This describes independently required target behavior; current code has no
presumption of correctness. Retention requires the evidence in the component
ledger, not merely a passing existing test suite. See [architecture scope](README.md) and the
[API proposal](api-contract.md).

The [F0 foundation contract](foundation-contracts.md) controls the first
implementation scope. This broader document includes later directory, vector and
resource-validation obligations; they are not hidden requirements of F0.

## Job phases and side effects

| Phase | May read input/backend state? | May write? | Failure meaning |
| --- | --- | --- | --- |
| Validate request | No source-dependent I/O | No | Invalid request; no output parent, scratch or staging created |
| Prepare source/capabilities | Yes; parse source metadata and resolve requested capabilities | No output publication; ideally no scratch | Unsupported/unreadable source or unavailable capability; target unchanged |
| Execute | Yes; source must remain unchanged during conversion | Private scratch/staging and in-memory state | Abort, close native resources and clean up; target unchanged |
| Finalize candidate | Yes, including deferred close results | Complete reports, accepted resource inventory, final staged artifact | Abort before publication if any required artifact/report fails |
| Publish | Final output checks | Install completed artifact under declared replacement policy | Either committed output, or actionable failure/recovery state |
| Return committed result | No required conversion work | No fallible required output work | Successful publication must not be misreported as an ordinary pre-commit conversion failure |

Preparation should not materialize an unbounded source merely to avoid naming
execution stages. Format readers that require a disk spool do that within an
execution job, after pure request validation and available read-only preflight.
Actual decoding/geometry errors can still occur during execution.

File paths are not immutable snapshots. The proposed contract requires sources
to remain unchanged during conversion; document and justify this assumption for
each reader. Audit hashes establish the tested inputs, not a general guarantee
against concurrent source changes.

## Publication semantics

`CreateNew` must not overwrite a destination created after preflight. A prior
`exists()` check is not a synchronization mechanism. Use a platform primitive
with no-replace semantics or fail with an explicit unsupported guarantee;
cooperative locking alone does not protect against unrelated writers.

`Replace` must preserve the old output on ordinary failures before commit.
Archive file replacement and directory replacement may use different platform
mechanisms. Define the commit point and test it for each artifact kind. On a
failed directory restore, retain the backup and return its location rather than
deleting the only surviving previous output.

Do not describe the existing two-rename directory replacement as atomic or
crash-durable. If that mechanism remains, document the visibility/crash window
and recovery location. Eliminating the window requires a separate versioned
output/indirection or platform exchange design; it cannot be achieved by renaming
the helper. The 0.4.0 contract must choose implementable guarantees per platform
and reject unsupported modes rather than silently weakening `CreateNew`.

All required close, flush, serialization, report and callback checks occur before
publication. After commit, a late diagnostic must not turn an installed artifact
into an indistinguishable failed conversion. This is also relevant to Python
signal checking and callback exceptions. See the API proposal for the intended
cancellation fence and post-commit notification policy.

## Feature acceptance and speculative artifacts

A source feature has a transaction spanning its SQL rows, counters, diagnostic
records and accepted fragment identities. Build a local delta while validating
and splitting; merge it only when the entire feature succeeds. A rollback must
not leave a success-like fragmentation record or a counter for absent rows.
Rejected-feature diagnostics are a separate explicit outcome, still recorded
when `skip_invalid` is enabled.

Do not equate every `Error::Data` or callback error with a skippable feature.
Model a feature rejection separately from an infrastructure/job failure. I/O,
SQLite resource/storage errors, failed native finalization, cancellation and
callback failures abort the job even when malformed input features may be
skipped. Recoverable source geometry/property problems carry source identifiers
and reasons; lower-level failures retain their causal error.

The current `vector/pipeline/reuse.rs` publication step already prunes
unreferenced generated tile files; the initial static suspicion of published
orphan content missed this cleanup. The executable control found no orphan
content. This supports that exercised case only; broader reuse/implicit/resource
behavior still needs independent evidence before the cleanup design is retained.

Candidate tile/LOD encoding can write private temporary content, but the final
artifact inventory includes only selected candidates and their dependencies.
Content-addressed deduplication does not make an unused candidate an accepted
resource. Manifest reachability or an explicit accepted inventory must account
for multiple contents, implicit subtrees, reports, metadata/schema resources,
reused content and byte-preserved source dependencies.

Inventory construction must preserve glTF relative URI bases and reject member
collisions and escapes. Shared URI normalization may remove duplicated rules,
but filesystem symlink containment and archive member validation remain distinct
checks. Imported content may have extension resources beyond the generated
writer's narrow model; do not discard or rewrite them incidentally.

## Resources, metrics and backend ownership

- Each job owns its counters, timings, event sink, worker budget and scratch
  lifetime. Repeated/concurrent jobs must not mix metrics or bypass a silent sink.
- Backend objects remain on their permitted thread. Owned feature/grid/buffer
  values cross worker boundaries; no global mutable source or transform handles.
- Resolve driver/codec/CRS support for the requested operation. An unrelated
  missing native driver must not be a hidden prerequisite for that operation.
  Retain compile-time feature boundaries and intentional supported-format limits.
- Record backend and capability decisions in the report where they affect output
  interpretation. Any justified CRS fallback must prove whole-batch failure/accuracy rules; do
  not silently mix differently resolved coordinate operations within a batch.
- Explicit worker limits bound aggregate per-worker caches. Measure payload,
  hierarchy, source mesh, scratch and file-descriptor growth separately. Tile
  bytes/vertices are output constraints, not a total process-memory limit.

## Acceptance matrix

This is a migration gate, not a statement that every listed case is currently
covered. Add targeted regression tests with fixes. Reuse existing suites only after
checking their oracle against the desired contract, including unchanged behavior. Mark unavailable environments as unverified, not passed.

| Dimension | Required evidence | Existing starting points |
| --- | --- | --- |
| Rust / CLI / Python | Same valid/invalid requests, defaults, domain error kinds and published reports; explicit adapter limitations | `tests/cli_contract.rs`, `tests/machine_results.rs`, `bindings/python/tests/test_api.py` |
| Portable / native, explicit / implicit | Decoded geometry, bounds, source identities and metadata; intentional backend differences | `tests/fidelity.rs`, `tests/portable_vector.rs`, `tests/native_vector.rs`, `tests/implicit_metadata.rs` |
| Coordinate units / axes / height / epoch / grids | Independent reference accuracy, finite transforms, unsupported-input refusal, varied batch boundaries | `tests/mesh_crs.rs`, `tests/native_point_cloud.rs`, `src/crs.rs` tests |
| Jobs 1 / 2 / 4, fresh / reused | Deterministic content and report order, reproducible bytes where promised, no cross-job metrics | `tests/test_vector_parallel.py`, `tests/test_vector_reuse.py`, `tests/output_digests.rs` |
| Directory / archive, create / replace | Concurrent destination creation, ordinary failure rollback, actionable backup recovery, cleanup | `src/output.rs` tests, `tests/fidelity.rs`, new deterministic publication probes |
| Valid / rejected / infrastructure failure | Only feature rejection skippable; rows, counters and diagnostics agree; disk/SQL/close failures abort | `tests/test_vector_invalid.py`, `tests/test_vector_parent_repair.py`, new injected-failure regressions |
| Accepted / discarded / reused candidates | Published inventory equals accepted output and resource closure; no rejected generated tiles | `tests/gltf_resources.rs`, `tests/convert_implicit.rs`, new budget/LOD inventory regressions |
| Callback / signal / cancellation timing | Before-commit abort preserves target; after-commit outcome is explicit; no deadlock/reentry regression | Python callback/reentry tests plus publication-state assertions |
| Increasing source size at fixed budgets | Peak RSS, scratch high-water mark, descriptors, worker count and resident hierarchy cost | Existing benchmark runners plus declared scaling cases |
| Installed artifact / viewer | Portable platforms, actual wheel and official Blender API; browser rendering, picking and cache-disabled reload | Wheel acceptance workflows, `scripts/release_acceptance.sh` strict mode |

Record fixture provenance, exact revision/feature set, toolchain, command, outcome
and limitations for each audit reproduction. Failure injection proves the
handling of a controlled fault, not its frequency in ordinary use. Existing
digests characterize historical output, not correctness. Explain changes against
the desired contract, compare decoded semantics with independent references and
use previous explicit settings when that helps isolate a default change. Never
simply regenerate expected digests to make a migration pass, or preserve an
incorrect output solely to keep a digest unchanged.
