# Performance lane

A standing check for slices that change output or a hot path. It has two parts:
an A/B processing benchmark and a headless Cesium runtime check. The lane blocks
a merge only on a clear regression. Every other result is a report attached to
the PR.

## Report

`report.py` is the normal entry point. It runs both scripts below and compares
the head CLI with the latest release (`v0.3.0`) and with the last milestone,
then rewrites `REPORT.md`.

```sh
python3 -I bench/perf_lane/report.py --head HEAD_BIN [--head-commit SHA] \
  [--explain 'validate:*.wallSeconds=C2 adds payload checks'] [--milestone-label 'C3 merged']
```

- Cesium runs in a GPU browser (see Runtime below). Before anything else,
  `report.py` runs `cesium.py --check-renderer`: it stops, before the long
  processing step, when the browser is unreachable or its WebGL renderer is
  not a GPU. `--cdp-url`, `--launch`, `--software-gl` and
  `--request-diagnostic` are passed through to `cesium.py`. `meta.json`
  records the browser (`browser`: mode, CDP endpoint, version, viewport,
  window size, whether the tab stayed visible, cache disabled) and the
  WebGL renderer (`renderer`: `UNMASKED_RENDERER`, vendor and class), and
  `REPORT.md`'s header repeats them.
- The release binary is built once with `--release --features
  native-geospatial` from `git archive` of the tag, under the shared cargo lock
  on CPUs 0 and 1 (`--cargo-wrap`). It is cached in
  `~/.cache/perf-lane/baselines/<tag>/` (`BUILD.json` records how) and reused
  after that. A corpus case whose command or flags the release CLI lacks
  shows `n/a`, with the reason under `## Not run or failed`.
- The last milestone's per-case numbers come from the data block at the end of
  the committed `REPORT.md` (git `HEAD`). If that version already records the
  head being rendered, the next older version is used instead. `--milestone-label` makes this run the
  new milestone and appends a `## Milestones` row. Without it, the stored
  milestone and the rows carry forward unchanged. Hand edits such as
  whitespace or note lines are tolerated, but a missing, unclosed or garbled
  data block, or no row for its commit, stops the run.
- Benchmarks run on a hyperthread sibling pair from CPUs 4-7 or 12-15
  (`--cpu-pool`; the cargo build locks use 0-3 and 8-11), not on the scripts'
  default 0 and 1. Before each step (each process.py attempt, each Cesium pass)
  a 5-second `/proc/stat` sample must show both cores at least 90% idle
  (`--min-idle`). The previous pair is kept while it qualifies; otherwise the
  idlest qualifying pair is used, and if none qualifies the step waits. The
  per-core idle figures go into the report header. The global load average is
  not used, because parallel lanes keep it high. `--cpus A,B` fixes the pair.
- process.py is rerun (up to `--attempts`) while any best-wall sample is
  contended (other work above 20% of its wall time on the pinned CPUs). The
  cleanest attempt is kept, and any remaining contended cases are named in the
  header.
- Raw results go to `~/.cache/rusty-tiles-evidence/perf/<date>-<shortsha>/`:
  the process and Cesium JSON, run logs, screenshots and `meta.json`, plus
  `raw.tar.zst`, whose sha256 is printed in `REPORT.md`. The stdlib `zstd`
  writer needs Python 3.14 or later. Older Pythons use the `zstd` CLI, or else
  write `raw.tar.gz`. Outputs stay under `--work` and are deleted unless
  `--keep-work` is given. `--from-raw DIR` re-renders a recorded directory without running anything, applying the
  `--explain` reasons recorded in its `meta.json` plus any given again.
- Cesium timings are compared only between runs with the same renderer
  class (`gpu` or `software`). The milestone data block records the
  renderer; blocks and `## Milestones` rows from before it was recorded ran
  on SwiftShader. When the classes differ, the last milestone's Cesium
  timings show as `software, superseded` with no delta or flag, and request
  counts and load results are still compared. Software rows stay in
  `## Milestones`, marked superseded once a GPU milestone exists, and a GPU
  re-render of a commit adds a row beside its software row instead of
  replacing it.
- Flags follow the gate below and are measured against the last milestone. Flags
  against the release are reported but not gated. The script exits non-zero on
  a gated regression.

