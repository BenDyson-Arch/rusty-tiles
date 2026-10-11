# C2 producer frame inventory

**v2.** This revision follows the precode review in
`/tmp/rusty-tiles-c2-precode-review-20261011/review.md`. v1 is preserved in `v1/`.

This inventory is built from static source reads. Every cited file is identical at `70d184f`
(the v1 read) and at `e83e880`, the final head of PR #155. The pins are in
[verify/source-pins-e83e880.txt](verify/source-pins-e83e880.txt). No producer was executed, so every verdict
below is a static prediction. Each one becomes evidence only after the independent oracle
runs on actual outputs (contract, acceptance step 4).

How each citation was checked:

- **[V]** means this lane read the cited lines itself.
- **[A]** means a read-only search subagent reported it and this lane did not reread it.

Terms used throughout:

- **U** is the 3D Tiles runtime y-up→z-up rotation, U(x,y,z) = (x, −z, y).
- **Exact strict** means the contract's zero-tolerance exact predicate.
- **Prior proposal** means the earlier preparation's interval/window/implicit-Unsupported
  policy.

## Summary

| Producer (CLI) | Content | Axis | Default tiling | Exact strict (explicit) | Default output under the prior proposal |
| --- | --- | --- | --- | --- | --- |
| Point cloud P1 (`point-cloud`) | GLB POINTS, one node, FLOAT | y-up, then U | **implicit** (octree) | Passes, except for a rare 1-ulp f64 risk when `rounding = 0` | **Not covered.** Implicit volumes are metadata overrides, which the prior proposal makes Unsupported |
| Vector (`vector`) | GLB points and lines; b3dm `BATCH_LENGTH:0` fills; optional u16 quantization and meshopt | y-up, then U | **implicit** (quadtree) | Likely passes, with the same 1-ulp caveat | **Not covered.** Same reason; the derived cells also do not contain the content |
| Terrain T1 (`terrain`) | GLB TRIANGLES, one node | y-up, then U | explicit directory | Passes | Not reachable: `inspect` rejects directories (`validate.rs:752-755`) [V] |
| F1c2/F1d mesh (`mesh`, `mesh_to_archive`) | GLB, one node, FLOAT | y-up, then U | explicit | Passes | Passes (outward `next_up`) |
| W1 model wrap (`model_to_archive`) | Unchanged user GLB/glTF | source y-up, then U | explicit | Passes (bounds enclose both quaternion readings) | v1 proposal: **could be refused** (windows narrower than W1's). v2: covered. The window is 2^-18, which admits everything W1 admits. Out-of-window input is not inspected, never refused |
| Legacy `mesh-to-3tz` HLOD | GLB with meshopt; optional texture extensions | y-up, then U | explicit or implicit | **Can fail.** Midpoint rounding with no outward step; implied cells | Same failures |
| Legacy small-model wrap (`tileset.rs`) | Unchanged user model | source | explicit or implicit (single tile) | **Can fail.** f32 corner transforms and no padding | Same failures |
| `convert-to-implicit` | Copied point/vector content | inherited | implicit | Not applicable (implicit) | Not covered |
| Raster R2 / D1 (`raster`, `raster-tile-to-directory`) | PNG/COG pyramid only; **no 3D Tiles content** | — | — | Not applicable | Not applicable |

The C2 decisions do not apply to these patterns:

- **No producer writes** b3dm `RTC_CENTER`, `CESIUM_RTC`, `asset.gltfUpAxis`,
  `EXT_mesh_gpu_instancing`, skins, morph targets, animations, region volumes or sphere
  volumes. This was a full-tree search; [A] across all three searches.
- **Every producer writes y-up glTF positions** and relies on U applied exactly once. The
  suspected z-up/ENU-in-POSITION convention for P1 or vector is **refuted**:
  - point cloud: `point_cloud/tiles.rs:305` [V];
  - vector: `vector/model.rs:183-190` returns `[d·E, d·U, −d·N]` [V];
  - terrain: `terrain.rs:246` [V].
- **Every bounding volume is an axis-aligned box in the tile frame.** Rotation, where there
  is any, lives in a root transform only.

## v2 slice-1 coverage prediction (static, to be confirmed by the oracle corpus)

| Output | Predicted slice-1 outcome | Depends on |
| --- | --- | --- |
| Point cloud, default (implicit) | NotInspected(`implicitTiling`) | D3: covered after A2 |
| Point cloud `--explicit` | Contained; possible Outside on 1-ulp edge cases | f32 rounding padding ≥ f64 slack |
| Vector, default (implicit) | NotInspected(`implicitTiling`) | D3 |
| Vector `--explicit` (plain, `--quantize`, `--meshopt`) | Contained, or Outside on 1-ulp edge cases | `3DTILES_content_gltf_vector` staying on the tileset allowlist (O must pin README `c48ebdc`). Polygon primitives are tested on core TRIANGLES draws only; loops are not draws (v3) |
| Terrain | Not reachable by `inspect` (directory output) | Packaging |
| F1c2/F1d mesh | Contained | — |
| W1 local / Wgs84 | Contained; NotInspected(`numericSpan`) beyond about 16 generic box levels or 10 with a sphere (W1 writes boxes only) | D2 window 2^-18; span cap |
| Legacy `mesh-to-3tz`, explicit | Contained, or Outside on mixed magnitudes; meshopt is oracle-decoded in v2 (R9) | `KHR_texture_basisu` / `EXT_texture_webp` allowlisted |
| Legacy `mesh-to-3tz`, implicit; `convert-to-implicit` | NotInspected(`implicitTiling`) | D3 |
| Legacy small-model wrap | Outside possible; NotInspected for unlisted source extensions, ambiguous scenes, skins and animation | Legacy disposition |

No producer is predicted to receive a new ResourceLimit. C2 caps give `workLimit` (not
inspected).

## Point cloud (P1)

- **Content.** One GLB per tile, using `MetadataGlb::new`.
  - The document has `scene:0`, one node `{"mesh":0}`, no matrix and no TRS
    (`src/glb.rs:312-321`) [V].
  - The primitive is POINTS with FLOAT positions [A].
- **Positions.** Each position is `[p.x−c.x, p.z−c.z, −(p.y−c.y)]` converted to f32. The
  `rounding` value is the maximum norm of that f32 error (`point_cloud/tiles.rs:300-316`) [V].
- **Transforms.**
  - Every tile gets a translation-only transform in f64, `translation(center − parent_center)`
    (`tiles.rs:141-142`, `tileset_node.rs:7-9`) [V].
  - The root adds the f64 `origin`. The axes are ECEF-aligned and there is no rotation
    (`point_cloud.rs:585-591`) [A].
- **Bounds.**
  - The half-extent is `max(extent/2 + rounding, 1e-6)`, where `extent` is taken from f64
    positions before the f32 conversion (`tiles.rs:140`) [V].
  - Parents grow by `|delta| + child_half` (`tileset_node.rs:57-72`) [V].
  - Nothing rounds upward.
- **Exact strict verdict.** Pass when `rounding` is at least the f64 slack lost in computing
  `c`, `extent/2 + rounding` and `|delta| + child_half`, so that the f32 error padding covers
  that slack on every axis. `rounding > 0` alone is not sufficient, because `rounding` can be
  smaller than one f64 ulp of `extent/2` (review note 4).
- **Residual risk: 1-ulp f64 escapes.** These can come from:
  - `c = lo + extent/2`;
  - `extent/2 + rounding` rounding down;
  - `|delta| + child_half` rounding down.

  The risk is real when `rounding` is below that slack. The clearest case is when every
  coordinate is exactly representable in f32, so that `rounding == 0`, for example integer
  synthetic fixtures. In that case the checker can find
  a genuine Outside of about one ulp. This is a producer defect under the exact contract.
  The fix is an outward `next_up` at `tiles.rs:140` and `tileset_node.rs:65`, matching
  `mesh_archive/partition.rs:30-45`. It has not been executed.
- **Default is implicit.**
  - `--explicit` opts out (`main.rs:299-301`, `PointCloudArgs`) [V].
  - The writer call is `implicit::write_tileset_recorded(…, Octree, …)`
    (`point_cloud.rs:601-608`) [V].
  - **This is the implicit P1 blocker; see below.**

## Vector

- **Content.**
  - GLB for points (POINTS) and lines (LINE_STRIP with `KHR_mesh_primitive_restart`).
  - b3dm with `BATCH_LENGTH:0` and no RTC for fills (`encoding.rs:324-369`, `939-948`) [A].
  - `scene:0` with one node [V via `glb.rs`].
  - Other extensions: `EXT_mesh_features`, `EXT_structural_metadata`, `EXT_mesh_polygon`
    (`encoding.rs:541-543`), `KHR_materials_unlit`.
  - Optional `EXT_meshopt_compression` with `--meshopt` (`main.rs:439-441`).
  - Tileset-level `3DTILES_content_gltf_vector` (`store.rs:970`, `encoding.rs:960`) [V].
- **Positions.** The spool holds y-up local metres, and each position is written as f32 of
  `p − center` [A].
- **Quantization (`--quantize`, opt-in, `main.rs:438`) [V].**
  - Unsigned 16-bit normalized values: `q = round((p−lo)/scale·65535)`, clamped.
  - The node gets `translation = lo` and `scale = scale` as f64 JSON
    (`encoding.rs:593-628`) [V].
  - Exact decoded value: `lo + scale·q/65535`. It is rational, which the contract handles
    by clearing denominators.
  - The quantization error estimate assumes f32 runtime dequantization and adds
    `|lo − f32(lo)|`, `|scale − f32(scale)|` and `|scale|·2^-24` (`:629-632`) [V]. That is
    conservative against the exact reading.
- **Transforms.**
  - Root: ENU at the first feature's anchor, f64, with translation
    `anchor + Σ axes·center` [A].
  - Children: translation only, `delta = child.zcenter − parent.zcenter`, f64
    (`store.rs:519-521`) [V].
- **Bounds.**
  - Half-extents come from `stats` (min/max over rendered points in f64), swizzled to z-up,
    and are floored at 1 mm.
  - Then padding is added: `positionRoundingMetres + quantizationErrorMetres`, maxed with
    each child's padding (`store.rs:500-529`) [V].
  - LOD vertices are source points or interpolations along source edges, so they stay inside
    `stats` [A].
- **Exact strict verdict.** Likely pass in explicit mode, with the same caveat as point
  clouds: padding is 0 when every local coordinate is f32-exact and there is no
  quantization, and then 1-ulp f64 escapes are possible. Polygon positions also take an f64
  round trip through `+center` and then `−center` (`encoding.rs:473-488`) [A].
- **Default is implicit.**
  - `--explicit` opts out (`main.rs:335-337`, `VectorArgs`) [V]. `MeshArgs` has its own legacy `--explicit` at `main.rs:468-470`.
  - `store.rs:976-982` calls `implicit::write_tileset(…, Quadtree, true)` [V].
  - Children take array-index slots, because vector never sets `implicitChildIndex`
    (`implicit/tileset.rs:117-121`) [V]. KD median splits are therefore mapped onto regular
    quadtree cells that do not correspond to the geometry.

## Implicit P1/vector convention blocker (concrete)

Applies to both default producers.

1. **No per-tile transforms.**
   - Implicit tiles carry no per-tile `transform`.
   - The writer adds each tile's accumulated z-up offset to the root node of **every**
     scene in the content, as a y-up glTF `translation` `[dx, dz, −dy]`, or adds it into
     `matrix[12..14]` (`implicit/tileset.rs:149-224`, line 190) [V].
   - Content therefore lives in the implicit root frame.
2. **Where the volumes live.**
   - The actual per-tile boxes are kept only as `TILE_BOUNDING_BOX` / `TILE_GEOMETRIC_ERROR`
     rows of the private `rustyTile` class (`metadata.rs:174-180`,
     `implicit.rs:188-208, 260-292`) [V].
   - The derived regular cells (`derived_box`, `implicit/tileset.rs:403-418`) are compared with
     those boxes only at subtree roots, with `1e-6` slack (`:284-297`) [V]. This is a **writer
     layout fallback**, not a validation gate: when it fires, the writer switches to
     `write_chunks` with external roots (review note 3).
   - Content can therefore lie outside its derived cell. Vector does so by construction.
     This is spec-legal, because the semantic override replaces the computed volume
     (metadata-semantics `adoc:145,161`, quoted in
     `bench/architecture_audit/c1_payload_integrity/production_controls/context_controls/2026-10-10/references/metadata-semantics.adoc`;
     A2 semantics audit `a2_semantics/audit.md:74-88`) [V].
3. **Consequences for C2:**
   - A C2 that checks raw derived cells would report false Outside against
     correctly-overridden output.
   - A C2 that marks any metadata override Unsupported, as the prior proposal did, can
     never cover a default point/vector output.
   - Honouring the overrides requires an effective-volume owner. Today that owner is
     `implicit::expand_tileset_bounded`, the old untrusted expander (`tileset.rs:1133-1135`
     substitutes the metadata box), and #126 A2 is replacing it.

   **A strict C2 cannot be adopted as the default for default point/vector outputs until
   A2's effective-volume owner exists.** The contract's slice 1 marks implicit as not
   inspected (reason `implicitTiling`), and decision D3 governs the next step.
4. **`translate_content`.** It rewrites roots in *all* scenes, not only the selected one.
   This is harmless for the producers' single-scene GLBs, but any third-party multi-scene
   source would have its unselected scenes shifted too.

## Terrain T1

- **Content.**
  - Hand-built GLB with `scene:0` and one node; TRIANGLES with u16 indices.
  - No extensions, skirts or normals [A].
- **Positions.**
  - Encoded as `[e, u, −n]` in f32 (`terrain.rs:246`) [V].
  - The decode at `:253-257` is exactly U [A].
- **Transforms.** One f64 ENU frame on the root only (`terrain.rs:619-622`) [V]. Leaves have
  none.
- **Bounds.**
  - `box_for` computes `half = ((hi−lo)/2 + 1e-8 + 64·EPS·max(|c|,1)).next_up()`
    (`terrain.rs:322-331`) [V].
  - It uses the **decoded f32** positions after U [A].
  - The root box is the padded union of the children.
- **Exact strict verdict.** Pass.
- **Reachability.** The output is a directory, which `inspect` rejects
  (`validate.rs:752-755`). C2 reaches terrain only through the oracle or after packaging.

## F1c2/F1d mesh archive

- **Content.**
  - GLB with one node `{"mesh":0}` and no transform (`mesh_archive/encode.rs:501-503` leaves,
    `:287-289` proxy) [A].
  - FLOAT positions with exact f32 min/max.
  - `EXT_mesh_features` and `EXT_structural_metadata`.
- **Source baking.** The source node chain is baked in f64 and then rounded to f32 [A].
- **Bounds.**
  - Built from the stored f32 positions after `to_tile` (U), with the half-extent rounded
    outward by `next_up`. Degenerate axes get an exact 0 (`partition.rs:21-45`) [V].
  - The root box is the outward union of the leaves [A].
  - Proxy vertices are original stored vertices [A].
- **Transforms.**
  - On the root only. Local placement uses the identity.
  - Wgs84 placement uses f64 ENU × the normalized quaternion (`placement.rs:150-224`) [V].
- **Verdicts.**
  - Exact strict: pass.
  - Prior proposal: pass (`next_up` dominates one-ulp interval widening in the tile frame).
  - The root rotation never enters a predicate.

## W1 static model wrap (quaternion and matrix convention blocker)

- **Content.** The user's GLB/glTF is emitted byte-for-byte unchanged (`mesh_archive/model.rs`) [A].
- **Source admission** (`mesh_archive/source.rs`) [V]:
  - Nodes may only have name, mesh, children, matrix or TRS (`:660-668`).
  - Matrix together with TRS is invalid (`:673-678`). The matrix must be affine (`:681-683`).
  - Local matrix column orthogonality: `|dot| ≤ 1e-6·|a||b|`, otherwise Unsupported
    (`:684-692`).
  - Rotation components must lie in [−1, 1] (`:711-713`).
  - **`|‖q‖ − 1| ≤ 1e-6`**, then normalized for W1's own bake/bounds use (`:714-720`).
- **Bounds.**
  - Outward f64 intervals over the authored chain.
  - Each rotation is the **hull of the raw-polynomial and normalized-quaternion matrices**
    (`source/original_bounds.rs:120-145`) [V].
  - Then `box_values` applies `next_up` (`model.rs:246-250`) [V].
  - Only the selected scene is evaluated [A].
- **Placement quaternion.**
  - User-supplied and admitted at `|‖q‖ − 1| ≤ 1e-12` (`placement.rs:11`, `:155-162`) [V].
  - Normalized in f64 (`:169`) and folded into the f64 root tile transform. It is never
    written as a node rotation.
  - The tile 3×3 is nonsingular but not exactly orthonormal. Exact C2 needs neither property.
- **v1 blocker (now resolved): quaternion window.**
  - The prior proposal admitted a node quaternion only if `|‖q‖² − 1| ≤ 2^-20` (≈ 9.54e-7).
  - W1 admits `|‖q‖ − 1| ≤ 1e-6` (`source.rs:714-717`). That gives `|‖q‖² − 1| ≤ 2.000001e-6`,
    which is ≤ 2.0000010018e-6 after W1's f64 evaluation slack, as the reviewer verified.
  - W1 emits the quaternion unchanged, so W1 outputs could have been refused.
  - **v2 resolution (D2):** C2 owns its own validator-side window, `|‖q‖² − 1| ≤ 2^-18`
    (≈ 3.81e-6). That admits everything W1 admits. Anything beyond the window is *not inspected*
    (`quaternionWindow`), never a failure.
  - W1's code and constant are untouched. There is **no shared constant**: #125 forbids
    writer/validator shared helpers, and `src/mesh_archive/**` is out of bounds for the C2
    authors.
- **v1 blocker (now resolved): matrix window.**
  - The prior proposal had a local-matrix rule `|cos| ≤ 9.54e-7`, against W1's `|cos| ≤ 1e-6`
    (`source.rs:689`).
  - v2 drops any C2 local-matrix orthogonality or decomposability check. Soundness does not
    need one (D2(iii)).
- **Exact strict verdict.** Pass under the normalized meaning (D2), because the W1 hull
  (`original_bounds.rs:128-139`) contains the normalized matrix. It would also pass under the raw
  meaning.
- **Expected v3 coverage (normative bound).** Deep rotated user hierarchies can exceed
  `containmentSpanBits`. At the 4096-bit cap, the admitted depth with box volumes is 16 generic
  or 14 near-axis rotated levels; with a sphere it is 10 or 6. A quaternion with a
  subnormal-scale component admits only 1 level (box) or 0 levels (sphere). Such archives are
  `numericSpan` (not inspected). The oracle corpus records the actual count.

## Legacy `mesh-to-3tz` HLOD (live: `main.rs:1232-1240`) [V]

- **Content.** GLB with one node, positions in f32, and meshopt required by default.
- **Baking.**
  - The node chain is baked in f32 with gltf-crate matrices (`mesh.rs:437-440`) [V for the
    call].
  - Whether the gltf crate normalizes the quaternion is [A]/uncertain.
- **Transforms.** Root only, `root_transform` (ENU or ENU×HPR, f64) (`tile.rs:512-514`) [A].
- **Bounds.** `aabb_to_box` uses f64 midpoint and half-extents of f32 min/max with **no
  outward step and no padding** (`bbox.rs:216-224`) [V].
- **Exact strict verdict: can fail.**
  - Mixed-magnitude midpoint rounding can push a stored vertex about one ulp outside. This
    is the defect class acknowledged at `mesh_archive/partition.rs:35-36` [V].
  - In this lane's same-scale probe, f64 midpoints of f32 pairs never escaped (Q1 = 0 of
    40,000). The risk is specific to mixed magnitudes.
  - Implicit mode assigns octants by triangle centroid (`tile.rs:666-672`) [V]. Content can
    therefore exceed derived cells, which is the same implicit blocker as above.

