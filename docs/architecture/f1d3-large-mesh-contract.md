# F1d3: streaming large-mesh tiler (umbrella contract)

Status: **READY TO FREEZE** (ML0, light process). Design v6.1 has landed with the v6 delta
review's N1–N5 folded in, and that review says the umbrella may freeze with them. What freezes
it is the **one fresh nonauthor review** of this text. That review also confirms the two places
where this umbrella decides something v6.1 leaves silent or inconsistent: the lattice origin and
δ (§4, O-D1), and how the vanished and filled sets are published (§6.7, O-D2). Nothing here
records acceptance.

This is the contract for 0.4.0's large textured photogrammetry tiler: `mesh-local-to-3tz` grown
into a streaming, certified, multi-level tiler. It is the umbrella over slices ML1–ML10 and over
the amendments it needs to other frozen contracts:
[C1](c1-large-mesh-ml0-amendment.md), [I1](i1-large-mesh-ml0-amendment.md),
[C2](c2-large-mesh-ml0-amendment.md) and [P](p-large-mesh-ml0-amendment.md). The public surface
and the retirement of legacy options are in [the dispositions record](f1d3-surface-dispositions.md).

## Identities

| Item | Identity |
| --- | --- |
| Base | `origin/develop` `1b1ff9d99fe5e66b844d2c0453abdbe6ed20dd26` |
| Design (basis) | **v6.1**: `design.md` `0bf9fd19…441b`, `slices.md` `705bb8f7…f225`, `open-decisions.md` `bbe4f928…ccd1`, `CHANGELOG.md` `9b4de127…7569` (`/tmp/rusty-tiles-mlarge-design-20261012/`; v1–v6 preserved there) |
| Reviews | v5 delta `698b3b61…c4ef` (B1–B4, C1–C4, F1, F2); v6 delta `3f2c0583…ae6d` (GO-with-changes; freeze after N1–N5, now in v6.1) |
| Evidence | P0 (design §12.1, `evidence/p0/`); p0b published-bytes snap audit (§12.2, `evidence/p0b/`); bulk in `~/.cache/rusty-tiles-evidence/` |
| Builds on | [F1a](f1a-contract.md), [F1b](f1b-contract.md), [F1b2](f1b2-contract.md), [F1b3](f1b3-contract.md), [F1c1](f1c1-contract.md), [F1c2](f1c2-contract.md), [F1d2](f1d2-certificate-contract.md), [P](p-impl-layout-contract.md) |

## 1. Decisions in force

| Ref | Decision | Owner |
| --- | --- | --- |
| Brief | 0.4.0 waits for this tiler; F1d3; textured proxies; KTX2/Basis in process; meshopt kept; profiles; local coordinates with `--anchor`; **no georeferenced (CRS) input** | USER |
| D1 | Leaves keep exact F1c2 per-triangle provenance; proxies carry primitive-level identity; tables bounded per tile | USER |
| D2 | Every present PBR slot is atlased and encoded; absent slots trimmed | USER |
| D3b | `lossless` uses PNG through core glTF, the only exception to KTX2-only | USER |
| D4, D5 | Name `mesh-local-to-3tz`; default profile `balanced` | USER |
| D12 | Unbounded input; streaming; memory bounded by configuration | USER |
| V3 | `--scratch-dir`, default the system temp directory; refuse before starting when free space is below the computed need | USER |
| V1/V2 (a)–(d) | Validator memory independent of size; linear work, no refusal for size; absolute volume caps opt-in; no producer tile-count ceiling | USER |
| U1 | Small primitives vanish from coarse tiles; present primitives keep exact identity; the certificate accounts for removed geometry | USER |
| U2 | Accept and document the 3TZ member ceiling (178,956,970); refuse early with a typed error | USER |
| **U4 = (c)** | At coarse depths small islands vanish and small holes are filled, both certified; holes reopen at finer depths; **the output publishes the vanished and filled sets** | USER (2026-10-12) |
| Rulings 1–7 | Re-parameterise and bake proxies; shared lattice; certify after quantization; validator sequencing; overclaims; one encoder, no ladder; slice sizing | Coordinator |
| Pinned selections | E33 in-tree charts; E35 spacing `δ_d`; E37 cell-aligned grouping; E38 joint seam pass; E45 repair with unsimplified fallback; E46 monotone per-depth lattice with vertex-keyed snap; E48 octant packing; E49 pass-5a cell tree; E50 pass-5b order | ENG, pinned here |

