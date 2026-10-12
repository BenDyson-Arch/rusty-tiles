# F1d3: streaming large-mesh tiler (umbrella contract)

Status: **DRAFT. NOT FROZEN.** Written by the ML0 author under the light process (one fresh
nonauthor review). The v5 precode delta review allows this text to be drafted but not frozen
until its blockers B1–B4 are folded into the design text. The design author is fixing them in
parallel (v6, in progress while this was written). Sections marked **[B1]**–**[B4]**, and the
review's must-fix items **[C1]**–**[C4]**, record the review's required text and must be
re-checked against the frozen v6 before this contract is frozen. Nothing here records acceptance.

This is the contract for 0.4.0's large textured photogrammetry tiler: `mesh-local-to-3tz` grown
into a streaming, certified, multi-level tiler. It is the umbrella over slices ML1–ML10 and over
the amendments it needs to other frozen contracts:
[C1](c1-large-mesh-ml0-amendment.md), [I1](i1-large-mesh-ml0-amendment.md),
[C2](c2-large-mesh-ml0-amendment.md) and [P](p-large-mesh-ml0-amendment.md). Its public surface
and the retirement of legacy options are in [the dispositions record](f1d3-surface-dispositions.md).

## Identities

| Item | Identity |
| --- | --- |
| Base | `origin/develop` `1b1ff9d99fe5e66b844d2c0453abdbe6ed20dd26` |
| Design (basis) | v5: `design.md` `70a1eeec…afd1`, `slices.md` `b0ddfd0c…aa7`, `open-decisions.md` `523ed2cd…eea`, `CHANGELOG.md` `5ed54eca…3f10` (preserved in `v5/`) |
| Review | v5 delta review `698b3b61…c4ef`: GO-with-changes; ML0 may start; umbrella freeze waits for B1–B4 |
| P0 evidence | design §12.1; `evidence/p0/src/main.rs` `a068cd11…`; bulk in `~/.cache/rusty-tiles-evidence/mlarge-p0-20261012/` |
| Builds on | [F1a](f1a-contract.md), [F1b](f1b-contract.md)/[F1b2](f1b2-contract.md)/[F1b3](f1b3-contract.md), [F1c1](f1c1-contract.md), [F1c2](f1c2-contract.md), [F1d2](f1d2-certificate-contract.md), [P](p-impl-layout-contract.md) |

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
| Rulings 1–7 | Re-parameterise and bake proxies; shared lattice; certify after quantization; validator sequencing; overclaims; one encoder, no ladder; slice sizing | Coordinator |
| P0 | E33 in-tree charts; E35 seam spacing; E37 cell-aligned grouping; E38 joint seam pass; E45 topology repair; E46 mixed-depth snap | ENG, pinned here (§5, §6) |

## 2. Domain

**Input.** One local static glTF 2.0 or GLB scene in metres, Y-up, with captured confined
dependencies (F1a, F1b2 snapshot semantics). F1b3 breadth: the five core PBR texture slots, UV0
and UV1, TANGENT, COLOR_0. 8-bit images only (16-bit stays Unsupported, F1b1). Textured
primitives have `0 ≤ uv ≤ 1` (D15). JSON count caps (design §2.5): nodes, meshes, primitives
≤ 65,536 each; materials, textures, samplers, images ≤ 4,096; accessors and bufferViews
≤ 262,144; buffers ≤ 4,096. Per image ≤ 2^28 pixels and ≤ 256 MiB encoded. No total triangle,
byte or tile cap other than the 3TZ ceiling (U2).

**Not in the domain** (typed Unsupported or InvalidRequest): source CRS, axes, height or offset
options (retired; see dispositions); skins, morph targets, animation (F1a); 16-bit images; UVs
outside [0, 1] on textured primitives.

**Placement.** F1c1 rigid placement: `--anchor` (WGS84 lon, lat, ellipsoidal height),
`--orientation-xyzw`, `--scene-offset`. The source stays local metres.

## 3. Profiles

