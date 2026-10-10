# DRAFT revision 2026-10-10 — vector compression lifecycle

**Gate remains HELD.** This additive revision resolves preparation inconsistencies identified by the independent precode review. It grants no implementation approval and establishes no codec, serializer, allocation, generated-artifact, filesystem or platform acceptance. Corrected C1 final-source acceptance and merge must finish first; bounded coordinator probes and fresh whole-slice adjudication must then resolve the remaining gates.

The prior lifecycle contract SHA-256 `568f0ef8324aeb98890db427ea06bf3d75edc1b2172686b1741f0375f64c05b0`, decisions and integrity freeze remain byte-exact in their original locations. Historical source examined remains `2ebfb749e9d71fdb72507fd1203472b226f23156`; no later C1 correction is retrobound to that audit. This revision reads codec proposal `c560f035786e2d966185f9fde9cbd6ac31bd61161fc23d25ff79a92c90561e1e` and independent precode review `17ae5e13a9df36adefd08d65166b6b493ce39b38251b2f766bf3a492b4b2171e`. All actual input hashes and preservation checks are in this revision's integrity.json.

The review's bounded Python results are 1,088 codec integer-formula comparisons and 1,024 metadata modular-framing checks; the fixed maximal-decimal meshopt object is 241 bytes. These support arithmetic consistency only. They do not prove FFI output/ABI, the complete JSON growth bound, actual allocation coefficients, selected-framing consumer conformance or platform behavior. This lifecycle lane ran only file hash/JSON integrity checks, with no new target or arithmetic probes.

## Smallest real operation and shared owner

Standalone exposes one intentional Rust in-place request/function/result and migrates the existing hidden CLI compression worker. No new standalone Python feature is proposed. Installed Python remains an actual consumer through vector production and its new ResourceLimit error mapping. The pure byte codec owns physical views and ordinary tagged checkpoint errors; standalone owns paths/capture/control/finalization/F0 Replace. Vector production remains inside its existing one Attempt and accepted inventory. No nested compression run, per-view pool, full C1 inspector, public IR, framework, path registry or general decoder pipeline is justified.

```rust,ignore
struct CompressionRequest { path: PathBuf, limits: CompressionLimits }
// in_place(path) uses one core default owner; with_limits(...) changes it.
struct CompressionResult {
    output: PathBuf,
    receipt: CompressionReceipt,
    cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
fn compress_file(request: CompressionRequest, run: &RunControl)
    -> Result<CompressionResult, JobFailure>;

// Private codec; no runtime/path/adapter imports.
fn prepare<'a, E>(input: &'a [u8], limits: ValidatedCompressionLimits,
    checkpoint: impl FnMut() -> Result<(), E>)
    -> Result<Plan<'a>, CodecError<E>>;
fn encode<E>(plan: Plan<'_>, checkpoint: impl FnMut() -> Result<(), E>)
    -> Result<Encoded, CodecError<E>>;
```

Receipt fields are only consumed finite facts: before/after byte counts, physical views/raw/newly encoded/already compressed counts, accessor count, admitted logical/decoded work and estimate/version. No unproved scene/bounds/metadata validation facts or report sidecar is introduced. Freeze all required receipt/embedded producer-report serialization before event closure and publication. Extension stream count is physical whole-view count, independently of any accessor's count/window.

## Source/path/permission/publication decisions