## 2. Domain

**Input.** One local static glTF 2.0 or GLB scene in metres, Y-up, with captured confined
dependencies (F1a, F1b2 snapshot semantics). F1b3 breadth: the five core PBR texture slots, UV0
and UV1, TANGENT, COLOR_0. 8-bit images only (16-bit stays Unsupported, F1b1). Textured
primitives have `0 ≤ uv ≤ 1` (D15). JSON count caps: nodes, meshes, primitives ≤ 65,536 each;
materials, textures, samplers, images ≤ 4,096; accessors and bufferViews ≤ 262,144; buffers
≤ 4,096. Per image ≤ 2^28 pixels and ≤ 256 MiB encoded. No total triangle, byte or tile cap
other than the 3TZ ceiling (U2) and the height cap (§5).

**Not in the domain** (typed Unsupported or InvalidRequest): source CRS, axes, height or offset
options (retired); skins, morph targets, animation (F1a); 16-bit images; UVs outside [0, 1] on
textured primitives.

**Placement.** F1c1 rigid placement: `--anchor` (WGS84 lon, lat, ellipsoidal height),
`--orientation-xyzw`, `--scene-offset`. The source stays in local metres.

## 3. Profiles

| Parameter | `fast` | `balanced` (default) | `quality` | `lossless` |
| --- | --- | --- | --- | --- |
| Leaf triangles `L` | 32,768 | 16,384 | 8,192 | 8,192 |
| Proxy target `P` (hard cap `2P`) | 8,192 | 16,384 | 8,192 | 8,192 |
| Geometry | u16 on the per-depth lattice | u16 on the per-depth lattice | f32, one origin per forest | f32, one origin per forest |
| Colour (baseColor, emissive) | ETC1S q40 | ETC1S q90 | UASTC q100 + Zstd | PNG |
| data-R (occlusion) | ETC1S q90 | ETC1S q90 | UASTC q100 + Zstd | PNG |
| data-RGB (metallicRoughness), normal | UASTC q100 + Zstd | UASTC q100 + Zstd | UASTC q100 + Zstd | PNG |
| `k_prof` (px per texel at the switch) | 2 | 1 | 0.5 | 0.5 |
| Atlas page edge `A` | 2048 | 4096 | 4096 | 4096 |
| Member budget `B_tile` | 96 MiB | 96 MiB | 96 MiB | 96 MiB |

The constants are provisional until ML9 freezes them. Profiles are the only tuning surface.

## 4. Output representation

- **Archive.** One `.3tz` through F0 staging and completed-file publication, written by the
  streaming archive writer (spilled central directory, sorted 3TZ index; ML3 part B, which keeps
  the full process with the archive owner).
- **Layout.** The bounded implicit forest (P), OCTREE, as amended by the P amendment: two-phase
  streaming input, bottom-up document packing, preorder root names, one `.glb` slot per tile, a
  row on every implicit tile.
