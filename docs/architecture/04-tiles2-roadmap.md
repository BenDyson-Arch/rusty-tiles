# 0.4.0 foundation and streaming; 0.5.0 format transition

Status: **development direction and ordered roadmap, 2026-10-12**. This document
consolidates the proposed ML0–ML10 sequence and records the accepted release
split. It authorizes planning and bounded implementation slices; it records no
new implementation acceptance or release authorization.

**0.4.0 ships real 3D Tiles 1.1/glTF 2.0 and implicit tiling, with the shared
foundation and the streaming, certified, textured large-mesh tiler. 0.5.0 makes
the official 3D Tiles 2.0/glTF 2.1 transition against finalized specifications
and supported viewers.** Version strings must describe the encoding actually
implemented. Draft experiments are optional design checks, never 0.4.0 blockers.

The tiler's fidelity, topology, source identity and resource requirements below
remain required for 0.4.0. The foundation work remains replacement-first:
retain code only for the domain independently established by its evidence.

## Scope and migration policy

- Generated tile content is glTF/GLB 2.0 inside real Tiles 1.1 datasets. Remove
  legacy b3dm, i3dm, pnts and cmpt handling when the replacement content and
  consumer paths are accepted.
- Implicit tiling remains required. Preserve Tiles 1.1 addressing, subtree
  framing, availability, metadata, placement and containment. Use tiny equivalent
  explicit/implicit fixtures to check the shared contract; this does not require
  two complete production emitters where the supported producer needs only one.
- Keep advertised outputs usable until their replacements pass corresponding
  acceptance gates. This is a migration safeguard, not a permanent dual exporter
  or a new compatibility option bag.
- Retain the Tiles 1.1 wire semantics and the 3TZ delivery boundary for 0.4.0,
  including the index and early typed refusal at the 178,956,970-member ceiling.
  Prove directory/server delivery, direct archive loading or a documented archive
  bridge from actual consumer loads; do not assume a viewer loads `.3tz` directly.
  Reassess packaging and derive new encoding limits at the 0.5.0 transition.
- Migrate producers through the shared output contract. Local mesh input stays
  local static metres with rigid geographic placement; the mesh tiler does not
  gain source-CRS conversion. Other producers keep their independently established
  coordinate domains while their boundaries are audited.
- Optional AEC semantics, splats, voxels, time, horizon and shapes are later work.
  They do not expand this roadmap's supported domain or delay 0.4.0 shipping.

