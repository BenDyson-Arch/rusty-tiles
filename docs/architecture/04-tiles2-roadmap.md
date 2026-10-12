# 0.4.0: Tiles 2 and large textured meshes

Status: **development direction and ordered roadmap, 2026-10-12**. This document
replaces the earlier decision to defer 3D Tiles 2.0 work until 0.5.0 and consolidates
the proposed ML0–ML10 sequence. It authorizes planning and bounded implementation
slices; it records no new implementation acceptance or release authorization.

**The development target now is the pinned 3D Tiles 2.0 and glTF 2.1 drafts.**
Ratification is not a prerequisite for development. Draft support must name the
exact revision and implemented domain, and must be described as provisional.
Changing version strings alone does not implement either target. Viewer support
and compatibility with a particular draft remain unproven until the first
milestone supplies real output and consumer evidence.

The streaming, certified, textured large-mesh tiler remains required for 0.4.0.
Its fidelity, topology, source identity and resource requirements below survive
the change of format target. The foundation work remains replacement-first:
retain code only for the domain independently established by its evidence.

## Scope and migration policy

- Generated tile content is glTF/GLB. Remove legacy b3dm, i3dm, pnts and cmpt
  handling when the replacement content and consumer paths are accepted.
- Implicit tiling remains required alongside explicit hierarchies. The migration
  must preserve addressing, availability, metadata, placement and containment.
- Keep the current advertised outputs usable until their replacements pass the
  corresponding acceptance gates. This is a migration safeguard, not a permanent
  requirement to maintain two exporters or a new compatibility option bag.
- Resolve the viewer delivery route early: directory/server delivery, direct
  archive loading, or a documented archive bridge. Prove the chosen route rather
  than assuming a viewer can load a `.3tz` file. Choose the delivery/container
  contract in milestone 1 and derive its limits there. The 3TZ member ceiling
  applies only if 3TZ is retained as an interim delivery option; it is not a
  requirement for the new target's packaging.
- Migrate existing producers through the shared output contract. Local mesh input
  stays local static metres with rigid geographic placement; the mesh tiler does
  not gain source-CRS conversion. Other producers keep their independently
  established coordinate domains while their boundaries are audited.
- Optional AEC semantics, splats, voxels, time, horizon and shapes are later work.
  They do not expand this roadmap's supported domain or delay its core migration.