1. Validate a nonempty file path and raw limit algebra without filesystem mutation, claim one Attempt, and resolve F0 supported file publication before candidate creation. Resolve absolute requested spelling/canonical source/replacement path and parent before observers. Final source must be regular, not a symlink/device/socket/FIFO/directory. Ancestor symlinks may be canonicalized initially; subsequent namespace/ancestor mutation is outside the stable ordinary-filesystem domain.
2. Open read-only, compare admitted metadata with opened regular object and held file identity, capture immutable source with bounded cancellation-aware reads and excess/short-read checks. Original absolute spelling and canonical path stay fixed. Captured bytes/identity are not a hostile concurrent snapshot. CWD changes in first/final observers cannot redirect any subsequent path or result.
3. In-place output equals captured canonical source by construction. There is no arbitrary destination, force/create-new/output-policy request flag or same-file exclusion. Standalone selects F0 Replace once. Hard links are admitted with named-entry semantics: selected entry gets a new inode, other aliases and old open readers keep original object/bytes. No all-alias mutation is promised.
4. **Windows readonly source is Unsupported during admission, before observers or staging.** Independent static locked-dependency review found tempfile 3.27.0 persist resets candidate attributes to FILE_ATTRIBUTE_NORMAL before MoveFileExW, so preseal copied readonly state cannot survive unchanged F0 publication. This is a source-derived design correction, not an executed Windows defect. Admit writable Windows sources only; do not clear the original source's attributes, mutate original permissions, repair permissions after commit or rewrite F0 publisher for this slice. Windows hidden/system/ACL/other attributes are not covered by portable permission preservation.
5. Unix captures/applies portable fs::Permissions to the private complete candidate before sealing. Failure is precommit Io and retains source. Windows writable-source candidates retain the admitted writable portable state through existing publication, subject to actual platform proof. No preservation of inode, timestamps, owner, ACLs or xattrs is promised. Both paths need platform-specific evidence; Linux results cannot qualify Windows/macOS.
6. After the final ready observer returns, recheck original spelling/canonical path, leaf regular type, held identity, length/mtime and captured permission state; then stream-compare exact source bytes with one 64KiB buffer and cancellation checks, including stable before/after read metadata. Detected rename/delete/replacement/symlink/permission/byte changes are Conflict, including callback same-length byte writes with restored mtime. On Windows callback transition to readonly is a detected source conflict, not permission clearing. Close source/recheck identity handles and observer admission before seal/install.
7. Retain F0 completed candidate sync/seal and named-entry Replace via persist, with no predelete or weaker copy fallback. **No atomic conditional update exists.** Final source recheck and install are separate. Caller must keep selected source/namespace stable through actual installation; same-target competing writers/compressions and hostile ancestor mutation remain excluded. A competitor after final check can be overwritten. A deterministic seam must demonstrate this limitation; repeated preflight is no substitute for compare-and-replace.
8. Explicit staging.fail owns precommit cleanup and secondary/retained-path diagnostics. A successful install remains core success despite later adapter output/cleanup diagnostics. Never unlink a reoccupied/ownership-uncertain postcommit temporary name. F0 supplies no crash-durability guarantee. Observers retain their nonpanicking trait contract; process termination, unwinding or internal infallible allocator abort do not receive invented graceful-return/guaranteed-cleanup claims.

## Selected framing and exact identity admission

The codec proposal owns two explicit emission/admission branches. Metadata view/component alignment and absolute chunk boundaries are separate checks. Core-only input/emission selects four-byte chunk padding, zero BIN slack 0..3, and:

`T_core(J,U) = 28 + align4(J) + align4(U)`.

Declared EXT_structural_metadata selects the proposed absolute-eight boundary interpretation:

`j8(J) = align8(20+J)-20 = J+p_json, p_json in 0..7`,

`BIN_origin = 28+j8(J)`,

`T_metadata(J,U) = 28+j8(J)+align8(U)`.

JSON chunk end 20+j8, BIN origin and final GLB end are absolute-eight aligned, and chunk lengths remain multiples of four. BIN slack 0..7 must be zero. For an already framed identity input, admitted J includes its original padding and must already meet the selected rule. A counting pass for a rewritten/bridge document measures actual JSON bytes before selecting padding. Every add/align/range/u32/host/isize conversion is checked before relevant allocation.

This interpretation is arithmetically supported by the independent review, but primary/actual consumer/framed artifact proof is HELD. Current glb::encode_glb's universal four-byte emission is not retained as passed for metadata output or producer bridge. No C1 metadata framing/semantics acceptance transfers. Keep its current source observation distinct from actual invalidity findings.

**Identity cases:** all-raw plan returns unchanged original GLB bytes with no synthetic meshopt declaration; existing-compressed canonical embedded buffer0 plus no-URI virtual fallback buffer1 ATTRIBUTES/NONE plan validates bounded streams/fallback coverage and returns unchanged original bytes. Each creates one owned candidate copy while standalone source capture remains alive. The operation still follows named-entry Replace rather than silently changing result semantics to a no-install path. One-view decoded scratch is discarded before the next view; no virtual fallback buffer or retained decoded array inventory is allocated.

Identity is conditional on selected-envelope validity. Do not claim every historical/generated metadata GLB is valid merely because a current writer produced it. An independently authored malformed legacy envelope is InvalidInput for standalone. Malformed/out-of-profile internally authored producer bytes are InvalidState at the producer owner; the bridge must emit the selected valid framing to retain generated semantic support. Repairing historical identity bytes implicitly, switching padding profiles, dropping metadata or refusing quantized/polygon/raw-metadata semantics is not an authorized fallback.