When to run it:

- **Per PR** that changes output or a hot path: run `report.py` on the PR's
  release build and paste the `## Flags` section and the affected case tables
  into the PR body. Do not commit the regenerated `REPORT.md`
  (`git checkout bench/perf_lane/REPORT.md`).
- **On merge to develop** of an output-affecting slice: run it on a release
  build of the merged develop with `--milestone-label '<slice> merged'` and
  commit the regenerated `REPORT.md`. Its git history is the trend log. Commit
  a milestone only when `report.py` exits 0, or when every remaining gated flag
  has a stated reason: re-render with `--from-raw DIR --explain
  CASE.METRIC=REASON`, with no rerun.

Raw results never go into git: `.gitignore` admits only the scripts,
`README.md` and `REPORT.md` under `bench/perf_lane/`.
`python3 -I bench/perf_lane/test_report.py` unit-tests the table, delta,
flag and renderer-comparability logic. `python3 -I
bench/perf_lane/test_cesium.py` unit-tests the finish rule (through `node`),
the renderer check and the Cesium gate.

## Replay

Put large work directories under `~/.cache/perf-lane/work`, not `/tmp`
(a RAM-backed tmpfs). Both scripts pin themselves to CPUs 0 and 1 (`--cpus`)
at nice 10; running them through `taskset -c 0,1 nice -n 10` has the same
effect. For `cesium.py` this pins only the driver: the attached browser runs
wherever its owner started it. `cesium.py` takes `--cesium`, `--node-modules`
and `--chromium`, which default to `CESIUM_DIR`, `NODE_PATH` and `CHROMIUM`,
or else to the local caches used for the recorded runs. `--chromium` is used
only with `--launch` or `--software-gl`.

`cesium.py` needs a running GPU Chromium with a DevTools port:

```sh
chromium --remote-debugging-port=9222 --user-data-dir=$HOME/.config/chromium-perf
python3 -I bench/perf_lane/cesium.py --check-renderer   # prints the renderer facts; exit 3 if unusable
```

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
the local zlib, GDAL or NumPy. They do depend on the C library's `sin` and
`cos`, which shape the LAS heights, the mesh heights and the DEM. A different
libm (musl, macOS) may fail the pin. `PINNED` in `corpus.py` holds the expected
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

Processing (`process.py`): each case runs N times per binary in ABBA order
(base first in rounds 1 and 4, head first in rounds 2 and 3).

- `wallSeconds`: best (minimum) wall time of N runs, from spawn to exit.
- `cpuSeconds`: best user+system time.
- `peakRssMiB`: the largest `ru_maxrss` of the N runs, from `os.wait4`. This
  is the measured process only. Descendants are not summed.
- `foreignCpuSeconds` (per sample): busy time on the pinned CPUs during a run,
  minus the run's own CPU time. The summary gives `foreignCpuSecondsAtBestWall`
  (the sample that set `wallSeconds`) and `foreignCpuSecondsMax`. Other agents
  also pin to CPUs 0 and 1. A large value means the wall time is not a clean
  measurement, so rerun when the CPUs are quiet or choose another pair with
  `--cpus`.
- `outputBytes`: archive bytes, or the sum of file bytes for a directory.
- `members` and `tiles`: zip members, or directory files. `tiles` counts
  `.glb/.b3dm/.pnts/.i3dm/.cmpt/.png/.subtree` members. `byExtension` gives the
  breakdown.
- `digest`: sha256 of the archive, or of a tree. `identicalOutput` compares
  base and head. Default vector archives include volatile diagnostics, so they
  differ run to run.

Runtime (`cesium.py` with `cesium.cjs`): Cesium 1.146.0 and Playwright
1.63.0, rendering on the GPU.

- Browser. By default the lane attaches over CDP
  (`chromium.connectOverCDP`) to a running GPU Chromium: `--cdp-url`, else
  `RUSTY_TILES_PERF_CDP`, else `http://localhost:9222`. If nothing listens
  there, the script stops and says how to start one. Each run opens its own
  browser context and tab at 1024×768 (device pixel ratio 1), disables the
  cache over CDP (`Network.setCacheDisabled`), so every run is cold, and
  brings the tab to the front (`Page.bringToFront`). It closes only that
  context. It never closes the browser or touches other tabs. A tab that is
  not `visible` before the load, or that turns hidden during it, fails the
  case, because background tabs throttle `requestAnimationFrame`. A fresh
  window occasionally aborts its first navigation (`net::ERR_ABORTED`);
  that is retried up to twice and counted in `navigationRetries`.
