# F0: small, explicit runtime foundations

Status: scoped as [implementation issue #115](https://github.com/BenDyson-Arch/rusty-tiles/issues/115) under [#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113), reviewed against `7f6abf0`; no production implementation. This document narrows the first implementation slice. The [mesh API proposal](api-contract.md) becomes downstream F1, not a prerequisite for F0. Read with the [runtime contract](runtime-contract.md) and [architecture plan](README.md).

Current code is an untrusted candidate. Build the smallest correct plumbing around a real operation; retain code only when independently tested against the requirements below. F0 does not promise perfect software or every filesystem guarantee. It gives failures and ownership explicit places, so converters do not accumulate policy branches.

## Independently justified requirements

- An invalid request cannot cause filesystem work: callers must be able to validate choices without creating a destination or scratch tree.
- Source/backend failure, observer failure, and cancellation cannot publish a partial candidate. Successful core results mean a completed artifact was installed.
- `CreateNew` never destroys a competing destination. `Replace` has a declared platform guarantee; unsupported guarantees fail explicitly.
- Recoverable invalid features and broken infrastructure are different outcomes. A request to skip bad data cannot authorize ignoring failed writes, storage, finalization, or observers.
- Only accepted work contributes success counters, diagnostics, and published inventory. Caller-visible receipts describe the operation actually completed.
- Library code has no implicit terminal policy, frontend arguments, or process-wide job state. Domain decisions and platform decisions each have one owning boundary.

These requirements follow from predictable conversion and non-destructive output handling, not from the current implementation's behavior. Geometry/format correctness requires its own independent evidence; a correct publisher does not certify its producer.

## Findings and missing definitions

Locations below refer to unchanged production sources at the design baseline. They identify candidate seams, not code to preserve automatically.

| Finding | Evidence | Foundation consequence |
| --- | --- | --- |
| Job setup creates output parents before all checks in some callers | [job setup](../../src/output.rs):65; [mesh entry point](../../src/tile.rs):183,199 | Consumer validation/resolution precedes job creation; producer-specific errors can still occur in execution |
| Packaging invokes `Job`, and `Job` invokes packaging serialization | [pack orchestration](../../src/pack.rs):33; [archive publication](../../src/output.rs):116,118 | Put format encoding below the publisher; conversion orchestration owns both |
| Archive construction and destination installation are one method | [archive publication](../../src/output.rs):111 | Separate seal from install; no required producer work occurs in publishing |
| Directory existence preflight can race with rename | [directory publisher](../../src/output.rs):161,162 | No-clobber is a platform installation property, not an `exists()` branch |
| Restore failure is encoded as an ordinary message with a recovery path | [directory restore](../../src/output.rs):173 | Recovery state must be structured; this directory path is outside F0 implementation |
| SQL rollback does not own counters/report writes | [feature insertion](../../src/vector/pipeline/store.rs):626,638,647 | Later vector slice needs one accepted delta; SQL savepoints alone are insufficient |
| Event sink is infallible and defaults to process stderr | [events](../../src/report.rs):32,40 | Observer failure is an abort input; frontend transport stays outside runtime |
| Python checks callback errors/signals after converter return | [binding runner](../../bindings/python/src/lib.rs):137,143 | Observe failures before commit permission; preserve committed core classification afterward |
| Error kinds also encode CLI statuses | [error mapping](../../src/error.rs):55 | Adapters map domain kinds; skip policy must never inspect display text/category |

The proposed API also needs these precise refinements:

1. Its “context” must not become a registry for paths, formats, defaults, capabilities, metrics, pools, and recovery. Separate run control, producer-owned data, and publication ownership.
2. Its commit gate grants **permission to publish**; it is not the filesystem installation point. After permission, publication can still fail. `Published` requires actual installation.
3. Joining workers before the gate is insufficient if an event dispatcher can enqueue later callbacks. Sealing must drain every event producer/dispatcher and close event admission.
4. Every fatal cause must enter the same abort arbitration. Use its single primary-error rule below; do not build a source/observer priority ladder or let Python replace arbitrary core errors with its stored callback exception.
5. Immutable embedded reports cannot acquire a cleanup warning after installation. Postcommit diagnostics belong beside the committed receipt, not by rewriting its report.
6. Preparation is read-only application inspection, not proof that later decoding cannot fail. Raw source paths are not snapshots; every consumer declares its source stability policy.

## Minimal ownership and types

Use concrete private types only where ownership prevents an invalid action. No public generic typestate engine, universal converter trait, backend registry, or speculative state hierarchy.

Consumer-owned phases are ordinary functions: `Request -> validate -> ValidatedInput -> resolve -> ExecutionInput`. Constructors for validated/resolved values stay private. Pure validation handles option algebra, finite values, and syntax; resolution handles files, source metadata, requested backend, and supported publication mode. Execution input owns reader/operation decisions with declared thread/lifetime/resource rules. Do not create a reusable `Validated<T>` or claim that source resolution makes source data infallible.

Runtime needs only three ownership-bearing artifact types:

```rust,ignore
// Proposed private contracts, not currently implemented APIs.
struct Staging { /* private candidate, writable resources, cleanup ownership */ }
struct SealedArtifact { /* closed candidate + frozen producer receipt */ }
struct Published { /* installed path/kind + receipt + postcommit diagnostics */ }

fn seal(staging: Staging, run: &RunControl)
    -> Result<SealedArtifact, JobFailure>;
fn publish(artifact: SealedArtifact, target: &ResolvedTarget, run: &RunControl)
    -> Result<Published, JobFailure>;
```

The consumer owns and joins its workers, finishes format encoding/report serialization, and completes producer-specific finalization. It waits for its synchronous observer calls to return (or drains its own dispatcher if one is justified later), then closes runtime event admission. F0 needs no asynchronous dispatcher. `RunControl` owns neither worker handles nor producer resources. `Staging` owns candidate storage, which the producer writes through scoped borrows; no writable alias survives the producer's finish.

Only then does `seal` consume staging, require closed event admission, finish candidate-storage checks, and freeze the candidate/inventory/receipt. It does not join arbitrary workers or call a format encoder. No writable alias or running producer survives sealing. A type name does not prove those properties: test the producer finalization and event barriers. Event emission is private to the consumer, not a public cancellation-handle method. External filesystem interference is separately scoped, not excluded by Rust ownership.

The sealed artifact owns its cleanup lifetime and is publishable once; publication consumes it. A published result owns identity/receipt only and cannot republish or mutate the installed artifact. A receipt is producer-owned typed data; runtime need not know mesh options or invent a universal report schema. If a producer embeds a report, seal the serialized bytes and return that same report. Later cleanup diagnostics are separate from that immutable report.

`JobFailure` contains a causal domain `JobError`, secondary diagnostics, retained work paths if cleanup failed, and optional structured `RecoveryInfo`. Recovery records the paths needed to inspect/restore surviving previous output/candidate and what destination state is known. Thus normal failure and recovery-required failure are distinguishable without an unrelated parallel error hierarchy. Do not claim “nothing published” when destination state is unknown. `Drop` is best-effort fallback, not a guarantee that cleanup succeeded.

Feature handling belongs to the later real vector consumer:

```rust,ignore
enum FeatureDecision {
    Accepted(FeatureDelta),
    Rejected(FeatureRejection),
}
fn process_feature(/* vector-owned inputs */)
    -> Result<FeatureDecision, JobError>;
```

`FeatureDelta` owns proposed rows/fragments, counter increments, success diagnostics, and candidate identities. Commit accepted deltas together under the vector transaction owner. Rejection records are explicit separate outcomes. `SkipInvalid` matches only `Rejected`; every `Err(JobError)` is fatal. Rollback/finalization failure is fatal even if the triggering feature was rejectable. Implement these types with that consumer, not as unused F0 framework code.

## One abort/commit arbitration boundary

`RunControl` owns observer admission and abort/commit arbitration only. It does not own source paths, codecs, CRS handles, worker pools, budgets, or producer statistics. Observer objects can retain adapter-specific exception state; runtime sees a domain observer-failure indication.

| Control state | Legal transitions | Meaning |
| --- | --- | --- |
| Open | Aborted; Publishing only with a sealed artifact | Work/events allowed until sealing closes admission |
| Aborted(reason) | Terminal failure/cleanup | No permission to install |
| Publishing | Terminal published/failure/recovery | Permission granted; late abort cannot stop the installation attempt |
| Finished | None | Result classified; no further callbacks or installation |

One synchronized transition arbitrates `Open -> Aborted` against `Open -> Publishing`; a separate atomic “cancelled?” check followed by publication is insufficient. Choose an understandable lock/state implementation first, not a lock-free protocol. Hold its lock only to change control state; never around callbacks, worker joins, filesystem calls, or Python attachment. A granted publisher owns completion and cannot publish twice.

The actual installation point is defined by the selected platform file operation. No required encoding/report/callback work remains once publishing starts. Pre-gate abort wins permission arbitration; post-gate cancellation is too late to cancel that attempt, which can still fail due to the publisher. No callback is scheduled after seal, including completion callbacks. The returned core result signals completion.

All prepublication fatal causes, including producer/backend failures, observer failure, and cancellation, request `Open -> Aborted` through the same synchronized gate. The first cause accepted by that gate is primary; this means gate order, not inferred wall-clock order. Later failures are secondary diagnostics. There is no converter-specific priority ladder. A publisher failure is primary if permission was granted, because no abort was accepted beforehand. Python rethrows its original callback exception only for the selected primary observer-failure outcome. Concurrent schedules may select different primaries; the arbitration rule and destination guarantee stay the same.

Successful installation remains `Published` in core even if cleanup or a later adapter diagnostic fails. Adapter-controlled signal checks must not deliberately relabel it as a precommit failure. Python VM asynchronous exceptions, Python value-allocation failures, process termination, and crash durability are outside that promise. Publication may have occurred when caller-side execution is interrupted.

## Platform and domain choices each have one owner

The producer chooses its input domain and inventory; format codecs encode that inventory into private candidate storage. The publisher installs an already sealed candidate. Publication dispatches on artifact kind and requested policy once, through a small platform module. Unsupported mode/platform combinations are resolved before staging; unexpected runtime OS failures still return structured failures. Converters never reproduce platform rename or rollback logic.

Separate **no-clobber**, **destination visibility**, **temporary-name cleanup**, and **durability**. `CreateNew` needs a proved no-replace destination install. It does not automatically promise one-step temporary-name removal or durable storage. `Replace` needs declared behavior for its supported artifact/platform; directory replacement is a separate follow-on slice. Do not describe an unspecified publisher as “atomic.” Exact primitives, filesystem assumptions, and tests belong in [platform evidence](platform-evidence.md), not scattered converter branches.

Dependency direction: adapter -> consumer orchestration -> runtime publisher and format codec. Codec -> ordinary writer/format types; publisher -> platform filesystem operation. Codec never calls `Job`; publisher never calls a converter or archive encoder. Runtime error/event/control types contain no clap, Python, GDAL, vector repair policy, or process-global metrics. Compatibility translation, if justified, lives entirely outside this new core.

## F0 implementation scope: one real packaging consumer

Implement the contracts with the existing package-conversion use case through Rust, CLI, and Python. The independently chosen operation is: package an explicitly resolved set of local tileset/resources into a complete 3TZ file without rewriting selected source bytes. This byte requirement follows from choosing packaging rather than content conversion, not from treating current wrapper output as correct.

The consumer owns validated paths/policy, safe member inventory, source-stability assumptions, and its typed receipt. F0 is opaque named-member/container packing: require `tileset.json`, normalize safe exact archive names once, reject missing/duplicate/unsafe/reserved members and source/destination overlap, and independently verify the generated 3TZ index. Do not silently discard selected inputs. Reserved names refer to the codec's generated entries, not runtime-chosen report names. The directory adapter resolves its declared regular-file input domain into that inventory; symlink/resource-discovery rules must be explicit before migrating callers. F0 does not resolve/rewrite glTF URIs or certify the input tileset's semantic correctness or external resource closure. Those belong to subsequent source/format validation contracts, not generic runtime. Encode the accepted inventory through the format layer into staging, seal, and call the completed-file publisher. Expose the same fallible observer path to Rust/CLI/Python so F0 exercises callback cancellation with an actual operation. Library default is silent.

Keep prepared result metadata separate from artifact bytes. F0's returned package summary may be memory-only; runtime does not insert or rewrite `conversion.json`. If the accepted source inventory contains that file, it is ordinary selected source content and its bytes are preserved. Later mesh production owns its report before sealing. The generic publisher therefore has no report-name exception and no JSON knowledge.

F0 supports completed **file** installation for the explicitly tested `CreateNew`/`Replace` platform modes. A known unsupported platform mode returns unsupported before staging. A runtime filesystem refusal must fail without silently switching to a weaker mechanism. Directory publication, vector transactions, and full mesh processing are separate bounded slices. The state model may explore their future contracts; that does not make them F0 production work.

Do not add a second converter just to justify abstraction. A single real producer and three adapters are sufficient to test this boundary. Add only the shared types used by it; the feature algebra above is a downstream constraint, not an invitation to ship unused generic machinery.

## Evidence, tests, and stop conditions

The [bounded lifecycle model](../../bench/architecture_audit/foundation/README.md)
passes 14 contract scenarios and detects seven broken variants over 2,354
state/history pairs and 4,258 edges. It supports the logical design within its
stated bounds and assumed primitives. It does not prove actual locking,
filesystem, FFI, multiple-publication prevention or cleanup behavior.


F0 acceptance must include:

1. Table-driven validation/error mapping through all adapters; invalid choices create no output parent/work tree. Read-only resolution failure leaves the destination untouched.
2. An independent archive reader checks the accepted inventory, unchanged member bytes, required root member, and 3TZ index structure. It does not certify the input scene/resource semantics. Current codec/validator agreement is not the sole oracle.
3. Finalization/report/write failure injection proves no installation; early/final-event callback failures preserve the selected Python exception and destination. Event drainage and callback reentry are tested without locks across callbacks.
4. Abort-before/after-gate interleavings and an attempted second publish; negative controls detect split-check races and callback-after-publication result rewriting. Model proofs state their assumed filesystem primitives and are accompanied by actual platform tests.
5. Competing destination creation for `CreateNew`, supported replacement failures, cleanup failure with retained paths, and committed-result preservation. Prove no-clobber, visibility, and durability claims separately; mark untested environments unverified.
6. Repeated/concurrent real calls prove no cross-job receipts/control/events. Installed Python binding tests exercise the migrated operation. Resource measurements state staging size and descriptor policy; no total-memory guarantee is implied.

Stop when this consumer passes the declared contracts and supported-platform checks, the old cyclic publication path is gone for this consumer, and adapters invoke the same core operation. Do not extend F0 to redesign mesh coordinates/LOD, add a directory strategy, fix every vector path, reorganize every module, or freeze universal report/capability schemas. Failed evidence warrants replacing the responsible F0 component or narrowing an explicit supported mode, not growing a special-case fallback.

Before implementation starts, record the supported platform file primitives, packaging input/resource domain, receipt fields, and fault-injection seams. If any is unresolved, settle that small decision with a focused probe. F0 is complete only with a real integrated consumer; passing an unused framework model is insufficient. Broader audit and replacement work continues after this foundation is demonstrated.