Raw JSON subtrees are the source of truth: preserve opaque number lexemes/schema/extras/node values without Value conversion. Root/buffer/view targeted key/whitespace normalization is a named permitted serialization change; do not advertise all source JSON bytes unchanged for repack. Untouched raw value subtrees remain byte-exact. Count then emit RawValue into the final GLB directly; no separate output JSON Vec. CountWriter/output-key escaping and full J+512V+1024 bound remain HELD.

## Complete candidate/control and causal errors

One standalone sequence is: begin/pure validation -> platform and source/permission admission -> immutable capture -> bounded raw/format plan/preflight -> start event -> encode/materialize/write one staged candidate -> Unix candidate permission copy and finalization -> freeze receipt -> final ready observer -> source/permission/identity/exact-byte recheck -> close handles/events -> seal/sync -> consume F0 Replace -> typed committed result.

All fallible producer work and callbacks precede seal/permission. Checkpoints run before/after JSON, every view/native encoder/decoder, counted serialization and materialization; source read/recheck/candidate writes are chunk-cancellable. One native view call and bounded JSON scan are indivisible; no instantaneous cancellation guarantee. Codec receives only a generic ordinary checkpoint and returns its exact tagged causal error, never imports runtime or emits user callbacks during serializer passes.

First prepermission fatal cause accepted by Attempt is primary; later causes are secondary. Publisher failure after permission owns its outcome; postpermission cancellation cannot rewrite it. Every staging-owned failure explicitly cleans or reports retained scratch through staging.fail. No substring classification or callback exception priority ladder.

| Cause | Standalone mapping | Internally authored producer mapping |
| --- | --- | --- |
| Invalid request/limit algebra | InvalidRequest | Existing vector request owner validates its choices once. |
| Malformed consumed source/bitstream | InvalidInput with codec cause | InvalidState with codec cause; generated malformed bytes are internal producer failure. |
| Valid excluded representation | Unsupported with codec cause | InvalidState with codec cause when producer authored it; explicit user/backend support refusal stays at its owning request boundary. |
| Limit or recoverable owned reservation failure | ResourceLimit with causal detail | ResourceLimit, fatal through fits_feature and final encoding; never FeatureRejected/skip_invalid or cheaper uncompressed retry. |
| Native zero/invalid encoder result under admitted plan | InvalidState with EncodingFailure(stage) cause | InvalidState with EncodingFailure(stage) cause. No invented invalid-source explanation. |
| Generic checkpoint abort/observer failure | Original selected causal JobError | Original selected causal JobError under the same existing Attempt. |
| Actual source/candidate/publisher I/O | Io retaining OS cause/path | Io at actual storage boundary, preserving cause. |
| Detected selected source identity/bytes/permission change | Conflict | Standalone source rule does not enter the pure producer codec. |

Minimal JobErrorKind::ResourceLimit plus explicit CLI category and installed producer-Python ResourceLimitError/kind mapping is justified. Error adaptation must preserve codec source/stage in the existing causal error field; it must not flatten it to a message or masquerade as Io. The small internal constructor/mapping needed to retain a non-I/O codec cause is a reviewable runtime/error-owner implementation task, not a new error framework. Corrupt decoder source is distinct from native encoder failure and requested resource refusal.

## Same proposed finite defaults; all unproved

| Policy value | Proposed default |
| --- | ---: |
| source bytes | 33,554,432 (32MiB) |
| JSON bytes | 1,048,576 (1MiB) |
| JSON depth (root zero) | 64 |
| JSON value nodes (keys excluded) | 65,536 |
| physical views | 4,096 |
| accessors | 4,096 |
| aggregate logical view bytes | 33,554,432 (32MiB) |
| candidate GLB bytes | 67,108,864 (64MiB) |
| estimated requested working bytes | 134,217,728 (128MiB) |

Aggregate work charges each distinct physical view object once, including decoded compressed lengths and raw metadata; shared references to one view are not recharged, distinct overlapping view objects are charged separately. Individual maxima need not simultaneously fit the estimate. Actual default generated corpus/large metadata/schema/default/raised-cap coverage is unexecuted. Larger finite standalone values are explicit request choices, not imported C1 defaults. Positive byte/node/cardinality values and checked host/u32/isize algebra are required; depth zero is a valid restrictive setting. No maximum-sized allocation is made for a small input.

