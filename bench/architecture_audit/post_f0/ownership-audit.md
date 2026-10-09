# F1 ownership audit and remaining subsystem disposition

Read-only review of `d69ba2a7facb99b5b3f5a6efba4e4039bef26be5` in
`/tmp/rusty-tiles-115-foundations`. No repository edits, new probes, or test runs
in this pass. No AGENTS.md found in this worktree. References below are source
inspection, not a complete call graph or independent algorithm certification.
Earlier proposals and comments are hypotheses; nothing earns retention by being
outside the first implementation slice.

## Immediate conclusion

Use F1 to establish one real mesh producer feeding F0's completed-file publisher
under one run. Do not migrate every converter or build a universal pipeline.
Start with an explicitly supported static local-metre triangle-mesh profile and
real GLB encoding; refuse unsupported source semantics before staging. Choose
and document whether that first profile supports textures, hierarchy, node
features and projected coordinates. An unchanged GLB wrapper alone does not
prove a mesh producer's lifetime, encoding or memory contract.

No new F0 acceptance blocker was established here. F0 is opaque packaging,
not scene closure, mesh fidelity, directory replacement or worker certification.
The existing mesh path still uses the legacy publisher; F0 evidence therefore
does not confer its guarantees on that path.

## Concrete mesh ownership failures and unproven properties

| Boundary | Source evidence | Required disposition |
| --- | --- | --- |
| Setup precedes source/capability validation | `tile.rs:183` starts `output::Job`; encoder check at 184, loading at 194, coordinate option checks at 202, supported-source check at 235 | Replace orchestration order: pure request validation, read-only source/capability resolution, then execution workspace |
| Source semantics depend on size/placement | `tile.rs:226–235` wraps a small unbaked scene before `validate_source`; 1046 rejects animations/skins/extensions/additional attributes for tiled scenes | One source-profile decision before execution; no budget-triggered semantic loophole. Wrapping is an explicit separate operation if offered |
| Multiple parsing/resource policies | `mesh.rs:137–161` parses GLB or calls `gltf::import_buffers`; `tile.rs:1046,1112` reparses JSON; `tile.rs:572` reparses sampler state; `tileset/resources.rs:18–80` separately resolves wrapper dependencies | Resolve document/resource semantics once into consumer-owned prepared input; never invent a generic resource resolver from one converter |
| Image references are not a closed inventory | `mesh.rs:406–413` joins URI directly to source parent and follows filesystem metadata; wrapper resolver canonicalizes within root at `tileset/resources.rs:65–78` | Replace mesh URI admission; direct joining does not establish containment, scheme handling, alias policy or a complete closure |
| Source representation has competing storage states | `mesh.rs:45–50` stores path/offset/length plus optional owned bytes; 381–403 constructs disk ranges or cloned buffer views | Use a private sum type for encoded byte source (checked file range versus owned embedded bytes); constructors enforce checked bounds and meaning |
| Full geometry remains resident | `mesh.rs:175–176,214–225` builds scene-wide vectors; 482–491 collects primitive positions/indices before copying into scene | Source-size-dependent memory is unproven, not bounded by tile limits; either explicit admission ceiling for first slice or independently measured spooling design |
| Worker counts come from process state | `tile.rs:283–288` derives decode batch from global Rayon; leaf work at 349; a separate parent pool derives threads at 440–442; `texture.rs:1621` also uses parallel iteration | Mesh execution owns one resolved worker budget and scoped task lifetime; inspect nested parallelism rather than adding another pool |
| Per-job metrics are global | `hlod.rs:34–42` static atomics; `tile.rs:488` reads cumulative summary | Replace with run-owned metrics merged from completed worker results |
| Diagnostic and process ownership leaks | `hlod.rs:150`, `gpu_texture.rs:89` emit stderr; `tile.rs:883–884` best-effort writes failed PNG files; GPU execution blocks on `Command::output` at `gpu_texture.rs:53–58` | Core emits fallible observations; explicit workspace diagnostics only. An external encoder needs owned child/termination/reaping and bounded log collection, or exclude it from first slice |
| Artifact inventory reconstructed from directories | implicit staging scans prefix names at `tile.rs:528–535`; explicit selection is named list at 543–547 | Producer returns its accepted inventory. Directory scans/prefixes must not decide acceptance; index/report construction uses that inventory |
| Report/postcommit behavior differs by output branch | implicit report at `tile.rs:538–540`; explicit publishes with no report at 548 then performs metadata I/O and note at 549–552 | Seal one typed producer report and its serialized bytes before commit. Postcommit diagnostics cannot turn installed output into a conversion failure |

