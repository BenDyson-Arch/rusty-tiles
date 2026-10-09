# Mesh request and follow-on runtime scope audit

Audit checkout: `/tmp/rusty-tiles-115-foundations`, `d69ba2a`. Read-only audit; no production changes or commits. No applicable AGENTS.md found in checkout or its ancestors. Current implementation and `docs/architecture/api-contract.md` are candidates, not correctness authorities. Recommendations below are proposed product boundaries, not implemented behavior.

## Recommendation and decision ownership

Name the next work **F1 mesh foundation**, with **F1a local static mesh-to-archive** its first independently reviewable milestone. Do not call the local subset a completed mesh replacement. F1a needs neither directory publication nor vector transactions. Keep those as separately scoped slices D1/D2 and V1/V2 under #113.

F1a should exercise a real parser -> validated mesh -> spatial leaves -> typed tileset -> archive -> F0 publication path, including a forced multileaf example. Start with embedded GLB, static triangles, untextured supported material factors and POSITION/optional NORMAL. A narrower untextured milestone makes the plumbing independently provable before atlas, texture, CRS and LOD algorithms. Textured photogrammetry remains a required follow-on release decision, not implicitly waived.

| Decision | One owner | F1a disposition |
| --- | --- | --- |
| Input interpretation/eligibility, supported semantics, float tolerances | Mesh source boundary | Explicit glTF local metre/Y-up; static embedded GLB; declared source allowlist |
| Request validation and effective processing defaults | Mesh request module | Positive leaf-triangle limit; no force/CRS/codec/environment branching in workers |
| Partition, primitive fidelity and bounds | Mesh implementation | Complete leaf geometry; no coarse proxy/simplification or atlas remapping |
| Typed generated hierarchy and serialization | Generated tileset boundary | Explicit hierarchy with leaf content; independently justified geometric-error/refinement behavior |
| Archive names/index/container completion | Existing private archive encoder | F0 container contract; exact generated inventory |
| Abort/events/permission/file installation | F0 runtime | Same RunControl/JobFailure/OutputPolicy; no mesh publication special case |
| Argument strings, callback objects, exception/status/JSON rendering | CLI/Python adapters | Parse into the same request, no duplicate domain validation |
| Pre-0.4 supported functionality | Release scope owner | Explicit retained/replaced/unsupported decision for every excluded current mesh feature |

