# Terrain T1 audit and acceptance evidence

> **Bulk evidence moved out of the tree (2026-10-12).** 1 of 15 files remain:
> this README plus what CI or tests read. The full directory last existed at
> `e29cd19d2f236dc688e91a80a45170d4a573e20b`
> (`git archive e29cd19d2f23 bench/architecture_audit/terrain_acceptance | tar -x`). Local archive:
> `~/.cache/rusty-tiles-evidence/architecture_audit/terrain_acceptance.tar.zst`, SHA-256
> `91d0ded63e05517840f5d4f4e06d70f9dbbbf0cba44eb52ff2dd3f36188e57cb`.
> Paths below that no longer exist refer to that snapshot.

The replacement implementation has passed the focused local checks below;
final PR CI and release gate #113 remain separate acceptance gates. The
[representation decision](../../../docs/architecture/terrain-representation-decision.md)
and [terrain contract](../../../docs/TERRAIN.md) record the finite standard
replacement, breaking changes and remaining limits.

## Approved finite 0.4 bar — 2026-10-24

The terrain half of the owner-approved non-mesh rescope retains native T1 and
packed LOD. Functional source: `1c760362abc3d8d4cc80e0943e13872c63fbad92`.
The GTiff driver is checked before source open: missing capability is
`Unsupported` in Rust, CLI and doctor; malformed data with GTiff present remains
`InvalidInput`. No Python, portable or projected-terrain capability is added.

Fresh tests retain plane, Float64, NoData, shared lattice/seams, discrete LOD,
skirt/bounds and publication/lifecycle guarantees. The Python oracle additionally
checks absolute bump heights against independently evaluated bilinear source
samples and an exact oriented-cell inventory for every decoded tile. A duplicate
face plus hole is rejected by that topology census without the plane oracle.
The 0.05 m leaf storage ceiling and absence of a continuous-DEM accuracy promise
remain unchanged.

Fresh checks: 23 terrain units, 10 native terrain, 12 native doctor, 13 preview
server and five Python terrain tests passed. Portable terrain (2) and doctor
(11), native/portable all-target clippy with warnings denied, formatting and
changelog checks passed. One 2048-square Float32 Injalak DEM run (64 cells/leaf,
internal NoData hole) used 123,204 KiB / **120.316 MiB** peak child RSS against
512 MiB. Peak sampled file allocation was **64.387 MiB** against 2 GiB scratch;
whole final scratch allocation including directories was 38,637,568 bytes.
Samples and a logical persistent-file estimate are separately labelled, not
called exact instantaneous disk maxima. The run emitted 1,365 meshopt/quantized
tiles; probe elapsed was 33.248 s (including post-exit inventory hashing),
after a 503.3 s measurement-lock wait. Independent nonauthor review supports
this bounded terrain acceptance with no blocking production defect. Input hash was preserved,
no staging remained, the task scratch was deleted, and Cargo clean removed
3.6 GiB from its dedicated target.

Full commands, native versions, final binary/source hashes, resource preparation
and receipts are in `~/.cache/rusty-tiles-evidence/040-bar-terrain/README.md`.
The frozen `receipt-manifest.json` in that directory pins those execution files
and README by SHA-256; manifest SHA-256:
`b4a54a407d4bb2a26a39198ee5eac96d1530509434275ca11fefc32c22c38616`.
The tested functional commit is the pin above; subsequent changes are receipts,
a tooling-contract amendment and assertion formatting only. The required
`chrt -i 0 nice -n 19` recipe was used; observed policy was SCHED_IDLE. Because
the harness inherited nice -8, the relative nice increment produced nice 11;
the receipt preserves both invocation and observation.

This finite receipt does not close the imagery half of #124 or authorize a
release. Exhaustive native-lifetime/oracle/hostile-input resource audits remain
0.4.x work; new GDAL-free/projected terrain (#82/#221/#222) remains targeted at
0.5. Browser, installed-wheel, Blender and other-platform release checks are
not rerun here; this task explicitly excludes browsers and mesh bakes.

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