These are architectural observations. Existing texture, simplification,
partitioning, coordinate and GLB algorithms remain unproven until their own
independent tests pass; none is declared correct because a move preserves bytes.

## Minimal layout and direction

Retain one crate initially. Module privacy and a minimal external API consumer
provide useful compiler enforcement; a new crate or semantic import analyzer is
not justified by this one consumer. Do not rename every root module in F1.

- Public mesh conversion facade: owned request, explicit coordinate/profile
  choices, typed result, `&RunControl`; adapters contain CLI/Python transport.
- Private mesh source owner: source document, checked dependency references,
  supported semantics and resolved coordinate/encoder choices. Existing loader
  and coordinate implementation stay isolated pending their evidence gate.
- Private mesh producer owner: scene/plan execution, workspace, scoped worker
  budget, accepted content records and run metrics. Keep planning and codec
  implementations separate only where their responsibility is already concrete.
- Existing `package` owns opaque-source selection and its archive phase;
  `archive3tz` owns stored ZIP/index serialization; `runtime` owns first cause,
  event admission, staged file, seal and namespace installation.
- Existing GLB and implicit-format writers remain lower-level codecs: no
  `RunControl`, output destination policy, adapter identity or converter dispatch.
  A writer with filesystem side effects must expose them honestly, rather than
  acquiring the right to publish a final destination.

Dependency direction: adapters -> mesh orchestration -> mesh source/producer,
placement/geometry/texture/GLB/implicit codecs; orchestration -> archive3tz/runtime;
package -> archive3tz/runtime (or an existing-attempt helper only if justified). Runtime must not import mesh/options/receipt;
codec must not import a conversion job. Check concrete import edges and private
APIs during review; do not claim source scanning proves semantic dependencies.

## Owned stages and the F0 reuse seam

`MeshRequest -> ValidatedMesh -> PreparedMesh -> CompletedMesh` is a consumer
sequence, not a public typestate framework. Constructors after request stay
private. Validation is pure. Prepared input owns metadata, checked source
references and resolved operation decisions; it owns no destination staging and
need not materialize a scene. Execution owns scratch and workers.

CompletedMesh owns immutable manifest/report data, accepted generated member
records and the workspace lifetime keeping their files alive. Worker results
are accepted in stable order; no task or mutable file alias survives completion.
Rejected candidates and scratch files never become generated member records.
Keep tile identities/resource names separate from arbitrary OS paths.

`package.rs:425` calls `control.begin()` and runtime rejects reuse at
`runtime.rs:232–240`. Therefore calling public package() after starting a mesh
run fails; starting a second control breaks one first-cause/commit decision.
Prefer mesh composing the codec and runtime directly with its already claimed
Attempt: its source closure/report policies differ from opaque packaging. Only
if concrete duplicated archive assembly justifies it, extract a crate-private
helper taking prepared named members and an existing Attempt; no second control
or converter flag. Public package() remains the standalone opaque-source adapter.
Reuse runtime publication rather than copy a publisher or introduce a callback-
configured pipeline. Archive serialization keeps its generic checkpoint callback
and must not import runtime, mesh options or report policy.

The mesh owner joins work and records producer errors before closing event
admission. Archive encoding can still fail and must reach the same abort gate.
All producer/report/observer work finishes before seal; publish consumes the
sealed file once. Workspaces require explicit cleanup: failure cleanup issues
are secondary/retained paths; successful publication followed by workspace
cleanup trouble returns cleanup diagnostics beside the committed result. F0
owns only its staged archive, not a future consumer's scratch directory.

