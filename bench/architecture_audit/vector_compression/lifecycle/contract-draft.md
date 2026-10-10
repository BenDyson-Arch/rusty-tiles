# DRAFT — bounded generated vector in-place compression lifecycle

Status: precode proposal only. Source observations are pinned in `source-pin.json`; this draft uses no new execution evidence. Current C1 must first complete acceptance. A fresh independent whole-slice precode review must then adjudicate the codec draft, this lifecycle draft, proposed numerical defaults and focused probes. This document grants no implementation approval, CI acceptance, develop merge or release acceptance. No repository files were edited. This freeze remains bound to the read source 2ebfb749; later independently adjudicated C1 accessor/codec-stride corrections are not retroactively claimed as examined here. Corrected C1 acceptance must precede new focused probes and independent whole-slice review. The compressed-identity branch and every storage/work coefficient/formula remain unresolved review gates, not proved choices or implementation permission.

## Operation and owners

The real operation compresses one admitted self-contained generated-family GLB and replaces its selected pathname with a completed candidate. Retain F0 completed-file staging/control/publication within its proved Unix/Windows domain. Producer uses the same pure codec on its private generated GLB under its existing vector Attempt. Do not create a nested standalone compression operation, second RunControl, per-view worker pool, package conversion or in-place mutation on producer workspace content.

Minimal proposed intentional facade:

```rust,ignore
pub struct CompressionRequest { path: PathBuf, limits: CompressionLimits }
impl CompressionRequest {
    pub fn in_place(path: impl Into<PathBuf>) -> Self; // owns canonical defaults
    pub fn with_limits(self, limits: CompressionLimits) -> Self;
}
pub struct CompressionReceipt {
    pub source_bytes: u64,
    pub candidate_bytes: u64,
    pub compressed_views: u64,
    pub raw_views: u64,
    pub processed_view_bytes: u64,
    pub estimated_working_bytes: u64,
}
pub struct CompressionResult {
    pub output: PathBuf,
    pub receipt: CompressionReceipt,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
pub fn compress_file(request: CompressionRequest, run: &RunControl)
    -> Result<CompressionResult, JobFailure>;
```

CompressionLimits owns only concrete format/storage admission values; no paths, jobs, force, workers, reporting, backend registry or output policy. Its public raw values are checked once into a private validated representation. All defaults come from one core Default implementation; adapters parse integer limits without lossy floating-point conversion or duplicating the defaults. Constructors/adapter syntactic parsing do not certify filesystem state. The old public mutator signature and incidental module export are removed after real callers migrate; no compatibility call routes through fs::write.

Pure shared codec boundary, coordinated with the codec lane:

```rust,ignore
fn prepare<'a, E>(source: &'a [u8], limits: ValidatedCompressionLimits,
    checkpoint: impl FnMut() -> Result<(), E>)
    -> Result<GeneratedCompressionPlan<'a>, CodecError<E>>;
fn encode<E>(plan: GeneratedCompressionPlan<'_>,
    checkpoint: impl FnMut() -> Result<(), E>)
    -> Result<EncodedCompression, CodecError<E>>;
```

The plan owns one classification per physical bufferView, separate accessor-window/role validation, borrowed raw JSON/source slices, a bounded set of edits/stream descriptors and checked candidate/working-storage estimates. Encoding consumes it and returns complete GLB bytes and immutable codec receipt facts. Generic checkpoint causes are tagged separately from format errors; the codec imports no runtime error/control. Callers supply their existing Attempt checks. Check before/after bounded JSON and view/decoder/serialization stages, with no user observer required inside the codec. No files, callbacks, Attempt or adapter options enter this interface. There is no extension-field Value roundtrip for opaque numbers. Raw-document rewrite uses counted RawValue emission directly into the final GLB, retaining no separate output JSON Vec. Exact descriptor/scratch/allocation layout remains a codec precode review obligation. Physical views with eligible whole fixed-stride layout compress once; mixed effective strides or partial tails remain raw under deterministic codec eligibility, rather than becoming malformed solely for shared/interleaved references. Existing generated ATTRIBUTES/NONE compressed inputs have a proposed bounded decode-validation then exact identity-output branch; this is a review-blocked draft branch, not acceptance of general imported codecs.

## Draft source and filesystem contract