The core work from [#84](https://github.com/BenDyson-Arch/rusty-tiles/issues/84)
(tileset writing), [#85](https://github.com/BenDyson-Arch/rusty-tiles/issues/85)
(implicit tiling), [#86](https://github.com/BenDyson-Arch/rusty-tiles/issues/86)
(coordinates), [#87](https://github.com/BenDyson-Arch/rusty-tiles/issues/87)
(vectors/layers), and [#75](https://github.com/BenDyson-Arch/rusty-tiles/issues/75)
(legacy content removal) belongs to 0.4.0. The core glTF migration part of
[#89](https://github.com/BenDyson-Arch/rusty-tiles/issues/89) belongs here; its
optional AEC work stays separate. [#83](https://github.com/BenDyson-Arch/rusty-tiles/issues/83)
tracks this consolidated direction. Issue membership is not proof of completion.

## Draft reference baseline

The following source revisions were inspected on 2026-10-12. They are inputs to
milestone 1, not a claim that the independently evolving drafts form an accepted
implementation profile. That milestone resolves differences, records file hashes
and pins the exact coherent subset used by its artifacts.

| Source | Inspected revision |
| --- | --- |
| Tiles 2.0 conformance classes | [`CesiumGS/3d-tiles` `3806d0c2`](https://github.com/CesiumGS/3d-tiles/blob/3806d0c2d68d23e0e8d3319887833975f904d82f/specification/2.0/CONFORMANCE.adoc) |
| Migration summary | [`CesiumGS/3d-tiles` `4d781014`](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/next/2.0/CHANGES.md) |
| glTF 2.1 draft | [`KhronosGroup/glTF` `289a9db8`](https://github.com/KhronosGroup/glTF/blob/289a9db8000a4b57fdf06c140423f9646506646b/specification/2.1/Specification.adoc) |
| Tileset, implicit and subtree extensions | [`CesiumGS/glTF` `4f123715`](https://github.com/CesiumGS/glTF/tree/4f123715831a8c362d2562901911fc4ae9508503/extensions/2.1/Vendor) |

The migration changes both representation and meaning: glTF nodes and external
assets replace Tileset JSON; glTF assets replace binary subtree framing; local
frames are Y-up; geometric error is no longer scaled by node transforms; and
actual bounds/errors move to accessor-backed subtree attributes. In particular,
the box override becomes a DOUBLE MAT4 rather than twelve metadata values.
Preserve application properties through structural metadata at the new attachment
points. Revalidate each mapping independently. Native glTF packaging is a
separate delivery contract; do not assume the existing 3TZ index or ceiling
applies to it.

## Four ordered milestones

### 1. Pin the drafts and prove a real viewer path

Pin the normative draft files, schemas and required extensions by repository
revision and content hash, and record the supported subset and unresolved draft
questions. Pin the consumer build and the delivered artifacts. Produce a small
but real end-to-end demonstration of explicit and implicit tiles, plain-glTF
vector fills, metadata picking and geographic placement. Choose and document
the delivery route from actual loads.

Acceptance requires independent decoding/inspection of generated artifacts and
viewer observations that exercise hierarchy traversal, refinement, vector fill
appearance, stable picked source identities and known placement controls. A
version-field change, schema pass, hand-authored mock output or successful load
of only one root payload cannot close this milestone. If a consumer cannot use a
required draft feature, record the failing feature and resolve the consumer or
delivery choice before claiming the target is supported.

**Current consumer blocker (source inspection):** CesiumJS 1.146.0 at
`b8d3a36fe98a3e432eb89253c95d5f20e605e0f1`, and inspected `main` at
`df52c781de3491a4b76839d420f7ca90a032efb6`, reject glTF asset version 2.1 in
[`GltfJsonLoader`](https://github.com/CesiumGS/cesium/blob/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1/packages/engine/Source/Scene/GltfJsonLoader.js#L265)
and tileset versions beyond 1.1 in
[`Cesium3DTileset`](https://github.com/CesiumGS/cesium/blob/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1/packages/engine/Source/Scene/Cesium3DTileset.js#L2386).
The same restrictions were inspected at the pinned `main` revision
([glTF loader](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/engine/Source/Scene/GltfJsonLoader.js#L267),
[tileset loader](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/engine/Source/Scene/Cesium3DTileset.js#L2386)). An upgrade
is not a demonstrated fix. Milestone 1 consumer integration remains blocked
until an executed compatible path is proved. Any feasible consumer adapter or
prototype needs a bounded scope and evidence; this roadmap does not authorize
building a renderer or adding fallback export formats.

Draft-dependent writer acceptance and legacy retirement wait for that proof.
Neutral source capture, geometry and resource work may proceed after its typed
contracts and focused evidence are settled; it must not claim draft or consumer
acceptance as a result.

### 2. Establish the shared typed tile and output contract

Use the proven vertical slice to define the smallest shared types for generated
nodes, finite bounds, transforms, errors, content inventory, metadata and
explicit/implicit hierarchy ownership. Separate format meaning from output
paths, staging and publication. Do not round-trip generated hierarchy state
through loosely interpreted JSON. Imported documents keep a distinct reader and
validation boundary, including a deliberate unknown-extension policy.

Migrate metadata and coordinate handling where the draft and actual producers
require it. Define each frame and unit once, compose placement at its owner,
and check decoded containment after final encoding. Use shared feature identity
semantics for mesh and vectors without forcing their geometry into one model.
Migrate the demonstrated producer and validator first, then the next concrete
consumer; avoid speculative framework or module-layout work.

Revisit public operation names, including existing `-to-3tz` names, against the
chosen delivery contract. This roadmap chooses no replacement public name.

Acceptance covers explicit/implicit equivalence of meaning, source picking,
known coordinate references, final decoded bounds, resource closure and typed
failure/publication behavior. Audit the current oracle before reusing its tests.
Keep legacy paths until the relevant migrated producer is independently accepted.

### 3. Deliver the streaming textured large-mesh tiler and migrate producers

Build the large-mesh pipeline on the shared contract in bounded dependency order:
in-process texture codec gate; streamed capture/sort and tree construction;
streamed hierarchy/delivery output; full-detail leaves; certified geometry LOD;
proxy texture re-parameterisation and baking; validator scale support. Migrate
the remaining existing producers to shared tiles, glTF content and metadata.
These are successive slices inside this milestone, not parallel workstreams.

Preserve the following agreed large-mesh requirements:

- One local static glTF/GLB scene, confined snapshot dependencies, local metres
  and rigid anchor/orientation/offset placement. Keep the established PBR slot,
  UV, tangent and colour domain and its explicit unsupported cases.
- Streaming input and output with memory bounded by declared configuration;
  scratch preflight by stage, memory-backed scratch reporting, cleanup on
  failure, and deterministic output per target independent of jobs, run size,
  fan-in and scratch location. Retain explicit hierarchy/individual-item limits
  and derive container-specific limits from the chosen delivery contract; do not
  add a total input-size refusal. A retained 3TZ route keeps its early typed
  refusal at the 178,956,970-member ceiling.
- Full-detail leaves preserve exact per-triangle source provenance. Proxies
  preserve identity for every present primitive; removed geometry remains
  accounted for. The draft's metadata representation must prove the same
  identities and picking semantics before replacing the current representation.
- Seam agreement is checked on decoded published bytes, including mixed-depth
  leaves, packed leaves, corners, f32 profiles and quantized/meshopt payloads.
  Retain exact shallowest-leaf incidence computation, final-position bounds,
  topology validation and explicit repair/refusal behavior.
- Certify after quantization and snapping. Published geometric error bounds
  both directions against source geometry, including removed regions and added
  faces. Keep sound composed and direct bounds, outward rounding and bounded
  certificate work; exhaustion must leave a sound bound.
- Small primitives and islands may vanish at coarse levels; small interior holes
  may be filled, with deeper tiles reopening them. Preserve the certified rules,
  border-hole restrictions and published vanished/filled records. A crack must
  never be relabelled as an intentional hole. These choices are fidelity policy,
  not permission to silently drop source geometry or provenance at leaves.
- Atlas and encode every present PBR slot. Bake proxies from pre-encode data,
  re-express normal maps in the proxy tangent frame, and verify container, mip,
  colour/data class and transcode semantics after encoding. Use in-process
  KTX2/Basis with meshopt; PNG is the agreed `lossless` texture exception. There
  is one encoder path, without an external-tool fallback ladder.
- Keep `fast`, default `balanced`, `quality` and `lossless` profiles, measured
  per-class appearance floors, the per-tile byte budget and reported lossy mip
  drops. `lossless` preserves its agreed texel-exact leaf requirement and typed
  refusal where a page cannot fit. Profile constants remain provisional until
  the final acceptance milestone.

The earlier v6.1 design supplies the detailed algorithms, constants and tests as
planning inputs. Preserve its requirements unless a recorded decision explicitly
changes them; adapt format details to the pinned drafts instead of treating old
extension names or JSON shapes as compatibility obligations. A successful codec
prototype does not accept the tiler, and untextured temporary proxies do not
accept textured output.

### 4. Accept resources, fidelity, platforms and performance; retire old code

Run the final production output auditor and independent validator on every
profile and scale gate, with the real glTF/meshopt/KTX2 bytes and the chosen
consumer route. Include coordinate and picking controls, seams, topology,
holes/islands, appearance and geometric-error checks. Freeze constants only
after those checks pass.

Retain the v6.1 pass/fail scale obligations: the spilled 5.26×10^8-triangle
production octree case, the 10^4-primitives and 10^4-holes/islands-per-root
feature-density case, the procedural-texture bake case, and the large hierarchy
and delivery cases (at least 2^22 tiles and over 4 GiB of output). Carry forward
the archive-format-specific probes if that container is retained, and supply
equivalent independent delivery checks for its replacement. Measure peak resident
memory, scratch, wall time and consumer load/refinement against their declared
budgets. Smaller tests establish bounded correctness evidence; they do not
substitute for these scale gates.

Validator memory and work must scale with the supported delivered-resource domain. Keep
absolute volume caps opt-in, explicit per-item limits and non-circular
amplification budgets; test malformed and multiply traversed resources with
sensitive controls. Complete applicable platform CI, installed-wheel and
official-Blender checks on the accepted final sources and artifacts.

Then remove superseded exporters, legacy mesh commands/options, codecs and
shared helpers only after their final consumers migrate. Update public APIs,
help, defaults, reuse-identity guidance and migration documentation together.
Review the retained public surface explicitly. Closing this milestone requires
the final acceptance evidence; document consolidation alone does not close
[#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113) or authorize a release.

## First implementation slice: draft consumer proof

**Inputs:** the pinned drafts and schemas; an exact converter source revision;
a candidate compatible viewer or a specifically scoped consumer adapter; one
small static local mesh with two independently known source
identities; a vector fill with a hole and known attributes; a known WGS84 anchor
with asymmetric orientation/offset controls; and a tiny hierarchy crossing an
implicit subtree boundary. Keep fixtures small enough to inspect independently.

**Output:** a runnable conversion/consumer path generating both explicit and
implicit draft-target datasets with glTF content, source metadata and geographic
placement; a reproducible local load through the selected delivery route; and a
short evidence record naming source, draft, viewer and artifact hashes. The
existing producer may supply captured geometry, but target serialization and
metadata must implement the pinned draft semantics. Production acceptance is
limited to this declared input domain.

The inspected Cesium builds cannot supply this proof unchanged. Settle the
bounded consumer route first; if no feasible route is established, record the
blocker and continue only the independent neutral work described above. Do not
publish a draft-support claim from a rejected artifact.

**Gates:** independently inspect all emitted resources and addressing; decode
coordinates and verify known transformed points and bounds; show the fill and
hole; pick both mesh identities and vector attributes; force traversal across
the subtree boundary and refinement; and include wrong-identity, wrong-frame
and missing-resource controls that demonstrably fail. Exercise failed conversion
without publishing an invalid dataset. Record consumer failures and proof limits.
One root load or matching historical bytes is insufficient. Carry forward only
the representation and ownership decisions these results justify.

This first slice chooses the consumer/delivery contract. It does not implement
the whole large-mesh tiler or retire advertised output modes. The next slice is
the shared typed contract required by this demonstrated path.

## Review and evidence discipline

Each bounded slice has one owner, focused checks and one independent nonauthor
review of its final source and evidence. State its supported domain, invariants,
failure meaning, dispositions and removal gates before coding. Use stronger
independent references, negative controls and exact artifact identities where
coordinate math, certificates, binary formats or hostile resource traversal
need them. Reuse existing independent auditors rather than inventing a new
approval ceremony for every ordinary edit. One coordinator owns shared builds
and sustained scale runs.

Prior P0 and p0b results are prototype evidence. They do not establish production
streaming, interval certificates, texture baking at scale, packed implicit
output, or the full vanished/filled policy. Record those limits alongside their
positive results. Earlier accepted foundation slices retain only their stated
domains; they do not accept the new draft target by inheritance.

Planning inputs read for this consolidation (not newly accepted contracts):
`f1d3-large-mesh-contract.md` from `/tmp/rusty-tiles-ml0/` (SHA-256
`2cb922a2dbaa5018a2974dc729a3f95afdb98d52db8c3812ff91aae3df4fbbe5`),
`f1d3-surface-dispositions.md` from that worktree (SHA-256
`bc9fc08f6d7e35a7b49261a20e394c54b20f340b2ffed9ef02da2f2cd30b9f95`),
and v6.1 `slices.md` from `/tmp/rusty-tiles-mlarge-design-20261012/` (SHA-256
`705bb8f775e18eb626c62cb27a20cd485bf28f71b24cab38332c0ae5f119f225`).
Preserve these inputs with the durable roadmap evidence before deleting the
temporary worktrees; their location is provenance, not a build dependency.