## Legacy small-model wrap (`tileset.rs` `glb_job` / `implicit_glb_job`, `bbox.rs`)

- **Bounds.** Built from **accessor POSITION min/max corners** when present (`bbox.rs:15-26`,
  `110-120`) [A]. The corners are transformed through f32 node matrices
  (`bbox.rs:98-136`) [A], then `aabb_to_box` is applied with no padding [V].
- **Scene selection.** With no default scene it takes the **first** scene
  (`bbox.rs:68-81`) [V]. C2 instead refuses an ambiguous multi-scene source.
- **Exact strict verdict: can fail.**
  - f32 transform rounding is not enclosed.
  - There is no outward step.
  - Quantized normalized min/max are raw integers [A].
  - `EXT_mesh_gpu_instancing` is ignored [A], whereas C2 marks it not inspected.
- **Mitigation already in place.** C1 already verifies declared min/max against decoded
  values for viewed accessors (`payload.rs:736-760`) [V]. Trusting min/max is therefore
  unsound only for viewless or normalized accessors.

## `convert-to-implicit` and raster

- **`convert-to-implicit` (`convert_implicit.rs`)** [A].
  - Copies content bytes as aliases. Each content tile becomes an external implicit root
    that keeps its original `transform`.
  - Metadata boxes are child boxes shifted into the parent frame.
  - `check_tree` admits overhang beyond regular cells of up to padding plus `1e-8·cell_h`.
  - Not applicable to slice 1 (implicit).
- **Raster R2 / D1.** Both write only COG/PNG/TileJSON and no glTF/tileset
  (`raster.rs`, `raster_directory.rs`) [A]. Out of C2's domain.

## What would change producers

None of these changes is recommended in C2 slice 1. They are listed so that inventory
outcomes are not mistaken for checker defects.

| Producer | Change, if the oracle confirms an escape | File |
| --- | --- | --- |
| Point cloud | Outward `next_up` on half-extent and enclosure | `point_cloud/tiles.rs:140`, `tileset_node.rs:65` |
| Vector | Same | `vector/pipeline/store.rs:503-517`, `tileset_node.rs:65` |
| W1 | None required by C2 v2. The C2 window 2^-18 is validator-owned and admits all W1 inputs | — |
| Legacy mesh routes | Retire, or apply outward bounds. Disposition is owned by the mesh migration, not C2 | `bbox.rs:216-224`, `tile.rs` |