Primary glTF reference confirms metre units/right-handed Y-up and node TRS/global-transform composition: [Khronos glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#coordinate-system-and-units), sections 3.4 and 3.5.3. Local-to-tile axis handling, routing/error values, chosen support limits and any later placement convention are separate proposed contracts requiring independent fixtures and target-viewer evidence. Existing signs/constants are not a standards oracle.

## Confidence and decisions before coding

Source observations and previously recorded natural/fault-injected reproductions are high-confidence within their pinned environments. The F1a/D1/V1 scope boundaries are recommendations with moderate confidence, not final public API decisions. Before F1a coding, approve the untextured first milestone, scene-selection rule, accepted core material fields, coordinate/storage tolerances, explicit leaf limit versus measured convenience default, and portable distribution/reader resource policy. Before D1/D2 coding, select supported platform primitives and visibility/recovery guarantees. Before V1 coding, select frame policy and a bounded feature-delta representation. These decisions are finite and belong to the owners named above; broad future enums/defaults need not be frozen.

## Concrete implementation evidence

| Finding | Current evidence | Required boundary |
| --- | --- | --- |
| Public mutable option bag permits contradictions; CRS setter interprets other fields already assigned | `src/tile.rs:33-59,89-107` | Private owned request; one explicit coordinate choice; validation independent of builder order |
| CLI JPEG default differs from Rust/Python lossless | `src/main.rs:439-445`; `src/tile.rs:67`; `bindings/python/src/lib.rs:325-328` | One domain processing-default source; later texture delivery defaults source-faithful with no unrequested loss |
| CRS guessed from magnitudes, and supplying an offset changes Auto into WebMercator | `src/mesh.rs:261-311` | Explicit local interpretation never changes with coordinate magnitude |
| Geographic offset ignored by core path but forbidden by CLI | `src/mesh.rs:334-338`; `src/main.rs:1055-1058` | Later horizontal owner defines units/order, applies or rejects uniformly |
| Rust placement allows arbitrary floats; Python performs finite/range checks; CLI requires origin for rotation while Python helper does not | `src/georef.rs:11-24,29-33`; `bindings/python/src/lib.rs:294-320`; `src/main.rs:1004-1007` | Later placement type owns finite/range and origin+rotation invariant |
| Job creates output parents/work before missing encoder/coordinate/source checks | `src/tile.rs:183-205,235`; `src/output.rs:66-72` | Pure validate, read-only source/capability resolution, then private execution |
| Small-source shortcut precedes source eligibility checks | `src/tile.rs:226-235`; static/extension/attribute/material checks at `1046-1108` | Same acceptance independent of input size, leaf count and execution optimization |
| maxBytes is a source-wrap threshold, not a tile byte or RAM limit; texel-density accepts only zero | `src/mesh.rs:100-101`; `src/tile.rs:180-181,228`; `src/main.rs:456-477` | Remove meaningless/ambiguous options from F1a; specify new limits precisely |
| Loader silently returns for missing POSITION and fills missing/mismatched normals/UVs; scene selection differs with scenes/default-scene presence | `src/mesh.rs:182-207,482-507,541-551` | Source eligibility is explicit; no silently synthesized/discarded accepted semantics |
| Implicit path has minimal report; explicit path publishes with no report then performs fallible metadata/observer work | `src/tile.rs:538-553` | Every F1 success has finalized report; no required postcommit work/callback |
| Broad internals and clap derive are public | `src/lib.rs:6-39,54-57`; `src/tile.rs:123` | Narrow mesh facade; adapter-owned parsing; do not hide unrelated converter APIs indiscriminately |
| Legacy library reporting writes stderr; events cannot fail | `src/report.rs:32-50,88-99` | F1 uses silent F0 default and fallible Observer |

These source observations establish present behavior, not every reachable defect. Existing API executable probes in `bench/architecture_audit/api/` separately demonstrate default drift and callback exceptions with published archives. Those probes remain baseline evidence, not acceptance tests for replacement code.

## Minimal F1a API and phase ownership

Illustrative facade, avoiding options/types that have no choice in this milestone:

```rust,ignore
let request = MeshRequest::local_gltf(input, output, leaf_triangles)
    .with_policy(OutputPolicy::CreateNew);
let result = mesh_to_archive(request, &RunControl::default())?;
```

Owned private request fields; raw builder values validated centrally. Naming the local constructor is sufficient to require coordinate interpretation while only one interpretation is implemented; an extensible coordinate enum is not needed yet. `leaf_triangles` is an explicit positive maximum on each output leaf's triangle count, not a memory/byte/geometric-error promise. Requiring it initially avoids inheriting 20,000 as an unproved default. If convenience defaults are wanted, selecting and measuring one domain default is a pre-acceptance decision; CLI/Python must consume that value rather than copy constants.

Return a small mesh result with committed output, typed mesh report and F0 cleanup diagnostics. Report owns schema version, explicit local mode, selected scene, accepted source triangles, output triangles/leaves, effective leaf limit and coordinate-storage tolerance/observed deviation. Serialize that exact report into conversion.json before finalization. No universal converter/result/report enum or new context object.

- Pure validation: paths/nonzero limits/checked representability/output policy, deterministic request errors before source I/O. RunControl is claimed once even on validation failure, matching F0.
- Read-only resolution: regular source, output/source alias rejection, GLB header/schema/resource declarations and explicit supported-semantic allowlist. Declare scene selection: default scene, or sole scene when unambiguous; otherwise actionable unsupported/invalid-input result. No implicit traversal of all nodes with duplicate child rendering.
- Execution: source reader owns decoded data, node transforms and resource lifetimes; mesh producer owns workers/scratch/receipts. Measured source/primitive/hierarchy limits are explicit eligibility limits, not a claimed hard RSS bound. Avoid unbounded preflight materialization or loading twice. Source-stability assumptions must be documented; mmap safety is not implied by path metadata checks.
- Completion: join producer work, finalize generated content/report/container, emit last synchronous event, close admission, sync+seal+publish through F0. Use the private encoder/runtime boundary; calling public package with the same already-claimed RunControl would be invalid reuse.
- Errors: reuse JobErrorKind/JobFailure. Invalid request, malformed input, unsupported valid semantics, I/O, conflict, observer/cancellation remain distinct. No Error::Message -> universal invalid-request mapping and no legacy Reporter bridge that masks observer failures.

F1a supports finite node-transformed positions and preserves accepted triangle/material/normal semantics within stated storage tolerances. Reject singular/unsupported transforms, animation, skins, morphs, unsupported attributes/extensions/materials and textures before staging. Missing optional NORMAL stays absent unless an independently specified normal policy is selected; mismatched accessor counts are invalid input. No guessed source interpretation, auto-wrap path or silent source-semantic loss. Select a declared coordinate-magnitude domain and test cancellation between bounded producer units.

## F1a independent acceptance and stop conditions

Existing `validate` is not an independent fidelity oracle: a sibling audit reproduced accepted GLB missing required BIN data and geometry outside its tile box. F1 test decoders/corruption controls must establish their own sensitivity; a successful current CLI validate result is insufficient.

1. External-consumer Rust test imports only facade; CLI and installed Python wheel create equivalent requests. Same effective values, errors and report content; absent coordinate mode/nonpositive limits fail before missing-source/output checks and create no output parents.
2. Hand-authored GLBs independently decoded by a test reader: indexed/nonindexed triangles; nested noncommuting node transforms; nonuniform and negative scale; repeated mesh instancing; absent/present normals. Compare transformed oriented triangle **multisets** and material assignment, not vertex totals or previous hashes. Include duplicate coincident triangles to detect accidental deduplication.
3. Force multiple leaves with a very small triangle limit. Every leaf respects it; decoded leaf union has every accepted source triangle exactly once. No parent proxy duplicates leaf geometry. Independently check content URI closure, archive index/CRC, typed hierarchy, leaf and ancestor bounds in one declared tile frame, and all finite serialized numbers.
4. Independently test degenerate/large-coordinate inputs and numeric storage tolerance. Reject values outside documented representability domain; bounding boxes conservatively enclose decoded geometry. A viewer test verifies leaf-only hierarchy traversal and meaningful geometric-error/refinement choices; zero placeholder error is not accepted solely because current tests pass.
5. Source eligibility cases run below/above former wrap thresholds and with one/many leaves: animations, skins, morphs, extra attributes, texture/material/extension variants, malformed accessor counts, unsupported scene/resource declarations. No size-dependent bypass.
6. All successful cases publish and return identical typed conversion report, including one-leaf inputs. No library-default domain stderr. Fail report/content/container finalization before publication and preserve existing output.
7. Reuse F0 cancellation/observer/publication tests through the mesh consumer: early/final callback error, simultaneous fatal producer/cancel, conflict race, Replace preservation, joined workers, cleanup diagnostic. Original Python exception only if selected primary; no controlled callback/signal checks after commit.
8. Measure increasing source triangle/primitive counts at fixed leaf limit: RSS, scratch high-water mark, file descriptors, retained hierarchy and runtime. State measured limits and fail cleanly when declared eligibility is exceeded; do not advertise total-memory guarantees.

Stop/narrow/replace a candidate at the first failed fidelity, hierarchy, source-eligibility or lifecycle gate. Do not regenerate goldens as justification. Old partition/parser/writer code may be retained only after these independent tests; F1a does not require a full algorithm rewrite to pass, nor forbid one where necessary.

## Explicit follow-on release gates and migration

Before naming the whole mesh API migrated for 0.4, resolve each row by verified support or a documented intentional unsupported/removal decision. A subset milestone is not blanket authorization to delete supported use cases.

| Follow-on gate | Decisions/evidence still needed |
| --- | --- |
| F1b supported textures/materials | Source-faithful base-color texels/alpha/sampler/UV semantics; resource forwarding vs atlas; no unrequested resampling/reencoding; **lossless delivery default** based on fidelity, not existing defaults. Exact source bytes are required only if forwarding's product contract states it. |
| F1c placement/horizontal CRS | Finite origin+rotation type; axes, units, ellipsoidal height, offset order; independent CRS/rotation accuracy fixtures and backend eligibility. No automatic guessed CRS compatibility in new core. |
| F1d external-resource glTF | URI-base/resource closure, escapes/symlinks/name collisions/source stability, selected scenes, extensions and richer attributes/materials. |
| F1e coarse LOD/implicit hierarchy | Explicit approximation/error policy, measured/reportable deviations, independent hierarchy/availability/bounds proof and representative viewer behavior; node picking/metadata preservation. |
| F1f optional optimization/codecs | Meshopt decoded equivalence; JPEG/WebP opt-in loss; UASTC executable/capability boundaries and portable wheel policy; justified numeric defaults/limits. |

Rust mutable options/SourceCrs::Auto/force/report signatures change; CLI/Python explicit local coordinate choice and processing-limit names change; CLI JPEG omission behavior later becomes lossless; removed no-op density/ambiguous maxBytes controls need migration explanations. Preserve spellings only through isolated adapters with proven semantics. Keep excluded current functionality tracked under #113, with a pre-release decision rather than indefinitely living inside new core as legacy exceptions.

The broader API proposal over-scopes F1a: Horizontal, placement, all encoders, generic ConversionContext, non-exhaustive future enums, universal report envelope and Directory/Manifest result kinds are not needed here. F0 already settled RunControl/JobFailure/exhaustive bounded events; consume those contracts instead of duplicating them. Required source fidelity and lifecycle invariants remain, but apply only to the explicitly declared F1a input domain.

## D1/D2: separate directory publication scope

Current `publish_directory` checks existence then calls rename (`src/output.rs:234-237`), so it does not establish CreateNew. On Unix rename may replace an empty directory inserted after the check. Two-rename Replace (`242-250`) has a target-absent window; restore failure retains backup but communicates it through a string, and successful cleanup relies on TempDir Drop. See independently reproduced directory race in `bench/architecture_audit/runtime/README.md:13-14` and documented primitive limits in `docs/architecture/platform-evidence.md`. No crash-atomic directory promise is implementable by renaming this helper.

**D1 first scope:** staged/sealed complete tree, CreateNew only, one existing directory-producing terrain/raster consumer. Consumer owns all generated files/report/finalizers, runtime owns completed inventory and publication; one platform boundary chooses a proven no-replace directory primitive or returns Unsupported. Same F0 admission/permission gate, no universal artifact/converter trait. Artifact type/policy capability resolution occurs before execution. Directory scope does not block F1a archive output.

**D2 Replace scope:** decide acceptable visibility window/platform matrix before implementing. Ordinary prepermission failure leaves old destination; after holding old output, install either commits complete new tree or restores old tree. Failed restore yields typed recovery state distinguishing previous-output backup from scratch cleanup; do not parse message text or overwrite an unrelated rival during restore. Committed backup cleanup errors are diagnostics. No power-loss, external writer serialization or crash-recovery claim without separate evidence.

Directory acceptance:

1. Deterministic competitor creates empty/nonempty destination between resolution and install; CreateNew preserves competitor and refuses, on every supported OS/filesystem/toolchain. Unsupported platform policy creates no private output work; no copy/delete fallback.
2. Prepermission cancel/observer/finalizer failure preserves destination and closes producers. Verify files/manifests/reports are complete before seal; no writable API or repeated publication afterward.
3. Inject hold-old/install/restore failure separately. Ordinary restore succeeds with original bytes/inventory; failed restore retains identified old backup and typed RecoveryRequired. Rival at restore destination remains untouched.
4. Successful replacement has truthful commit result even if backup cleanup fails; report retained locations; no uncontrolled TempDir destructor erases recovery data or hides required cleanup outcomes.
5. Competing Replace publishers explicitly either unsupported/serialized under a declared cooperating-writer contract or independently tested; do not imply universal serialization from one process mutex.
6. Real migrated consumer plus platform tests proves phase integration; no generic publisher framework without a consumer. Document absent-target visibility interval and source/target permissions policy.

## V1/V2: separate vector feature-transaction scope

Verified defect: SQL savepoint covers rows (`src/vector/pipeline/store.rs:626-651`) while polygon splitting increments `fragmented_polygons` and appends report bytes beforehand (`159-164`); SQL rollback cannot undo those effects. Natural evidence retains success-like polygon reports/counters for a skipped feature (`bench/architecture_audit/runtime/README.md:9`). Both portable/native readers classify every callback error except Environment as rejection (`src/vector/portable.rs:315-324`, `src/vector/pipeline/source_native.rs:372-380`). Source callback injected ENOSPC is therefore skipped; SQLite errors are flattened into Data (`src/vector/pipeline.rs:42-43`). Neither is an acceptable skip policy.

Additional unproved transaction surface: source frame is selected before feature acceptance (`src/vector/portable.rs:291-296`, native reader before `src/vector/pipeline/source_native.rs:354`). Decide whether frame is resolved from source metadata or chosen from first accepted geometry; a rejected first feature must not incidentally determine an undocumented policy. No published-orphan bug is asserted: `src/vector/pipeline/reuse.rs:424-429` prunes unreferenced content and exercised controls found none. Broader inventory behavior remains unproved.

**V1 first scope:** portable GeoJSON ingestion with one real vector archive consumer; typed `Rejected(FeatureRejection)` versus `Fatal(JobError)` at source/geometry/store boundary. Only explicitly constructed feature rejection is skippable. Feature-local acceptance delta owns counter/report/fragment/frame/cache changes; SQL changes stay in savepoint until acceptance. Merge success effects only after the whole feature succeeds. If applying accepted reports or SQL release/commit fails, abort the job, do not skip the feature. Avoid an all-errors-whitelist or per-backend priority ladder. Reject diagnostics identify source/layer and outcome separately from accepted-operation diagnostics.

**V2 follow-on:** migrate native readers to same typed outcomes; prove speculative tile/LOD/dedup/reuse inventory acceptance and report/cache ownership; source-spool/resource finalization and mesh-like job integration. Keep geometry repair/CRS/LOD algorithms untrusted and separately evidence-gated; V1 does not certify them or rewrite all vector geometry.

Vector acceptance:

1. Reproduce oversized polygon+valid point natural fixture: published rows/fragments/source IDs/counters/report success records agree; rejected polygon has only rejection outcome, no accepted fragmentation count. Strict mode publishes nothing.
2. One feature accepts some fragments then rejects a later fragment: SQL, counters, reports, frame/cache/accepted fragment identity have no partial acceptance. Test a rejected first feature followed by a valid feature under declared frame policy.
3. Inject source callback I/O, SQLite resource/storage failure, report-write/flush failure, rollback/release failure, cancellation and observer failure: all abort under skip-invalid, preserving original causal kind and prior destination. Only independently declared geometry/property rejection can continue.
4. Independently query accepted spool rows and decode final archive source IDs; compare them to reports/counters. Reports must not be the only oracle for their own counters. No publish after required report/resource finalization fails.
5. V2: rejected candidate before budget acceptance, reused content, multiple contents/implicit subtrees/resources and cleanup failure; final inventory equals accepted reachable dependencies with no missing or surplus generated tile. Do not substitute file-name heuristics for source/resource ownership.
6. Repeated/concurrent runs do not mix counters/timings/sinks/caches; source ordering and backend choices obey documented determinism/resource policy. Native checks remain explicitly unverified until their environments run.

F0 blocker observed in this read-only scope: none new. That conclusion is limited to API integration/source inspection; parent acceptance owns final builds, platform tests, installed-wheel tests and merge review.