- One local regular file, self-contained GLB v2, within the explicitly admitted generated-family codec profile, including the codec draft's review-blocked validated identity branch for already-generated compressed content. Final symlink, directory, socket/FIFO/device entries are outside this profile and refused before candidate creation. Unknown codec/extension/resource/chunk dispositions are adjudicated in the codec draft. All currently generated quantized/feature/polygon/restart/raw metadata roles remain mandatory; numeric ceilings do not authorize quietly dropping them.
- Resolve requested spelling to one absolute path before callbacks. Admit its final entry using symlink_metadata, canonicalize its source and parent, open read-only, verify regular type/metadata/held file identity, and capture immutable bytes with bounded chunk reads and Attempt checks. Both original spelling and canonical replacement path are retained privately. Source must stay stable during capture; this is not a hostile-filesystem snapshot.
- Output equals that captured canonical source path by construction. No destination or OutputPolicy request field, create-new alternative, force flag, alias-exclusion or arbitrary source/destination equality test is needed. F0 Replace is selected once by this operation owner. Canonical parent resolution precedes all callbacks, staging and publication.
- Ancestor symlinks are permitted when canonicalized during capture; final symlinks are refused. CWD changes by observers cannot redirect operation paths. Ancestor/namespace mutation after admission is outside the profile; canonicalization does not lock directories or provide openat confinement.
- Hard links are admitted with named-entry semantics. The selected pathname receives a new inode; other names remain linked to unchanged original bytes. Existing original-file handles can continue reading that original object. Neither all-alias compression nor inode identity preservation is promised.
- Preserve captured `fs::Permissions` on the completed private candidate after writing and before seal. Failure to apply them aborts with typed I/O. This consciously extends consumer finalization without changing generic F0 policy. Preserve only portable permissions: no old inode, timestamp, ownership, ACL or xattr guarantee. Platform readonly behavior requires focused evidence before acceptance.
- After the final observer returns, verify requested spelling still canonicalizes to the selected path and the selected entry is regular with the held expected identity. Reopen/recheck and stream-compare every source byte against immutable capture using one 64KiB buffer; require exact length and stable before/after metadata. This catches same-length observer mutation even when mtime is restored. Source mismatch is Conflict. Check cancellation during this recheck. Close all capture/recheck identity handles before seal/publication.
- F0 Replace is unconditional named-entry replacement. Final comparison and install have a race. Caller must keep source and selected namespace stable through actual installation; same-target concurrent writers/compressions are outside the admitted domain. Mutations observed before permission abort; mutations after the final check may be overwritten by Replace. No compare-and-replace, advisory-lock participation, hostile ancestor protection, filesystem-wide snapshot or crash-durability guarantee is implied. A deterministic seam demonstrating this window is required to prevent overstated documentation.

## Candidate lifecycle and error ownership

1. Claim one Attempt; validate raw request path/limits and publication support before filesystem mutation.
2. Bind and capture source/path/identity; parse/admit the codec plan and complete resource preflight before creating candidate storage or invoking the first observer. The existing-compressed identity branch still validates bounded streams, records a byte-identical candidate, and follows the same named-entry replacement/permission/source checks; it does not silently reinterpret virtual fallback bytes as embedded source. Source/plan failure is read-only.
3. Emit start, encode complete output with checkpoints before/after each view/native codec call and between serial stages, and create/write one destination-parent F0 staging file. Native encoding of one admitted stream can be noninterruptible; cancellation latency is bounded by that stream work, not falsely advertised instantaneous.
4. Apply source permissions, complete candidate write/finalization, record immutable receipt and emit final ready event. No receipt/report serialization required for installation remains afterward.
5. Recheck source spelling/identity/bytes after final observer, close handles and event admission, then consume staging into sealed/synchronized candidate. No callback occurs after event closure or publication.
6. Consume sealed candidate through F0 Replace. Successful namespace install produces CompressionResult with the fixed resolved output and receipt. Postcommit uncertain temporary names/cleanup diagnostics remain beside success; never unlink an uncertain reoccupied path or rewrite success because terminal output failed.

Every fatal prepermission cause goes through the same Attempt arbitration. First accepted cause is primary; later source/producer/observer/cancel causes are secondary. Staging ownership uses explicit staging.fail and existing retained-path cleanup behavior. A publisher failure after permission is primary publication failure. No message matching or Python callback priority ladder is allowed. Observers retain the existing nonpanicking trait contract; ordinary observer errors are covered. Unwinding/process abort and library-internal infallible allocator failure cannot be advertised as graceful typed return plus guaranteed cleanup.

