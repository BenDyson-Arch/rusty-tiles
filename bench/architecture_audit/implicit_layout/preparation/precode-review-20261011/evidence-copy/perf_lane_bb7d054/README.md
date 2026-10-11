# Performance lane

A standing check for slices that change output or a hot path. It has two parts:
an A/B processing benchmark and a headless Cesium runtime check. The lane blocks
a merge only on a clear regression. Every other result is a report attached to
the PR.

## Replay

Put large work directories under `~/.cache/perf-lane/work`, not `/tmp`
(a RAM-backed tmpfs). Both scripts pin themselves to CPUs 0 and 1 at nice 10;
running them through `taskset -c 0,1 nice -n 10` has the same effect.

```sh
W=~/.cache/perf-lane/work/<run>
# Processing: base and head CLIs on the deterministic corpus (generated under $W/inputs).
python3 -I bench/perf_lane/process.py --base BASE_BIN --head HEAD_BIN \
  --work $W --out $W/process.json --repeat 3 \
  [--explain 'validate:*.wallSeconds=C2 adds payload checks']
# Runtime: load each kept output in Cesium 1.146.0 (paths default to the local caches).
python3 -I bench/perf_lane/cesium.py --work $W/cesium-head --out $W/cesium-head.json \
  vector=$W/head/vector.3tz vector-explicit=$W/head/vector-explicit.3tz \
  point-cloud=$W/head/point-cloud.3tz mesh=$W/head/mesh.3tz terrain=$W/head/terrain \
  raster=$W/head/raster --hole "$(python3 -c 'import json,sys;print(*json.load(open(sys.argv[1]))["environment"]["rasterHoleDegrees"],sep=",")' $W/process.json)" \
  [--baseline $W/cesium-base.json]
```

`python3 bench/perf_lane/corpus.py DIR` only generates the inputs. The corpus
uses only the Python standard library. PNG and TIFF data are stored
uncompressed, and noise comes from a fixed LCG, so the bytes do not depend on
the local zlib, GDAL or NumPy. `PINNED` in `corpus.py` holds the expected
sha256 of each input, and a mismatch stops the run.

| Input | Content | Cases |
| --- | --- | --- |
| `vector.geojson` | 3,600 polygons (720 with holes), 400 lines with 100 vertices each, 6,000 points near Rome | `vector`, `vector-explicit` (`--jobs 2`) |
| `points.las` | LAS 1.2 PDRF 3, 200,000 points, UTM 56S | `point-cloud`, `point-cloud-explicit` |
| `mesh.glb` | Textured Y-up height field: 96,800 triangles and a 512² PNG | `mesh-local` (`--leaf-triangles 20000`), `glb-wrap`, `mesh`, `mesh-explicit` |
| `tests/fixtures/t1-multipatch-plane.tif` | Repository T1 DEM fixture, 33×17 | `terrain-fixture` (`--cells-per-leaf 16`) |
| `dem.tif` | Float32 DEM, 1025², EPSG:4326 | `terrain` (defaults) |
| `rgba.tif` | RGBA 3000², EPSG:3857 at zoom-14 resolution, transparent central 1000² square, half-alpha band | `raster` (z10–16, `--jobs 2`; about 3,200 tiles) |

Every `.3tz` case also gets a `validate:<case>` case, which runs each binary
on its own archive. Raster and terrain need a native GDAL build. A binary
whose `doctor` does not report the command ready is recorded as skipped. So is
one whose `--help` lacks a flag the case uses. The run fails if head skips or
fails a case that base passes. Runs use `PATH=''`, `RAYON_NUM_THREADS=2` and
`PROJ_NETWORK=OFF`.

## Metrics

Processing (`process.py`): each case runs N times per binary. The order
alternates between base-first and head-first.

- `wallSeconds`: best (minimum) wall time of N runs, from spawn to exit.
- `cpuSeconds`: best user+system time.
- `peakRssMiB`: the largest `ru_maxrss` of the N runs, from `os.wait4`. This
  is the measured process only. Descendants are not summed.
- `foreignCpuSeconds`: busy time on the pinned CPUs during a run, minus the
  run's own CPU time. Other agents also pin to CPUs 0 and 1. A large value
  means the wall time is not a clean measurement, so rerun when the CPUs are
  quiet or choose another pair with `--cpus`.
- `outputBytes`: archive bytes, or the sum of file bytes for a directory.
- `members` and `tiles`: zip members, or directory files. `tiles` counts
  `.glb/.b3dm/.pnts/.i3dm/.cmpt/.png/.subtree` members. `byExtension` gives the
  breakdown.
- `digest`: sha256 of the archive, or of a tree. `identicalOutput` compares
  base and head. Default vector archives include volatile diagnostics, so they
  differ run to run.

Runtime (`cesium.py` with `cesium.cjs`): each case runs in a fresh Chromium
1243 with SwiftShader (`--use-angle=swiftshader`), at 1024×768. Cesium is
1.146.0 and Playwright 1.63.0. The local server binds 127.0.0.1. Any other
request is aborted and counted.

