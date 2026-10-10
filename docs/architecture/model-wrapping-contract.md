# W1: static model wrapping and standalone manifest disposition

Implementation [#145](https://github.com/BenDyson-Arch/rusty-tiles/issues/145),
under #126/#120/#125/#113, coordinated with F1c2 #144 and package facade
retirement #146. Baseline is merged F1c1 #143 at
`29b96e501b1426e08cf7d3f01ba93964c6618a57`. This contract/design was written before
production changes. It is not an acceptance record. The coordinator selected
the single-model sibling-manifest product before implementation; focused
independent Decimal probes selected the original-node interval representation.
Final operation/numerical/platform acceptance remains separately required.

## Products and ownership

W1 wraps one explicitly local metre/Y-up, static GLB or glTF source document and
its declared confined resources as an explicit, full-detail 3TZ. It preserves
every admitted source document/resource byte. It does not generate F1c2 feature
extensions inside an unchanged document or perform partition, atlas, LOD,
compression, source CRS conversion, or implicit delivery.

The source owner admits document semantics and selected scene once; the binding
owner captures bytes, resolves confined URI-to-member associations and excludes
source/destination aliases once. The placement owner resolves Local/Wgs84 once
before source I/O, following F1c1. A pure source evaluation establishes bounds
for the unchanged node hierarchy. The wrapper owns member inventory, required
report and its private materialization workspace. Pure manifest serialization
consumes prepared bounds, a prepared content URI and a resolved root transform.
One F0 Attempt owns events, cancellation, sealing and completed-file publication.
There is no call to public package or a second RunControl within this run.

Reuse the actual accepted source, binding and placement owners through narrow
private interfaces. Do not copy their allowlists, URI parser, frame mathematics
or filesystem checks into a second producer. A public source IR, mode bag,
universal converter abstraction and compatibility routing are not justified.

The intentional facade is `ModelWrapRequest::local_gltf(input, output)`,
`with_placement(MeshPlacement)`, `with_policy(OutputPolicy)`,
`model_to_archive(request, &RunControl) -> Result<ModelWrapResult, JobFailure>`.
The operation owns no leaf limit because it produces one full-detail content
tile. CLI and installed Python must select this same operation, domain
defaults, report and typed failures.

## Finite source domain and capability disposition

Retain the F1b3/F1c1 static source admission domain, with any source metadata
admission changed by #144 separately named and independently proved. This means
the admitted selected scene, forest/depth/instance policy, complete TRIANGLES,
validated POSITION/index payloads, finite admitted node transforms, authored
companions, five core PBR slots and PNG/JPEG resource limits. The wrapper has no
small-file or triangle-count eligibility bypass. Complete document admission
and declared buffer/image capture still apply to unused declarations.

Exact wrapping retains source omission/presence, image encoding, material,
sampler, attribute storage, indices and node hierarchy rather than reproducing
the retiling encoder's output. Existing admitted node names remain unchanged labels; source extras are
excluded by the shared admission profile. Names do not establish feature picking. It retains
all declared resources because the complete unchanged source document remains
present, including declarations outside the selected scene. This differs from
retiled selected-resource closure and must be reported honestly.

| Existing advertised use | W1 disposition and remaining owner |
| --- | --- |
| One static admitted GLB/glTF with confined core resources | Replace old wrapping facade with typed exact-byte W1 and independent closure/bounds/publication proof. |
| Core PBR and admitted POSITION/NORMAL/TANGENT/UV0/UV1/COLOR_0 | Retain unchanged source encoding within the existing source profile; no new broad shader or codec proof inferred. |
| Animation, skins and morph targets | Excluded from W1. Existing wrapping was advertised as an escape hatch; time-dependent bounds and source fidelity remain unproved. Advertised legacy wrapping support must be explicitly withdrawn in migration docs if its facade is deleted. |
| Required/optional unknown extensions, extension-owned URIs, existing EXT_mesh_features/EXT_structural_metadata | Excluded unless explicitly admitted by a separately proved source contract. Exact JSON bytes alone do not prove dependency closure or metadata semantics. #144 authors metadata in retiled content; it does not admit arbitrary imported extensions for W1. |
| Other vertex semantics, additional UV/color sets, BLEND, texture transforms, data-URI buffers/images, broader source formats/large models | Excluded by the declared static source profile. Removal from new wrapping is a documented capability reduction, not merely renamed API syntax. Existing data-URI example.gltf does not demonstrate W1 acceptance. |
| Legacy HPR/Euler placement | Replace W1 syntax with the F1c1 anchor/quaternion/scene-offset grammar; document conversion guidance and no automatic source CRS inference. Legacy mesh HPR remains owned by the old mesh route. |
| Geographic/projected source coordinates, source axes, Metashape E/N/A shifts, height/datum/grid/epoch decisions | No W1 admission. Manual WGS84 placement is an output rigid placement of declared local metres, not a source CRS conversion. |
| Directory/multiple-model standalone manifests | Exclude from proposed single-model manifest replacement below; deliberately remove old directory enumeration if that replacement is selected. |
| Old mesh small-input explicit/implicit wrapping shortcuts | Internal legacy consumers remain until broader mesh disposition; public W1 migration does not accept or delete those Job-based paths. |

Breaking changes are permitted, but none of these uses may disappear behind a
new API while documentation continues to advertise them. The old broader mesh,
raster pyramid and explicit-to-implicit operations retain their distinct
unmigrated ownership. This slice cannot delete global Job/Reporter/results.

## Captured member associations and limits

The binding result must expose immutable root bytes and each requested external
resource's admitted decoded relative member path alongside its shared byte
owner. It must not expose live source paths to the codec. The same URI parser
that bound source bytes owns these paths. Preserve distinct resolved names even
when hard links or aliases share a captured Arc; deduplicate repeated references
to the same resolved name, never distinct names just because bytes or file
identity agree. A collision assigning different byte owners to one archive name
is InvalidInput. Source-document aliases remain Unsupported under the captured
source profile.

The proposed archive inventory is `tileset.json`, `conversion.json`,
`model/source.glb` or `model/source.gltf`, each declared dependency at
`model/<decoded confined relative path>`, and the archive-generated index.
The source root kind comes from admitted envelope semantics, not an inferred
extension. A dependency equal to the chosen synthetic source name or nested
beneath that name (for example `source.gltf/data.bin`) is Unsupported during
inventory preparation, before report evaluation, workspace/staging or creation
of the output parent. The synthetic root is a file, so it cannot also be the
ancestor directory of a dependency. Other shared name prefixes such as
`source.gltf-extra/data.bin` remain admissible. The `model/` namespace prevents source
dependency spellings from colliding with the generated manifest/report/index.
Original resource URI text remains unchanged. Percent escapes, dot segments and
distinct URI aliases must resolve to exactly the stored associations; encoded
separators and escaping paths remain outside the binding profile.

Retain source limits: 32 MiB per root/resource, 1 MiB root JSON, 64 MiB unique
captured input bytes, at most 64 dependency requests and 65 unique source files,
URI length 4096 bytes and 128 components, 32 logical buffers, 4096 nodes,
depth 128 and 100000 expanded selected triangles. Existing image ceilings remain
32 images, 4096 pixels per dimension, 4 MiPixels per image, 16 MiPixels aggregate
and 32 MiB aggregate encoded images. The wrapper additionally admits at most
64 MiB of distinct emitted model/dependency payload bytes, counting each distinct
alias name, and at most 67 declared members before the generated index. This
prevents hard-linked aliases from turning a bounded capture into unbounded
materialization. Counts are not a whole-job memory guarantee.

The required ModelReport distinguishes product (`archive` or `manifest`), root bytes, unique external files/bytes,
captured bytes, emitted payload files/bytes, expanded selected triangles,
coordinates, actual resolved placement/root matrix, conservative box,
`tileset_geometric_error_metres` and `root_geometric_error_metres`. Schema/profile are new operation values rather
than branches in MeshReport: schema 1/profile `w1-static-model-v1`.
`conversion.json`, Rust/CLI/Python results contain the same prepared report.
Generate it before sealing; no required source read, serialization, callback,
signal check, validation or report work may fail after commit.

## Original-node bounds and placement

The model's glTF runtime basis is B(x,y,z)=(x,-z,y), applied once by the consumer.
The root box is tile-local B-space; placement follows F1c1's resolved root matrix.
There is one content-bearing root, no children/additional transforms, explicit
root `refine: REPLACE`, root geometricError zero, and explicit root transform
including identity for Local. The tileset geometricError is separately the
error of omitting the entire model, whereas root geometricError measures the
detail absent from the rendered root; see [OGC 3D Tiles 1.1, Tileset geometric
error](https://docs.ogc.org/cs/22-025r4/22-025r4.html). Giving both zero can prevent
ordinary clients from considering the root for rendering.

Set the tileset omission error to the maximum of one metre and the diagonal of
the emitted conservative box, rounding each square, sum, square root and final
diameter multiplication outward in f64. The diagonal conservatively covers the
admitted geometry's spatial extent; the one-metre floor keeps this visibility
policy positive even for degenerate or sub-metre geometry. This policy does
not promise visibility at arbitrary camera distances or prescribe a client SSE
threshold. Full-detail root error remains exactly zero. These prepared values
are shared by the manifest and report. Remove historical 512/4096 constants at
this boundary.

Bounds must enclose decoded referenced source positions transformed through
the unchanged authored node chain, not the retiled f32 positions. Exact source
accessor min/max are validated but do not substitute for decoded geometry.
Reflection does not rewrite source triangle order. POSITION companions and all
resource bytes remain untouched by placement.

Original source JSON TRS and matrices remain unchanged. In particular the
retiled source owner normalizes admitted near-unit node quaternions, while an
unchanged consumer may evaluate the authored values directly. Reusing the
retiled accumulated matrix without disposition is therefore insufficient.
The original-node evaluator uses private outward f64 intervals and conservatively
encloses authored and normalized unit-quaternion evaluation, including every
intermediate node composition and point operation. Each multiplication/addition,
norm square root and reciprocal is rounded outwards; raw and normalized rotation
matrices are hulled before node composition. A final next_up on a rounded point
alone would not bound prior composition error. Independent Decimal reference
and negative controls remain required for final evidence. Reject nonfinite
evaluation or interval endpoints outside [-1e6,1e6] metres as Unsupported before
workspace/staging. This may refuse a boundary case that a rounded/normalized
retile bake admits; it is a declared conservative unchanged-content profile,
not a silent clamp or wider placement-accuracy claim.

The accepted F1c1 forward placement allowance of 1e-6 metre applies separately
to the admitted original local coordinates. Source transform uncertainty,
conservative local enclosure and consumer rendering precision must not be
hidden in that allowance. An independent high-precision reader must evaluate
original authored source transforms, decode source buffer geometry, apply B
and the emitted root transform, and verify local/world box containment. Report
the conservative bounds semantics and finite numerical limits explicitly.

## Standalone manifest product decision

Standalone publication is distinct from wrapping. The selected useful
replacement admits one model file and writes a fixed sibling `tileset.json`
through F0 completed-file publication; the source content URI is the correctly
escaped original UTF-8 filename. The declared document directory remains the
resource authority. All source/resource bytes stay in that directory, and the
manifest neither copies resources nor claims to publish a self-contained bundle.
The captured admitted source is used for validation/bounds; callers must retain
the unchanged admitted source tree throughout the entire call, including all
observer callbacks, and throughout subsequent use of the manifest. This
precondition includes path/alias retargeting as well as byte edits. Capture
consistency checks do not police future external writes, and there is no
atomic snapshot/publication promise for the live referenced directory. A subsequent `package` can
archive the completed directory, or callers can use W1 for an immutable
self-contained result.

`ModelManifestRequest::local_gltf(input)` derives the fixed output
from the source parent, accepts placement/publication policy, and returns an
operation-specific result/report with cleanup diagnostics. There is no
arbitrary output-location option, recursive enumeration, multiple-model mode or
sidecar conversion.json mutation. Report serialization completes before
publication and the result carries it. A source itself named `tileset.json`, an
unsupported portable filename, or output aliases to source/dependencies is
refused at the owning boundary. A common pure one-content manifest writer is
justified by these two actual consumers; neither facade invokes the other.

The coordinator selected this concrete publishing replacement before production
implementation. Old directory/multiple-model/arbitrary-location uses are
deliberately retired, with refusals and updated help/docs. The old public
create_tileset_json writer is removed; private legacy mesh shortcut helpers
retain their residual behavior. All input/output binding checks, absolute output
identity, one Attempt and replacement/no-clobber rules apply.

## Failures, acceptance and deletion

Invalid request/path/placement combinations are InvalidRequest; malformed
document/resources, collisions and source changes are InvalidInput; valid
semantics outside the finite domain are Unsupported; filesystem failures are
Io; output conflict, cancellation, observer failure and reuse of a RunControl
retain their F0 meanings. No message-string classification or legacy Reporter
bridge owns these outcomes. Read-only validation/capture and complete member
admission precede private workspace/staging; all captured bytes are immutable
after preparation. Cleanup failures retain their actual paths and causal
diagnostics. Final artifact publication has no fallible required follow-up.

Independent acceptance must cover:

- Exact root/dependency bytes and complete URI closure, including unused declared
  resources, encoded names/dot segments, repeated URI paths, hard-link aliases,
  exact and ancestor synthetic-root name collisions, admissible shared prefixes,
  source mutation after capture, and root/output
  aliases. Controls remove an alias member or substitute another captured file.
- Original-node geometry and bounds under Local/Wgs84, nested noncommuting
  matrices/TRS, quaternion normalization differences, reflected/degenerate
  geometry, mixed magnitudes and payloads inconsistent with authored min/max.
  Controls replace bounds with baked f32 bounds, swap transform order, shrink
  stored axes, select the wrong scene and mutate root/report matrices.
- Actual browser loading of unchanged model/resource bytes and emitted placement;
  ordinary SSE traversal must load the root without overriding the emitted
  geometric errors or forcing the client SSE threshold to zero. Independent
  Decimal checks bound the tileset error by the emitted box diagonal and verify
  the positive floor for degenerate/sub-metre geometry; controls replace the
  tileset error with zero or an underestimate and remove root refinement.
  Source/format truth remains separate from rendering and #144 picking truth.
- Manifest resource references relative to the actual final output, explicit
  refusal of excluded products/paths, and source preservation after CreateNew,
  Replace, cancellation and injected publication failures.
- Rust/CLI/installed Python parity, required report agreement, callback errors,
  cancellation, reused controls, concurrent publishers, source/output aliases,
  absolute output binding under CWD changes, required precommit failures and
  observable cleanup diagnostics.
- Boundary/over-boundary byte/member/alias/image/node/triangle limits and measured
  resident memory/scratch/descriptors; repeated aliases must not multiply capture
  memory or evade emitted-byte admission.

Freeze exact source/fixture/oracle/binary/wheel/browser identities, execute
sensitive controls, obtain separate nonauthor final source/evidence review,
and run applicable portable/native/platform/browser/wheel/official Blender CI.
Only then delete superseded public glb_to_3tz/glb_to_3tz_reported/
CreateTilesetOptions and replaced create_tileset_json surfaces, migrate all
known Rust/adapter callers and update the surface inventory. Internal old mesh
helpers may remain private with documented residual dependencies. #113/#120/
#121/#125/#126 remain open and no release is authorized by this bounded work.