Add the minimal domain `JobErrorKind::ResourceLimit` if exposing the estimated-storage and finite cardinality limits. Map it explicitly in CLI/Python along with existing invalid_request, invalid_input, unsupported, io, conflict, cancelled, observer_failure and invalid_state. Invalid limit algebra is InvalidRequest; malformed supported source is InvalidInput; well-formed out-of-profile source is Unsupported; requested/admitted resource refusal is ResourceLimit; detected source mutation is Conflict; actual I/O retains its causal source/path. This is a small extension for a real consumer, not a parallel error framework.

## Concrete numerical defaults proposed for review

These are design selections, not measured accepted ceilings. All sizes are bytes, not decimal MB. They are deliberately finite, fit a 32-bit host, exceed ordinary default vector 4MiB tile budgets and permit existing quantized/polygon/raw-metadata forms. Boundary probes and real maximum generated content must decide whether these values remain appropriate. The new standalone defaults are not inherited C1 policy.

| CompressionLimits value | Draft default | Rationale / independent gate |
| --- | ---: | --- |
| max_source_bytes | 33,554,432 (32MiB) | Finite single capture; verify actual generated high-metadata and quantized cases at/around ceiling. |
| max_json_bytes | 1,048,576 (1MiB) | Bound borrowed grammar/string/key storage independently of BIN. Raw metadata payload lives in BIN; large authored JSON must be explicitly refused rather than rounded/lost. |
| max_json_depth | 64 | Finite admission traversal, root depth zero. Root raw scan can precede depth detection and stays accounted by JSON-byte limit. |
| max_json_nodes | 65,536 | Include root/containers/scalars, exclude object keys consistently with accepted JSON owner; bound slots/descriptors. |
| max_buffer_views | 4,096 | Bound stream/edit/header work even for zero/small views; verify generation with many schemas and primitive groups. |
| max_accessors | 4,096 | Bound separate reference/window validation; does not assign one stream per accessor. |
| max_processed_view_bytes | 33,554,432 (32MiB) | Sum physical declared view lengths; prevents repeated/overlapping ranges from multiplying processed/copied bytes without corresponding source size. Overlap admission remains codec-owned. |
| max_output_bytes | 67,108,864 (64MiB) | Candidate total GLB framing limit in addition to u32/host limits; input may compress larger. No must-be-smaller rule. |
| max_estimated_working_bytes | 134,217,728 (128MiB) | Precisely defined requested-storage estimate, deliberately separate from RSS and vector final max_bytes. Requires estimator sensitivity and actual resource evidence. |

Raw request validation requires positive byte/node/cardinality limits; depth zero is a valid restrictive profile and admits only root scalar/source structure accordingly. Values must fit checked host usize/isize allocation and GLB u32 limits when relevant; relationships are not arbitrarily forced (a caller can intentionally choose output or estimated-storage below source). Do not reserve caller maxima for tiny inputs. Charge actual admitted counts and lengths, bounded by maxima. Exact maximum/max+1 controls for each field and host width are required.

Current producer's options.meshopt selects the pure codec once. The vector core supplies these same validated defaults for this draft; no producer public compression-limit option is added speculatively. Existing max_bytes still checks exact final tile bytes, not this storage estimate. A larger requested tile budget does not itself fail: an actual generated candidate outside the finite codec profile fails with ResourceLimit rather than falling back to uncompressed output or quietly changing flags. Document this deliberate bounded support; test non-meshopt producer behavior remains separately unchanged. All fitting/final encoding paths must use identical codec profile/admission. Future larger profiles require explicit producer request support and proof.

## Checked storage and work model v0 (engineering estimate)

Definitions: I=actual source GLB bytes; J=actual input JSON bytes; K=admitted JSON value-node count (for admission preflight, use min(max_json_nodes,J)); V=physical view count; A=accessor count; R=sum raw-view lengths; N_i/S_i=full physical stream count/stride; P=256*(V+A)+128*K+2*J; Q=1KiB fixed receipt/state/path storage estimate; C=64KiB fixed read/recheck buffer. Source path text storage must also be charged by actual encoded/native path lengths, and unsupported unbounded path length needs explicit platform handling rather than pretending Q covers it.

For fixed S in 4,8,12 and the pinned meshoptimizer 0.6.2 static source formula:

`B(N,S)=1+ceil(N/256)*S*(S/4+260)+32`.

For a broader explicitly eligible stride S<=256, the codec draft supplies b=min(256,16*floor(8192/(16*S))) and B=1+ceil(N/b)*S*(S/4+ceil(b/64)+b)+max(32,S+S/4), with every term checked. This source-derived generalization and deterministic raw eligibility require independent precode review; all currently generated S=4/8/12 roles remain mandatory.

