# T1 producer and consumer independent implementation review

Reviewed candidate source head: `1c87f0c704728c0c0f602eb17fb59196859ec45e`.
Executed immutable native binary SHA256:
`3bd3d8b2986b8b84ea110d500a6d058329159203f9bfabcf26a69d9ba02c8bc2`.
Build manifest identifies `native-geospatial`, rustc 1.98.0 and GDAL 3.13.3.
The reviewer verified the reviewed producer, codec, CLI and preview file hashes
against that manifest before writing this review. Compact execution evidence is
[oracle-final-summary.json](oracle-final-summary.json).

## Independence and scope

This reviewer did not author `src/terrain.rs`, `src/terrain/glb.rs`, the CLI
terrain adapter, or the preview consumer changes. Review concentrated on their
coordinate, bounds, topology, publication, report and runtime ownership behavior.
The reviewer did author the native source decoder and independent Python oracle;
this document therefore does **not** claim a nonauthor review of those components.
Executing that oracle provides an independent reference calculation against the
producer, not independent authorship certification of the oracle itself.

The oracle imports no producer code and does not use GDAL to resample source
values. WGS84 calculations use semi-major axis and inverse flattening, analytic
source values define expected lattice heights, and emitted GLB bytes are decoded
independently. GDAL was used only to write the specified source fixture files.

## Findings and resolution

1. **Ancestor box containment defect, resolved.** The initial producer formed
   root bounds from raw decoded positions and padded each leaf/root box
   independently. The six-leaf fixture reproduced emitted child boxes extending
   beyond the root by approximately `1.24e-10` to `2.33e-10` metres. This was a
   real contract violation despite its small magnitude; the original tolerant
   content-point oracle concealed it. The revised producer unions emitted child
   box extents before constructing the root box. The strengthened oracle checks
   emitted child minima/maxima against root minima/maxima without tolerance.
   Both final candidate fixtures pass this check.
2. **Codec/runtime dependency, resolved.** The initial GLB writer imported
   `Attempt` directly. The revised writer takes a checkpoint closure, preserving
   the private codec boundary without importing directory/run ownership or
   creating a generic converter framework.
3. **Storage topology admission, implemented.** Producer preparation rejects
   f32 triangles that collapse or face away from their geographic cell normal.
   This is necessary for a queryable surface; finite coordinates and small
   rounding error alone did not establish that property. The final source has
   the explicit check. The independently executed plane cases verify all
   oriented cells, rather than accepting triangle counts alone.

4. **Source-domain versus Cartesian boundary gap, consumer correction reviewed.**
   An actual decoded-mesh ray at longitude `10.125`, latitude `21.00001`
   intersected the plane fixture at height `97.25617587856748` metres despite
   the source north bound being `21` degrees. Constant-latitude Cartesian
   boundary chords bow poleward; source-domain vertices do not prove that raw
   mesh intersections remain inside the geographic rectangle. The earlier
   pinned [counterexample](chord-roi-counterexample.json) remains evidence.
   The revised preview reads the T1 profile's finite, ordered report bounds
   and partitions queries against their closed radian rectangle before invoking
   Cesium. Outside entries remain undefined in their original array positions.
   Height queries clone input coordinates; clamp queries derive fresh
   cartographic coordinates and normalize query height to zero. The exclusion
   list covers other scene/ground primitives and entities/data-source entities.
   This reviewer inspected that fix; final browser execution is attributed to
   its separate owner and recorded in [browser-final.json](browser-final.json).
   The oracle now supplies a near-boundary outside point with an independently
   confirmed underlying mesh hit, distinguishing ROI clipping from a natural
   intersection miss. Direct Cesium methods retain raw mesh-intersection
   semantics. The final delta adds a generic absent-coordinate guard, so the
   Earth-centre Cartesian value for which Cesium cannot construct a Cartographic
   remains undefined instead of throwing. NaN longitude/latitude fail the
   closed-domain comparisons. The separate final browser controls exercise
   both Earth-centre and NaN cases; this reviewer inspected the guard and
   verified the consumer file against the final build manifest.