- **Frames and lattice origin (decided here; O-D1).** The global lattice origin `O` (the
  snapshot box minimum) is carried in the root tile's `transform`, composed with the F1c1
  placement. Below the root, translation-only transforms appear **only on explicit tiles and
  implicit roots**, and each is an integer multiple of the coarsest step `s_0` (so of every
  `s_d`), chosen so that every payload offset below it satisfies `|offset| < 2^24`. Implicit
  tiles below an implicit root carry none, so P's rebase δ is 0 for every tile, implicit payloads
  are byte copies, and `ε_rebase` = 0. Payload node TRS: `scale` = `s_d` and `translation` = an
  integer multiple of `s_d` with `|offset| < 2^24` (u16 profiles), or the identity with positions
  relative to the tile frame's lattice-aligned origin (f32 profiles). Every decoded position is
  then an exact dyadic sum, and a shared vertex has the same exact value in every tile (§6.4).
  A forest whose offsets stay below 2^24 at every depth needs only the root transform; with
  `s_d ≈ ext_d/65535`, offsets reach 2^24 near depth 8, and deeper forests use the extra
  transforms. Nothing is refused for width. This replaces
  v6.1's "the node TRS carries the lattice origin" (§7) and "a forest wider than `2^24·s_d` uses
  the per-forest rebase" (§3.4): P's δ is a per-tile sum of child translations below an implicit
  root, not a per-forest shift, so the shift must be a tile transform above the payload.
- **Payloads.** GLB with `KHR_mesh_quantization` (fast, balanced), `EXT_meshopt_compression`
  (NONE filter; vertex codec v0, index and sequence codec v1, A-C1-3), `KHR_texture_basisu`
  required on fast, balanced and quality textures, `EXT_mesh_features` and
  `EXT_structural_metadata` for identity and for the published sets (§6.7). Nothing outside the
  C2 allowlist.
- **Leaves** are non-indexed with F1c2 exact tables (D1). Degenerate leaf triangles created by
  quantization are **kept** (zero area; each provenance row keeps its triangle). **Proxies** are
  indexed, TRIANGLES, with `source_primitive` identity per region.
- **Bounds.** Every tile's box contains its own decoded content and every descendant's box. It is
  computed after the final geometry (after the snap) with exact or directed arithmetic (A-C2-3).

## 5. Hierarchy (E37, E48, E49)

- **Cells.** An isotropic cube around the snapshot box, subdivided as an octree on a **63-bit
  Morton key (21 bits per axis)** of each triangle's centroid. Cell depth 0–21.
- **Cell tree (pass 5a, E49).** One streaming pass over the Morton-sorted `(key, projected bytes,
  box)` records with a stack of at most 22 open cells (`M_tree` ≈ 13 KiB) decides leaves, internal
  nodes, packing and run sub-hierarchies, and gives the exact tile count, the height and the
  per-depth tile boxes.
- **Projected bytes** per triangle: geometry bytes plus, per present slot, its UV footprint in
  texels at the leaf texel size × the class's worst bytes per texel × the packing factor
  **φ = 4**; overlapping UVs counted twice. This is a factor, not a guarantee (N5). If a leaf's
  actual pages in pass 6a exceed its share of `B_tile`, the leaf is **never split**: lossy
  profiles drop one mip level for that leaf's pages until they fit (reported in `appearance`
  as a texel size above `t(n)`; geometry and `e` unchanged); `lossless` is a typed Unsupported
  naming the tile. The pass-5a tile count is therefore exact and final.
- **Leaves** are candidate cells (≤ `L` triangles and ≤ `B_tile` projected bytes) whose parent is
  internal, or packed runs of them. **Internal nodes** are cells over either limit; children are
  non-empty octants or packed runs, at most 8, slot = octant. LOD steps are depths.
- **Octant packing (E48).** Under one internal parent, consecutive **leaf** octants merge while
  the sum stays ≤ `L` and ≤ `B_tile`; an internal octant ends a run; empty octants do not break
  one. A run is one contiguous range of Morton ranks. Identity `(parent cell, first octant)`;
  slot = first octant; depth (lattice, LOD step, LEAFNBR) = parent depth + 1; box = union of its
  triangles' boxes, inside the parent cell but not the slot's octant; at depth parent + 1 it is
  **one** region per primitive for the ≥ 3-regions rule. Count bound per parent
  `2c/L + m + 1` (m internal children), summed `leaves ≤ 2N/L + 2I`; `I` is not bounded by
  `N/L`. ML2 target: Floreat quality ≤ 200 leaves.