| Parameter | `fast` | `balanced` (default) | `quality` | `lossless` |
| --- | --- | --- | --- | --- |
| Leaf triangles `L` | 32,768 | 16,384 | 8,192 | 8,192 |
| Proxy target `P` (hard cap `2P`) | 8,192 | 16,384 | 8,192 | 8,192 |
| Geometry | u16 on the per-depth lattice | u16 on the per-depth lattice | f32 | f32 |
| Colour (baseColor, emissive) | ETC1S q40 | ETC1S q90 | UASTC q100 + Zstd | PNG |
| data-R (occlusion) | ETC1S q90 | ETC1S q90 | UASTC q100 + Zstd | PNG |
| data-RGB (metallicRoughness), normal | UASTC q100 + Zstd | UASTC q100 + Zstd | UASTC q100 + Zstd | PNG |
| `k_prof` (px per texel at the switch) | 2 | 1 | 0.5 | 0.5 |
| Atlas page edge `A` | 2048 | 4096 | 4096 | 4096 |
| Member budget `B_tile` | 96 MiB | 96 MiB | 96 MiB | 96 MiB |

These constants are provisional until ML9 freezes them. Profiles are the only tuning surface;
there are no per-constant options (O-U3).

## 4. Output representation

- **Archive.** One `.3tz` through F0 staging and completed-file publication, written by the
  streaming archive writer (spilled central directory, sorted 3TZ index; ML3 part B, which keeps
  the full process with the archive owner).
- **Layout.** The bounded implicit forest (P), OCTREE, as amended by the P amendment: streaming
  two-phase input, bottom-up document packing, preorder root names, one `.glb` slot per tile,
  rows on every implicit tile.
- **Frames.** The root tile's `transform` is the F1c1 placement composed with a translation by
  the global lattice origin O. No other tile has a transform, so the forest rebase δ is 0
  everywhere and implicit payloads are byte copies (P-A1; O-D1 below asks the design author to
  confirm, because design §3.1 still carries `ε_rebase`).
- **Payloads.** GLB with `KHR_mesh_quantization` (fast, balanced), `EXT_meshopt_compression`
  (NONE filter; vertex codec v0, index and sequence codec v1, A-C1-3), `KHR_texture_basisu`
  required on fast/balanced/quality textures, `EXT_mesh_features` and `EXT_structural_metadata`
  for identity. Nothing outside the C2 allowlist.
- **Leaves** are non-indexed with F1c2 exact tables (D1). **Proxies** are indexed, TRIANGLES,
  with `source_primitive` identity per region.
- **Bounds.** Every tile's box contains its own decoded content and every descendant's box,
  computed after the final geometry with exact (or directed) arithmetic (A-C2-2 obligations).

## 5. Hierarchy (E37; [B2], [B3], [B4])

- **Cells.** An isotropic cube around the snapshot box, subdivided as an octree on the Morton key
  of each triangle's centroid. **[B3]** The key depth must be stated (the v6 draft: 63 bits, 21
  per axis).
- **Leaves** are maximal cells with ≤ `L` triangles and ≤ `B_tile` projected member bytes.
  Internal nodes are cells with more; their children are their non-empty octants, slot = octant.
  LOD steps are depths.
- **Octant packing [B2].** Required text from the review: adjacent *leaf* octants of one parent,
  with no internal octant between them, merge while ≤ `L` and ≤ `B_tile`; identity (parent cell,
  first octant); slot = first octant; lattice and LEAFNBR depth = parent depth + 1; incidence
  keyed by the parent cell lifted. The bound is `leaves ≤ 2N/L + 2I` and `I` is not bounded by
  `N/L`, so packing does not restore an `N`-only bound. A concrete leaf-count or mean-fill
  target on Floreat-quality is set in ML2.
- **Degenerate cells [B3].** A cell at the key depth with more than `L` triangles becomes an
  8-ary run sub-hierarchy with synthetic slots 0–7, counted in the height.
- **Height [B3].** Height > 32 is refused with a typed Unsupported before any LOD work, at the
  same point as the U2 check (P-A6).
- **Counts [B4].** The exact tile count and height come from a bounded-memory pass, not from
  v5's two-sweep histogram, which the review found unimplementable. The v6 draft builds the cell
  tree in pass 5a from the sorted `(key, projected bytes, box)` records; stage 1 refuses on a
  sound lower bound and a stage "1b" on exact counts. Subtree and document counts are bounds.

## 6. LOD per depth (F1d3)

### 6.1 Order per depth [C1]

On each depth's final keep set K and vanished set, and nothing earlier: seam fixed point →
removal of vanished regions → per-region proposal → fold cleanup, validation and relock →
`2P` check → quantize → snap → box, offset and u16 range → **topology re-validation on the
quantized indices** → phase A/B certificate → direct bound on the same final proxy. Every earlier
certificate is discarded.

### 6.2 Seams (E38, E34, E47, E35 [C4])