One core default/validation owner supplies the producer's codec profile; low stages consume validated values. Existing vector max_bytes is exact final content size, not source memory or RSS. A larger requested max_bytes is not itself invalid; actual candidate beyond finite codec/resource profile fails ResourceLimit. Fitting/final encoder use identical profile/selected framing/cause propagation. No speculative public producer limit option or fallback is introduced by this draft; deliberate finite support must be documented.

## Phase storage/work model — HELD estimate v1

I=owned source GLB length; J=input JSON length; K=admitted node count (pre-admission min(max_nodes,J)); V/A=physical view/accessor counts; C=65,536 fixed chunk buffer; X=actual path storage; Q=1,024 provisional fixed state/receipt estimate. E_JSON=8J+256K+C and E_PLAN=2J+128K+256(V+A) remain unproved engineering coefficients. Pin actual types/parser/hash-key capacities, scratch and reallocation overlap before treating them as faithful requested-storage estimates. They are not hard allocator bounds/RSS guarantees.

For eligible positive divisible-four S<=256, b=min(256,16*floor(8192/(16*S))) and:

`B(N,S)=1+ceil(N/b)*S*(S/4+ceil(b/64)+b)+max(32,S+S/4)`.

Current S=4/8/12 reduces to 1+ceil(N/256)*S*(S/4+260)+32. Use quotient/remainder ceil and checked operations. Selected ineligible natural strides remain Raw after validity checks; do not turn legitimate U8/U16/matrix/shared views into malformed source. Independent formula comparisons support literal source agreement, not actual FFI/capacity/encoding acceptance.

Repack BIN upper U0=sum(raw logical lengths)+sum(B per encoded physical view)+7V; final selected chunk padding is separately included by T_core/T_metadata. Virtual fallback byteLength is address space only, never resident payload. JSON upper J0=J+512V+1024 is a proposed full rewrite bound; CountWriter must validate it against actual output before retention. T0 is the selected T(J0,U0); actual count/materialized lengths are checked against exact output cap again. Source, plan, maximum encoder-bound BIN tail and copied final GLB coexist when this schedule uses them.

For existing compressed identity, Dmax=max logical decoded compressed view length, bounded by aggregate_view_bytes. Proposed helper scratch D_stage=4*ceil(Dmax/4)+Dmax<=2Dmax+3 includes aligned backing and byte copy; discard per view. Narrow accepted-C1 stream-helper extraction/ABI/capacity remains HELD. All-raw identity has D_stage=0. Both identity branches allocate candidate I separately from source capture I. Identity does not allocate U0 or rewritten JSON.

Explicit branch-sensitive standalone phase peaks:

- Capture/admission: I+C; I+E_JSON.
- Repack encode/materialization: I+E_PLAN+U0+T0+C.
- Existing-compressed validation: I+E_PLAN+D_stage+C.
- All-raw/compressed identity candidate copy: I+E_PLAN+I+C. If decoder scratch survives while candidate copying, add D_stage here too; proposed schedule requires decoder scratch to drop first and its proof is HELD.
- Postcodec candidate write/final comparison: I+candidate_owned_capacity+C, with plan/BIN/decoder dropped when ownership permits.

`E_standalone=max(actual admitted branch phase peaks)+Q+X`.

Count requested capacities and growth overlap, not merely lengths. No separate JSON Vec is planned; if one appears, add full capacity. Source and owned identity candidate are distinct allocations even when bytes match. Drop plan/BIN/decoder when no longer used, retain source through final comparison. Candidate scratch file occupies disk, not a second in-process Vec. OS cache/native stacks/allocator overhead/infallible serde allocation abort remain outside a graceful requested-storage limit claim. Receipt/evidence records estimate version and actual capacity facts; RSS/scratch/descriptors measured separately.

Producer peak is maximum of bridge and codec phases, not sum of mutually exclusive phases. H=live persistent geometry/model/worker state; O=old generated Value/BIN still alive. Bridge phase H+O+I_bridge+serializer_scratch, emitted with selected T_core/T_metadata. Codec phase H+O_if_alive+E_codec, with I_bridge already counted once in E_codec. Consume/drop old Value/BIN before codec when valid; count every real clone. Source parser overhead does not excuse uncounted producer Value; producer-wide RSS is not bounded by the codec estimate.