The new-format migrations in [#84](https://github.com/BenDyson-Arch/rusty-tiles/issues/84)
(tileset writing), [#85](https://github.com/BenDyson-Arch/rusty-tiles/issues/85)
(implicit tiling), [#86](https://github.com/BenDyson-Arch/rusty-tiles/issues/86)
(coordinates), and [#87](https://github.com/BenDyson-Arch/rusty-tiles/issues/87)
(vectors/layers) belong to 0.5.0. Foundation work and current-content conformance
needed to ship 0.4.0 remain in 0.4.0. [#75](https://github.com/BenDyson-Arch/rusty-tiles/issues/75)
(glTF-only vector fills and legacy content removal) remains in 0.4.0 with a
successful viewer gate. The new-format/AEC work in
[#89](https://github.com/BenDyson-Arch/rusty-tiles/issues/89) is later work;
necessary glTF 2.0 conformance does not wait for it.
[#83](https://github.com/BenDyson-Arch/rusty-tiles/issues/83) tracks the consolidated
direction. Issue membership is not proof of completion.

## Four ordered 0.4.0 milestones

### 1. Establish the shared typed output contract with a tiny consumer slice

Define the smallest owned tile records needed by an actual current producer:
hierarchy, finite bounds, transforms, geometric errors, content references and
source identity. Establish a shared generated explicit-node/document serializer
at the existing `src/tileset_node.rs` boundary, consumed by the real writers in
`src/mesh_archive/encode.rs` and `src/mesh_archive/model.rs`. Retain only writer
behavior independently established for this supported domain. Keep
format meaning separate from paths, staging and publication; do not round-trip
generated hierarchy state through loosely interpreted JSON. Imported documents
keep a distinct reader and validation boundary, including a deliberate
unknown-extension policy.

Generate a tiny real Tiles 1.1/glTF 2.0 dataset and prove its delivery through a
pinned supported viewer. Decode emitted coordinates, hierarchy and bounds
independently; exercise traversal, refinement, placement and stable picking.
Use small equivalent explicit/implicit fixtures for addressing, availability,
subtree boundaries and containment as those invariants enter the contract. A
schema pass, version change, hand-authored mock or root-only load is insufficient.

This milestone establishes only the types and ownership justified by that slice.
It does not design a general framework for drafts or require a parallel draft
exporter. Audit the current oracle before reusing its tests, and record typed
failure/publication behavior and the retained writer's proof limits.

### 2. Integrate retained producers, writer and metadata ownership

Migrate the remaining supported producers through the shared contract in bounded
slices. Assign one owner for coordinates, transforms, bounds, hierarchy and
metadata; lower stages consume validated values. Define each frame and unit
once, compose placement at its owner, and check containment on final decoded
Tiles 1.1/glTF 2.0 bytes. Use shared feature identity semantics for mesh and
vectors without forcing their geometry into one model.

Retain/rework/replace/remove existing producer and writer components according
to their independent evidence. Preserve current implicit subtree and metadata
semantics and the 3TZ boundary. Plain-glTF vector fills, including holes and
stable source attributes, need successful viewer appearance and picking before
#75 retires their legacy path. Complete current-content conformance and resource
closure where each migrated producer needs them.

Acceptance covers the supported explicit/implicit meaning, known coordinate
references, source picking, final bounds, resources and typed failures. Tiny
equivalent fixtures can establish representation equivalence; full production
emitters are required only where an advertised supported path needs them.
Keep each superseded path until its migrated final consumer is independently
accepted. Review public operation names and retained APIs against the 0.4.0
contract; no new delivery naming or speculative 0.5.0 abstraction is required.

### 3. Deliver the production streaming textured large-mesh tiler

Build the large-mesh pipeline on the shared contract in bounded dependency order:
in-process texture codec gate; streamed capture/sort and tree construction;
streamed hierarchy/delivery output; full-detail leaves; certified geometry LOD;
proxy texture re-parameterisation and baking; validator scale support. These are
successive slices inside this milestone, not parallel workstreams. Emit current
Tiles 1.1/glTF 2.0 using its accepted subtree, metadata and delivery encoding.

Preserve the following agreed large-mesh requirements:

- One local static glTF/GLB scene, confined snapshot dependencies, local metres
  and rigid anchor/orientation/offset placement. Keep the established PBR slot,
  UV, tangent and colour domain and its explicit unsupported cases.
- Streaming input and output with memory bounded by declared configuration;
  scratch preflight by stage, memory-backed scratch reporting, cleanup on
  failure, and deterministic output per target independent of jobs, run size,
  fan-in and scratch location. Retain explicit hierarchy/individual-item and
  current container-specific limits; do not add a total input-size refusal. The
  3TZ boundary keeps its early typed refusal at the 178,956,970-member ceiling.
- Full-detail leaves preserve exact per-triangle source provenance. Proxies
  preserve identity for every present primitive; removed geometry remains
  accounted for. The current metadata representation must prove these identities
  and picking semantics; a later encoding must reprove them before replacement.
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
changes them; use the real Tiles 1.1/glTF 2.0 semantics for 0.4.0. Rederive
extension names, JSON shapes and new wire encodings at the official 0.5.0
transition. A successful codec prototype does not accept the tiler, and
untextured temporary proxies do not accept textured output.

### 4. Accept resources, fidelity, platforms and performance; retire superseded code

Run the final production output auditor and independent validator on every
profile and scale gate, with the real glTF/meshopt/KTX2 bytes and the proven
Tiles 1.1/glTF 2.0 consumer route. Include coordinate and picking controls, seams, topology,
holes/islands, appearance and geometric-error checks. Freeze constants only
after those checks pass.

Retain the v6.1 pass/fail scale obligations: the spilled 5.26×10^8-triangle
production octree case, the 10^4-primitives and 10^4-holes/islands-per-root
feature-density case, the procedural-texture bake case, and the large hierarchy
and delivery cases (at least 2^22 tiles and over 4 GiB of output). Carry forward
the 3TZ format-specific probes and independently check the proven viewer delivery
route. A replacement container at 0.5.0 needs its own delivery checks. Measure
peak resident memory, scratch, wall time and consumer load/refinement against their declared
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

## Immediate queue and first new implementation slice

Finish final-source independent acceptance of the existing C1 meshopt correction
at `538c0bde4623e8ad9561e444a19769274805eaa9`, using its nine retained independent
controls and applicable checks. Then implement the bounded shared explicit
Tiles 1.1 serializer below; the next concrete caller is one retained implicit
producer. Preserve and diagnose C2's `BQ.references-pair` failure at its owning
reference-budget boundary: the charged total must admit and one fewer reference
must refuse. This is an unresolved resource-accounting gate, not an established
geometry defect or a blanket prerequisite to the explicit serializer. Reuse C2
logic only where its required domain has independent acceptance; do not require
completion of every old C2 integration before the new bounded slice.

The first new implementation slice is **owned tile records to the current
writer**, within milestone 1:

**Inputs:** an exact converter source revision and retained-writer audit; the
current Tiles 1.1/glTF 2.0 specification and validator references; a pinned
supported viewer; one small static local mesh with two independently known
source identities; a known WGS84 anchor with asymmetric orientation/offset
controls; and a tiny generated hierarchy with known finite bounds and errors.
Keep fixtures small enough to inspect independently.

**Change:** establish a small generated explicit Tiles 1.1 node/document
serializer at `src/tileset_node.rs`, whose current helpers interpret JSON. Have
`src/mesh_archive/encode.rs::tileset` and the single-node
`src/mesh_archive/model.rs::manifest` consume it. Introduce only the owned
hierarchy, bounds, placement, content references and identity these two callers
need;
reuse the established `Bounds`, `ResolvedPlacement` and `RootContent` contracts
where their evidence supports retention. Resolve values at their owning
boundaries, and audit serialization and metadata attachment for this domain.
Avoid a broad abstraction for future formats and avoid requiring full explicit
and implicit production emitters in
this first slice. Existing implicit output remains supported; its migration and
subtree-boundary acceptance belong to a subsequent concrete caller slice.
Plain-glTF vector fills and their viewer gate belong to milestone 2.

**Output:** a runnable conversion generating a small real Tiles 1.1/glTF 2.0
dataset from owned records; a reproducible local load through the proven current
delivery route; and a short evidence record naming source, specification,
viewer and artifact identities. Preserve the current 3TZ boundary and validate
its resources if used in this route. Production acceptance is limited to this
declared input domain.

**Gates:** independently inspect emitted hierarchy, addressing, metadata and
resources; decode coordinates and verify known transformed points and final
bounds; pick both source identities; force hierarchy traversal and refinement;
and include wrong-identity, wrong-frame and missing-resource controls that
demonstrably fail. Exercise failed conversion without publishing an invalid
dataset. Record consumer failures and proof limits. One root load or matching
historical bytes is insufficient. Extend implicit coverage to a tiny subtree
boundary fixture when that representation enters the owned contract.

This slice supplies the first shared contract and real current-format consumer
evidence. It does not implement the whole tiler or retire advertised output
modes. The next dependency-ready slice integrates the next retained producer and
its metadata ownership through this demonstrated boundary.

## 0.5.0 reference context

The following draft source revisions were inspected on 2026-10-12. Preserve
these pins as design context for 0.5.0, not as the 0.4.0 wire contract or a claim
of accepted support. The official transition must use the finalized specification
and supported viewers, pin its normative sources and consumer builds, and
reconcile any differences from these drafts. Optional draft experiments can
check internal design choices; they cannot block 0.4.0 shipping.

| Source | Inspected revision |
| --- | --- |
| Tiles 2.0 conformance classes | [`CesiumGS/3d-tiles` `3806d0c2`](https://github.com/CesiumGS/3d-tiles/blob/3806d0c2d68d23e0e8d3319887833975f904d82f/specification/2.0/CONFORMANCE.adoc) |
| Migration summary | [`CesiumGS/3d-tiles` `4d781014`](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/next/2.0/CHANGES.md) |
| glTF 2.1 draft | [`KhronosGroup/glTF` `289a9db8`](https://github.com/KhronosGroup/glTF/blob/289a9db8000a4b57fdf06c140423f9646506646b/specification/2.1/Specification.adoc) |
| Tileset, implicit and subtree extensions | [`CesiumGS/glTF` `4f123715`](https://github.com/CesiumGS/glTF/tree/4f123715831a8c362d2562901911fc4ae9508503/extensions/2.1/Vendor) |

The inspected drafts propose changes to both representation and meaning: glTF
nodes and external assets replace Tileset JSON; glTF assets replace binary subtree framing; local
frames are Y-up; geometric error is no longer scaled by node transforms; and
actual bounds/errors move to accessor-backed subtree attributes. In particular,
the box override becomes a DOUBLE MAT4 rather than twelve metadata values.
Preserve application properties through structural metadata at the new attachment
points. Revalidate each mapping independently against the finalized 0.5.0
specification. New subtree, coordinate, metadata and delivery encoding must be
rederived at that transition. Native glTF packaging is a separate delivery contract; do not
assume the existing 3TZ index or ceiling applies to it.

**Current consumer blocker (source inspection):** CesiumJS 1.146.0 at
`b8d3a36fe98a3e432eb89253c95d5f20e605e0f1`, and inspected `main` at
`df52c781de3491a4b76839d420f7ca90a032efb6`, reject glTF asset version 2.1 in
[`GltfJsonLoader`](https://github.com/CesiumGS/cesium/blob/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1/packages/engine/Source/Scene/GltfJsonLoader.js#L265)
and tileset versions beyond 1.1 in
[`Cesium3DTileset`](https://github.com/CesiumGS/cesium/blob/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1/packages/engine/Source/Scene/Cesium3DTileset.js#L2386).
The same restrictions were inspected at the pinned `main` revision
([glTF loader](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/engine/Source/Scene/GltfJsonLoader.js#L267),
[tileset loader](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/engine/Source/Scene/Cesium3DTileset.js#L2386)). An upgrade
is not a demonstrated fix. These source observations constrain the future 0.5.0
transition; they do not block a 0.4.0 consumer using real Tiles 1.1/glTF 2.0.
Do not claim draft consumer support without an executed compatible path. Any
optional adapter or prototype needs bounded scope and evidence; this roadmap
does not authorize building a renderer or adding fallback export formats.

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
domains; they do not accept the full 0.4.0 tiler or the 0.5.0 format transition
by inheritance.

Planning inputs read for this consolidation (not newly accepted contracts):
`f1d3-large-mesh-contract.md` from `/tmp/rusty-tiles-ml0/` (SHA-256
`2cb922a2dbaa5018a2974dc729a3f95afdb98d52db8c3812ff91aae3df4fbbe5`),
`f1d3-surface-dispositions.md` from that worktree (SHA-256
`bc9fc08f6d7e35a7b49261a20e394c54b20f340b2ffed9ef02da2f2cd30b9f95`),
and v6.1 `slices.md` from `/tmp/rusty-tiles-mlarge-design-20261012/` (SHA-256
`705bb8f775e18eb626c62cb27a20cd485bf28f71b24cab38332c0ae5f119f225`).
Preserve these inputs with the durable roadmap evidence before deleting the
temporary worktrees; their location is provenance, not a build dependency.