As design v5 §3.4, pinned:
- adjacency on welded positions after the node bake (exact f32 bits); primitive borders are chains;
- classes SAME, LEAFNBR (locked), GLOBAL, NONMAN (never decimated);
- local endpoints, the joint keep set K published by both sides, iterated to a fixed point;
  **Jacobi semantics**: K and the vanished set change only between sweeps, so the result is
  independent of `--jobs` [C2(iii)]; a stated iteration bound with a typed failure if a configured
  cap is reached (review L1);
- closed chains start at the minimum-key vertex and keep ≥ 3 vertices (subject to U4); a border
  inside one node is processed once, by the smaller primitive index;
- GLOBAL chains are decimated and then locked (E34); the source incidence flag applies to
  non-GLOBAL chains only (E47);
- **spacing [C4]:** `δ_d = d0·sqrt(T̄_d/P)` with `T̄_d` the mean source triangle count over
  *internal nodes at depth d*. P0 ran the approximation `4^k·L̄`; ML5 tests the `T̄_d` form on an
  octree, non-2.5D input;
- link veto: full vertex **and** edge link condition on both sides; a failing side adds the
  vertex to K.

### 6.3 Topology repair (E45 [C3])

Per region after meshopt: fold cleanup, validation (no new edge with > 2 triangles; output
boundary equals input boundary), relock of offending vertices with their input 1-rings, at most
16 rounds. **Give-up (review C3, adopted in this draft):** fall back to the unsimplified region,
which is valid by construction, then let the `2P` cap decide (typed Unsupported naming the node).
`InvalidState` is only for bugs. The negative test injects folds and three-triangle edges through
a test hook, so a meshopt bump cannot make it vacuous (the repository pins `meshopt = "0.6"`).

### 6.4 Lattice and mixed depths (E46 replaced [B1])

Required text from the review (the v6 draft adopts it):
- one lattice per depth for **every** tile at that depth, leaf or proxy, forced monotone from deep
  to shallow: `s_d = max(2^⌈log2(ext_d/65535)⌉, s_{d+1})`, with one origin O;
- the snap is keyed on the **vertex**: every tile at depth d that publishes a vertex incident (by
  source incidence) to a tile at a shallower depth stores it at the shallowest such lattice; this
  covers proxy–leaf, leaf–leaf and corner (SAME-neighbour) cases;
- offset, u16 range and box are computed after the snap;
- a leaf's `e` is `sqrt(3)/2 ×` the coarsest step any of its vertices was snapped to;
- f32 profiles store positions relative to the one origin O, so shared vertices have identical
  bits and need no snap;
- node translations are integer multiples of `s_d` with `|offset| < 2^24`, exact in f32. This
  needs O outside the node translation (§4 Frames; O-D1).

### 6.5 U1 vanishing [C2]

As design v5 §3.5, with the review's additions:
- regions are ranked by source area (ties by primitive index); a region whose area-share target
  rounds to 0 vanishes; if region floors still exceed `2P`, the smallest-area regions vanish in
  order until they fit; one remaining region alone over `2P` is Unsupported;
- **persistence across depths [C2(i)]:** hole edges are written into the published content's
  seam records, so a shallower depth finds them through the chain record first;
- **re-run trigger [C2(ii)]:** a vanish re-runs every node with a chain facing the vanished region;
- **cap after repair [C2(iv)]:** the `2P` check runs after the topology repair;
- identity: every proxy triangle comes from one (node, primitive) region and carries that
  primitive's `_FEATURE_ID_0`; primitives are never merged;
- the certificate bounds every child face against the **whole** proxy; the nearest-point search
  is never filtered by primitive.

### 6.6 Feature density (U4, needs the user)

Closed chains (holes and islands inside one primitive) each keep ≥ 3 vertices at every depth.
The review requires the choice to be made here, not in ML5, because it changes output semantics
as U1 does. The v6 draft raises it as USER decision U4 with options: (a) keep every closed chain
and refuse inputs whose floor exceeds `2P`; (b) islands vanish by the U1 rule per component,
holes stay; (c) islands vanish and small holes are filled by a certified fan (recommended there).
**This draft cannot freeze until the user decides.**

## 7. Certificate and error

- `e(n) ≥ d_H(C_n, S_n)` over the tile's whole surface, where `C_n` is the published geometry
  after quantization and snap, and `S_n` the source surface of the node's cell.