## Resource closure and path contract to decide before implementation

For the first declared source profile, enumerate every supported reference:
external buffers, image URIs, embedded data and checked bufferView ranges;
metadata schema URIs only if that extension is deliberately supported. Unknown
required extensions are Unsupported, not a reason to omit dependencies. Reject
unsupported semantics identically for small and large sources.

A URI parser/resolver in the mesh source boundary distinguishes embedded data
from supported relative local references, normalizes URI components once and
checks decoded-path containment. Declare percent-encoding/query/fragment/dot
segment policy explicitly; the existing wrapper bans percent encoding and
normalizes dot segments (`tileset/resources.rs:85–105`), while mesh uses raw
joining. Neither is automatically the desired new contract. If a deliberately
narrow first profile refuses those URI forms, test the refusal.

Apply an explicit regular-file/symlink/alias policy, including ancestor/root
rules, before opening sources. Canonical containment is not a filesystem
snapshot; document source stability and recheck sizes/checked ranges as consumed.
Generated archive names use the F0 rules, including reserved index, length,
forbidden suffix substrings and member-size limits. Rewrites update references
and member records together; never copy unrelated neighboring files. Exact
closure includes manifest/content/subtree/metadata/report dependencies required
by the chosen output profile, excludes discarded candidates and scratch, and
is checked independently of the producer's own validator.

## F1 independent acceptance gates

1. Hand-authored static mesh fixture decoded by an independent glTF/GLB reader:
   expected transformed positions, topology, normals and optional supported
   texture channels; independent 3D Tiles schema/reference checks. Compare
   semantic values, not only historical digests or the same writer's validator.
2. Required-resource fixtures: missing resource, traversal/absolute/scheme URI,
   declared encoding policy, symlink root/leaf, aliases, malformed bufferView,
   unsupported extension/animation/attribute. Refuse before scratch/output parent
   creation where the defect is resolvable; preserve prior destination bytes.
3. Archive member inventory derived by an independent URI walker: every emitted
   reference resolves exactly as declared; no rejected tile, stale index, spill,
   GPU log or unrelated source neighbor. Validate ZIP local CRC/size/index offsets
   using F0's independent reader, not an opaque-package success as closure proof.
4. Inject source/decode/encode/spill/report/archive/finalize failures and cancel
   during each stage; observer failure while workers run must join/stop them,
   prevent publication and retain first cause. Final-event cancellation/conflict
   controls prove the same gate reaches actual installation. No callback after
   seal; nested independent runs and concurrent runs do not share metrics.
5. Worker limits 1/2/4, repeated runs and independently interleaved jobs: stable
   identities/accepted inventory/report ordering and semantic output; byte
   reproducibility only for encoders where explicitly promised.
6. Increasing vertices/images/image dimensions/tile count at fixed workers:
   measure peak RSS, mapped versus heap geometry, decoder/atlas/work result
   high-water marks, descriptors and scratch bytes. Include failure/cancellation
   cleanup high-water marks. A decoded batch <=4 is not a byte bound. If first
   slice is materialized, enforce and test its admitted limits and describe
   O(source geometry + inventory + worker working set), not constant memory.

## Remaining roadmap ownership ledger (steps 2–4)

