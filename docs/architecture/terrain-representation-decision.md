# Terrain foundation: representation decision

Tracking: [#135](https://github.com/BenDyson-Arch/rusty-tiles/issues/135), under
[#124](https://github.com/BenDyson-Arch/rusty-tiles/issues/124) and release gate #113.
Audit baseline develop `5df15e7`; point P1 subsequently merged as `c4912f8`.
Status: standard replacement investigation complete; replacement producer not
accepted or implemented by this document. The user selected investigation of a
standard replacement rather than a custom quantized-mesh viewer contract.

## Decision and consequences

Use **3D Tiles 1.1 with embedded glTF 2.0 mesh content** as the next finite terrain
profile. It supports an arbitrary spatial tree over the source footprint, ordinary
Cartesian bounds and no mandatory horizon-occlusion point. The format explicitly
includes terrain/surfaces among its mesh use cases. This removes the requirement
to invent filled hemispherical roots for a small DEM.

This is a breaking output change: `tileset.json` and GLB leaves replace
`layer.json` and quantized `.terrain` files. Consumers load a `Cesium3DTileset`.
It is not a drop-in `CesiumTerrainProvider`: `sampleTerrain`, terrain-based
clamping and imagery-layer draping are outside the initial profile. An adapter,
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

## Revised T1 contract to implement

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
sample records retain null coverage before fill. There is no geometry outside the
source footprint. Holes instead of fill would be a separate explicit policy.

Partition the complete sampled triangle mesh without simplification or parent
proxies. Initially omit textures, encoded normals, imagery draping, compression
and optional culling extensions. Define the untextured material/rendering profile
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
negative heights and root bounds, then final candidate/platform/package checks.
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