- **Phase A** (reserved, `Q_A` = 64 face evaluations per face, out of `W_n = 512·(|S|+|T|)`)
  bounds every face. **Phase B** (budgeted, F1d2 dyadic subdivision, `D` = 8, `Q` = 4096) refines
  faces above `τ = 1.25λ`; an unfinished face keeps its phase-A bound. No face is unbounded.
- λ is uncapped, or the open-frontier inward bound when capped. Box distances round inward.
  F1d2's interval witnesses and outward rounding are kept.
- `e(n) = min(s(n) ⊕ max_c e(c), direct(n))`; the direct bound streams the node's contiguous
  Morton range (cell-aligned nodes are one range) with a patch budget `M_patch` (default 2^22
  patches); when full, refinement stops and the current sound bounds stand.
- Leaves: §6.4. f32 profiles: `sqrt(3)/2 · ulp_f32(max |p − O|)`.
- Published `geometricError` is `e(n)`; the texel bound is reported separately (§8).

## 8. Textures and appearance

- **Leaves:** atlases keyed by (tile, image, UV set, class), real-texel gutters, no rotation,
  paged at ≤ `A`, full resolution; tiles are cut so a member stays ≤ `B_tile`. The single-triangle
  exception: lossy profiles use mip ≥ 1; `lossless` is Unsupported.
- **Proxies:** in-tree charts (E33: 60° normal cones, orthographic projection, strict-interior
  overlap check, translation-only shelf packing; per-chart density compensation and chart merging
  in ML6), then a closest-point **bake** from the children's **pre-encode** atlases for every
  present slot; normals re-expressed in the proxy's tangent space with generated, stored TANGENTs.
- **Encoding:** one encoder, vendored Basis Universal v2.50, in process, silent, single-threaded
  calls. Per-class flags and DFD as design §5.2; linear classes get their primaries patched to
  UNSPECIFIED (E42). Every page is verified after encoding (container, DFD, level count,
  transcode floor); a failure is `InvalidState`. If ML1's five-target or determinism gate fails,
  work stops for USER decision U3.
- **`lossless`:** PNG; texel-exact at leaves against the pinned decoders (`zune-jpeg` 0.5.15
  through `image` 0.25.10, `png` 0.18.1), checked by independent decode.
- **`appearance` report per tile:** displacement ≤ `e(n)` per texel; resolution
  `t(n) = max(k_prof·e(n)/16, max_c t(c))`; one bilinear resample per level (stated, not bounded).

## 9. Resources

- **Memory** ≤ `M_base + M_dec + M_sort + M_layout + M_page + jobs × max(M_node, M_bake + M_atlas
  + M_enc, B_tile)` (design §2.6), every term a configuration value or a per-item cap. tmpfs scratch
  is RAM, outside the bound, and reported (`scratch_memory_backed`).
- **Scratch preflight** is a true upper bound, not a reservation: stage 1 before the workspace
  (geometry passes), a stage after the hierarchy is exact [B4], and stage 2 after pass 5 before
  any texture byte (exact chart and page plans, texel reuse counted per tile). Refusal is
  `Io { InsufficientSpace, stage }`; after workspace creation F0 cleans up. ENOSPC stays a typed Io
  failure.
- **Per item:** members ≤ `B_tile`; leaves ≤ `L` triangles; proxies ≤ `2P` (hard); seam vertex
  incidence ≤ 16 entries (more is an endpoint).
- **Producer obligation for C1:** per payload, view bytes plus meshopt-decoded bytes ≤ 128 MiB
  (`documentDecodedBytes`).

## 10. Archive ceiling (U2)

The 3TZ index must fit a u32 local header, so an archive holds at most **178,956,970 members**
(about 1.7×10^12 balanced triangles). Over it is `Unsupported { reason: "3TZ member ceiling",
members, ceiling }`, checked from a sound count before the workspace and from exact counts once
the hierarchy is known [B4], and rechecked at stage 2. The CLI help and `docs/` state the
ceiling. No ZIP64 index header is designed.

## 11. Determinism

Output bytes are a function of the input bytes, the profile and the placement only. They do not
depend on `--jobs`, run size `R`, fan-in `F`, scratch location or timing (Jacobi seam sweeps,
total-order keys, single-threaded encoder calls). Promised per target, not across targets (D14).
Reports carry no timings.

## 12. Failures