No additional producer/CLI publication blocker was found within this review's
finite scope. That statement is not a whole-library correctness certification.

## Executed final candidate evidence

| Fixture | Leaves | Global source-lattice nodes | Oriented triangles |
| --- | ---: | ---: | ---: |
| Standalone 4x4 Float64 plane | 1 | 25 | 32 |
| Standalone 33x17 Float64 plane, 16 cells per leaf | 6 | 612 | 1122 |

Both cases passed strict GLB envelope/payload decoding, complete source-node and
oriented-cell inventory, analytic bilinear height plus explicit offset checks,
independent ECEF/ENU frame reconstruction, stored-coordinate comparison,
content bounds, exact emitted child-box containment, report counts, routing
report agreement and exact published file/byte inventory. The six-leaf case
also passed exact stored-coordinate equality at shared seams.

The final candidate rejected seven deliberate corruptions on the single-leaf
case: position, winding, root bounds, root frame, exchanged rows, a missing
triangle replaced by a degenerate triangle, and report inventory. The six-leaf
case rejected those seven plus a one-f32-ULP seam corruption. Source/frame/row
checks execute before receipt checks so their corruption results are not merely
artifacts of changed JSON serialization lengths.

CLI options construct the same typed `TerrainRequest` and `RunControl` as Rust,
with `--force` selecting the accepted D2 replacement policy. Preparation binds
source/destination identities before source callbacks; the producer uses one
attempt and performs grid planning before final-output staging. Writers flush
and sync before directory seal. Producer receipt and required observations
finish before publication. No required producer report or callback follows
commit. The report's generated-byte fixed point includes its own serialized
size, and the independent output walk agrees with that value on both cases.

The coordinator reports thirteen native source tests, the native Rust/CLI
checks, 124 native Python tests (two optional skips), focused portable checks,
and workspace clippy checks passing. Those are attributed coordinator results,
not suites rerun by this reviewer. The separate final
[browser evidence](browser-final.json) is pinned to the same final source and
binary and reports query/clamp, near-boundary ROI refusal despite an actual mesh
hit, unrelated-surface isolation, unlocatable/NaN-coordinate controls, imagery
draping and uncached reload. This reviewer inspected consumer code and checked
the evidence pins but did not independently execute the browser. The separate
[resource evidence](resources-final.json) records observed Linux RSS/descriptor
and native-close/staging behavior; its workload and sampling limitations remain
explicit. The direct final producer oracle executions above use the final
reviewed binary and independently verify its hash.

## Remaining boundaries and gates

T1 is a finite geographic raw-metre DEM-to-3D-Tiles profile. It intentionally
replaces quantized-mesh terrain-provider output, horizon points, zoom pyramids,
simplification and source coverage sidecars. Preview queries/clamps intersect
decoded triangles, including filled samples; they do not expose source-bilinear
query parity or source coverage identity. Imagery drapes through Cesium's tileset
imagery layers, without baking textures into generated GLBs.

Per-vertex f32 storage admission is not a certified error bound against the
continuous source DEM between samples. The contentless root's routing error is
an omission/selection metric, not a measured LOD approximation bound. Leaf
counts, source/block/array caps and output admission do not by themselves prove
a total-process RSS bound or all-filesystem crash durability. Resource evidence,
native lifetime/fault evidence and the D1/D2 platform matrix retain their own
bounded provenance. Projected/vertical/compound CRS, external source resources,
other sample interpretations, optional codecs and broader raster/terrain support
are outside this profile.

**Final remote CI is pending at the time of this review.** Passing these focused
oracles and resolving the findings above does not close all of #124, satisfy
#113's release gate, authorize publication, or establish acceptance on platforms
and viewer configurations not actually tested.
