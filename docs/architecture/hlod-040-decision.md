# 0.4 mesh direction: classic octree HLOD

Decision date: **2026-10-24**. Owner rule: **“go with best practice that doesn't
compromise on client speed and processing speed”.** The owner adopted classic
octree HLOD for `mesh`, replacing H17 certified geometry and H27's per-texel
proxy correspondence bake. This records that direction and proposes its detailed
implementation/acceptance policy. The codec selection, refinement calibration,
numeric budgets and non-mesh rescope below are reviewable recommendations, not
newly accepted output. Base: `develop` `cda7ca6110c714f8eaceea04df8f5945eb6443c8`.

This decision governs milestone 3/4's mesh successor wherever historical H17,
H27, H16, H28 or mesh-guide wording conflicts. The existing implementation is
still H17/H27 until the replacement slices land. This documentation change
builds, bakes and accepts nothing, closes no issue, and authorizes no release.
Tiles 1.1/glTF 2.0, implicit forest delivery and `.3tz` remain the 0.4 format.

## Evidence and its limits

Paths beginning `E/` mean `/home/bend/.cache/rusty-tiles-evidence/`. Branch drafts
were read with `git show`; pinned revisions below remain recoverable even if
those PRs close. Research code is design input, not a production dependency.

| Key | Evidence path / revision | Finding used here |
| --- | --- | --- |
| E1 | `E/lod-bakeoff-20261024/METRICS.md` | Exact leaf provenance/IDs; decoded same/mixed-depth seams; CPU user+sys per source Mtri; two-sided source samples; bytes and first views. Original crop-only rules preceded the separately recorded full-model experiment. |
| E2 | `E/lod-bakeoff-20261024/cluster-dag/README.md`; `exp/lod-cluster-dag` at `816d0670`, `bench/lod_cluster_dag/{README.md,octree_hlod.py,charts.cpp,evaluate_octree.py,evaluate.py}` and its execution/artifact manifests | Adopted experiment is a production-ownership octree **tree**, not a general cluster DAG. Outer interfaces survive all measured depth mixes. |
| E3 | `E/lod-bakeoff-20261024/uv-downscale/README.md` | Strict UV/chart/border locks stall tex3 at 244,947 root triangles (29.9× the cap); reject this variant, not every conceivable UV-preserving algorithm. |
| E4 | `E/lod-bakeoff-20261024/astra-keep-h17/README.md`; #266 `exp/fast-lod-bake` at `5d6b3ce7`, `docs/architecture/fast-lod-experiments.md` | Best tested H17 path gains only 1.39× total / about 1.60× coarse CPU; v0.3.0 tex3 anchor is 7 CPU-s, with different tree, codec and guarantees. No evidence of a 10× H17 tuning route. |
| E5 | `E/leaf-gaps-20261024/README.md` | H17 m2048 mixed-depth gaps reach 0.79 m / 1× GE; leaves have zero cross-leaf mismatch. A certificate does not make REPLACE depth mixes watertight. |
| E6 | #262 `docs/scale-gate-a-plan` at `79b4c17b`, `docs/architecture/scale-gate-a-plan.md` | No declared total memory cap; balanced reserves 3.031 GiB of texture workspace even for image-free input. Stage-only ledgers omit live ownership. |
| E7 | #264 `docs/lod-refinement-design` at `82acb61f`, `docs/architecture/lod-refinement-budget.md` | Smaller refinement estimates can avoid large descendant requests; E/4 forecasts are on H17 bytes, not HLOD measurements. |
| E8 | #265 `docs/bake-time-history` at `dde68c1c`, `docs/architecture/bake-time-history.md` | H27 accounts for 84–92% of recorded full-model wall; worker elapsed timers are not CPU. #260 halves bake wall but reduces total CPU only 6%. |
| E9 | #267 `docs/foundation-rescope-040` at `986b1cf4`, `docs/architecture/040-foundation-rescope.md` | Proposed finite non-mesh acceptance costs 8–14 agent-days versus 45–75 for the broader audit programme. |
| C | [H17](h17-lod-contract.md), [H16](h16-delivery-contract.md), [H28](h28-tile-batching-contract.md), [forest consolidation](implicit-forest-consolidation.md), [mesh guide](../MESH.md), [roadmap](04-tiles2-roadmap.md) at the base above | Retain source admission/placement, leaf identity/encoding, batching, forest and safe publication; replace coarse geometry, identity and bake semantics explicitly. |

Full 2048 model E2: 9,999,999 source triangles, 2,806 occupied cells, 1,959 exact
leaves, 847 parents at most 8,192 triangles. **788 CPU-s (78.80/Mtri), 720.05
coarse CPU-s (72.00/Mtri), 546.1 s wall, 6.06 GiB RSS**. All **15,536 mixed-depth
and 6,859 same-depth pairs** have zero decoded interface gaps. Root PSNR is
20.89/18.85 dB (proxy→source/source→proxy), against 17.00/16.59. First views are
5.442/119.280/197.521 decimal MB against 35.010/63.366/230.805, in the three-view
order specified below.

The wide-view regression is real: six leaves fetch 69 whole source images,
78.22 MB, plus 21.80 MB of proxy images. **Do not port that leaf delivery.**
832/847 parents hit the 1024² cap, so GE/16 was often unattainable. Independent
root sampled max 2.8369 m exceeds published 2.8106 m. Three parents needed two
protected interface faces each; no parent used the prototype's over-budget
merged-children fallback. The complete-segment census establishes shared-source
interfaces, not gaps between unrelated surfaces, invisible texture seams or
all input topology. The prototype admits a narrow base-colour/identity-node
scene domain and writes JPEG without production KTX2 mips. Its whole timing
**excludes creation of the reused baseline partition**. Historical 12.7× CPU
comparison is indicative; integrated production speedup remains unmeasured.

## Decisions

### D1 — geometry and exact leaves