Work facts: source capture/final comparison <=2I plus bounded excess detection; per-view logical sum <=aggregate cap; one native work unit per eligible encoded/compressed-identity view; no retained decoded virtual buffer. Nested raw JSON may rescan substrings, so (2*depth+4)*J remains only a proposed scan-volume metric, not certified CPU/adversarial hash work or instant cancellation. Arithmetic, formula, coefficient and deadline claims remain separate.

## Producer bridge and dependency ownership

The producer creates authored Value/BIN under existing geometry/feature acceptance. A small bounded bridge must count its JSON and emit selected metadata absolute-eight or core-four framing before shared prepare. Current glb::encode_glb4 is not assumed valid for metadata. If framing is shared, expose only the actual consumed private format helper; do not route producer through standalone source capture or full validation. Source-generated semantic roles remain mandatory even where legacy byte envelopes require correction.

Both fits_feature and final candidate encoding use the same bridge/planner/limits/checkpoints. ResourceLimit and tagged infrastructure/checkpoint causes remain fatal; only independently defined feature invalidity may become FeatureRejected. Update actual vector reuse dependency fingerprint to include moved/introduced codec/raw/framing/helper dependencies; a hash list change itself is not reuse correctness proof. Existing one-Attempt final archive inventory, conversion report and max_bytes remain before outer publication permission. No private workspace content is published in-place by compression.

## Remaining actual coordinator probe tasks

P0: Finish corrected C1 independent exact-source/artifact/package/CI acceptance and merge. No subsequent target execution/implementation begins before this prerequisite.

P2 after P0: Pin new baseline/compiler/serde/std/meshopt/tempfile/same-file inputs. Audit actual Slot/key/hash/plan/edit/path/receipt types and requested capacities/reallocation/root/string scratch. Run bounded independent serializer/count/identity-copy scaffolding with sensitivity controls removing source/parser/key/slots/BIN tail/final GLB/identity candidate/bridge storage. Adjudicate all coefficients and nine defaults; each max/max+1 control adjusts other caps explicitly. Record RSS separately. No author estimate becomes accepted merely through integer formula agreement.

P3 after P0: Independently author full G1–G9 generated/raw preservation cases and actual portable/native generated artifacts: quantized/unquantized position bytes, padded IDs, feature tables, line/polygon/restarts/fills, raw BOOLEAN/INT64/FLOAT64/UTF8/noData/list fields, external UINT64/structural arrays, shared/interleaved matrices/partial tails. Include corrupt codec bitstreams, mixed raw/fallback graphs, repeated/all-raw/compressed identity and opaque number tokens. Selected metadata framing needs JSON/BIN residues, absolute view/component offsets and zero-slack 0..7 controls versus core 0..3, literal primary interpretation and actual pinned consumer probes. Classify legacy malformed generated envelopes explicitly; no default exclusion or semantic omission.

P4 after P0: Exercise selected-framing bridge old Value/BIN lifetimes and actual fitting/final encoding under one existing Attempt. Reach native EncodingFailure, authored InvalidInput/Unsupported and ResourceLimit separately; verify fatal typed routing, no skip_invalid/uncompressed fallback, original checkpoint causes, reuse fingerprint and exact accepted inventory/final max_bytes. Add producer source/bridge/worker overlap measurements without falsely promising whole-process memory.

P5 after P0: Root coordinates bounded Linux/macOS/Windows source/path/open-handle/permission/capture/recheck/Replace/fault controls at considerate two-worker priority. Windows readonly admission must have zero observer/staging/original-permission side effects; a control clearing source readonly must be caught. Unix candidate mode copy, writable Windows publication, callback permission/byte/identity changes, CWD, aliases and source handles require actual platform evidence. Precommit failure preserves source/alias bytes; cleanup and postcommit reoccupation stay typed. Deliberately demonstrate non-CAS final-check/install competition and no callbacks after seal.

Final fresh whole-slice precode adjudication binds this additive revision plus exact post-P0 inputs/probes. Only afterward can authorized production work begin, followed by separate nonauthor final-source evidence, actual CLI/installed producer-Python parity, pinned browser vector decoding/table picking and applicable final CI/wheel/official Blender gates. Blender regression success does not establish vector metadata/picking. #113/#121/#125/#126, full A2 and main/tag/release authority remain unchanged.