- Every browser run holds `~/.cache/rusty-tiles-chromium.lock` (`flock`),
  since other agents drive the same browser. Waiting for the lock does not
  count toward a case's budget.
- `--launch` starts a private headless Chromium (`--chromium`) instead,
  with ANGLE on the GPU (`--angle gl`, the default, or `gl-egl` or
  `vulkan`). Chromium's resolver then also maps every host except
  127.0.0.1 to NOTFOUND.
- Renderer check. A preflight page reads WebGL `UNMASKED_RENDERER_WEBGL`. A
  software renderer (SwiftShader, llvmpipe, lavapipe, softpipe, Microsoft
  Basic Render) stops the run with exit 3, unless `--software-gl` is given.
  `--software-gl` implies `--launch` with `--use-angle=swiftshader`, and its
  results are labelled `software` and never timed against GPU runs. Every
  pass records its renderer. A renderer that differs from the preflight one
  fails the case.
- The local server binds 127.0.0.1. Any other HTTP request is aborted and
  counted, and every WebSocket is closed and counted.

Why GPU: under SwiftShader about 98% of a fully refined terrain load
(35–37 s) was CPU rasterisation, at about 10.5 ms per 8,192-triangle tile
per frame. The same load takes 0.69–0.80 s on the RTX 4080
(`~/.cache/rusty-tiles-evidence/perf/terrain-zoom-20261012/findings.md`).

`--timeout` (120 s by default) bounds each asynchronous browser step: page
load, the Cesium global, `fromUrl` or the TileJSON fetch, each settle and the
screenshot. The case budget is that timeout times the step count, plus 60 s
for browser start. A tileset case has 5 steps. A raster case has 5 steps plus
2 per view. When a
case overruns its budget, its process group is killed and the case fails with
`timedOut`, while the run continues.

- Finish rule. A load is done when, on every rendered frame for 250 ms of
  wall time (`--settle-ms`), all of these hold:
  - the content is loaded: for 3D Tiles, `tilesLoaded` is true and
    `allTilesLoaded` has fired; for raster, `globe.tilesLoaded` is true and
    the tile queue is empty;
  - no requests are pending: no pending or processing tileset requests, no
    active `RequestScheduler` requests and no in-flight XHR or fetch.

  The reported time is when that state began. The confirmation window is
  not included, and `secondsToSettled` records the time including it. Any
  frame that breaks the state restarts the window. The window is wall time,
  not a frame count. The previous rule, five consecutive frames, cost
  31–37% of a SwiftShader terrain load, because each of those frames redrew
  every tile.
- 3D Tiles: `Cesium3DTileset.fromUrl` with `maximumScreenSpaceError` 16 (the
  default camera). The camera is `viewBoundingSphere(root sphere,
  HeadingPitchRange(-0.8, -0.65, max(3r, 5)))`, the preview's framing.
  `.3tz` archives are unpacked and served as files, as the archive viewers do.
  Each tileset case then loads again in a fresh context with
  `maximumScreenSpaceError` 0 (the fully refined pass, `refined` in the
  JSON, the `SSE 0` rows in `REPORT.md`). A failure in that pass fails the
  case. `--no-refined` skips it.
