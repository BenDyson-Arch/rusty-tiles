# 0.4.0 foundation and streaming; 0.6.0 format transition

Status: **development direction and ordered roadmap, amended 2026-10-24**. This document
consolidates the proposed ML0–ML10 sequence and records the accepted release
split. It authorizes planning and bounded implementation slices; it records no
new implementation acceptance or release authorization.

**0.4.0 ships real 3D Tiles 1.1/glTF 2.0 and implicit tiling, with the shared
foundation and the streaming, textured classic octree HLOD mesh tiler. 0.6.0 makes
the official 3D Tiles 2.0/glTF 2.1 transition against finalized specifications
and supported viewers.** Version strings must describe the encoding actually
implemented. Draft experiments are optional design checks, never 0.4.0 blockers.

The [2026-10-24 HLOD decision](hlod-040-decision.md) supersedes the H17/H27
coarse-mesh requirements. Leaves retain exact provenance; coarse error is
sampled, not certified, and all independently refined outer interfaces must
agree on decoded bytes. The detailed codec, calibration and acceptance budgets
are proposals for review; this amendment is not an implementation pass.

**Non-mesh amendment proposed for owner approval in this review:** adopt
[#267](https://github.com/BenDyson-Arch/rusty-tiles/pull/267)'s finite 0.4 bar,
summarized in the [decision](hlod-040-decision.md#proposed-non-mesh-04-bar--owner-approval-in-this-review).
Retain accepted point/vector/native imagery/terrain foundations within their
established domains; close small decoded-output, placement/bounds, realistic
resource, lifecycle and frontend gaps with existing suites green. Deeper
oracle/component audits and worst-case resource proofs move to 0.4.x, with
limits explicit. Known wrong output or destructive publication still blocks
release. This proposed exception replaces blanket non-mesh rewrite requirements
only after approval; it is not a waiver of mesh scale gates or terrain's existing
numerical guarantees.

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
  Reassess packaging and derive new encoding limits at the 0.6.0 transition.
- Migrate producers through the shared output contract. Mesh input is a local
  static scene in local metres with rigid geographic placement, or a declared 2D
  horizontal source CRS (EPSG/WKT/PROJ, with explicit or Metashape E/N/A offsets)
  mapped once at capture; there is no automatic CRS detection (owner decision
  2026-10-14, [#218](https://github.com/BenDyson-Arch/rusty-tiles/pull/218)). Other
  producers keep their independently established coordinate domains while their
  boundaries are audited.
- Optional AEC semantics, splats, voxels, time, horizon and shapes are later work.
  They do not expand this roadmap's supported domain or delay 0.4.0 shipping.

The new-format migrations in [#84](https://github.com/BenDyson-Arch/rusty-tiles/issues/84)
(tileset writing), [#85](https://github.com/BenDyson-Arch/rusty-tiles/issues/85)
(implicit tiling), [#86](https://github.com/BenDyson-Arch/rusty-tiles/issues/86)
(coordinates), and [#87](https://github.com/BenDyson-Arch/rusty-tiles/issues/87)
(vectors/layers) belong to 0.6.0. Foundation work and current-content conformance
needed to ship 0.4.0 remain in 0.4.0. [#75](https://github.com/BenDyson-Arch/rusty-tiles/issues/75)
glTF-only vector fills passed their viewer gate and merged in #185; the
remaining legacy-content removal stays in 0.4.0 through H6 (explicit output)
and H7 (validator reports b3dm/i3dm/pnts/cmpt as Unsupported). The new-format/AEC work in
[#89](https://github.com/BenDyson-Arch/rusty-tiles/issues/89) is later work;
necessary glTF 2.0 conformance does not wait for it.
[#83](https://github.com/BenDyson-Arch/rusty-tiles/issues/83) tracks the consolidated
direction. Issue membership is not proof of completion.

## Four ordered 0.4.0 milestones

### 1. Establish the shared typed output contract with a tiny consumer slice

Define the smallest owned tile records needed by an actual current producer:
hierarchy, finite bounds, transforms, geometric errors, content references and
source identity. Shape them by what the large-mesh tiler of milestone 3 will
need, so that the tiler becomes their long-term consumer. Establish a shared
generated explicit-node/document serializer at the existing `src/tileset_node.rs`
boundary, consumed first by vector's default implicit output (after H6) in `src/vector/pipeline/store.rs`. No work goes into migrating the legacy mesh
paths that the large-mesh tiler supersedes (user decision: retired once the
tiler lands): `mesh-to-3tz` in `src/tile.rs` and the mesh archive tileset in
`src/mesh_archive/encode.rs`. This roadmap does not change the disposition of
the W1 API in `src/mesh_archive/model.rs` recorded in the
[public surface inventory](public-surface-inventory.md). Retain only writer
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
semantics and the 3TZ boundary. GLB vector fills (#75) merged in #185 with their bounded viewer appearance
and picking evidence, including holes and stable source attributes. Complete current-content conformance and resource
closure where each migrated producer needs them.

Acceptance covers the supported explicit/implicit meaning, known coordinate
references, source picking, final bounds, resources and typed failures. Tiny
equivalent fixtures can establish representation equivalence; full production
emitters are required only where an advertised supported path needs them.
Keep each superseded path until its migrated final consumer is independently
accepted. Review public operation names and retained APIs against the 0.4.0
contract; no new delivery naming or speculative 0.6.0 abstraction is required.

### 3. Deliver the production streaming textured large-mesh tiler

Build the large-mesh pipeline on the shared contract in bounded dependency order:
in-process texture codec gate; streamed capture/sort and tree construction;
streamed hierarchy/delivery output; full-detail leaves; classic octree HLOD
with preserved outer interfaces; fresh coarse charts and cheap source-surface
baking; validator scale support. These
tiler slices are successive and dependency-ordered among themselves. The tiler
becomes the long-term caller of the shared serializer; mesh-specific fixtures,
including a small static local mesh with WGS84 anchor/orientation/offset
controls, enter here. Emit current Tiles 1.1/glTF 2.0 using its accepted
subtree, metadata and delivery encoding.

Milestones 2 and 3 are not strictly sequential. Producer migration, validator work and
performance fixes on retained paths may run concurrently with the tiler slices
when their file ownership is disjoint. GLB vector fills (#75, #185) and C2
slice 3 (#179) are merged within their recorded bounded scopes. The ML1 encoder work
([#163](https://github.com/BenDyson-Arch/rusty-tiles/pull/163)) may be prepared
alongside the earlier tiler slices. Only milestone 4 waits for both milestones.

Preserve the following agreed large-mesh requirements:

- One local static glTF/GLB scene, confined snapshot dependencies, local metres
  and rigid anchor/orientation/offset placement, or a declared source CRS mapped
  at capture (#218). Keep the established PBR slot,
  UV, tangent and colour domain and its explicit unsupported cases.
- Streaming input and output with memory bounded by declared configuration;
  scratch preflight by stage, memory-backed scratch reporting, cleanup on
  failure, and deterministic output per target independent of jobs, run size,
  fan-in and scratch location. Retain explicit hierarchy/individual-item and
  current container-specific limits; do not add a total input-size refusal. The
  3TZ boundary keeps its early typed refusal at the 178,956,970-member ceiling.
- Full-detail leaves retain today's ownership, at most 8,192 triangles, exact
  per-triangle source provenance and IDs, and current per-profile encoding
  fidelity. No source triangle is lost or duplicated. Coarse triangles carry
  one representative source primitive feature ID; remove `proxy_source` and
  any claim of exact coarse triangle ancestry.
- Build each parent from its children with meshopt, locking only its outer
  interface to other subtrees; release interior/UV/attribute seams. Preserve
  complete segments and junctions, including nonmanifold interfaces, through
  decoded encoding. No blanket source-hole border lock. The target is at most
  8,192 triangles or an error stop, with a hard published 8,192 cap; refuse an
  unsatisfiable protected-interface constraint explicitly. Keep the permanent
  all-pair mixed-depth/same-depth crack census; no skirts or seam consensus.
- Publish a sampled two-sided maximum against captured source, made monotonic
  up the tree and measured on final decoded geometry. It is **not certified**.
  Archive validation checks finite/nonnegative ordered errors, availability,
  resource/codec/metadata integrity and decoded containment; independent
  source-aware acceptance checks exact leaves, cracks, error and appearance.
  Do not claim the archive-only validator proves source-distance bounds.
- Plain simplification has no separate hole-fill or island-vanish policy. Remove
  published vanished/filled/reopening records. Coarse topology changes are
  checked by quality fixtures; leaves recover source geometry. A missing tile
  interface is always a defect, never an intentional hole.
- Give coarse tiles fresh nonoverlapping charts, cheap bounded source-surface
  baking and content-sized atlases. Balanced targets GE/16 spacing with a
  provisional 1024-side page cap; report actual spacing/cap hits. Keep all
  admitted material/PBR, MASK, normal-frame and lit/unlit semantics. Remove
  H27's per-texel ledger, drift gates and facing/donor search. Child pre-encode
  sampling is an optimization requiring separate quality evidence.
- Leaves use cropped atlases with correct wrap/filter/mip alignment, never
  unneeded whole source images. Audit the old dedicated mipmapped-binding
  exception as part of the crop work. Retain exact leaf picking and the
  `lossless` texel policy.
- Retain in-process KTX2/Basis and meshopt, PNG for `lossless`, class-correct
  mips and per-page quality checks. Recommend ETC1S for fast/balanced coarse
  colour (and leaves if accepted), UASTC for quality and class-specific needs.
  Freeze that choice only after one 2048 campaign measures bytes, CPU, all
  three first views and visual/client behavior. No JPEG production path.
- Keep `fast`, default `balanced`, `quality`, `lossless`, the complete per-tile
  byte budget and reported lossy mip drops. The leaf cap is settled at 8,192;
  density, page caps and codec effort remain provisional. HLOD's sampled maximum
  is the initial refinement error; a guarded half-error/p99 calibration can
  ship only after the separate view/approach gate in the decision, without
  changing bake density or claiming certification.
- One declared whole-job memory budget must cover capture, source indexes,
  active children/parent, images/codecs and delivery. Add CLI `--memory-budget`
  and Rust/Python `memory_budget_bytes`; these are planned, not available APIs.
  Process subtrees postorder and release consumed children; spill inventories
  and admit work before allocation. Image-free inputs reserve zero texture
  workspace. The proposed 2048 cap is 4 GiB; untextured scale gate (a) stays
  2 GiB. Report simultaneous scratch/staging, not capture-only spill.

Recorded decisions for the tiler:

- Public surface. The CLI is `rusty-tiles mesh` and the Python API is
  `rusty_tiles.mesh_to_tileset`. This is the one public entry point; the H8
  leaf-directory and H16 delivery APIs become internal. `--jobs N` is a
  memory-capped maximum and reports the jobs it actually used; the default is
  auto and memory-aware. Output is `.3tz` only in 0.4. Profiles stay
  provisional. Placement is rigid local metres or a declared source CRS (#218).
  `mesh-to-3tz` and `mesh-local-to-3tz` were removed before the 0.4.0 tag
  (owner decision 2026-10-21; on 2026-10-23 the owner waived the remaining
  acceptance gates for the deletion). Old command names are unknown
  subcommands and the migration notes are in the mesh guide. The
  [retirement record](legacy-mesh-retirement.md) lists what was removed and
  what was kept.
- Leaf size. The 8192-triangle cap per leaf stays. Measured: CesiumJS 1.146
  resolves mesh picks through one property table per model. 32k-triangle leaves
  split into 4 tables picked correctly only 8 of 56 times and could not refine
  at the default 1 GiB cache.
- Cesium 1.146 sRGB limitation. KTX2 sRGB colour textures are uploaded as
  linear (UNORM) formats on the ASTC, BC7, ETC2 and uncompressed paths. The
  measured worst case is 72/255 on high-contrast test textures; linear data
  textures are unaffected. The user decided this is documented, not a 0.4
  blocker, and not filed upstream. The tiler adds no viewer-specific
  compensation.
- Validator. `validate` accepts KHR_texture_basisu / KTX2
  ([#202](https://github.com/BenDyson-Arch/rusty-tiles/pull/202)), so the
  tiler's own default output validates. VS-A
  ([#206](https://github.com/BenDyson-Arch/rusty-tiles/pull/206)) handles the
  catalog at scale. VS-B traversal/containment (#217) and VS-C derived limits
  with opt-in caps (#214) are merged; the old default 1 GiB / 2^18-visit
  refusal is historical. VS-D final HLOD output/scale acceptance remains open.

Annotation readiness. Leaves keep exact per-triangle source provenance for a
future, post-0.4 annotate pass. Triangle-aligned annotations can add an
`EXT_mesh_features` feature-ID set/property table; boundaries through triangles
need ID textures with nearest/nonblending sampling, lossless IDs and no-feature
gutters. Feature-ID sets remain extensible. Coarse tiles retain only a
representative primitive label: source-triangle or regional annotations cannot
be propagated exactly through it. Such propagation needs a later explicit
aggregation/rebake policy. Keep reusable leaf ID-texture semantics without
retaining the obsolete H27 baker. Coarse nonoverlapping UV charts establish
texture editability, not exact source ancestry.

CesiumJS 1.146 support: feature ID textures and property tables have mature
picking and styling. Property textures work in custom shaders since 1.92.
Property-texture picking via `scene.pickMetadata` is experimental since 1.122.

The earlier v6.1 design supplies the detailed algorithms, constants and tests as
planning inputs. The HLOD decision explicitly replaces its certified LOD, coarse
provenance, hole/island and per-texel bake requirements; retain unrelated
requirements and use the real Tiles 1.1/glTF 2.0 semantics for 0.4.0. Rederive
extension names, JSON shapes and new wire encodings at the official 0.6.0
transition. A successful codec prototype does not accept the tiler, and
untextured temporary proxies do not accept textured output.

### 4. Accept resources, fidelity, platforms and performance; retire superseded code

Run the final production output auditor and independent validator on every
profile and scale gate, with the real glTF/meshopt/KTX2 bytes and the proven
Tiles 1.1/glTF 2.0 consumer route. Include coordinate and picking controls, seams, topology,
source holes/components as simplification-quality cases, appearance and sampled
geometric-error checks. No H17 fill/vanish accounting or certification remains.
Freeze constants only after those checks pass.

Retain the v6.1 pass/fail scale obligations: the spilled 5.26×10^8-triangle
production octree case, the 10^4-primitives and 10^4-source-holes/islands-per-root
feature-density input (no special fill/vanish/reopening rules), the procedural
texture bake case with all admitted slot/mip semantics, and the large hierarchy
and delivery cases (at least 2^22 tiles and over 4 GiB of output). Carry forward
the 3TZ format-specific probes and independently check the proven viewer delivery
route. A replacement container at 0.6.0 needs its own delivery checks. Measure
peak resident memory, scratch, wall time and consumer load/refinement against their declared
budgets. Smaller tests establish bounded correctness evidence; they do not
substitute for these scale gates.

Validator memory and work must scale with the supported delivered-resource domain;
VS-B and VS-C close the gap for large real archives noted in milestone 3. Keep
absolute volume caps opt-in, explicit per-item limits and non-circular
amplification budgets; test malformed and multiply traversed resources with
sensitive controls. Complete applicable platform CI, installed-wheel and
official-Blender checks on the accepted final sources and artifacts.

The superseded exporters, legacy mesh commands/options, codecs and shared
helpers were removed on 2026-10-23, ahead of the acceptance work above, by
owner decision (the staged gates were waived). Public APIs, help, defaults,
reuse-identity guidance and migration documentation changed together; see the
[retirement record](legacy-mesh-retirement.md).
Review the retained public surface explicitly. Closing this milestone requires
the final acceptance evidence; document consolidation alone does not close
[#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113) or authorize a release.

#### Status at 2026-10-23

The merged entries below record history, not acceptance of the replacement.
**2026-10-24 correction:** H17/H27 are superseded by classic HLOD; replacement
slices S1–S8 in the [decision](hlod-040-decision.md#ordered-implementation-slices)
remain open. Proposed non-mesh rescope credits existing foundations and replaces
blanket audit completion with the five finite checks below **if the owner
approves it in review**. Nothing is newly marked passed by this amendment.

An item is marked done only where a merged PR delivered it. Merged work shows
that a slice exists; it does not close the acceptance item it feeds. Milestone
4 waits for milestone 3, so the tiler slices are listed first. PR numbers are
merged into `develop`.

Tiler prerequisites (milestone 3):

- [x] Streamed capture, sort and tree: [#180](https://github.com/BenDyson-Arch/rusty-tiles/pull/180), with the external sort in [#166](https://github.com/BenDyson-Arch/rusty-tiles/pull/166).
- [x] Full-detail leaves with the in-process KTX2/Basis encoder: [#187](https://github.com/BenDyson-Arch/rusty-tiles/pull/187), [#199](https://github.com/BenDyson-Arch/rusty-tiles/pull/199) (the encoder draft #163 was closed unmerged).
- [x] Streamed Tiles 1.1 forest in `.3tz`: [#192](https://github.com/BenDyson-Arch/rusty-tiles/pull/192).
- [x] Historical H17 certified geometry LOD (superseded; replacement pending): [#209](https://github.com/BenDyson-Arch/rusty-tiles/pull/209); seam overlap fix [#236](https://github.com/BenDyson-Arch/rusty-tiles/pull/236).
- [x] Historical H27 proxy texture baking in `mesh` (superseded; replacement pending): [#215](https://github.com/BenDyson-Arch/rusty-tiles/pull/215), [#216](https://github.com/BenDyson-Arch/rusty-tiles/pull/216), [#231](https://github.com/BenDyson-Arch/rusty-tiles/pull/231), [#233](https://github.com/BenDyson-Arch/rusty-tiles/pull/233), [#237](https://github.com/BenDyson-Arch/rusty-tiles/pull/237); chart growth [#242](https://github.com/BenDyson-Arch/rusty-tiles/pull/242) and facing fallback [#246](https://github.com/BenDyson-Arch/rusty-tiles/pull/246).
- [x] One public entry point, `mesh` / `mesh_to_tileset`: [#213](https://github.com/BenDyson-Arch/rusty-tiles/pull/213); source CRS [#218](https://github.com/BenDyson-Arch/rusty-tiles/pull/218); docs lead with it [#238](https://github.com/BenDyson-Arch/rusty-tiles/pull/238).
- [x] Per-tile primitive batching (H28): [#244](https://github.com/BenDyson-Arch/rusty-tiles/pull/244).
- [x] Page escalation, archive size and scratch admission: [#229](https://github.com/BenDyson-Arch/rusty-tiles/pull/229), [#234](https://github.com/BenDyson-Arch/rusty-tiles/pull/234), [#226](https://github.com/BenDyson-Arch/rusty-tiles/pull/226), [#228](https://github.com/BenDyson-Arch/rusty-tiles/pull/228), [#230](https://github.com/BenDyson-Arch/rusty-tiles/pull/230).

Milestone-4 items:

- [x] Validator scale support (milestone 3 handoff): VS-A catalog [#206](https://github.com/BenDyson-Arch/rusty-tiles/pull/206), VS-B traversal and containment [#217](https://github.com/BenDyson-Arch/rusty-tiles/pull/217), VS-C limits with opt-in caps, derived budgets and cancellation [#214](https://github.com/BenDyson-Arch/rusty-tiles/pull/214). KTX2 admission: [#202](https://github.com/BenDyson-Arch/rusty-tiles/pull/202).
- [x] Legacy payload retirement in `validate` (H7): [#200](https://github.com/BenDyson-Arch/rusty-tiles/pull/200). Explicit point-cloud and vector output (H6): [#188](https://github.com/BenDyson-Arch/rusty-tiles/pull/188).
- [x] Scale-gate harness, bench only: [#190](https://github.com/BenDyson-Arch/rusty-tiles/pull/190), combined (d) case [#193](https://github.com/BenDyson-Arch/rusty-tiles/pull/193). These add generators and budgets, not passing runs.
- [x] Release gating on CI, release candidates kept off PyPI: [#239](https://github.com/BenDyson-Arch/rusty-tiles/pull/239).
- [x] Public-surface decisions recorded: [#182](https://github.com/BenDyson-Arch/rusty-tiles/pull/182), [#243](https://github.com/BenDyson-Arch/rusty-tiles/pull/243). The review of the retained surface itself is not closed (below).

Remaining before 0.4.0:

- [ ] HLOD S1–S4: bounded subtree geometry, exact decoded interfaces/leaves,
  sampled source error, fresh charts/cropped leaves, KTX2/mips and H16 forest
  integration. No certificate or per-texel ledger requirement survives.
- [ ] HLOD S5 / VS-D: full-audit independent acceptance of final **mesh** output
  across profiles: format/resources/availability, decoded bounds, feature IDs,
  error ordering, all-pair cracks, source-aware leaf and sampled-quality checks.
  Keep C1/I1/C2 and VS-A/B/C; remove only superseded H17 certificate expectations.
  Non-mesh full-profile VS-D expansion is proposed for 0.4.x; small final producer
  fixtures, existing corruption suites and truthful unsupported claims stay now.
- [ ] HLOD S6/S7: delete H17/H27 code and normative requirements, obsolete coarse
  records and reports; finish the memory CLI/API and annotation/error/codec
  migration docs. Do not preserve an H17 compatibility mode.
- [ ] HLOD S8: one integrated 2048 campaign at the three pinned views. Proposed
  balanced gates: ≤50 whole / ≤30 coarse CPU-s/Mtri; ≤6.0/63.366/197.521 MB at
  1600×1000/range 3, 2560×1600/range 3, 2560×1600/range 0.75; declared/measured
  ≤4 GiB RSS; 0 cracks/leaf identity errors, with the decision's appearance gates.
  Measurements include source capture/partition and all image dependencies.
- [ ] Scale gate (a): spilled 526,174,760-triangle production octree through
  `mesh`, explicit 2 GiB cap, no untextured texture reserve and postorder releases.
  Harvest #262's diagnosis/run ladder; rederive scratch for HLOD including
  indexes/handoffs/staging. Existing 4× input scratch, 7,200 s producer,
  2 GiB/3,600 s validator and 30/300 s loaded/refined consumer targets remain
  unresolved gates; do not silently replace them with its old 400 GB estimate.
  Resolve any budget amendment before the 100M/526M runs.
- [ ] Scale gate (b): 10^4 primitives and 10^4 source holes/islands per root;
  exact leaf recovery, representative coarse IDs, hard tile cap, decoded seams
  and quality/resource controls. No vanish/fill/reopening policy or records.
- [ ] Scale gate (c): procedural textures through the new chart/bake/codec path,
  cropped leaves, mips/normal/MASK/material classes, measured CPU/memory/scratch.
- [ ] Scale gate (d): combined ≥2^22 tiles and >4 GiB archive against final
  `validate` and the proven Cesium delivery route; #193 is only a fixture.
- [ ] Preset finalisation: hold leaves at 8,192; freeze chart/page caps, density,
  ETC1S/UASTC choices/effort, octahedral bits and per-tile budgets from S8.
  Keep PNG lossless and class gates. Publish G initially; smaller refinement R
  needs its own quality/streaming pass, not #264's H17 forecasts.
- [ ] Proposed non-mesh bar, owner approval pending — point P1/current forest:
  finite decoded fidelity, placement/bounds, lifecycle, paired CLI/Python and
  10M-point resource receipt (≤1 GiB RSS / ≤10 GiB scratch).
- [ ] Proposed non-mesh bar — vector: direct analytic published-forest geometry,
  hole/line coverage, metadata/identity, placement/reuse/seams, lifecycle,
  CLI/Python and 100k-feature receipt (≤1 GiB RSS / ≤4 GiB scratch).
- [ ] Proposed non-mesh bar — native imagery R2: literal display/COG/phase/bounds,
  lifecycle/capabilities and 8192² RGBA receipt (≤768 MiB RSS / ≤3 GiB scratch).
- [ ] Proposed non-mesh bar — native terrain T1/packed LOD: existing discrete
  lattice/height/bounds/LOD/skirt guarantees, lifecycle/capabilities, missing-GTiff
  classification and 2048² DEM receipt (≤512 MiB RSS / ≤2 GiB scratch).
- [ ] Proposed non-mesh shared closeout: existing applicable Rust/Python/native/
  format/platform/package/consumer suites on final candidate, supported domains
  and deferred audits recorded; reconcile #113/#120–#126 and blanket architecture/
  AGENTS wording after approval. No new bindings or tiler rewrites required.
  Combined proposed non-mesh effort is 8–14 agent-days, not 45–75.
- [x] Legacy `mesh-to-3tz` / `mesh-local-to-3tz`, superseded exporters/codecs/
  helpers removed by owner decision on 2026-10-23; this corrects the stale open
  checkbox, not an acceptance waiver for the HLOD successor.
- [ ] Final retained-public-surface review, platform CI, installed-wheel and
  official-Blender checks on accepted final sources/artifacts. Documented
  capability exclusions and known consumer limits stay explicit.
- [ ] Coordinate, picking, exact interfaces/leaves, coarse appearance/topology
  and sampled-error controls on final production output across profiles.
- [ ] The release itself: [#110](https://github.com/BenDyson-Arch/rusty-tiles/pull/110)
  (develop to main) remains open; #113/#121 acceptance is not closed here.

## Immediate queue and first new implementation slice

**Historical 2026-10-12 queue; sequencing superseded.** The current mesh queue
is HLOD S1–S8 above. The following records the original
M1 planning context; merged serializer/forest work is retained, not restarted.

The C1 meshopt correction (#176), GLB vector fills (#185), outline runs
(#186), point outward bounds (#183) and C2 slice 3 (#179) are merged.
H6 retires all vector/point explicit output modes before M1; its prerequisites
are complete. M1's first caller is vector's **default implicit output**.
H7's legacy payload retirement still requires its own contract amendments,
independent oracle/controls and final nonauthor audit. H14 records these
public-surface decisions in the [inventory](public-surface-inventory.md#acceptance-and-removal-sequence).
H6, H7 and H11 (C2 8-corner fast path) are transferred to Claude; H14
only records their statuses and decisions.
Do not migrate superseded mesh paths; the large-mesh tiler becomes the
serializer's long-term caller in milestone 3. C2's bounded accepted scope does
not establish decoded containment of derived cells, RTC, regions or rotated boxes.

The first new implementation slice is **owned tile records to the current
writer**, within milestone 1:

**Inputs:** an exact converter source revision and retained-writer audit; the
current Tiles 1.1/glTF 2.0 specification and validator references; a pinned
supported viewer; one tiny point/line vector source with two independently
known source identities (`_source_id`); a known WGS84 anchor with asymmetric
east/north/up offset controls for its local frame; and a tiny generated
hierarchy with known finite bounds and errors. Keep fixtures small enough to
inspect independently; mesh fixtures belong to milestone 3.

**Change:** establish a small generated explicit Tiles 1.1 node/document
serializer at `src/tileset_node.rs`, whose current helpers interpret JSON. Have
the default implicit output of the vector writer in
`src/vector/pipeline/store.rs` consume it. This producer already uses
`src/tileset_node.rs`, writes point/line and, after #185, polygon-fill content
as GLB, carries a stable per-feature source identity that
Cesium can pick, and places a local frame at a WGS84 anchor. That identity holds
with point aggregation off, its default; opt-in aggregated point parents omit
`_source_id`, so the fixture runs without aggregation. The point cloud writer's
feature IDs are per-tile indices, and `src/tile.rs` is the superseded legacy
`mesh-to-3tz` path. Introduce only the owned hierarchy, bounds, placement, content
references and source identity this caller needs, in a shape the large-mesh
tiler can later consume. The `Bounds`, `ResolvedPlacement` and `RootContent`
contracts in `src/mesh_archive/` are design input for the tiler's needs; move a
type into the shared contract only where its evidence supports retention,
without migrating the superseded mesh paths. Resolve values at their owning
boundaries, and audit serialization and metadata attachment for this domain.
The serializer retains explicit skeleton, external-document, explicit-leaf,
link and implicit-root records within that default implicit forest; retiring
an explicit output mode does not delete these format roles. The forest owner
keeps layout, addressing, rebasing and subtree binaries. Include the vector
subtree-boundary controls in this first caller slice; other producers retain
the existing adapter until their own migration. Avoid a broad abstraction for
future formats. GLB vector fills and their bounded viewer gate merged in #185.

**Output:** a runnable conversion generating a small real Tiles 1.1/glTF 2.0
dataset from owned records; a reproducible local load through the proven current
delivery route; and a short evidence record naming source, specification,
viewer and artifact identities. Preserve the current 3TZ boundary and validate
its resources if used in this route. Production acceptance is limited to this
declared input domain.

**Gates:** independently inspect emitted hierarchy, addressing, metadata and
resources, confirming that every content is plain GLB; decode coordinates and
verify known transformed points and final bounds; pick both `_source_id`
values; force hierarchy traversal and refinement;
and include wrong-identity, wrong-frame and missing-resource controls that
demonstrably fail. Exercise failed conversion without publishing an invalid
dataset. Record consumer failures and proof limits. One root load or matching
historical bytes is insufficient. Require a tiny implicit subtree-boundary
fixture in this first default-implicit caller slice.

This slice supplies the first shared contract and real current-format consumer
evidence. It does not implement the whole tiler or retire advertised output
modes. The next dependency-ready slice integrates the next retained producer and
its metadata ownership through this demonstrated boundary.

## Release sequence after 0.4.0

Decided 2026-10-14. The official format transition moves from 0.5 to 0.6 so that
each version line has one meaning:

- **0.4.x: completion and fixes, no new public capability.**
  - HLOD fixes and further measured refinement tuning; no revival of H27
    per-texel ledgers. Initial preset freeze remains a 0.4.0 acceptance gate.
  - Deferred non-mesh foundation audits/hardening, if the proposed finite bar
    above is approved. #82/#221/#222 investigation may proceed; portable/projected
    terrain capability delivery remains 0.5.
  - Documentation of known consumer limitations, and follow-ups from the
    large-mesh scale runs.
  - Indexed mesh capture (compact sort records with in-place or deduplicated
    vertex attributes). It belongs here only if archives stay byte-identical to
    0.4.0; otherwise it moves to 0.5.
- **0.5: new capabilities on Tiles 1.1/glTF 2.0.**
  - A pure-Rust GeoTIFF/COG DEM reader that makes `terrain` available in the
    standard build. It admits COG base images and ZSTD, refuses LERC, and keeps
    GDAL as the native fallback.
  - Projected-CRS terrain.
  - A JSON C ABI with a Go wrapper first, and other languages on demand.
  - A WASM feasibility spike for a GDAL-free subset.
  - Imagery (`raster`) stays on GDAL.
- **0.6: the official 3D Tiles 2.0/glTF 2.1 transition**, with the scope
  previously recorded for 0.5.

Scope documents for these items are working evidence, not acceptance. Each
item still needs its own slice contract, review tier and gates.

## 0.6.0 reference context

The following draft source revisions were inspected on 2026-10-12. Preserve
these pins as design context for 0.6.0, not as the 0.4.0 wire contract or a claim
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
points. Revalidate each mapping independently against the finalized 0.6.0
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
is not a demonstrated fix. These source observations constrain the future 0.6.0
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
streaming, new HLOD texture baking at scale, packed implicit output or all-depth
interface preservation. Interval certificates and vanished/filled policy are
retired mesh requirements, not unfinished HLOD gates. Record those limits
alongside their positive results. Earlier accepted foundation slices retain only their stated
domains; they do not accept the full 0.4.0 tiler or the 0.6.0 format transition
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