- **Degenerate cells (N4).** A depth-21 cell still over `L` or `B_tile` becomes an 8-ary run
  sub-hierarchy of Morton-rank runs of at most `L`, synthetic slots 0–7, lattice depth
  21 + level. Identity `(degenerate cell, run path)`; parent by dropping the last slot. Every
  SAME, K, V and F key and the ≥ 3-regions incidence use this identity, so siblings never share
  one.
- **Height.** ≤ 21 + 1 + ⌈log8(N_cell/L)⌉. Over 32 is a typed Unsupported ("hierarchy height
  h > 32", naming the outlier box) at **stage 1b**, after pass 5a and before any LOD work.

## 6. LOD per depth (F1d3)

### 6.1 Order (E50)

Per depth, certification runs once, on the final state:
1. **Seam pass** (contraction only) to a fixed point; K and V change only between sweeps
   (Jacobi), so the result is independent of `--jobs`. A sweep re-runs the nodes that had a newly
   kept vertex as a removable chain vertex and every node facing a newly vanished region or
   component. Cap: 64 sweeps, then a typed **Unsupported** naming the depth.
2. Per node with the final K and V: chain contraction → removal of vanished regions and islands
   → hole fills (§6.6) → meshopt proposal per region → fold cleanup, validation and relock
   (fallback: the unsimplified region) → **2P check** (typed Unsupported naming the node).
3. **Quantize** on the depth lattice with the vertex-keyed snap → box, offset and u16 range from
   the snapped codes → **post-quantization validation** (proxies: weld identical codes, drop
   degenerate triangles, cancel or deduplicate coincident pairs, no edge with more than 2
   triangles unless the source had it). **Fallback (N3):** lock the offending triangles'
   vertices and input 1-rings at their pre-quantization positions, re-simplify through the
   repair loop and re-quantize, at most 4 rounds; then quantize the node's unsimplified regions
   (the children's content, already valid on finer lattices); only then a typed Unsupported
   naming the node. Leaves (pass 7) are quantized with the snap and keep degenerate triangles.
4. **Certificate** (§7) on the quantized published proxy against the children's quantized
   published content, then the **direct bound** on the same proxy. Earlier bounds are discarded.

### 6.2 Seams (E38, E34, E47, E35)

As design v6.1 §3.4, pinned:
- adjacency on welded positions after the node bake (exact f32 bits); primitive borders are
  chains; capture drops degenerate source triangles;
- classes SAME, LEAFNBR (locked), GLOBAL, NONMAN (never decimated), and **HOLE** (facing a
  vanished region or component: locked, treated as GLOBAL for chains, persisted in the seam
  records with the vanished id so that shallower depths find it through the chain record);
- local endpoints, and the joint keep set K published by both sides;
- closed chains start at the minimum-key vertex and keep ≥ 3 vertices unless U4 removes them;
  a border inside one node is processed once, by the smaller primitive index;
- GLOBAL chains decimated then locked (E34); the source incidence flag applies to non-GLOBAL
  chains only (E47);
- **spacing** `δ_d = d0·sqrt(T̄_d/P)`, `T̄_d` = mean source triangles over internal nodes at depth
  d (exact from the cell tree); retention by endpoint, hash level or the gap rule; digon rule;
  collapse onto the nearer kept neighbour;
- **link veto:** full vertex and edge link condition on both sides; a failing side adds the
  vertex to K.

### 6.3 Topology repair (E45)

Per region after meshopt: fold cleanup; validation (no new edge with > 2 triangles, output
boundary equals input boundary); relock of offending vertices with their input 1-rings; at most
16 rounds, or until no new lock is possible; then the **unsimplified region**, valid by
construction; then the 2P check. `InvalidState` only for internal bugs. Folds and three-triangle
edges are injected by a test hook in the negative test.

### 6.4 Per-depth lattice and the vertex-keyed snap (E46, N1)

- **Lattice (u16 profiles).** One step `s_d` for **every** tile at depth d, leaf or proxy, one
  origin `O`: `s_d = max(2^⌈log2(ext_d/65535)⌉, s_{d+1})` from the deepest depth up, `ext_d` the
  largest tile extent at depth d after snap growth (`box ⊕ s_ℓmin(t)/2` per side), computed at
  the end of pass 5a; a depth that still fails the u16 range doubles its step and monotonicity is
  re-imposed. Steps are powers of two and monotone, so each depth's lattice nests in every
  coarser one.
- **ℓ(v) (N1).** The shallowest depth of any leaf containing v: the **exact minimum over all
  incident triangles**, computed in pass 5a by a sort of `(welded vertex id, leaf depth)` per
  triangle corner reduced by a streaming minimum, then joined back by vertex id. Never from the
  16-entry incidence list.
- **Snap.** Every tile at depth `d ≥ ℓ(v)` that publishes v, leaf or proxy, stores it at
  `q(v, s_ℓ(v))`; tiles at `d < ℓ(v)` quantize v normally. Tiles that can be shown together at one
  level (proxies at depth d with leaves at depth ≤ d) therefore store the same lattice point for
  every shared vertex.
- **f32 profiles** store positions relative to their tile frame's lattice-aligned origin (§4);
  shared vertices have identical exact values; no snap.
- Box, offset and u16 range are computed **after** the snap.

### 6.5 U1: vanishing primitives

Regions (node, primitive) are ranked by source area (ties by primitive index). A region whose
area-share target rounds to 0 vanishes; if region floors still exceed `2P`, the smallest-area
regions vanish in order until they fit; one remaining region alone over `2P` is Unsupported.
Every proxy triangle comes from one region and carries that primitive's `_FEATURE_ID_0`;
primitives are never merged. The certificate bounds every child face against the **whole** proxy;
the nearest-point search is never filtered by primitive. A vanished primitive cannot be picked at
that level.

### 6.6 U4 (c): islands vanish, small holes are filled

Per node at depth d, after U1 removal and before meshopt:
- **Island:** a connected component of a region lying wholly inside the node whose area-share
  target rounds to 0. It vanishes by the U1 rule applied per component (ranked by area, ties by
  minimum vertex key). **Component id** `(node, primitive, minimum welded vertex key)`; V
  records components (a whole vanished region is the component set of its primitive). An island
  bordering another primitive's region in the same node makes that region's facing edges HOLE
  edges keyed by the component id; those chain vertices stay locked.
- **Small hole:** a closed GLOBAL chain wholly inside one region of the node whose decimated arc
  length is `< δ_d`. It is filled by a fan from its minimum-key kept vertex over its kept
  vertices; the fill triangles belong to the enclosing region and carry its `_FEATURE_ID_0`.
  **Fill check (N2b):** the fan is added only if no fan edge already exists in the region outside
  the hole, no fan triangle is degenerate or duplicates an existing one, and every chain edge
  then has exactly 2 triangles; otherwise the hole is not filled. **Unlock:** a chain vertex is
  unlocked for meshopt only if it is not in K, not an endpoint and not on another chain.
- Holes touching a node border are open chains and are never filled; seams are unaffected.
- **Certified:** fill faces are faces of the proxy and are bounded against the children by phase
  A/B and by the direct bound's proxy→source direction, so `e(n)` grows by about the hole radius;
  removed islands are bounded in the children→proxy direction, as in U1.
- **Reopen:** deeper tiles keep their own holes. The cross-level bound `e(n_k) + e(n_{k+1})`
  covers the transition.

### 6.7 Published vanished and filled sets (decided here; O-D2)

U4 (c) requires the output to publish what each proxy removed or filled. Design v6.1 names only
"the sidecar", which §2.7 counts as scratch. This umbrella pins the published form:

- **Where.** Inside each proxy's own GLB, as one `EXT_structural_metadata` property table
  `rustyLodChanges` in the payload's schema. No new archive member (an unreferenced member is C1
  InvalidInput), no tile or content `metadata` in tileset JSON (that would exclude the archive
  from C2 as `tileMetadata`), no new glTF extension, no extras. Leaves have no table.
- **Rows**, one per record of that node at its depth, sorted by `(kind, primitive, key)`:
  - `kind` (UINT8 enum): `VANISHED_REGION`, `VANISHED_ISLAND`, `FILLED_HOLE`;
  - `primitive` (UINT32): the source primitive id used by the tile's identity tables;
  - `key` (UINT64): the minimum welded vertex key of the island or hole chain; 0 for a whole
    region.
- **Complete per tile.** A tile's table lists every record of its own node, so a reader needs
  only that tile. A proxy with no record has no table.
- **Bounded.** Rows per tile ≤ regions + closed chains of the node, inside `B_tile` and
  `accessor_elements`; no JSON value node per row (binary property table).
- **Consumers.** The output auditor accepts a missing GLOBAL boundary at depth d only for a
  `FILLED_HOLE` row of that tile, and a HOLE edge only for a `VANISHED_*` row. `validate` gives
  the table no semantic claim (`metadataSemantics` stays in `notInspected`); C2's treatment is in
  A-C2-4.
- **Report.** The F0 result counts vanished regions, vanished islands and filled holes per depth.

## 7. Certificate and error

- `e(n) ≥ d_H(C_n, S_n)` over the tile's whole surface, where `C_n` is the published geometry
  after quantization and snap (and after fills and removals), and `S_n` the source surface of the
  node.
- **Phase A** (reserved, `Q_A` = 64 face evaluations per face out of `W_n = 512·(|S|+|T|)`)
  bounds every face. **Phase B** (budgeted, F1d2 dyadic subdivision, `D` = 8, `Q` = 4096)
  refines faces above `τ = 1.25λ`; an unfinished face keeps its phase-A bound. No face is
  unbounded. λ is uncapped, or the open-frontier inward bound. Box distances round inward;
  F1d2's interval witnesses and outward rounding are kept.
- `e(n) = min(s(n) ⊕ max_c e(c), direct(n))`. The direct bound streams the node's contiguous
  Morton range with a patch budget `M_patch` (default 2^22 patches); when full, the current
  sound bounds stand.
- **Leaves:** `e = sqrt(3)/2 · max_v s_ℓ(v)` (u16 profiles); `sqrt(3)/2 · ulp_f32(max |p − O_frame|)`
  (f32 profiles, `O_frame` the tile frame's origin). `ε_rebase` = 0 (§4).
- Published `geometricError` is `e(n)`. The texel bound is reported separately (§8).

## 8. Textures and appearance

- **Leaves:** atlases keyed by (tile, image, UV set, class), real-texel gutters, no rotation,
  paged at ≤ `A`, full resolution except the N5 mip drop (§5). Single-triangle exception: lossy
  profiles use mip ≥ 1; `lossless` is Unsupported.
- **Proxies:** in-tree charts (E33: 60° normal cones, orthographic projection, strict-interior
  overlap check, translation-only shelf packing; per-chart density compensation and chart
  merging in ML6), then a closest-point **bake** from the children's **pre-encode** atlases for
  every present slot; normals re-expressed in the proxy's tangent space with generated, stored
  TANGENTs.
- **Encoding:** vendored Basis Universal v2.50, in process, silent, single-threaded calls; per-
  class flags and DFD; linear classes get primaries patched to UNSPECIFIED (E42); every page is
  verified after encoding (container, DFD, level count, transcode floor), and failure is
  `InvalidState`. If ML1's five-target or determinism gate fails, work stops for USER decision U3.
- **`lossless`:** PNG; texel-exact at leaves against the pinned decoders (`zune-jpeg` 0.5.15
  through `image` 0.25.10, `png` 0.18.1), by independent decode.
- **`appearance` per tile:** displacement ≤ `e(n)` per texel; resolution
  `t(n) = max(k_prof·e(n)/16, max_c t(c))`, or larger where the N5 mip drop fired; one bilinear
  resample per level (stated, not bounded).

## 9. Resources

- **Memory** ≤ `M_base + M_dec + M_sort + M_layout + M_page + M_tree + M_K + jobs ×
  max(M_node, M_bake + M_atlas + M_enc, B_tile)` (design §2.6), every term a configuration value
  or a per-item cap. tmpfs scratch is RAM, outside the bound, and reported.
- **Scratch preflight**, a true upper bound and not a reservation: **stage 1** before the
  workspace (geometry passes, worst-case counts); **stage 1b** after pass 5a (exact counts);
  **stage 2** after pass 5b, before any texture byte (exact chart and page plans; texel reuse
  counted per tile). Refusal is `Io { InsufficientSpace, stage }`; after workspace creation F0
  cleans up. ENOSPC stays a typed Io failure. With δ ≡ 0 and final names after pass 5a (P-A1,
  P-A2), stage 2 needs no term for a second copy of an implicit subtree's payloads.
- **Per item:** members ≤ `B_tile`; leaves ≤ `L`; proxies ≤ `2P`; seam vertex incidence ≤ 16
  entries (more is an endpoint; ℓ(v) is computed separately, §6.4).
- **Producer obligation for C1:** per payload, view bytes plus meshopt-decoded bytes ≤ 128 MiB.

## 10. Archive ceiling (U2)

At most **178,956,970 members** (the 3TZ index must fit a u32 local header; about 1.7×10^12
balanced triangles). **Stage 1** refuses only on the sound lower bound `⌈N/L⌉ + 1` tiles
+ 1 subtree + 1 document + 3; **stage 1b** refuses on the exact tile count from pass 5a plus
bounds for subtrees and documents. Both give `Unsupported { reason: "3TZ member ceiling",
members, ceiling }`. The CLI help and `docs/` state the ceiling. No ZIP64 index header.

## 11. Determinism

Output bytes are a function of the input bytes, the profile and the placement only. They do not
depend on `--jobs`, run size `R`, fan-in `F`, scratch location or timing. Promised per target, not
across targets (D14). Reports carry no timings.

## 12. Failures

| Kind (F0 `JobErrorKind`) | Cases |
| --- | --- |
| InvalidRequest | Unknown profile; malformed placement; retired options (dispositions) |
| InvalidInput | Malformed source (F1a/F1b2 admission); source changed during capture |
| Unsupported | Outside the domain (§2); height > 32; 3TZ member ceiling; one region alone over `2P`; a node over `2P` after repair; post-quantization validation after its fallback; seam sweep cap; `lossless` single triangle or leaf pages over `B_tile` |
| ResourceLimit | Per-item caps of §2 and §9 |
| Io | Including `InsufficientSpace` with its stage; ENOSPC |
| Cancelled, Conflict, ObserverFailure | As F0 |
| InvalidState | Internal breaches only: encoder post-verification failure, a P topology/record mismatch, an unreachable repair state |

Nothing invalid is ever published.

## 13. Report

The F0 result plus: the resolved profile and its constants; per-depth counts; seam statistics
(edges kept, sweeps, hole edges); post-quantization fallback rounds; certificate statistics
(phase-A caps, phase-B exhaustion, composed and direct per depth); vanished regions, vanished
islands and filled holes per depth (§6.7); the `appearance` object, including any N5 mip drops;
preflight stage fields (`scratch_need_bytes` by stage, `output_need_bytes`, peaks,
`scratch_memory_backed`). `conversion.json` names the profile and constants.

## 14. Acceptance (light process, per slice)

Each slice's tests are in `slices.md` v6.1. This umbrella relies on these in particular:
- **The output auditor** (design §12): reads production archives (GLB, `KHR_mesh_quantization`,
  meshopt, with δ(t) applied, in client order) and checks bit-identical decoded shared vertices
  for every set of tiles shown together (packed leaves, snapped corners, leaf–leaf across
  depths, f32 path); every boundary edge without a SAME partner is GLOBAL, NONMAN, LEAFNBR, a HOLE
  edge with a `VANISHED_*` row, or a missing boundary with a `FILLED_HOLE` row; no degenerate
  proxy triangle and no edge with more than 2 triangles unless the source had it (degenerate leaf
  triangles counted, not failed); ℓ(v) recomputed from the emitted leaves equals the production
  value; a sampled brute-force lower bound ≤ `e(n)`; proxy ≤ 2P, member ≤ `B_tile`, height ≤ 32.
  It runs in ML5, ML6 and ML9.
- **Negative tests:** veto off; repair off with injected folds; snap off on really different
  lattices; certify before quantize or snap; a crack relabelled as a hole; a primitive-filtered
  nearest search; an ℓ(v) taken from the capped incidence list (ML2 fixture beyond 16 entries);
  a fill that fails the N2b check is not added.
- **Combined certificate fixture:** a mixed-depth leaf, a packed leaf, a vanished primitive, a
  vanished island, a filled hole and a relocked region, under one brute-force check.
- **Pass/fail scale gates** (design §13): the spilled 5.26×10^8 synthetic on the production
  octree path; the U1/U4 input with 10^4 primitives and 10^4 holes and islands per root tile,
  every proxy ≤ 2P and every missing GLOBAL boundary accounted for in F; the ML6 bake gate; the
  N3 post-quantization gate on the steep synthetic and Floreat (0 Unsupported from that path).
- **Validation:** every perf-lane output validates with `implicitAddressingAndAvailability` and
  `decodedContentContainment` (within caps; at scale after ML7c).

## 15. Freeze checklist

1. Design v6.1 landed with N1–N5 (done; §5, §6.1, §6.4, §6.6 fold them in).
2. U4 decided by the user: (c) (done; §6.6, §6.7).
3. The fresh nonauthor review of this text, confirming O-D1 (§4) and O-D2 (§6.7).
4. Not gating this freeze: the C1, I1 and C2 amendments go through their own full treatment, and
   the P amendment through its own review, before the slices that implement them.

## 16. Open questions

- **O-D1 (design author; confirmed at the review).** Origin `O` in the root transform;
  lattice-aligned translations only on explicit tiles and implicit roots; δ ≡ 0 and
  `ε_rebase` = 0 (§4). The auditor's client-order decode must then apply tile translations as
  the client does (in f64, before the f32 vertex stage) when it compares shared vertices across
  implicit roots.
- **O-D2 (design author; confirmed at the review).** The `rustyLodChanges` property table as the
  published form of V and F (§6.7).
- Public-surface questions for the user (O-U1 to O-U3) are in the dispositions record. They do
  not change this contract's algorithms.

## 17. Proof limits

P0 is a prototype on one synthetic family (2.5D quadtree, integer x and y) and Floreat (10^6
triangles); certificates, the direct bound and baking were not run at its scale; packing and
U1/U4 were not exercised there. The p0b audit tested the snap rule on modelled bytes with an
oracle ℓ(v) and packing off (v6 review limits i–v); the auditor closes these in ML5. The
prototypes are f64 without interval arithmetic. Watertightness rests on the validate-and-retry
repair loop, not a proof. One host and one target (x86_64). C2 time per GB for this output is not
yet measured (A-C2-1).
