# Terrain T1 audit and acceptance evidence

The replacement implementation has passed the focused local checks below;
final PR CI and release gate #113 remain separate acceptance gates. The
[representation decision](../../../docs/architecture/terrain-representation-decision.md)
and [terrain contract](../../../docs/TERRAIN.md) record the finite standard
replacement, breaking changes and remaining limits.

## Replacement candidate

Functional source: `1c87f0c704728c0c0f602eb17fb59196859ec45e`. Immutable native
binary SHA256: `3bd3d8b2986b8b84ea110d500a6d058329159203f9bfabcf26a69d9ba02c8bc2`.
Evidence-only commits can follow this source pin without changing the producer
or preview. Each execution record retains the tested files' hashes; the complete
[build manifest](build-manifest.json) pins the binary's Rust/preview inputs.

- [Independent implementation review](independent-review.md) records author
  independence, reproduced defects, corrections and bounded review scope.
- [Oracle summary](oracle-final-summary.json) records analytic source-to-GLB
  truth for complete single- and six-leaf meshes, exact shared seams and bounds,
  report inventories and deliberate corruption controls.
- [Browser evidence](browser-final.json) records Cesium 1.146.0, Playwright
  1.63.0 and Khronos glTF Validator 2.0.0-dev.3.10. Four GLBs have zero validator
  errors/warnings. Actual positive/negative-height queries and clamping match
  independently decoded triangles; imagery changes rendered terrain pixels;
  unrelated surfaces, outside-domain queries, invalid coordinates and uncached
  reload are exercised. Tileset imagery is an experimental Cesium API.
- [Resource evidence](resources-final.json) records ten fresh-process runs,
  source/patch scaling, native-handle closure before observed staging, file
  descriptors, exact output accounting and rejected oversized native blocks.
  Observed peak RSS is 65,060–76,608 KiB; this is not a whole-job memory guarantee
  or a throughput promise.
- [Boundary counterexample](chord-roi-counterexample.json) shows why Cartesian
  boundary chords can intersect outside the source rectangle. Preview helpers
  explicitly fence the query domain; raw scene intersections retain mesh
  semantics.

The coordinator also ran the complete native Rust suite, 124 Python tests
(two optional skips), native and portable Clippy, and the portable Rust suite
with a focused rerun after correcting its feature-dependent golden expectation.
Writer/native-close/observer/cancellation and source-mutation controls precede
publication. A subsequent eight-test terrain integration run also passed a
concurrent native-source barrier with distinct requests and reports, exact
inventories, used-control rejection, CreateNew preservation and isolated Replace.
Only the test and evidence files changed after the functional source pin.
These local Linux results do not certify unexecuted platform,
wheel, Blender or release checks; CI retains those gates.

Reproduce the independent consumer gate with a native CLI and the pinned npm
packages above:

```sh
python tests/t1_terrain_viewer.py --binary /absolute/path/to/rusty-tiles \
  --cesium-dir /absolute/path/to/node_modules/cesium/Build/Cesium \
  --node-modules /absolute/path/to/node_modules \
  --validator-modules /absolute/path/to/node_modules \
  --json-output /absolute/path/to/browser-evidence.json
```

`tests/test_native_terrain.py` executes the independent source/lattice matrix;
`resource_probe.py --help` documents the separate resource runner. The format
uses ordinary 3D Tiles 1.1/glTF 2.0, with a bounded raw-metre EPSG:4326 source
profile, discrete mesh semantics and no terrain-provider compatibility claim.

## Historical baseline

`baseline-probe.py` authored constant, explicit-mask and Float64 GeoTIFF sources
using GDAL. Literal source values, transforms and options are in the script;
`baseline-results.json` records executed outputs. Baseline binary
`/home/bend/.cache/rusty-tiles-132-12d98ab-native` has SHA256
`b38d6726bb4271b725b3a08292b8b45fb9be977ac6368f70e465814d5146b334`;
its terrain source matches develop `5df15e7`. The audit establishes Float64
sampling precision loss before quantization, and supported mask behavior only
for that fixture. It does not mistake raw delivery or absent optional normals
for a format defect.

`baseline-decode.py` independently reads little-endian quantized-mesh bytes,
including optional gzip, zigzag vertex arrays, high-water indices, edge lists and
extension framing. It checks winding, incidence and exact summed domain area.
Those checks alone do not prove absence of overlapping triangles; the replacement
needs complete reference topology/directed incidence. Truncation, attribute and
index corruptions were rejected during the initial audit. This reader shares no
historical encoder or GDAL resampling implementation.

`quantized_reader.py` is an initial independent reader and literal-grid
sampler, not the replacement's acceptance runner. The early eight-test
source/lifecycle groundwork did not certify the abandoned quantized output
plan. The replacement's independent matrix and mutation controls are recorded
above rather than inferred from that groundwork.

A small baseline grid129/error1/zoom8 measurement emitted 15 tiles in 0.622 s
with 101784 KiB child peak RSS via `resource.getrusage`. It is not a fixed-option
scaling series, a whole-job memory bound or a replacement resource result.