**Owner amendment, 2026-10-24 — exact seams only (S1, PR #275).** No tolerance-based
interface matching can prove bounded work, never refuse and stay crack-free together.
The S1 review's dense-false-box probe took Θ(H·G) exact tests, and the design note
`astra-s1-interface-bound-20261024` found no construction that closes that gap. So S1
follows industry practice (Nanite, Simplygon) and locks only exactly shared items:

- **Canonical key:** a vertex's leaf-canonical decoded position, the f32 triple every leaf
  publishes for it, compared bit for bit (`LatticePlan::canonical`).
  - **Lossy profiles:** `round_ties_even(p / step[ell]) * step[ell]` per axis, computed in
    f64 and stored as f32, with −0 written as +0. `p` is the captured source position after
    node transforms and placement. `ell` is the welded vertex's minimum incident leaf depth.
  - **Quality and lossless:** the captured f32 position, −0 written as +0.
  - **From the archive alone:** the bit patterns of the decoded leaf vertex positions in the
    tileset's common frame.
- **Edge key:** the unordered pair of the two endpoint keys of an edge of a surface
  triangle. A triangle whose corners decode onto fewer than three distinct keys has no
  surface and contributes nothing, so no edge key is ever degenerate.
- **Interface items:** a vertex is an interface vertex of a tile iff its key occurs in a
  leaf outside the tile. A segment is an interface segment iff its edge key occurs in a leaf
  outside the tile. Both come from one external sort of at most six items per triangle:
  O(n log n), bounded memory, staged scratch. Nothing is inferred: a segment between two
  shared vertices is not shared unless another leaf holds that very edge.
- **Consequence for source defects:** T-junctions, partial collinear overlaps and unwelded
  near-seams in the source are not locked. They may open cracks at mixed depths exactly
  there. They are reported, never matched by distance (see D3).
- **Infeasible parents (replaces the `unsupported` rule above; rounds 7–8):** 8,192 stays the
  hard published cap. A parent whose exact interface, or topology plain simplification
  cannot reduce (D3), needs more triangles than its cap has its coarse tile **omitted**. It
  is never refused.
  - **Upward propagation:** every ancestor up to the root is omitted too. Its parent cannot
    be built without a hole where it was.
  - **Omitted cells:** they publish no content and hold no geometry; nothing is merged from
    them and nothing is cloned.
  - **No vanishing:** a content-less cell has only content-less ancestors, so it never sits
    between content levels (the Cesium vanishing lesson). Delivery refuses a coarse tile above
    an omitted cell.
  - **Delivery:** the omitted region is a top-connected cap of plain availability (explicit or
    implicit, an omitted root included), partitioned under the existing document caps. Each
    omitted cell takes the empty-parent error (at least 16 × its box diagonal), so it refines
    whenever it is visible; it has no content ancestor to fall back on.
  - **Jobs:** omission is decided before merging, so every job merges at most eight capped
    children.
  - **Reported:** `hlod.omittedProxies`, `omissionSources` (cells that could not fit) and
    `omittedByDepth`.
  - **Client cost:** local. One source omission costs about 8 × depth content-less cells plus
    its children's tiles in the first view.
  - **Scope:** this is the bounded last resort. D11/S1b's topology simplification and border
    scheme are to make it rare.
- **Winding:** simplified faces keep their source orientation. A face collapsed onto an
  opposite-facing source face is dropped beside its twin, or turned.


Keep today's ownership octree, packed/synthetic addresses and **8,192-triangle
leaf cap**, every source triangle exactly once, with exact primitive/triangle
provenance and picking IDs. “Exact leaves” preserves current leaf fidelity:
lossy profiles retain their documented position/normal/texture encoding error;
it does not claim bit-identical source f32 positions in quantized profiles.
Do not resimplify or reassign leaves. Their decoded seam coordinates must agree
exactly, and lossless leaf texture sampling remains exact under its admitted policy.

Build each parent from its immediate children using position-welded meshopt
simplification. Lock **only its outer interface with other ownership subtrees**;
release interior child boundaries and UV/attribute/primitive seams. This is not
blanket `LockBorder` on every open source hole. Reconstruct coarse attributes,
material-class batches and UVs after simplification. No skirts or stitching.

Retain complete interface **segments**, including junctions and all incident
nonmanifold owners, not just vertex positions. Source-edge incidence and DFS
leaf intervals identify the required interface. Protect affected incident faces
and retry a bounded number of times if simplification deletes a locked segment.
Use the leaf-compatible canonical decoded positions for interfaces through every
encoding depth; independent per-tile quantization must never move them. Decode
and check before publication. By induction each ancestor retains the same outer
segments as its descendants, so any disjoint independently refined neighbours
agree at every depth mix. The permanent census tests that construction.

8,192 is the hard published triangle cap. A profile's simplifier error target
may stop reduction earlier only if the result still fits; report the actual
stopping constraint and measured source error. Initially use the demonstrated
budget-led target; no unmeasured finite meshopt error constant becomes a public
source-error promise. An interface/error constraint that cannot fit returns
`unsupported` with tile/count/constraint diagnostics, preserving old output.
Do not ship the research fallback that can exceed the cap or silently break an
interface. Broader high-boundary topology feasibility is an explicit S1 test.

**Rationale:** outer-interface preservation permits cheap interior simplification
and crack-free REPLACE traversal without seam consensus. **Evidence:** E1–E5, C/H16.

### D2 — sampled geometric error, not certification

**Owner amendment, 2026-10-24 (exact seams, see D1).** Unchanged. Where a source defect is
left unlocked, the crack it may open at a mixed-depth frontier is a seam defect, not
geometric error. Sampled error does not measure it; the D3 counts report it.


For final decoded parent P and captured source surface S in its descendant
range, measure both directed distances with deterministic area-stratified
samples; include boundaries, component representatives and sliver controls in
addition to area samples. Let `M = max(sampled S→P, sampled P→S)` and
`G = max(M, max(child G))`. Preserve the current leaf encoding-error term.
Publish `geometricError = G` and the same value in implicit error metadata.
Record policy version, sample counts/seeds, directed maxima and distribution
summaries in the report; incomplete required sampling is a typed resource
refusal, never zero error. Sampling operates against source, not recursively
summed child estimates. Measure after meshopt/position decode, before charts
that leave positions fixed. Nonfinite results fail.

Minimum build sampling: 2,048 per direction per parent and 200,000 aggregate
per direction per occupied parent depth; deterministic tile-seeded scheduling
and bounded batches make this independent of jobs. Independent acceptance uses
a different seed and decoder, 200,000 samples/direction at cuts 0, 2, 4 and 6,
carrying terminal leaves forward. Report mean/p95/p99/max. A sampled maximum
is neither Hausdorff certification nor a guarantee against unsampled outliers;
independent exceedances are reported and investigated, not hidden by a label.

`validate` checks finite/nonnegative errors, parent ordering, explicit/implicit
consistency, hierarchy/availability, resource closure, bounds/decoded containment,
codec structure and admitted metadata/feature-ID integrity. Source-aware acceptance
checks leaf exactness, cracks and sampled error/appearance separately. The current
archive validator does **not** recompute source-distance certificates; remove
H17-specific certificate expectations from producer audits/tests/contracts, not
unrelated C2 containment checks. No universal geometric bound is claimed.

**Rationale:** source-based sampling gives useful SSE estimates without certificate
cost or a false proof claim. **Evidence:** E2, E7, E8, C/H17 and validator source.

### D3 — holes and islands

**Owner amendment, 2026-10-24 (exact seams, see D1).** Source defects are counted
combinatorially after welding by canonical key, with no distance radius, and reported in the
HLOD manifest (`hlod.incidence`):
- `sourceOpenEdges`: edge keys with exactly one incident surface face over all leaves.
- `sourceOpenEdgesWithSharedEndpoints`: those open edges whose two endpoint keys each also
  occur in a leaf other than the edge's own. These are candidate T-junctions and unwelded
  seams: their ends meet other geometry and the edge does not.

Both come from one external sort and merge join. Neither is locked.


Use plain simplification; no explicit vanished-island selection, hole filling,
reopening rules or `lod_vanished`/`lod_filled` records. Coarse holes/components
may change through ordinary simplification, assessed by source-distance and
visual controls. Leaves retain all source geometry; a missing outer interface
is always a defect, never an intentional hole.

**Rationale:** omit special topology policy and bookkeeping while keeping exact
leaf recovery and visible-quality gates. **Evidence:** E2, E3, E5, C/H17.

### D4 — representative coarse identity and annotation

**Owner amendment, 2026-10-24 (exact seams, see D1).** Unchanged. An omitted cell
publishes no content, so it carries no identity table.

**Owner amendment, 2026-10-26 (ties).** Validators accept any primitive within τ of the
minimum distance; the producer breaks ties by source order. τ = 16 · 2⁻⁵² · (|centroid| +
descendant source box diagonal), a float-rounding allowance, not a metric tolerance.


Each coarse triangle carries one `source_primitive` feature ID selected from
the nearest source primitive at its centroid in the tile's descendant source
range (deterministic distance/source-order ties). Duplicate that ID at all three
corners. It is a **representative**, not ancestry or an exact triangle mapping;
it need not preserve a row for every primitive absent from coarse geometry.
Drop coarse `proxy_source` and per-triangle source IDs. Leaves retain exact
`source_primitive` and `source_triangle` with original vertex references.

Keep H28 semantic material-class batching, one property table per ID set/model
within the consumer row limit, and appendable feature-ID sets. A later annotate
pass can map source-triangle annotations exactly to leaves. Coarse primitive
labels can be representative; arbitrary leaf regions require a later explicit
aggregation/rebake policy and cannot propagate exactly through these IDs.
Keep reusable leaf ID-texture rules (nearest, nonblending IDs, lossless/null
gutters); retaining H27's unused coarse ID baker is not annotation readiness.
Fresh coarse charts are texture-editable when decoded footprints do not overlap;
this does not make their source provenance exact.

**Rationale:** primitive picking stays useful while avoiding large false-precision
coarse provenance tables. **Evidence:** E1, E2, E7, C/H28 and mesh guide.

### D5 — fresh, content-sized coarse charts and cheap bake

Create charts per coarse tile across source UV seams, grouped only where material
semantics/attribute presence require separate delivery. Start with the prototype's
bounded planar chart growth/overlap rejection and #260's packing lessons. Bake
covered texels through bounded nearest-surface queries, interpolate source colour
in linear light, and fill gutters per chart. **Default to the captured source
surface**, with a bounded spatial index/image cache; this has the measured E2
quality and avoids accumulated child-image blur. A child pre-encode surface is
an allowed later optimization only after equivalent quality/resource checks;
never repeatedly transcode/recompress children as the source of truth.

Target chart spacing `max(G/16, 1e-5 m)` for balanced, where G is D2's uncalibrated
error. Start with the smallest fitting power-of-two dimensions, minimum 128 and
maximum **1024 per side** for balanced coarse pages; allow rectangular pages
supported by the mip packer. Page count also obeys the current 96 MiB complete
tile cap, including all PBR slots. Multiple semantic material classes may need
multiple pages. Report requested/achieved spacing, occupancy, page bytes and
cap hits; cap-limited density is an explicit quality trade-off, not a GE/16
claim. Never allocate every tile's maximum page size by default. Quality can
trial 2048 coarse pages; the 189 versus 74 CPU-s tex3 rehearsal warns against
making that the default. Final constants remain S8 decisions.

Retain all admitted PBR slots, material factors, MASK coverage, source sampler
semantics, lit/unlit attribute parity and normal-map tangent-frame conversion.
These are small per-page/semantic checks, not a per-texel source ledger. No
same-primitive facing search, donor-distance gate, terminal witness propagation
or deep drift repair. Tiny/degenerate no-raster faces must keep geometry/IDs
with explicit appearance handling. A material/normal/MASK case that cannot be
represented correctly is a typed refusal, not a silent slot drop.

**Rationale:** recharting frees geometry, source rebaking avoids cumulative blur,
and content-sized caps bound both queries and transfer. **Evidence:** E2–E4, E8, #260, C/H28.

### D6 — cropped leaf atlases

Retain/rework `leaves/{crops,atlas,source_sampler_mips}.rs` to deliver only each
leaf's needed source regions, with correct wrap, gutters, filters and per-level
mip alignment. No whole-source-image dependency merely because one triangle
uses that image. Audit the existing dedicated mipmapped-binding path: H28
records an original-source-chain exception, so “as today” is reuse of the crop
machinery, **not** permission to retain that exception. A crop covering the
whole image is legitimate only when the leaf actually needs that domain.
Keep leaf geometry, IDs, exact lossless sampling and shared/unique UV ownership.

**Rationale:** local leaf dependencies remove the measured 78.22 MB wide-view
penalty and preserve spatial streaming. **Evidence:** E2, E3, C/H28 and mesh guide.

### D7 — KTX2/mips and presets

**Owner amendment, 2026-10-24:** bring the colour default forward independently
of S8: fast uses ETC1S quality 50 (lean), balanced uses quality 90 (good), both
at effort 0 after the processing-cost sweep (initially 3), for leaf and coarse
colour pages. Quality keeps UASTC and lossless keeps PNG. Recalibrate ETC1S RGB appearance floors from fixture/tex3 decoded
measurements while retaining alpha, MASK, source-sampler and escalation checks.
See [the mesh profile contract](../MESH.md#profiles-and-resources). The integrated
2048/GPU/viewer campaign below remains a later acceptance obligation; this
amendment authorizes the default change without claiming that campaign passed.

Recommend in-process **KTX2 ETC1S for fast/balanced coarse colour pages**, and
for leaf colour pages if the same quality gate passes; **UASTC + Zstd for quality**
and for normal/data/MASK cases where ETC1S misses class-specific floors. Keep
`lossless` PNG. Retain meshopt, authored colour-correct mips, explicit colour/data
transfer, normal renormalization, MASK coverage and ID-safe lossless handling.
PNG's existing lossless sampler policy remains distinct from authored KTX2 mips.

ETC1S is a bandwidth-oriented recommendation, not an assertion that its encoder
or every GPU transcode target is fastest. Make the final default choice from
**one integrated 2048 acceptance campaign**: hold geometry/charts fixed, compare
ETC1S and UASTC page delivery without rebuilding the tree, record encode CPU,
output bytes, all three first-view dependency totals, transcode/upload and
first-render/refinement observations, and inspect the fixed views plus an
approach/orbit. Keep UASTC where visual/class gates fail; freeze the measured
combination only if the complete S8 speed/transfer gates pass. No JPEG production
path or external encoder fallback. The known Cesium sRGB upload limitation stays
documented without compensating the authored colours.

**Rationale:** GPU-ready mip chains and compact transfer reduce client work;
measure codec cost/quality rather than assuming the prototype JPEG win transfers.
**Evidence:** E2, E8, C/mesh guide, `src/basisu_ktx2.rs` and leaf delivery gates.

### D8 — declared memory and subtree streaming

Add Rust `TilerRequest.memory_budget_bytes: Option<u64>`, Python
`memory_budget_bytes=None`, CLI `--memory-budget BYTES` with documented binary
suffixes (`2GiB`). Resolve once before allocation: proposed default is the lesser
of **4 GiB and half detected host/cgroup headroom**, or 2 GiB when detection is
unavailable. An explicit positive budget is a ceiling, never silently raised;
reject zero/overflow/bool/noninteger requests consistently. `--jobs` remains a
maximum constrained by the same budget. Report requested/resolved budget,
reserved base/active leases, actual workers and stage high-water observations.

Charge source/JSON metadata, sort buffers, interface incidence, hierarchy stack,
source spatial index, decoded images, codec scratch, active children/parent,
archive/index buffers and allocator/runtime reserve together. Stream/spill
input-dependent inventories; no global resident mesh/BVH, all-node triangle
lists or depth-wide bake ledger. Build postorder, subtree at a time: at most the
admitted active child summaries/payloads and parent; spill completed sibling
payloads if needed. Release child geometry/pages/query state as soon as their
parent is complete. Source ranges still needed by ancestors remain disk-backed;
published leaf members remain staged for archive delivery, not resident RAM.

No images means **zero texture/chart/mip/codec workspace reservation**. Vertex
colour interpolation has its own small charged path; colourless input bypasses
colour search independently of source-cache residency. Admit indivisible work
before allocation; serialize or spill, then `resource_limit` if it cannot fit.
Memory pressure must not silently change geometry, texture quality or output.
Disk-backed scratch is the default recommendation; charge every tmpfs owner,
including output staging, to memory. Track simultaneous spill, retained source,
parent handoffs and archive staging, not just capture files. Keep cancellation,
cleanup and completed-file/no-clobber publication semantics.

**Rationale:** explicit ownership/admission makes streaming scale without
host-sized allocations or expensive discarded retries. **Evidence:** E6, E8, E2's
unbounded prototype limitations, C/H16.

### D9 — removal, not a second permanent path

Replace `src/mesh_large/lod/` with new `src/mesh_large/hlod/`; remove H17
certification, seam thinning/consensus, island/fill logic, old sidecars and
metadata. Replace `bake/` and `bake_pass.rs` with the simple HLOD chart/bake stage;
remove terminal per-texel packets/ledger, drift gates, facing/donor search and
H17 handoff adapters. Extract only demonstrated reusable chart, material,
filter/mip/codec primitives; do not wrap the old baker in compatibility records.
Remove obsolete options, reports, progress phases and assertions at integration.

Retire H17/H27 normative contracts together with code. In **this PR**, retain
`h17-lod-contract.md` with a superseded header; preserve historical evidence,
provenance and useful independent decoders/controls. H27's bake design and audit
README become historical, not successor requirements. Amend H16's error/handoff,
H28's coarse identity/position assumptions and `MESH.md` when S6/S7 lands. Remove
certified-bound checks in H17 acceptance; keep format/containment checks and
unrelated terrain/other producer guarantees. There is no shipping H17 mode.

**Rationale:** deleting obsolete machinery secures the processing savings and
avoids maintaining incompatible contracts. **Evidence:** E4, E6, E8, C.

### D10 — refinement calibration: useful, but gated separately

Recommend **G as the initial/default published error**, then a guarded smaller
refinement estimate only if the fixed-view/approach checks pass in S8. Crack-free
interfaces permit holding parents closer without opening cracks; silhouettes,
thin components, texture blur and popping still remain. HLOD already removes
H17's certificate slack, so do not apply #264's fourfold reduction blindly.

Trial `q = max(sampled p99 in each direction)`,
`R_raw = min(G, max(G/2, 2q))`, `R = max(R_raw, max(child R))` with leaf `R=G`.
This bounds the trial to `G/2 ≤ R ≤ G`; a silhouette/component control that fails
uses R=G for that tile and propagates ordering upward. Collect G/q with the same
D2 samples, so the trial adds no distance-query pass. Keep G for texture spacing
and diagnostic metadata; publish R in both explicit and implicit errors with a
versioned policy only if accepted. No certified C metadata is retained.

Smaller R delays refinement and can reduce requests/bytes, at the cost of larger
visible approximation and popping (up to 32 nominal pixels for G at SSE 16 in
the half-error trial, **not** a bound on unsampled error). Leave R=G if quality
fails or data are incomplete; the main HLOD acceptance must not depend on an
unproven reduction. Quarter-error exploration is post-0.4 calibration, not an
extra required bake. #264's 2.6/7.1/68.3 MB forecasts must not be quoted as HLOD
acceptance numbers.

**Rationale:** a bounded metadata-only trial can improve client streaming cheaply,
while keeping the measured geometry/appearance baseline reviewable. **Evidence:** E2, E5, E7.

### D11 — scalable borders (owner decision, 2026-10-24)

Adopt **corrected A** from `E/astra-hlod-scalable-borders-20261024/DESIGN.md`.
This governs S1b and supersedes conflicting D1–D3, D10 and numeric/slice seam
requirements below. Retain the exact-incidence and representative-identity
semantics of the unmerged [D1–D4 amendment, PR #275](https://github.com/BenDyson-Arch/rusty-tiles/pull/275)
(`feat/hlod-s1-geometry`, `055be853`); its AboveCap policy is rejected here.

**Problem:** leaf-resolution borders exceed the cap even on regular sheets.
Sol's `E/hlod-s1-review7-20261024/REVIEW.txt` estimates a 500M-triangle sheet's
interior depth-2 tile at 31.25M triangles and about 15,811 border edges/faces,
nearly twice the cap. This is arithmetic, not a bake. AboveCap compounds:
duplicated-soup probes grew 16,384 → 32,768 → 65,536 faces without border locks;
source-sized ancestors defeat bounded jobs and client payloads.

**Owner-approved policy:**

1. **Corrected A.** Shared level-wise border chains from one disk-backed interface graph: exact source incidence, all owners, stable chain IDs and source-subarc correspondence. Each chain is certified against the common leaf-decoded source arc:
   - `sup|Q_d − C| ≤ ε_d ≤ E_d/16` per side, with a predetermined nonincreasing scale ladder E_d;
   - `G ≥ max(E_d, sampled error, child G)`;
   - same-depth shared gaps are 0, and mixed-depth gaps are ≤ GE/8 for every depth difference k ≥ 1;
   - the terminal-leaf exception keeps exact arcs wherever a terminal leaf can abut a proxy;
   - source defects (unwelded seams, T-junctions) stay reported, outside the guarantee.
2. **Topology simplification at coarse levels is ALLOWED within the measured error:**
   - drop interior components smaller than the tile's G;
   - close small holes;
   - never remove anything touching a border chain;
   - S2's measured two-sided error must reflect the loss.
3. **The omit-proxy safety net,** where capped descendants become the frontier and no content-less cells sit between content levels, is the last resort only. It's used only when cap fit is still impossible, never refuses, and every use is reported in the receipt and summary.
4. **AboveCap publication is rejected,** and the 8,192 hard cap stays.

**Correctness:** depth increases toward leaves. With common source parameter s,
`|Q_d(s)−Q_(d+k)(s)| ≤ ε_d+ε_(d+k) ≤ E_d/8 ≤ G_coarse/8` for every k≥1.
GE here is the coarse proxy's published G; D10 cannot lower it without retaining
this bound against published GE. Include codec displacement; check all combined
piecewise-linear parameter breakpoints, not nearest-curve samples. All owners
consume identical decoded segments and shared live junctions at each level.
Exact terminal arcs cover zero-error leaves too. S2 remains sampled surface
error; this is a border certificate, not a Hausdorff surface guarantee.

**Cap/domain:** a 64×64 quad sheet has 8,192 triangles and 256 border edges,
independent of source density. For a connected manifold patch,
`T=2V_i+B+2b+4g−4`; a disk with B≤1,024 and V_i≤3,585 fits. These are feasible
budgets, not consequences of ε or a meshopt convergence proof. Bounded border
complexity/topology after the permitted simplifications is the cap-fit domain;
oscillations, many surviving loops, mandatory junctions and terminal locks can
still defeat it. Remesh stalled interiors before the last-resort frontier;
cap infeasibility never refuses. D8 resource admission remains applicable.
No skirts or extra seam requests; safety-net frontiers may increase ordinary
requests/bytes, as may a larger G. Client non-regression must be measured.

**Work/memory (conditional):** with n source triangles, height h, P parents,
cap C=8,192, admitted cache/sort memory M and j workers, ordered chains and
feasible patches permit `O(n log n + nh + P·F(8C))` geometry work. Balanced
shortcut certificates and streaming levels avoid quadratic rescans/owner pairs.
For h=O(log n), P=O(n), fixed C this is O(n log n); it excludes unproved arbitrary
graph extraction/remeshing and nearest-source query costs. The 8C input bound
requires capped eight-child jobs, not an unbounded promoted-frontier merge.
Spill siblings for `O(M+jC+Ch)` resident geometry, stack payloads too for a fixed
budget. Precharge O(n) incidence and up to O(nh) certificate/interface scratch,
plus source/output staging and tmpfs. At 1,700 B/merged triangle, 8C alone costs
111.4 MB/job before encode, texture and shared reserves.

**S3/S5:** chart after border construction, cut at delivery borders and share
source-side donor/filter conventions with compatible gutters/mips. Validate
decoded edge colour, normal and MASK continuity, including mixed LOD. S5 must
independently decode cap/rows, all-owner edge/junction equality, complete subarc
coverage, displacement certificates, E/G inequalities, terminal locks, bounds,
leaf identity, source-defect reports and frontier continuity; producer hashes
alone are insufficient. Audit topology-loss and safety-net counts in receipts.

## Numeric acceptance contract (proposed, not measured passes)

One source Mtri is one million **input** triangles; CPU is whole-process user+sys,
including capture, partition, leaves, source decode, HLOD, texture encoding and
archive. Independent audits are timed separately. Pin input/binary/tool hashes,
compiler, profile, CPU affinity, actual jobs and host load; use a quiet fixed
hardware lane. Wall time is recorded, not substituted for CPU by multiplying jobs.

For `mgal_detail_offset_2048.glb` (9,999,999 triangles), balanced, propose:

- **Whole conversion ≤50 CPU-s/Mtri (≤500 CPU-s); coarse stage ≤30 CPU-s/Mtri.**
  These are optimistic 20×-class production gates, tightened at the owner’s
  request; stretch goals are 30 whole / 20 coarse CPU-s/Mtri. They are not
  extrapolated prototype passes. E4’s 8.9 CPU-s/Mtri legacy crop motivates a
  low-tens target, but its different tree/codec/domain is not production proof.
  A miss remains work or an explicit reviewed budget revision.
- First-view ceilings, decimal MB including ancestors, GLBs, images, JSON and
  subtrees: **≤6.0 at 1600×1000/range 3; ≤63.366 at 2560×1600/range 3;
  ≤197.521 at 2560×1600/range 0.75**. Stretch targets 5.442/40/150 MB are
  optimization goals, not substituted pass thresholds. Run the calibrated
  simulator at SSE 16, heading −0.8, pitch −0.65, horizontal FOV 60°, range in
  bounding-sphere radii; also record actual consumer requests with pinned cache,
  server and GPU settings. Pin mismatches; a simulator is not load-time proof.
- **Declared 4 GiB cap and measured peak process RSS ≤4 GiB**, no swap, no
  uncharged resident source index. Record cgroup/page-cache peak separately.
  Gate (a)'s untextured 526,174,760-triangle run still requires **2 GiB**.
  Low-cap refusal/cleanup and successful within-cap processing are separate tests.
- **0 missing decoded interface segments; max/p99 crack width 0 m and 0×GE**
  across all independently refinable depth pairs, including nonmanifold owner
  combinations. Reproduce at least E2's 15,536 mixed / 6,859 same-depth pairs
  when ownership is identical; explain any pair-count change, never sample only
  a convenient frontier. Small fixtures also cover corners/point contacts.
- **0 missing/duplicate leaf source triangles or wrong IDs**, ≤8,192 triangles
  per leaf/parent, exact shared decoded interface positions in every profile,
  and bounds containing final decoded content. Check original corner order and
  per-profile attribute/UV/texture fidelity, including quantization-sliver controls.
- Geometry quality at the independent root: sampled mean ≤0.15 m, p95 ≤0.40 m,
  max ≤3.0 m. Root PSNR ≥20.0/18.0 dB in the two directions; report deeper cuts
  and ΔE as in E2. These rounded tolerances allow modest codec changes from
  E2's 20.89/18.85, not silent large degradation. No visible new cracks, wrong
  picks, gross chart bleeding, lost markings or objectionable silhouette/popping
  in the fixed views and approach. Preserve existing class/MASK/normal floors.

These speed/MB ceilings apply to balanced at R=G. Record quality/fast/lossless
costs separately; all share structural, identity, memory and crack obligations.
Codec/refinement alternatives must not evade the baseline gate by changing the
camera, source, leaf ownership or quality criteria. Author exact scratch bound
before each run and measure all owned directories; no scratch pass inferred from
compression or a capture-only counter. Gate (a)'s existing ≤7,200 s producer,
≤3,600 s validator, ≤2 GiB validator RSS and 30/300 s consumer loaded/refined
budgets remain pending on declared idle hardware. Its 4×-input scratch target
is unresolved: rederive the new ownership envelope before the 100M/526M run;
any replacement bound needs an explicit roadmap/budget amendment, not #262's
old 400 GB estimate copied into this design.

## Ordered implementation slices

Effort includes focused tests and independent review; agent-days are additive,
not elapsed days. S8 is the one full 2048 acceptance campaign; earlier slices
use small analytic fixtures and tex3/crop3. No implementation or run is part of
this documentation PR. The numeric contract above is **A**: its CPU, three-view
MB, memory and crack gates are shared integration criteria. Per-slice allocations
below are proposed diagnostic budgets, not measured performance or permission to
trade away A. Reserve ≤10 CPU-s/Mtri for capture/partition/leaf preparation;
S1–S4 allocate the other 40, with ≤7 of the codec allocation attributable to
coarse pages to meet the coarse total of 30. One coordinator owns sustained
builds/runs; parallel file work has explicit ownership.

### S1 — bounded HLOD geometry and interface ownership: 5–7 days

- **Scope/files:** new `src/mesh_large/hlod/{mod,geometry,interfaces,store}.rs`
  and `src/mesh_large/resources.rs`; rework `config.rs`, `full.rs`,
  `attributes.rs`, `records.rs`,
  `leaves/{producer,schedule}.rs` and `pipeline.rs` orchestration. Reuse
  `capture.rs`, `tree.rs`, `source_crs.rs`, `input.rs`, `images.rs`, the external
  sorter and leaf geometry/writer only within their established domains.
  Replace depth-wide incidence/consensus with spillable source incidence and
  postorder children/parent ownership; retain the leaf lattice's canonical
  shared decode, not H17 seam thinning. A new compact HLOD handoff carries tile
  identity, decoded bounds, representative IDs and G, with no certificates.
- **Tests:** analytic sheet, T-junction, nonmanifold shared edge, thin island,
  packed leaf and depth>21 synthetic address; all independently refined pair
  combinations; exact decoded leaves/provenance; cap failure with protected
  interfaces; a deliberately moved/dropped segment must fail. Budget equality,
  one-below refusal, untextured zero image reserve, cancellation/cleanup and
  jobs/cache/scratch determinism. Inspect source-index and descriptor growth.
- **Numbers:** geometry/interface work allocation ≤8 CPU-s/Mtri; RSS ≤declared
  cap (4 GiB 2048, 2 GiB image-free lane); 0 gaps/leaf identity errors; 8,192
  triangles hard cap. Three-view A ceilings 6.0/63.366/197.521 MB are reserved
  for S8, not asserted by an untextured intermediate.
- **Dependencies/parallel:** starts first; agree the handoff and leases before
  S2/S3/S4 integration. S5 oracle preparation and S7 adapter parsing can proceed
  on the agreed schema without editing these owned files concurrently.

### S1b — scalable shared borders: after S1, before S8

- **Scope/files:** in `hlod/`, extend `interfaces::{build,reduce,merge_rows}` with
  disk-backed all-owner chains, junctions, source-subarc maps and certificates;
  owner extrema alone cannot recover adjacency. Replace
  `geometry::{build_parent,attempt,missing}` leaf-edge locks with shared level
  boundaries, boundary retriangulation and permitted interior simplification;
  remove AboveCap and implement the reported last-resort capped frontier.
  Extend `store::{TileGeometry,CoarseTile,HlodRow}` for contracts/frontiers;
  `encode::{encode,check}` for canonical border decode; and
  `mod::{plan,job_charge,walk,memory_backed_need,admit_scratch}` for spill,
  admission and no content-less gaps. Retain exact leaves, tree addresses and
  D4 representative IDs; use S2's source queries, not the uncapped high-valence
  `representatives` scan. Integrate S3/S5 obligations in D11.
- **Tests:** 500M/billion-sheet arithmetic; oscillatory borders; >24,576 mandatory
  junctions; many sheet loops; nonmanifold/high-valence/duplicated soup; unequal
  neighbouring G; zero-error terminal leaves; every k=1…h; corner meetings;
  reversed chains; codec displacement; source T-junctions; checkerboard,
  normal/MASK borders; permitted interior loss versus protected border contact;
  forced safety-net/frontier continuity; jobs/budget determinism, tmpfs exhaustion.
- **Acceptance:** tex3/crop3 first-view bytes must not regress against the S3
  **0.41 / 0.84 / 3.68 MB** anchor with identical cameras and complete dependency
  accounting; retain the complete census and explain pair-count changes, with
  D11's zero same-depth / ≤GE/8 mixed-depth bounds. Leaves remain byte-identical
  with exact IDs. Compare whole/coarse CPU per source Mtri on the same lane,
  require non-regression and retain S1's ≤8 CPU-s/Mtri geometry allocation.
  Run `mesh-1e7` sanity with cap/census/identity/CPU/bytes/resource receipts,
  explicitly counting topology simplifications and safety-net uses (including
  zero); this is not S8 acceptance or a 500M/billion-scale pass.
- **Commitment:** this is the owner's design commitment, not measured acceptance.
  Scale gates tune constants (E ladder factors, cap, budgets), not the algorithm
  or guarantees; the approved hard cap remains 8,192. S1b integrates after S1
  with S2/S3/S5 and is required before S8. The earlier effort total excludes S1b.

### S2 — decoded two-sided error measurement: 2–3 days

- **Scope/files:** new `hlod/{error,source_index}.rs`; bounded disk-backed source
  queries/stratified reservoirs; versioned G summaries in HLOD handoff.
  Rework `delivery/{source,plan}.rs` to preserve G and monotonicity across
  explicit, implicit, external-link and synthetic records, retaining H16 bounds.
  Collect q for D10 but initially publish G.
- **Tests:** independent analytic offset surfaces, tiny distant component,
  degenerate triangles and boundary samples; seed/jobs determinism, exact
  sample counts, finite/monotone errors and leaf encoding-error floor. Different
  seed source audit reports exceedances honestly; understated/nonmonotone
  metadata and swapped explicit/implicit values are sensitive controls.
- **Numbers:** error/query allocation ≤5 CPU-s/Mtri, D2's 2,048/tile and
  200,000/depth/direction minimum; root ≤0.15/0.40/3.0 m quality gate; common
  cap/zero cracks and A's three-view ceilings apply on S8 output.
- **Dependencies/parallel:** S1 interfaces/store; runs alongside S3 after the
  source-query and error handoff is fixed. S2 owns delivery source/plan edits.

### S3 — fresh charts, cheap appearance and cropped leaves: 4–6 days

- **Scope/files:** new `hlod/{charts,bake,appearance}.rs`; extract proven small
  math/frame/packing helpers from `bake/{charts,math,frames}.rs`, not the H27
  packet interface. Reuse `batching.rs`, `leaves/{crops,atlas,planning}.rs`;
  rework `leaves/source_sampler_mips.rs` where whole images leak into leaves.
  Source-index/cache ownership follows S1/S2. Rebuild normal/data/MASK and
  vertex-colour handling against the representative coarse model.
- **Tests:** UV seams free to simplify; nonoverlapping charts and gutters;
  constant/ramp/impulse source; close parallel surfaces, normal/tangent frames,
  MASK, all PBR slots, lit/unlit parity; cropped repeating/mipmapped leaves and
  lossless source sampling; exactly preserved geometry/IDs through charting;
  image-free/colourless bypass. Verify cap-hit and achieved-density reporting.
- **Numbers:** charts/source appearance allocation ≤10 CPU-s/Mtri; balanced
  coarse page ≤1024/side, minimum fitting page, complete tile ≤96 MiB; source
  quality floors ≥20.0/18.0 dB root; 0 whole-image dependencies without a full
  footprint; common 4 GiB/0 cracks and A's MB gates.
- **Dependencies/parallel:** S1 geometry and S2 source-index/G contract; chart
  implementation can run alongside S2. S3 owns crop/atlas files; S4 owns codec
  and mip delivery files and coordinates the shared sampler boundary.

### S4 — KTX2/mip delivery and forest integration: 2–3 days

- **Scope/files:** reuse `src/basisu_ktx2{.rs,/}`, `leaves/{delivery,mips,
  sparse_mips,mask,id_texture,writer}.rs`; adapt their page interface for HLOD,
  not H27 ledgers. Integrate `delivery.rs`, `delivery/archive.rs`,
  `src/tileset_node.rs` and `src/implicit/tileset{.rs,/stream.rs}` without
  changing non-mesh producers. Keep forest consolidation (480 objects/4,096
  records/12 levels, existing root exception), availability and 3TZ indexing.
- **Tests:** independent KTX2 decode/transcode, all retained mips and colour/data
  classes, MASK/ID controls, PNG lossless; cropped mip seams; final-GLB
  geometry/feature-ID readback; explicit/implicit/external traversal, placement,
  resource hashes, member ceiling and failed-publication controls.
- **Numbers:** combined leaf/coarse codec work allocation ≤15 CPU-s/Mtri,
  archive/hierarchy ≤2; bounded leases within 4 GiB; 0 decode cracks; A's
  6.0/63.366/197.521 MB ceilings with complete dependency accounting. Coarse
  portions of S1–S4 together must also meet ≤30 CPU-s/Mtri.
- **Dependencies/parallel:** schema/interface preparation parallels S2/S3;
  integration waits for their final output. S2 owns plan/source, S4 archive/codec.

### S5 — validator and permanent independent acceptance: 2–3 days

- **Scope/files:** `src/validate.rs`, `src/validate/{schema,facts,rows,
  traversal_controls}.rs`, existing C1/I1/C2/VS tests; new
  `bench/architecture_audit/mesh_hlod/` small oracle/controls and hash manifests.
  Harvest E2's decoded interface census and strengthen it with corner contacts,
  quantized delivery, arbitrary supported scene transforms/materials and
  metadata validation. Do not call producer facts independent evidence.
- **Tests:** wrong feature IDs, missing/duplicated leaf, moved interface segment,
  incomplete availability, bad bounds, stale resource hash, negative/NaN/
  nonmonotone G and malformed codecs; all must fail their named checks.
  Valid sampled-error records pass without H17 certificates. Source-distance
  audit is separate from archive-only validation and does not certify maxima.
- **Numbers:** 0 false accepts on the named mutants, 0 identity/seam errors;
  full 2048 pair counts and A's CPU/MB/RSS are checked in S8 (validator CPU is
  reported separately). Gate (a) validator remains ≤2 GiB / ≤3,600 s.
- **Dependencies/parallel:** oracle/control design starts with S1, implementation
  follows S2/S4 schema. Can run in parallel on validator/test-owned files.

### S6 — remove H17/H27 and obsolete contracts: 1–2 days

- **Scope/files:** delete `src/mesh_large/lod/`, old `bake/`, `bake_pass.rs` and
  their dead call sites after S1–S5 replacement; rewrite `mod.rs`, `phases.rs`
  and affected producer/report tests. Retire H17/H27 normative material, update
  H16/H28 contract references. Preserve historical audit evidence and useful
  independent geometry/decoder controls; no synthetic terminal records.
- **Tests:** static reference/export/feature inventory, no hidden legacy switch;
  new pipeline end-to-end tests; existing placement, leaf, batching, archive and
  publication regressions. Do not delete unrelated certificate/terrain tests.
- **Numbers:** 0 production H17/H27 calls or coarse `proxy_source`/vanish/fill
  records; no extra CPU/RSS allowance over A, unchanged 0-crack/leaf gates and
  three-view ceilings. Deletion is not itself a performance pass.
- **Dependencies/parallel:** after successor integration and S5 controls; one
  integrator owns shared pipeline/mod/report edits to avoid racing S1/S7.

### S7 — Rust/Python/CLI and documentation: 1–2 days

- **Scope/files:** `pipeline.rs` public `TilerRequest`/diagnostics, `src/main.rs`,
  `bindings/python/src/lib.rs`, `bindings/python/README.md`, `docs/MESH.md`,
  architecture index/H16/H28, help/migration/changelog. Expose D8 once; replace
  certified/proxy-source/bake-ledger claims, update annotation limitations and
  profile/sampled-error policy. Never advertise the new knob before it exists.
- **Tests:** shared valid/invalid budget matrix, CLI/Python decoded/report parity,
  bool/overflow/zero refusal before source I/O, installed-wheel API/callback
  checks, docs/help references and no removed fields promised as current.
- **Numbers:** 4 GiB default ceiling (headroom rule in D8), explicit 2 GiB
  support for gate (a); 0 divergent defaults/identity/errors; same A CPU,
  three-view MB, memory and crack requirements through the public operation.
- **Dependencies/parallel:** draft/docs/adapter tests parallel S2–S5; final
  shared-core integration follows S6, before acceptance. This slice includes
  final-source independent documentation review.

### S8 — one 2048 acceptance campaign and preset decision: 2–3 days

- **Scope/files:** `bench/architecture_audit/mesh_hlod/README.md`, independent
  metrics scripts/manifests, `bench/scale_gates/` harness/budget documentation,
  `config.rs` final profile constants, mesh guide and roadmap receipts. Keep
  bulk runs outside git. Measure a complete public `mesh` build, not a reused
  partition. Save pre-encode pages for ETC1S/UASTC comparison under a declared
  disk budget so codec trials need no geometry rebuild.
- **Tests:** full decoded leaf and all-pair crack census; independent cut quality;
  all three fixed views and live refinement/picking/approach; every profile's
  small material/codec/memory/cancellation fixtures plus applicable final-source
  CI/wheel/Blender checks. Trial R=G/2 with q guard only after R=G passes; one
  accepted setting is frozen, failures remain explicit. No CI polling required.
- **Numbers:** all of A: ≤50 total / ≤30 coarse CPU-s/Mtri; first views
  ≤6.0/63.366/197.521 MB; ≤4 GiB RSS with declared cap; 0 gaps across the full
  15,536/6,859 pair reference census; 0 wrong leaf IDs; root error and PSNR
  floors. Record per-stage CPU, wall, scratch, mips, requests, draw calls and
  observed client timings; do not infer speed solely from compressed bytes.
- **Dependencies/parallel:** S1–S7 complete; one measurement owner serializes
  sustained work. The full 526M and 2^22-tile scale campaigns remain separate
  milestone-4 gates; 2048 acceptance does not close them.

**Mesh total: 19–29 agent-days**, including S8 and focused independent review,
excluding waiting for a quiet machine and the separate large scale/platform
campaigns. S1 → (S2 ∥ S3 with fixed query interfaces) → S4 integration →
S6/S7 final integration → S8 is the principal sequence; S5 runs alongside.
Combined with proposed non-mesh closeout below: **27–43 agent-days**. Additional
scale-gate redesign or substantive acceptance failures need separate estimates;
this is not a total remaining release estimate.

## Proposed non-mesh 0.4 bar — owner approval in this review

**This is part of the proposed decision, not an already-adopted non-mesh change.**
Adopt #267's finite release bar: retain P1 point, transactional vector, native
R2 imagery and T1/packed terrain within their established domains; use small
independent decoded fixtures, one realistic resource receipt per producer,
placement/bounds, safe lifecycle and honest frontend capabilities, with existing
suites green. Best-practice rationale: close demonstrated user-facing gaps
without paying for blanket rewrites of functioning bounded-domain producers.

- Point **1.5–2.5 days**: LAS/LAZ fields/multiplicity/IDs, current implicit forest,
  local/WGS84/UTM and bounds, lifecycle, CLI/Python parity; 10M-point workload
  ≤1 GiB RSS / ≤10 GiB scratch.
- Vector **3–5 days**: direct published-forest analytic coordinates, line coverage,
  polygon/hole/multipart topology, metadata/IDs, placement/reuse/seams, atomic
  rejection/lifecycle, CLI/Python; 100k mixed features including a 4k-vertex
  feature at two jobs ≤1 GiB RSS / ≤4 GiB scratch. Keep experimental consumer
  and aggregation identity limits explicit.
- Raster **1–2 days**: literal constant/ramp/impulse/masked-alpha PNG/COG samples,
  finest/coarse phase and XYZ bounds, lifecycle and native capability; 8192²
  RGBA workload ≤768 MiB RSS / ≤3 GiB scratch. No new Python imagery API.
- Terrain **1–2 days**: retain discrete height/lattice/seam/LOD/skirt guarantees
  (storage error ≤0.05 m), plane/bump/Float64/NoData controls, bounds, lifecycle
  and consistent missing-GTiff classification; 2048² DEM ≤512 MiB RSS / ≤2 GiB
  scratch. Native Rust/CLI only; no new projected/portable terrain or Python API.
- Shared closeout **1.5–2.5 days**: truthful supported-domain/API/validator
  claims, final candidate suites/platform/package/consumer receipts and deferred
  audit ledger; no duplicate charging of per-producer test execution.

**8–14 days total**, compared with #267's estimated 45–75-day broader audit
programme (neither is a measured productivity ratio). Resource numbers are
proposed finite-workload gates, not universal memory guarantees. Known wrong
output, data loss or destructive publication remains blocking. Defer exhaustive
oracle/component audits, hostile-input/worst-case non-mesh resource proofs and
broader supported-domain matrices to 0.4.x; preserve existing controls and
terrain's cheap implemented guarantees. Portable/projected terrain delivery
stays 0.5; Tiles 2.0/glTF 2.1 stays 0.6. After approval, reconcile #113/#120–#126
and architecture/AGENTS blanket audit wording in the non-mesh closeout slice;
this PR changes no issues or repository instructions.

## Open PR disposition (recommendations only; take no PR action here)

| PR | Disposition | What survives / why |
| --- | --- | --- |
| [#251](https://github.com/BenDyson-Arch/rusty-tiles/pull/251) bounded bake | **Harvest** | Reuse early admission, bounded image/cache and tile-read lessons in S1/S3; do not merge H27 packet/ledger architecture. Preserve CPU regressions/receipts as evidence. |
| [#260](https://github.com/BenDyson-Arch/rusty-tiles/pull/260) proxy density | **Harvest** | Tight content-sized packing, cross-source material-class charts and source-order query locality in S3; no dependency on merging #251 or old H17 geometry. |
| [#262](https://github.com/BenDyson-Arch/rusty-tiles/pull/262) scale gate (a) | **Harvest** | Keep hardware/run ladder, no-texture admission diagnosis and whole-job accounting; rederive HLOD lifetimes/scratch and remove H17/certification-specific prescriptions. |
| [#264](https://github.com/BenDyson-Arch/rusty-tiles/pull/264) refinement error | **Harvest** | Simulator/camera accounting and percentile guard into D10/S8; discard certified-C dual contract and unsupported E/4 HLOD forecasts. |
| [#265](https://github.com/BenDyson-Arch/rusty-tiles/pull/265) time history | **Merge** | Historical CPU/elapsed/provenance analysis remains useful unchanged in meaning; retain date/pipeline caveats and point forward to this decision. No implementation dependency. |
| [#266](https://github.com/BenDyson-Arch/rusty-tiles/pull/266) H17 speed experiments | **Close as evidence** | Keep pinned branch/results and useful independent decoders/controls; do not ship experimental flags or compatibility terminal records. |
| [#267](https://github.com/BenDyson-Arch/rusty-tiles/pull/267) non-mesh rescope | **Merge after owner approval** | Adopt finite bar above and reconcile its issue/roadmap proposals in shared closeout; approval of mesh direction alone does not silently approve it. |

“Harvest” means port the named ideas/tests into successor slices, then close the
old PR with evidence links when consumed; it does not mean cherry-pick the whole
branch. No PR was merged, closed, retargeted or otherwise changed by this record.