- Raster: `UrlTemplateImageryProvider` on `tiles/{z}/{x}/{y}.png`, with min
  and max zoom and bounds from `tilejson.json`. As in preview, the layer is
  added at startup. Each view sets the camera with `setView`: the overview uses
  the bounds. The globe base colour is magenta. A view is loaded once
  `globe.tilesLoaded` is true and the tile queue is empty for five frames.
  - With `--hole`, a `hole-edge` view sits near maximum zoom across the hole's
    west edge, where opaque and hole-boundary tiles meet.
  - `--corner-views` adds one camera per bounds corner, four finest-zoom tiles
    across.
  - `--inset` shrinks the provider rectangle by DEG, W,S,E,N or `Npx`
    finest-zoom pixels (T1's preview fix is `1/1024px`). It changes what is
    tested, so it prints a warning, adds a case `warnings` entry and records
    `spec.insetDegrees`. See the R2 bounds isolation below.
- `secondsToLoaded`: from `fromUrl`, or from adding the imagery layer, until
  loaded under the finish rule. `secondsToFirstTile`, reported separately:
  until the first tile content is loaded (`tileLoad`), or until the first
  imagery tile is received. `framesToLoaded` counts frames from the start
  of the settle to the onset.
- Request scheduling is a viewer setting, not a property of the output.
  Every gated or reported number uses Cesium's defaults (`throttleRequests`
  true, `maximumRequestsPerServer` 18, `maximumRequests` 50). Each pass
  records them under `scheduler`, with `peakActiveRequests`. Cesium issues
  requests only in its per-frame update, so a fully refined terrain load
  needs about 289/18 ≈ 16 issue frames. `--request-diagnostic` loads every
  tileset pass a second time with `throttleRequests` false and records both
  under `requestCapDiagnostic` (in `REPORT.md`, `## Request cap
  diagnostic`). That is a measurement of the cap's effect. It is never
  gated.
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
  (`preserveDrawingBuffer`) with the layer shown, then hidden, under the same
  camera. A bare-globe frame that does not settle fails the case.
  `spec` records the probe rectangles and views. A pixel counts as content when an RGB channel differs from the bare
  globe by more than 8. The `inside-hole` probe takes a 15×15 grid inside
  `--hole` (degrees), inset 10%, and must be at most 5% content. The
  `outside-hole` probe takes the same grid over the bounds, skipping the hole
  plus a 10% margin, and must be at least 95% content. Each probe needs at
  least 20 on-screen samples. `changedPixelFraction` is the content fraction
  over the whole canvas.
- Screenshots are written to `--work/screenshots/<case>.png`, with
  `<case>-<view>.png` canvas captures for raster views. `captureSha256`
  records their digests, with `<case>-refined.png` for the fully refined
  pass.

## Gate

The scripts implement this rule. A run exits non-zero on a regression, and the
JSON lists it under `gate`.

- `process.py` flags a case when head/base exceeds 1.25 and the absolute
  growth exceeds a floor. The floors are 0.1 s for best wall time, 16 MiB for
  peak RSS and 0 bytes for output bytes. It also flags a case that base passes
  and head fails or skips. A matching `--explain CASE.METRIC=REASON` (an
  fnmatch pattern) records a stated reason, and that flag no longer fails
  the run.
- `cesium.py` always fails a case on any of these:
  - a load failure;
  - a step timeout or case-budget overrun;
  - an unsettled reference frame;
  - a page or console error;
  - a render, tile or imagery failure;
  - a failed or external request;
  - a failed render probe. Console warnings are recorded
  but do not fail. With `--baseline`, it also flags these metrics by case name
  when they exceed 1.25× and the floor: `requests` (+2), `requestBytes`
  (+64 KiB) and `secondsToLoaded` (+0.25 s). Load time is compared only
  when the baseline used the same renderer class. A baseline recorded
  before renderer capture counts as software. Cesium times (first tile,
  loaded, SSE 0 loaded) stay advisory in `REPORT.md`, and imagery request
  counts depend on timing. GPU noise, measured on the two 2026-10-12
  baselines (d1874ef and b13a86d, nine cases with byte-identical outputs,
  one sample each): sub-0.4 s loads moved by up to ±30% (at most 0.09 s),
  and terrain by up to 8% (0.59/0.63 s default camera, 0.72/0.66 s SSE 0).
  That noise is why the floor is 0.25 s.

Anything else, such as improvements, sub-floor noise or explained growth, is
reported, not gated.

## R2 bounds isolation

`aligned_bounds.py` builds TileJSON variants of the develop R2 output and loads
them with `--corner-views`, first with the exact bounds and then with every
edge inset by `1/1024px`. In each variant one edge is left exact and the others
are moved 1e-7° inward; the tiles are symlinked from the output. It also tiles
three real 2048² pyramids near Rome: one aligned on every grid down to z9, one
aligned at z14 only, and one shifted 100 m as an unaligned control.