- 3D Tiles: `Cesium3DTileset.fromUrl` with `maximumScreenSpaceError` 16. The
  camera is `viewBoundingSphere(root sphere, HeadingPitchRange(-0.8, -0.65,
  max(3r, 5)))`, the preview's framing. The case is loaded once `tilesLoaded` is
  true and `allTilesLoaded` has fired, for five consecutive frames.
  `.3tz` archives are unpacked and served as files, as the archive viewers do.
- Raster: `UrlTemplateImageryProvider` on `tiles/{z}/{x}/{y}.png`, with min
  and max zoom and bounds from `tilejson.json`. The camera is
  `setView({destination: bounds})`. The globe base colour is magenta. The case
  is loaded once `globe.tilesLoaded` is true and the tile queue is empty for
  five frames. With `--hole`, a second `hole-edge` view sits near maximum zoom
  across the hole's west edge, where opaque and hole-boundary tiles meet.
  `--inset DEG` shrinks the provider rectangle by DEG on each edge. Use it only
  as a labelled workaround: Cesium 1.146 throws in
  `ImageryLayer._createTileImagerySkeletons` when the bounds lie exactly on
  Web Mercator tile edges, and `rusty-tiles preview` uses the exact bounds.
- `secondsToLoaded`: from `fromUrl`, or from adding the imagery layer, until
  loaded.
- `requests`: server-side counts and bytes per category. The categories are
  `manifest` (.json), `subtree`, `content` (glb/b3dm/pnts/i3dm/cmpt) and
  `imagery` (png/jpg/webp). `duplicateRequests` counts repeated paths.
  `runtimeRequests` counts Cesium's own files. `imageryColorTypes` gives the
  PNG IHDR colour types of the tiles served. For mesh archives, the PNG
  textures land in `imagery`.
- `tilesLoadedEvents`, `selectedTiles` (private `_selectedTiles`, diagnostic
  only) and `tilesetMemoryBytes` (`totalMemoryUsageInBytes`).
- `usedJSHeapSize` comes from `performance.memory`.
- `console.errors` holds console errors and page errors. `console.warnings`
  holds warnings. `failedRequests` holds aborted requests and HTTP ≥ 400.
  `externalRequests` holds non-local attempts.
- Raster render probe: the page reads the canvas back
  (`preserveDrawingBuffer`) before and after adding imagery, with the same
  camera. A pixel counts as content when an RGB channel differs from the bare
  globe by more than 8. The `inside-hole` probe takes a 15×15 grid inside
  `--hole` (degrees), inset 10%, and must be at most 5% content. The
  `outside-hole` probe takes the same grid over the bounds, skipping the hole
  plus a 10% margin, and must be at least 95% content. Each probe needs at
  least 20 on-screen samples. `changedPixelFraction` is the content fraction
  over the whole canvas.
- Screenshots are written to `--work/screenshots/<case>.png`, with
  `<case>-<view>.png` canvas captures for raster views. `captureSha256`
  records their digests. Software rendering demonstrates behaviour, not GPU
  performance.

## Gate

The scripts implement this rule. A run exits non-zero on a regression, and the
JSON lists it under `gate`.

- `process.py` flags a case when head/base exceeds 1.25 and the absolute
  growth exceeds a floor. The floors are 0.1 s for best wall time, 16 MiB for
  peak RSS and 0 bytes for output bytes. It also flags a case that base passes
  and head fails or skips. A matching `--explain CASE.METRIC=REASON` (an
  fnmatch pattern) records a stated reason, and that flag no longer fails
  the run.
- `cesium.py` always fails a case on a load failure or timeout (120 s by
  default), a page or console error, a tile or imagery failure, a failed or
  external request, or a failed render probe. Console warnings are recorded
  but do not fail. With `--baseline`, it also flags these metrics by case name
  when they exceed 1.25× and the floor: `requests` (+2), `requestBytes`
  (+64 KiB) and `secondsToLoaded` (+1 s).

Anything else, such as improvements, sub-floor noise or explained growth, is
reported, not gated.

## Recorded runs

Recorded on 2026-10-11 under `results/`. Screenshots stay in the work
directories and are not committed.

| File | Run |
| --- | --- |
| `baseline-process.json` | Noise floor: f2238cd (native develop) against itself, `--repeat 3` |
| `baseline-cesium.json` | Cesium on the baseline head outputs: defaults, explicit variants, terrain and raster |
| `c2-process.json` | f2238cd against the C2 portable release snapshot (sha256 `41a07d60…`). Raster and terrain are explained skips. The validation slowdowns are left unexplained so they stay visible. |
| `r2-develop-raster.json` | Current-develop R2 raster output (8192² source, z9–14) used below |
| `r2-cesium-exact-bounds.json` | The three R2 outputs with exact TileJSON bounds, as preview loads them. All fail with the Cesium skeleton error. |
| `r2-cesium-inset-bounds.json` | The same outputs with `--inset 1e-7`. All load, pass the probes and render pixel-identical captures. |

## Not covered

- GPU frame rate, hardware rendering and real network latency.
- Interaction at finer zoom: only one framing per case is checked.
- Aggregate process-tree memory.
- Pixel-exact correctness of 3D content: only load, errors and requests are
  checked.
- Non-RGBA raster displays, such as gray.
- LAZ, GeoPackage and other OGR inputs.
- Python wheel and Blender paths.