| Kind (F0 `JobErrorKind`) | Cases |
| --- | --- |
| InvalidRequest | Unknown profile; malformed placement; retired options (dispositions) |
| InvalidInput | Malformed source (F1a/F1b2 admission); source changed during capture |
| Unsupported | Outside the domain (§2); one region alone over `2P`; `lossless` single triangle over `B_tile`; height > 32; 3TZ member ceiling; feature density under U4 option (a) |
| ResourceLimit | Per-item caps of §2 and §9 |
| Io | Including `InsufficientSpace` with its stage; ENOSPC |
| Cancelled, Conflict, ObserverFailure | As F0 |
| InvalidState | Internal breaches only: encoder post-verification failure, a P topology/record mismatch, an unreachable repair state |

Nothing invalid is ever published.

## 13. Report

The F0 result plus: the resolved profile and its constants; per-level counts; seam statistics
(edges kept, iterations, hole edges); certificate statistics (phase-A caps, phase-B exhaustion,
composed and direct per level); the `appearance` object; preflight stage fields
(`scratch_need_bytes` by stage, `output_need_bytes`, peaks, `scratch_memory_backed`); vanished
primitives per level. `conversion.json` names the profile and constants.

## 14. Acceptance (light process, per slice)

Each slice's tests are listed in `slices.md`. This umbrella adds, from the review (F1, F2), which
make the light process sufficient:
- **A reusable output auditor** (a test utility, not a validator change) that decodes the
  emitted tiles as the client does (GLB → meshopt → `u·scale + translation` in f32) for every
  tile that publishes a shared vertex, including leaves, packed leaves, snapped corners and
  leaf–leaf boundaries across depths; checks 0 unmatched same-depth seam edges, hole edges
  against the published vanished set, and a sampled brute-force lower bound ≤ `e(n)`. It runs in
  ML5, in ML6 (positions unchanged) and in ML9's scale gates and constant tuning.
- **Negative tests:** snap disabled on a fixture whose lattices really differ; certify before
  quantize or snap on a coarse-lattice fixture; a crack relabelled as a hole; a primitive-filtered
  nearest search (the 10 m floating-primitive fixture).
- **One combined certificate fixture:** a mixed-depth leaf, a packed leaf, a vanished primitive
  and a relocked region, under one brute-force check.
- **Scale rows that can fail:** ML5 gates 0 unmatched edges and 0 decoded mismatches on published
  bytes, every hole edge accounted for, the sampled lower bound, VmHWM within §9, scratch within
  the stage-1 bound and a wall-time budget; ML6 gates displacement ≤ `e(n)` on every texel and
  VmHWM within `M_bake`; ML5's run uses the production octree path; a U1 row with about 10^4
  primitives per root.
- **Validation:** every perf-lane output validates with `implicitAddressingAndAvailability` and
  `decodedContentContainment` (within caps; at scale after ML7c).

## 15. Freeze checklist

1. v6 frozen with B1–B4 folded; §5, §6.4 and §10 re-checked against it.
2. C1–C4 of the review reflected (§6.1, §6.5, §6.3, §6.2).
3. U4 decided by the user (§6.6).
4. O-D1 (origin and δ) answered.
5. The C1, I1 and C2 amendments through their full treatment (owners, oracle, mutants, precode
   review); the P amendment through its nonauthor review.
6. The dispositions record's USER questions answered.
7. One fresh nonauthor review of this text.

## 16. Open questions

- **O-D1 (design author).** Where does the lattice origin O live? The v6 draft says both "the node
  TRS carries the lattice origin and step" (§7) and "offsets … `|offset| < 2^24`, so the f32
  translation is exact" (§3.4). Both hold only if O is outside the node translation. This draft
  puts O in the root tile transform with no child transforms (δ ≡ 0). The v6 draft also says a
  forest wider than `2^24·s_d` "uses the per-forest rebase (P §4.1)". P's δ is a per-tile sum of
  child translations below an implicit root, not a per-forest shift, so that sentence needs
  restating.
- **O-D2 (design author).** Confirm the C3 fallback (unsimplified region) is adopted. The v6
  draft's E45 does.
- **U4 (user).** §6.6.
- **O-U1 to O-U4 (user):** in the dispositions record.

## 17. Proof limits

P0 is a prototype on one synthetic family (2.5D quadtree, integer x and y) and Floreat (10^6
triangles); certificates, the direct bound and baking were not run at its scale; packing and U1
vanishing were not exercised; the mixed-depth snap check was tautological (review B1). The
prototypes are f64 without interval arithmetic. Watertightness rests on the validate-and-retry
repair loop, not a proof. One host and one target (x86_64). The time-per-GB of C2 for this
output is not yet measured (A-C2-1).