With exact bounds in Cesium 1.146.0 the result is:

| Variant | Exact edges | Result |
| --- | --- | --- |
| `r2-exact` | W (0°), S, E, N | `renderError` |
| `r2-only-west-exact` | W (0°) | loads, overview and 4 corners |
| `r2-only-north-exact` | N | loads |
| `r2-only-south-exact` | S | `renderError` |
| `r2-only-east-exact` | E | `renderError` |
| `r2-west-nudged` | S, E, N | `renderError` |
| `rome-aligned-z9`, `rome-aligned-z14-only` | all, away from 0° | overview loads; `renderError` at the SW corner view |
| `rome-unaligned` | none | loads |
| `r2-shifted-east` | all; the bounds cover absent tiles | 404 imagery failures only, so not evidence either way |

Every `renderError` has the same stack: `Rectangle.southwest` ←
`rectangleToNativeRectangle` ← `ImageryLayer._createTileImagerySkeletons` ←
`loadTile` ← `endFrame`. The error is `TypeError: Cannot read properties of
undefined (reading 'west')`.

So the trigger is a provider rectangle whose south or east edge lies exactly on
an imagery tile edge. The 0° meridian is not the trigger: a west edge on 0°,
or a north edge, loads by itself. This matches `positionToTileXY`, which
truncates, so a boundary point maps to the tile east or south of it. The
skeleton then takes the outside neighbour, its intersection with the
rectangle is undefined, and rendering stops.

Insetting every edge by 1/1024 of a finest-zoom pixel (T1's preview fix) loads
every aligned variant, including all corner views, with no errors.
`r2-shifted-east` still shows only its 404s.

These pyramids stop at z14. There the inset is about 2.3 µm at the equator, or
roughly 600 ulps of a Web Mercator metre coordinate near ±20,037 km. The inset
halves with each zoom level, so at z24 it is only about 2.5 ulps there. That
margin is untested.

The R2 outputs in these runs were loaded by this harness. `rusty-tiles preview`
itself was not run here; it passes the exact bounds and also adds the layer at
startup, which is the path this harness exercises.

## Recorded runs

These runs were recorded on 2026-10-11 and were committed under `results/`
until `REPORT.md` replaced them. They are archived locally in
`~/.cache/rusty-tiles-evidence/perf/legacy-results.tar.zst` (sha256
`b600ab4a…`). Each file records the sha256 of the scripts that produced it. The
first `REPORT.md` milestone was seeded from `baseline-process.json` (head side)
and `baseline-cesium.json`.

| File | Run |
| --- | --- |
| `baseline-process.json` | Noise floor: f2238cd (native develop) against itself, `--repeat 3`. Wall time is within ±2.3% and RSS within ±4.4%. |
| `baseline-cesium.json` | Cesium on the baseline head outputs: defaults, explicit variants, terrain and raster |
| `c2-process.json` | f2238cd against the C2 portable release snapshot (sha256 `41a07d60…`). Raster and terrain are explained skips. The validation slowdowns of 3.7–9.9× (up to 1.13 s) are left unexplained so they stay visible. Base is a native build and head a portable one, which confounds the RSS and producer-time deltas. |
| `r2-develop-raster.json` | Develop f2238cd R2 raster output (8192² source, z9–14) used below |
| `r2-cesium-exact-bounds.json` | The three R2 outputs (old GDAL, exact Fast, develop) with exact TileJSON bounds. All fail with the `renderError` above. |
| `r2-cesium-inset-bounds.json` | The same outputs with `--inset 1/1024px`. All load and pass the hole probes, and their captures are pixel-identical. |
| `aligned-bounds-exact.json`, `aligned-bounds-inset.json` | The R2 bounds isolation above (`aligned_bounds.py`) |

## Not covered

- Frame rate after load, other GPUs and drivers, and real network latency
  (the server is local).
- Interaction at finer zoom: 3D cases are checked at one framing, raster cases
  at the overview plus the `--hole` and corner views.
- Aggregate process-tree memory.
- Rendering of 3D content: only load, errors and requests are checked, with no
  render probe.
- Non-RGBA raster displays, such as gray.
- LAZ, GeoPackage and other OGR inputs.
- Python wheel and Blender paths.