Compute ceil as N/256 + (N%256 != 0), with checked arithmetic. BIN upper B0=R+sum_i B(N_i,S_i)+7*V. Virtual fallback F=sum aligned original lengths of compressed views is a declared address space, not allocated decoded fallback payload; account only actual allocation, while checking F against host/glTF declaration range. J_out is counted by a bounded raw serialization pass after planned edits; proposed preliminary JSON allocation ceiling J+512*V+1024 covers compression-field growth and must be proved by counted-output controls; G=28+align4(J_out)+align4(BIN_actual), with output/preallocation upper G0 using B0. Check both exact actual framing and prospective allocation upper against max_output_bytes. Source view ranges/stride/count and fallback/view/chunk absolute eight-byte requirements are independent format checks.

Draft raw admission storage estimate E_json=8*J+256*K+C. Draft retained plan storage estimate P above covers raw field/edit descriptors, decoded owned keys/text and growth overlap. These intentionally generous coefficients are engineering choices pending pinned actual type/capacity/scratch review; they are not a proof of exact Rust allocator consumption. They may be replaced with actual size_of/capacity calculations in the independent precode review. P is not added while the separate JSON traversal has already dropped all its transient scratch.

Output JSON is emitted directly after a counting pass into the final GLB, so its bytes are included in G0 once. If a separate output-JSON Vec is retained, add its full requested capacity explicitly. If compressed BIN is retained while final GLB copies it, include both B0 and G0. Temporary encoder bound tails are included in B0 (never merely final encoded lengths). Cloned input/docs, any Value tree, producer bridge serialization or extra decoder/native arrays must be independently added, never assumed free.

For proposed already-generated ATTRIBUTES/NONE identity admission, D=max one decoded compressed physical view length and D_total=sum logical decoded view lengths, both bounded by max_processed_view_bytes. Decoder helper scratch estimate is 4*ceil(D/4)+D <=2*D+3; scratch is discarded per view. Caller must charge it before any decoder allocation. The proposed accepted-C1 helper extraction and actual native/helper capacity behavior are whole-slice review blockers. Identity output uses G0=I; no new compressed BIN is constructed (B0=0 in that branch). Generic mode/filter codecs are not admitted merely by this branch.

Proposed peak estimate, for mutually exclusive encoding versus identity-validation branches:

`E_standalone = I + max(E_json, P+B0+G0+J_separate+C, P+decoder_scratch+C, C) + Q + path_storage`.

Max estimated-working admission occurs before expensive encoder/output capacity allocation. Report the same formula/version and requested-storage accounting facts in the immutable receipt/evidence. Drop plan/BIN when no longer needed, but source capture remains alive through exact source recheck. Candidate file bytes are scratch-disk bytes, not another Vec; OS cache/native stack/allocator overhead/RSS are outside this requested-storage definition. Recoverable operation-owned reservation failures are ResourceLimit; accepted JSON library internal infallible allocation/abort is a documented implementation proof limit, not a fabricated recoverable error promise.

Producer peak is the maximum of bridge-serialization and shared-codec phases. Let H be persistent producer geometry/model/worker state excluded from the bridge source and pure codec, and T be the old generated Value/uncompressed BIN that survives a phase. Bridge phase is H+T+I_bridge+serializer_scratch. Codec phase is H+T_if_still_alive+E_codec, where E_codec already includes its I_bridge input once; do not add I_bridge again. After bridge serialization, consume/drop old Value/BIN when valid ownership permits. Avoid double counting the same allocation transferred by ownership, but count every real clone. If generated content is serialized into a bridge source GLB for the shared planner, bounded serialization checks byte/JSON limits while writing; its Vec is charged before allocation. Existing in-memory generated Value may be consumed/dropped when borrowed raw source replaces it. Exact producer bridge schedule and geometry overlap must be audited/measured; no claim of producer-wide 128MiB RSS bound follows from the codec estimate.

Work counts: source capture and final exact comparison read at most 2*I plus bounded excess-detection bytes; codec processed bytes = sum physical view lengths <= max_processed_view_bytes, with one native stream call per compressed physical view; raw output copy bytes and encoded output bytes are recorded separately. JSON raw-admission traversal may rescan nested substrings; a conservative scan-volume estimate `(2*depth+4)*J` is a design accounting metric, not a certified wall-clock or adversarial HashSet-operation bound. Cardinality/depth/source ceilings bound requested work; no universal CPU deadline or instantaneous cancellation promise is made. View roles/counts must not be charged per referencing accessor.

## Independent sensitive acceptance before implementation acceptance