| Subsystem | Responsibility/evidence inspected | Disposition and next gate |
| --- | --- | --- |
| Point cloud | `point_cloud.rs:56–59` legacy Job; 139–185 chunked scratch ingestion; `point_cloud/tiles.rs:115–129` entire leaf read + encoding; 248–265 representative map | Unproven source/layout, partition, sampling and encoding semantics. Replace run/publication/report ownership; independently measure chunk+leaf+attribute+hierarchy memory before accepting bounds. Reader/schema resolution stays source-local |
| Vector | `vector.rs:257–269` legacy publisher; `pipeline/store.rs:626–650` SQL rollback excludes mutable counters/JSONL; `encoding.rs:499–519` persists before budget acceptance; `source_native.rs:372–380` treats non-Environment failures as invalid features; `geometry.rs:13–18` string-prefix policy | Rework transaction as one accepted rows/counters/diagnostic/content delta; typed rejection distinct from infrastructure failure. Candidate disk persistence alone is not proof it leaks into final closure; test actual published inventory and accepted counters. Native/portable ownership/failure semantics and reuse state remain unproven |
| Raster | `raster.rs:118–121` legacy directory publication; `raster/native.rs:32–81` source/capability checks inside producer; 84 process-derived workers | Unproven styling/coverage/output semantics. Resolve recipe/bands/driver support before work; native datasets belong to caller thread, consumer owns derivative finish/errors/closure and explicitly supported directory publisher. F0 file guarantee cannot certify this path |
| Terrain | `terrain.rs:77–80` legacy directory publication; 309–322 ordered batches <=4; `terrain/raster.rs:68–75` warped source borrow | Ownership-shaped code is a candidate, not certification. Independently prove native finish/source lifetimes, sampled-grid/encoded-grid memory, accepted batch/report updates, and directory rollback; quantization/topology/simplification await numerical agent gates |
| Native backend / CRS | `geospatial/native.rs:144,217` owned dataset and explicit finish; process cache initialization at 22–35; mesh CRS and reader backend choices are above codecs | Native lifetimes/caches/thread confinement/capability/error translation unproven beyond existing evidence. Keep handles inside backend owners, resolve capabilities once per operation, preserve borrowed-source lifetime and check deferred close failure. Process cache is shared and is not a per-run memory bound |
| GLB/glTF / textures / metadata | `glb_write.rs:20` returns complete Vec; `glb.rs` MetadataGlb owns buffers; mesh images/atlases and `gpu_texture.rs:53` subprocess are separate resources | No algorithm retained yet. Independently verify framing/accessors/extensions/material semantics and allocation growth. Shared codecs may accept domain values; must not own workers, final destination, feature-skip policy or stderr |
| Tileset / implicit / grid | `tileset.rs:156–194` resource resolver plus publisher; `implicit.rs:24–26` 16MiB availability bound; `convert_implicit.rs:145,199–200` producer validates then legacy publishes | Separate source/resource closure and conversion orchestration from format models/codecs. Availability cap is narrow, not aggregate hierarchy bound. Independent subdivision/URI/transform tests precede reuse; convert-implicit needs owned accepted output and same publication contract |
| Validators | `validate.rs:604–607` uses production archive validator; 392–416 expands implicit data; independent reference handling at 43,298,341 | Keep inspection/validation read-only and independent from producer acceptance policy. Current validator is unproven as an oracle; mutation/fault fixtures must distinguish writer/validator common-mode defects, cycles/expansion limits, CRC and referenced-data bounds. Directory validation is expressly absent at 581 |
| Runtime/package/archive | F0 tests/evidence establish bounded opaque payload copy and file publication only; legacy `archive3tz::write_archive` remains separate | Candidate for F1 reuse only within stated F0 guarantees. Verify one-run composition and consumer workspace cleanup; no broadened crash durability or whole-scene claim |
| Adapters/reports/support modules | `lib.rs:7–39` exposes IR/atlas/HLOD utilities; `report.rs:32,98` infallible sink/process stderr; doctor/preview/fixtures are distinct support tools | Replace migrated consumer's event/report transport with runtime Observer and typed receipt; keep support-tool authority separate. Public model/utilities need explicit disposition, not blanket hiding or blanket retention. Main/Python parity is an acceptance gate, not source of core defaults |

Sequencing: finish F0 acceptance first; specify F1 supported profile/oracles and
single-run seam; implement one real mesh vertical slice with measured limits;
then separately admit textures/HLOD/projected interpretation only after their
fidelity/resource proofs. In parallel, scope vector transaction repair and native
source ownership; point/raster/terrain migrations follow explicit contracts and
independent gates. Directory publication is a separate platform task before
raster/terrain can claim F0-style output safety. Broad layout cleanup follows
proven ownership, rather than moving untrusted subsystems wholesale.
