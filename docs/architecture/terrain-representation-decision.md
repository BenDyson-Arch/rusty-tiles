# Terrain foundation: representation decision

Tracking: [#135](https://github.com/BenDyson-Arch/rusty-tiles/issues/135), under
[#124](https://github.com/BenDyson-Arch/rusty-tiles/issues/124) and release gate #113.
Audit baseline develop `5df15e7`; point P1 subsequently merged as `c4912f8`.
Status: replacement implementation and focused independent local checks are
complete; final PR CI is pending. [Executed evidence](../../bench/architecture_audit/terrain_acceptance/README.md)
retains candidate pins and review limits. This decision document alone earns no
producer acceptance. The user selected investigation of a
standard replacement rather than a custom quantized-mesh viewer contract.

## Decision and consequences

Use **3D Tiles 1.1 with embedded glTF 2.0 mesh content** as the next finite terrain
profile. It supports an arbitrary spatial tree over the source footprint, ordinary
Cartesian bounds and no mandatory horizon-occlusion point. The format explicitly
includes terrain/surfaces among its mesh use cases. This removes the requirement
to invent filled hemispherical roots for a small DEM.

This is a breaking output change: `tileset.json` and GLB leaves replace
`layer.json` and quantized `.terrain` files. Consumers load a `Cesium3DTileset`.
It is not a drop-in `CesiumTerrainProvider`. The first slice includes mesh surface
queries, terrain-based clamping and georeferenced imagery draping through the
pinned Cesium 1.146.0 consumer. Queries/clamps use public scene mesh-intersection
APIs; imagery attaches to `tileset.imageryLayers`, an explicitly experimental
Cesium API. It does not promise terrain-provider `sampleTerrain` compatibility. An adapter,
nonfinite header sentinel, huge finite proxy or legacy fallback is not part of
this decision.

Heightmap 1.0 is not the replacement: its primary documentation deprecates it,
its grids are fixed at 65×65, and its parser uses unsigned 16-bit heights at
0.2-metre steps with a −1000-metre offset. It would preserve terrain-provider
integration while introducing a poor precision/range foundation.

## Why the previous representation cannot earn retention

The quantized-mesh header requires that an occluded horizon point imply an
occluded tile. Full zero-height root meshes contain antipodal surface vertices
`p` and `−p`. For any finite proposed point `q`, at least one has `q·p < 1`.
At camera `c=p`, the culling plane term is positive while the distance to the
limb is zero; the point is occluded. Continuity gives a sufficiently small
positive altitude at which that point remains occluded while the surface vertex
is visible. Consequently no finite header point gives the required guarantee
for that admitted full-root case.

The independent audit also executed Cesium's scalar predicate on a decoded
17×17 zero-height eastern-root case: proxy `(0, 1e15, 0)` was invisible while
vertex `(0.999999998858649, 0, 0.0000477776343128956)` was visible from
`(0.9999999988586492, −1, 0.00004777763431289561)`. This is a boundary counterexample,
not a claim about the size of the visible rendering defect. Negative-height
worker recomputation does not repair undefined bounds: the consumer falls back
to its header point. A zero point also does not disable culling.

Separate baseline source evidence reproduces Float64 height `100000000.125`
becoming sidecar `100000000.0` before height quantization. The mask fixture was
honored in that one exercised case; it is not proof of arbitrary mask support.
See [baseline records](../../bench/architecture_audit/terrain_acceptance/README.md).

## Revised T1 contract

The initial source profile remains bounded: a standalone one-band Float32 or
Float64 GeoTIFF with internal WGS84/EPSG:4326 PixelIsArea north-up georeferencing.
The caller declares raw samples as metres and supplies an ellipsoidal height
offset and fill height. Nonidentity band scale/offset, conflicting unit metadata,
external dependencies, overviews, alternate georeferencing, unsupported masks,
nonfinite valid samples and all-invalid sources are refused. Scalar NoData uses
an explicit pure f64 sampler: closed outer footprint, clamped outer pixel centres,
and only nonzero-weight contributors required valid. Source decoding finishes
its native handle before private output staging and returns owned values.

Construct one fixed rectangular sample lattice over the source footprint. Prefer
one cell per source pixel for the first profile, with explicit bounded patch
partition size; there is no zoom pyramid or inferred resampling resolution.
Before acceptance, freeze exact lattice counts/coordinates, triangle diagonal,
resource caps and precision domain. Boundary patches share coordinates and heights.
Missing source samples inside the footprint use the caller's fill; independent
fixtures prove null coverage before fill. Only source-footprint lattice nodes and their triangles are emitted;
Cartesian boundary chords can project slightly beyond its geographic rectangle. Holes instead of fill would be a separate explicit policy.

Partition the complete sampled triangle mesh without simplification or parent
proxies. Initially omit embedded textures, encoded normals, compression
and optional culling extensions. Include live imagery draping from an explicit
georeferenced imagery provider on the terrain tileset. Define the opaque material/rendering profile
explicitly. Use one shared Cartesian local frame and identical encoded boundary
coordinates so adjacent patches remain identical after decoding. Apply glTF Y-up
to 3D Tiles Z-up conversion exactly once, then the declared Earth-fixed transform.

Bounds enclose actual decoded Float32 geometry, including triangle chords; use
Cartesian boxes/spheres rather than assuming geographic height regions enclose
the mesh. Ancestor bounds contain all descendants. A contentless routing root
needs a finite positive selection metric: a zero-error empty root can stop Cesium
traversal. A decoded bound diameter is a candidate routing metric, not a certified
parent approximation error. Leaf error zero means full detail of the declared
sampled mesh, not perfect reconstruction of the continuous DEM. Viewer selection
and far-distance behavior must earn acceptance.

Report source interpolation, numerical geodetic/frame evaluation and actual
Float32 storage error separately. Enforce a declared local magnitude/precision
domain before publication. Retain one typed request/result, source/output identity
protection, bounded checkpoints, finalized member receipts, required reports and
D1/D2 directory publication under one Attempt. Domain rules remain in the producer.
A future #82 decoder can return the same owned elevation representation; no source
trait framework or GDAL policy enters runtime.

Delete the quantized encoder, common 15-bit height interval, horizon proxy,
quantized UV contract, geographic availability pyramid, legacy overload family
and obsolete terrain-provider examples when the replacement producer/callers are
ready. No compatibility wrapper or output-format switch is planned.

## Amendment: decimated level of detail (T-LOD)

The user approved replacing the flat leaf list with a level-of-detail pyramid
(design T-LOD, 2026-10-12). This amends "no zoom pyramid" and "without
simplification or parent proxies" above, in these terms:

- **Decimation only.** Level `L` of `D` keeps every `2^(D−L)`-th leaf lattice
  node plus the clipped far edges, in tiles of at most `cells-per-leaf` cells.
  Coarse vertices are the stored leaf vertices; there is no resampling,
  filtering, inferred resolution or mesh simplification. NoData and fill
  semantics are unchanged, and leaf surface primitives are byte-identical to
  the flat profile. A meshopt byte-reduction slice may follow separately.
- **Certified parent errors.** A tile's error is the exact overlay maximum of
  the distance from its decoded surface to the next finer level at equal lattice
  parameters, plus the largest child error. It bounds the distance to the leaf
  mesh and replaces the uncertified routing diameter. It is not a bound on the
  continuous DEM.
- **Skirts.** Edges shared with a same-level tile carry a separate skirt
  primitive of depth `max(e + e_root, 1 m)`, which covers any T-junction gap
  between levels. Outer footprint edges have none.
- **Layout.** The explicit hierarchy is published only as the bounded implicit
  forest; there is no `--explicit` flag. The explicit hierarchy remains a
  crate-internal test path so that C2 decoded containment can run on it.
- **Always on.** There is no `--no-lod` option.

The report moves to schema 2, profile `t1-terrain-lod-mesh-3dtiles-1.1`. The
validator is unchanged: packed output passes its implicit, bounds and error-order
checks, and C2 proves decoded containment of the explicit test variant.

## Amendment: packed tiles (T-PACK)

The terrain output-size slice (2026-10-12) amends T-LOD's "coarse vertices are
the stored leaf vertices", "byte-identical leaf surface primitives" and
"separate skirt primitive":

- **Storage.** Tile GLBs require `EXT_meshopt_compression` and store positions
  as unnormalized uint16 `KHR_mesh_quantization` on one power-of-two grid per
  level (the same step on every axis), with node translations on the grid, so decoding is exact and
  a level's tiles agree on shared nodes. Leaves keep Float32 where their grid
  could move a vertex more than 0.05 m; such leaves keep the flat profile's
  surface bytes after decoding (up to the index codec's corner rotation).
- **Certification on decoded positions.** Errors, skirt depths and boxes are
  computed on the decoded positions of every level, with outward rounding.
  Skirts are deepened by twice their level's storage error bound.
- **One draw call.** The skirt shares the surface primitive and material: its
  triangles follow the surface triangles and its tops are the surface vertices.

The report moves to schema 3 with `quantizationStepMetres`.

## Owner amendment, 2026-10-29 — NoData holes

NoData holes replace fill (`--fill-height` and `fillHeightMetres` are
removed): only triangles of three valid nodes are emitted, and all-NoData
tiles are unavailable. Errors are measured where both levels have a surface;
a cell whose coverage may differ adds one coarse-cell diagonal, and a
content-less tile above content takes 16× its surfaces' box diagonal (the
mesh empty-parent floor; the box diagonal alone let Cesium stop there). Skirts drop
`max(e + e_beside, 1 m)` (see the terrain guide) instead of `e + e_root`, and
none hang along NoData. The default `--cells-per-leaf` is 128.

Owner decision, 2026-10-30 (as option A for meshes): a tile without content is
exempt from parent ≥ child error ordering, here and in `validate`'s
`geometricErrorOrder`; tiles with content answer to their nearest ancestor
with content, which takes the largest of their errors. The 16× floor stays on
the empty tile only, so ancestors, `e_beside` and boxes never inherit it.
Interior edges are also measured against the leaf nodes on them, and a
segment is skirted where any level drawable across has a triangle on it.

## Acceptance before retaining the replacement

Independent literal/analytic sources must verify every lattice coordinate, height,
null/fill choice, triangle and shared edge. Decode GLB payloads and reconstruct
rendered positions through node/tile transforms; source metadata alone is not a
placement oracle. Corrupt heights, frames, rows, winding, triangle inventory,
shared edges and bounds to demonstrate failures. Require native/default capability
parity, source aliases/ancestry protection, actual partial member/report/flush/
finalizer/observer failures, cancellation, concurrent runs, native close and
required cleanup before publication. Measure source/patch/member growth and
state limits without treating sample counts as whole-process memory bounds.

Run actual Cesium loading/traversal/rendering for a forced multipatch case,
negative heights and root bounds. Compare public scene queries and clamping
against independent intersections of decoded GLB triangles; prove absent results
outside the footprint and isolation from unrelated geometry. Prove imagery
changes the terrain surface pixels and uses local georeferenced tile requests,
including an uncached reload, then final candidate/platform/package checks.
Existing F1a GLB encoding is a candidate building block; do not make terrain
import mesh source/partition models just to reach it. Extract only a small private,
runtime-free codec if both actual consumers justify it.

## Primary references

- [3D Tiles specification](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/README.adoc), pin `4d781014b52294759834018a931223b98ac1ce47`.
- [glTF 2.0 specification](https://github.com/KhronosGroup/glTF/blob/19e83cf9890e798170f3326623d75ae10d3deca8/specification/2.0/Specification.adoc), pin `19e83cf9890e798170f3326623d75ae10d3deca8`.
- [Quantized-mesh header contract](https://github.com/CesiumGS/quantized-mesh#quantized-mesh-10-terrain-format).
- [Cesium occlusion predicate](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/core/Source/EllipsoidalOccluder.js#L446).
- [Cesium recomputation fallback](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/engine/Source/Core/QuantizedMeshTerrainData.js#L338).
- [Cesium empty-root traversal](https://github.com/CesiumGS/cesium/blob/df52c781de3491a4b76839d420f7ca90a032efb6/packages/engine/Source/Scene/Cesium3DTilesetBaseTraversal.js#L50).
- [Heightmap 1.0 primary documentation](https://github.com/CesiumGS/cesium/wiki/heightmap-1.0-terrain-format), live wiki; parser behavior checked against the same Cesium pin.

## First-slice consumer domain

Sampled and filled ellipsoidal vertex heights are within −10,000…8,000 metres.
The pinned public query implementation starts a downward ray at 9,000 metres;
this profile makes that limitation explicit rather than using private viewer APIs.
Actual Euclidean Float32 position storage error must be at most 0.05 metre.
This is distinct from interpolation or between-sample triangle approximation.

Preview hides the global ellipsoid when the bounded terrain mesh is selected,
so below-ellipsoid geometry remains visible. Mesh-only query/clamp helpers exclude
other scene primitives and require 3D mode/depth texture support. Missing source
samples produce caller-declared fill geometry; queries intersect that geometry.
Preview helpers fence the closed geographic source footprint separately from
Cartesian triangle intersections. Boundary chords can project beyond the
rectangle, so raw scene intersections alone cannot guarantee outside-domain
absence. The helpers return undefined outside this explicit query domain.

Cesium 1.146.0 pin `b8d3a36fe98a3e432eb89253c95d5f20e605e0f1`:

- [Public scene sampling/clamping](https://github.com/CesiumGS/cesium/blob/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1/packages/engine/Source/Scene/Scene.js#L5293).
- [Tileset imagery layers](https://github.com/CesiumGS/cesium/blob/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1/packages/engine/Source/Scene/Cesium3DTileset.js#L1220), experimental.
- [Height references](https://github.com/CesiumGS/cesium/blob/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1/packages/engine/Source/Scene/HeightReference.js).