Preparation probes are serial/bounded, CPU-considerate and coordinated; builds/resource runs belong solely to root after C1 acceptance. Every run pins final sources, binary/wheel, inputs, independent oracle and controls. A fresh nonauthor checks both implementation and evidence.

- Independently authored generated-role matrix: f32/normalized U16 positions, padded U16/f32/U32 feature IDs, point/line/fill triangle/polygon/restart streams, BOOLEAN/INT64/FLOAT64/UTF8/string offsets and source JSON-string arrays. External decoder compares whole stream bytes including padding/restarts and raw metadata values/bytes; raw JSON comparison detects opaque-number/unknown-field changes. Controls reorder IDs, strip padding, change restart words or numeric lexemes and must be detected. Codec/profile acceptance is not inferred from common writer/validator agreement.
- Rust request/error/default matrix and CLI hidden-worker migration test; invalid syntax/limits/source/refusals make no candidate or source write. Negative control directly writes source and independent preservation monitor catches reached write/finalization failures. No bytes-smaller requirement: incompressible valid input may grow.
- Filesystem probes in isolated subprocesses: CWD changes at first/final event; dot and canonical ancestor-symlink spellings; leaf symlink/socket/FIFO/directory refusal; hardlink named-entry bytes/inodes; portable permissions and platform readonly behavior; source deleted/renamed/replaced in observers; same-length changed bytes with restored mtime. Remove final comparison/capture in controls and expect detection. Windows/macOS/Linux each need actual supported-path/replace proof, not transferred Linux results.
- Fault seams exercise bounded read, planner/reservation/native encoder/counting serialization/candidate write/permissions/flush/sync/install and precommit cleanup separately. Prove reached fault and exact preserved selected/alias bytes; retained cleanup paths and secondary causes are independently inspected. Postcommit uncertain-name reoccupation must survive. Controls suppress cleanup, emit after seal or rewrite committed success and must fail monitor assertions.
- First/final observer failure, prepermission cancellation, native-call boundary cancellation, reentry/reuse and repeated/concurrent independent targets use one-attempt event traces. Control nesting is detected. Same-target concurrency is unsupported; deliberately demonstrate final-check/install competitor window and mark unconditional F0 Replace behavior as a limitation, rather than counting it as no-loss proof.
- Each draft limit has valid maximum and max+1 cases plus small-many-views, metadata/schema cardinality, huge key/number lexeme, deep source and count/stride/host overflow controls. Compute B(N,S) against pinned primary codec bound across N=0/1/255/256/257 and S=4/8/12, broader admitted-stride block-boundary cases, compressed-identity source bytes and decoder scratch; malformed zero-count source policy remains format-owned. Sensitive estimator variants omit parser slots/keys, source, BIN tail, output GLB or producer bridge and must be rejected by independent capacity accounting.
- Root measures actual requested-capacity high water, fixed-worker RSS, scratch bytes and descriptors on maximum and typical admitted inputs, recording sampling/platform/artifact identity and allocation/reallocation overlap. Every estimate coefficient receives source audit and sensitivity evidence; RSS is reported separately. Producer archive inventory/final encoded max_bytes and actual pinned vector consumer metadata/picking remain correct.
- This bounded standalone operation is deliberately Rust plus the migrated existing CLI worker; no standalone Python binding is added or advertised in this draft because there is no current Python caller to migrate. Existing producer Python paths still require installed parity for compressed artifacts and typed failures. A future advertised standalone Python binding must use run_job and prove shared request/default/error, original primary callback exception, cancellation/result identity, permission/source preservation and installed-wheel platform parity. Never claim three-adapter standalone parity from this slice. Applicable final CI, wheels and official Blender remain gates; Blender regressions do not prove vector-draft decoding.

## Remaining precode adjudication

Concrete profile/default/permission/source-conflict choices above are now proposed consistently, not accepted facts. Whole-slice reviewer must resolve raw rewrite/allocation layout and generic checkpoint integration with codec lane, proposed compressed-identity/C1 decoder extraction and 2D+3 scratch, exact JSON growth bound and parser/hash-key coefficient accounting, generated-boundary fixture coverage, Windows permissions/handle/replace feasibility, producer bridge memory ownership, and the deliberate Rust/CLI-only standalone facade scope. Any failed proof changes its owning choice/representation; no uncompressed fallback or silent generated-role omission is authorized. After choices and focused probes resolve, final implementation/evidence requires separate nonauthor review and exact-source applicable platform acceptance. Parent #113/#121/#125/#126 and full A2/release gates remain open.
