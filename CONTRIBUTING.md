# Contributing to rusty-tiles

Bug reports, documentation improvements and focused code contributions are welcome. For a large change or new format, open an issue first so we can agree on scope and fidelity requirements.

## Report a problem

Include the command you ran, expected and actual results, rusty-tiles version, operating system, and relevant Rust/GDAL/Python versions. For rendering problems, include the viewer version and whether hardware acceleration is enabled. Share the smallest synthetic or openly licensed reproduction you can make.

Do not upload private survey data, sensitive locations, personal metadata, access tokens, or data you do not have permission to redistribute. Redact file paths and credentials from logs. Private datasets are not required for the default test suite.

## Branches and pull requests

| Branch | Purpose | How changes arrive |
| --- | --- | --- |
| `main` | Stable, releasable code; version tags are cut here | Release PR from `develop`, or a focused hotfix PR |
| `develop` | Integration branch for the next release | Feature, fix, test and documentation PRs |
| `feat/<description>`, `fix/<description>`, `docs/<description>`, `chore/<description>` | Short-lived work | Branch from `develop`; open a PR back to `develop` |
| `hotfix/<description>` | Urgent correction to a released version | Branch from `main`; PR to `main`, then sync `main` back into `develop` |

External contributors should fork the repository; repository write access is not required. For example, after cloning your fork and adding this repository as `upstream`:

```sh
git fetch upstream
git switch -c fix/describe-the-change upstream/develop
# Make and test your changes.
git push -u origin fix/describe-the-change
```

Open a PR targeting `develop`. Describe the problem, resulting behavior, validation, and any fidelity or performance tradeoff. Keep unrelated changes separate. Follow the existing code style; no special commit-message convention or contributor agreement is required.

Maintainers normally squash short-lived contribution PRs. Release promotions from `develop` to `main` use a merge commit so the shared branch history is preserved. After a release or hotfix, merge `main` back into `develop` through a PR before the next promotion. Do not rebase or force-push either shared branch. Delete your short-lived branch after merging; retain `main` and `develop`.

## Build and check

Use current stable Rust with rustfmt, a C++ compiler and pkg-config. libjpeg-turbo is optional for local builds. Python tests require Python 3, NumPy and GDAL/GEOS; CI pins its environment in `.github/workflows/ci.yml`.

```sh
cargo fmt --check
cargo test --locked
python3 -m unittest discover -s tests -p 'test_*.py'
RUSTY_TILES_DISABLE_NATIVE_JPEG=1 cargo test --locked --lib jpeg::tests
```

For native geospatial work, install GDAL >= 3.12, PROJ >= 9.2, their headers and `pkg-config` files, SQLite development files, and libclang, then run `cargo test --locked --features native-geospatial`. CI tests the minimum GDAL 3.12 and the GDAL 3.13 stack separately from the existing Rust and Python jobs. Native CRS tests use independent coordinate references and synthetic local grids; they require no Python or grid downloads. Create native handles within each worker: spatial references and transformations deliberately cannot be sent or shared between threads. Native CRS operations use GDAL's process-wide offline policy; do not re-enable PROJ networking while they are running.

`cargo clippy --all-targets` is useful during review. Existing style warnings are not a required CI gate; avoid adding new warnings. Optional external-oracle and user-supplied-model tests are ignored by default and must be invoked explicitly. See the README for their dependencies and inputs.

For geometry, texture, coordinate or archive changes, add a regression that checks the meaningful output: triangle membership/winding, texels/materials, independent coordinate references, metadata values, archive indexing, or failure publication behavior. For preview changes, check rendering and picking using invented fixtures. For performance changes, compare the same source, settings, camera and hardware, and report memory and fidelity as well as timing. Do not claim hardware FPS from a software-rendered browser.

## Native point-cloud validation

`cargo test --locked` exercises real local LAS/LAZ CLI conversions with an empty executable path. The native feature suite also checks header/explicit CRS placement against an independent UTM/ECEF reference. `RUSTY_TILES_BIN="$PWD/target/debug/rusty-tiles" python3 -m unittest discover -s tests -p test_point_cloud.py` uses laspy/pyproj as independent fixture readers and decoded-output oracles. It checks all supported point formats, scalar types and flags, original/decoded values, first-point voxel samples, full-detail coverage, error/bounds/budgets, and rejected input cleanup. Build with `native-geospatial` for the geospatial cases.

The optional browser probe uses invented data and the existing preview. Install `scripts/point-cloud-requirements.txt`, native build dependencies, Node/npm, Playwright and Chromium, then:

```sh
cargo build --locked --features native-geospatial
mkdir -p target/cloud-browser-case
PYTHONPATH=tests python3 - <<'PY'
from pathlib import Path
from pyproj import CRS
from test_point_cloud import fixture
fixture(Path('target/cloud-browser-case/cloud.laz'), crs=CRS.from_epsg(32632))
PY
target/debug/rusty-tiles point-cloud -i target/cloud-browser-case/cloud.laz \
  -o target/cloud-browser-case/cloud.3tz --sourceCrs header --heightOffset 10 \
  --maxPoints 16 --chunkPoints 11
python3 -m zipfile -e target/cloud-browser-case/cloud.3tz target/cloud-browser-case/tiles
npm install --prefix target/cloud-browser --no-save --package-lock=false cesium@1.143.0 playwright
rusty-tiles preview --point-cloud target/cloud-browser-case/tiles \
  --cesium target/cloud-browser/node_modules/cesium/Build/Cesium --port 9271
# In another terminal:
NODE_PATH="$PWD/target/cloud-browser/node_modules" CHROMIUM=/usr/bin/chromium \
  node tests/fixtures/point_cloud.cjs http://127.0.0.1:9271
```

The probe checks the Cesium 1.143 IIFE, coarse-to-full refinement, actual rendered feature picking and original scalar properties, then repeats after a cache-disabled reload. Software-rendered Chromium verifies behavior; it does not establish hardware performance.

## Review and permissions

Both shared branches require a PR, the `Rust` and `Python` CI checks, an up-to-date branch, and resolved review discussions. Direct pushes, force-pushes and branch deletion are blocked, including for administrators under the core ruleset. Workflow tokens have read-only repository access and cannot approve PRs. CI does not receive deployment secrets.

The separate `main` review rule requires one code-owner approval and dismisses stale approvals when code changes. CODEOWNERS requests review from `@BenDyson-Arch`. While the project has one maintainer, repository administrators may bypass **only this approval rule, and only through a PR**, for their own changes. The PR and CI requirements remain enforced. Other contributors cannot merge without repository write permission. Review the exception when adding more maintainers.

Maintainers create versioned releases from verified commits on `main`. A passing contribution does not automatically publish a release or grant repository permissions. Contributions are licensed under the project's MIT license.


## Opt-in public point-cloud validation

The public Autzen source has 10,653,336 classified points. The download helper
records its CC BY 4.0 license and attribution. Its horizontal coordinates are
international feet and its NAVD88 heights are US survey feet; the helper converts
both to local metre XYZ and removes CRS declarations. This validates local point
conversion and metadata fidelity, without claiming an ellipsoidal datum transform.

```sh
python3 scripts/public_data.py autzen /path/to/cache
rusty-tiles point-cloud -i /path/to/cache/autzen-local-metres.las \
  -o /path/to/cache/autzen-local-metres.3tz --sourceCrs local \
  --maxPoints 50000 --chunkPoints 100000
python3 scripts/audit_point_cloud.py /path/to/cache/autzen-local-metres.las \
  /path/to/cache/autzen-local-metres.3tz
```

On 2026-10-05 the debug-build conversion took 31.9 seconds and 181.8 MiB peak
subprocess RSS, producing 613 tiles (307 leaves). A full leaf audit matched every
numeric LAS field and found every source record exactly once. The maximum
reported local float32 position rounding was 0.0000337 m. These measurements
apply to one machine/run, not a performance guarantee. Downloads and generated
archives stay outside the source tree and are not run in CI. NumPy,
`laspy[lazrs]` and pyproj are required for preparation; the audit uses an
uncompressed LAS file for memory-mapped source access.

Source and attribution: [PDAL Autzen data](https://github.com/PDAL/data/tree/main/autzen),
[CC BY 4.0 license](https://github.com/PDAL/data/blob/main/LICENSE).

## GeoPackage diff compatibility

Replacement belongs to the tiler; diff creation/application stays in external
libraries. The optional test uses upstream C++ geodiff and the local Go port:

```sh
# Run from a go-geodiff v0.4.3 or newer checkout to resolve its Go module.
go build -o /tmp/go-geodiff-driver /path/to/rusty-tiles/tests/fixtures/geodiff_driver.go
# Run from rusty-tiles with the actual upstream binary (2.3.0 tested).
GEODIFF_CPP_BIN=/path/to/geodiff GO_GEODIFF_DRIVER=/tmp/go-geodiff-driver \
  python3 -m unittest discover -s tests -p 'test_geodiff_compat.py'
```

The suite generates invented GeoPackage fixtures, checks byte-identical
changesets, cross-applies them, and compares replacement output with fresh
world geometry and scalar properties. It separately exercises GDAL spatial-index
triggers. Both implementations must successfully apply indexed geometry moves,
inserts and deletes; the resulting R-tree rows, spatial queries and reused tiles
are compared with the fresh GDAL source. Older Go versions that lack the spatial
index functions fail this regression instead of being skipped.
No upstream source or database fixtures are bundled; CI's core replacement tests
run without external diff binaries.

## Native runtime and development oracles

Run `rusty-tiles doctor` before starting a job, or select converters explicitly:

```sh
rusty-tiles doctor --command vector --command terrain
rusty-tiles doctor --command point-cloud --json
```

The native, read-only check reports linked GDAL/GEOS/PROJ versions and capabilities,
PROJ database readiness, effective search paths and top-level local grid files.
It does not import Python, install dependencies or fetch grids. Listing a grid
is not proof that a particular height operation is available: conversion still
validates its source-specific, offline, only-best operation.

Selected readiness is independent of unrelated commands. Mesh/archive utilities
and local XYZ point clouds work in the default build; unavailable geospatial
commands report the missing `native-geospatial` feature. With that feature,
`raster` checks COG/GTiff/PNG and the raster tile algorithm, and `vector` checks
GEOS triangulation. Missing native capabilities/database produce environment
exit code 4 with the inventory retained in the single `--json` result. Missing
source-specific grids are reported by conversion, also as environment errors.

Known-good, exact Python profiles tested on 2026-10-05 are in
`scripts/gdal-requirements.txt` (development vector/raster/terrain oracles) and
`scripts/point-cloud-requirements.txt` (development LAS/LAZ fixtures and independent audits). Both require Python 3.12 or newer. Point-cloud, terrain, raster and vector conversion themselves have no Python dependency.
The GDAL profile requires matching GDAL 3.13.3 native headers/libraries and a GEOS
build supporting constrained triangulation; a pip binding cannot replace those
system libraries. Install a profile into a suitable environment explicitly:

```sh
python3 -m pip install -r scripts/gdal-requirements.txt
python3 -m pip install -r scripts/point-cloud-requirements.txt
rusty-tiles doctor --json
```

These are reproducible reference profiles, not the only supported environments.
CI also checks the conda-forge GDAL 3.12 stack. PROJ grids and their licences remain
separate from Python requirements; use the grid inventory and conversion's precise
operation check when selecting a height reference.

## Conversion diagnostics

Native data/conversion errors use the stable machine result categories. Failed
conversions publish nothing. For compatibility, `RUSTY_TILES_PYTHON_TRACEBACK=1`
now enables native GDAL geometry warnings; no interpreter or traceback is involved.
The frozen Python fixtures remain development oracles and are never embedded.

## Replacing outputs

All conversion commands accept `-f`/`--force`. Without it, existing outputs are
rejected with the correct option hint. Archive replacement uses the existing
atomic file publication. Raster/terrain stage the complete directory first, then
swap it with a private backup of the old output; publication failure restores the
backup. This directory swap has a brief rename gap. If restoration itself fails,
the diagnostic names the retained backup rather than deleting it. A conversion
failure before publication leaves the original file/directory untouched.

### Calling the CLI from another program

Pass the global `--json` flag before or after the command to receive exactly one
JSON result on stdout. Human diagnostics remain on stderr. Successful conversion
results include `ok`, `output`, numeric `counts`, `skippedFeatures`, `reuse`, and
`conversionReport` (a directory path or an archive/entry pair). Converters without
a conversion report return empty counts and null report fields. `doctor --json`
returns the dependency inventory with `ok`; failures retain that inventory.

Failures include `error.code`, `error.message` and `exitCode`. Stable exit codes
are 0 (success), 1 (I/O or subprocess failure), 2 (usage), 3 (input/data), 4
(environment: Python, dependencies or unavailable strict CRS operation), and 5
(existing output without `--force`). Help and version requests exit successfully
with their ordinary text. Programs should inspect the code rather than parse the
message. An unexpected subprocess termination is a data failure unless the
subprocess supplied a more specific category.

`--progress json` writes newline-delimited JSON events to stderr: `event`, `phase`,
`done`, `total`. Unknown totals are null. Every converter emits conversion start
and completion; vector ingestion and encoding also emit intermediate phase events.
Other stderr lines remain human diagnostics: consume only JSON lines with
`event: "progress"` or `event: "failed"`. Completion is emitted only after output
publication succeeds. This reports work units, not an estimated time remaining.

### Validating a published archive

Run `rusty-tiles validate out.3tz` (or add `--json` for CI). Validation is read-only:
ZIP CRCs and the complete 3TZ index are checked, then the bundled upstream tileset
schema, child bounds, non-increasing geometric error, local content/resource
references, hash-named payload checksums, build-state checksum, unused entries,
and recorded encoded-byte/vertex/point/tile budgets. Equal geometric errors are
allowed for routing nodes and parents whose child error is the larger bound.

The built-in path covers explicit, self-contained archives containing GLB, glTF,
b3dm and external tileset JSON. Boxes and spheres use relative tile transforms;
region-to-region containment handles antimeridian crossing. Mixed region/Cartesian
bounds, implicit tiling, remote/percent-encoded URIs and other content formats are
reported as unsupported rather than silently certified. Sphere containment under
nonuniform transforms uses a conservative scale bound. This is a publication
check, not a replacement for content-extension validation in the official tool.

To additionally run a locally installed
[official validator](https://github.com/CesiumGS/3d-tiles-validator), use
`--external-validator /path/to/node_modules/.bin/3d-tiles-validator`.
The command passes `--tilesetFile` and a temporary `--reportFile`, preserves tool
logs on stderr, and rejects reported errors even if the tool exits successfully.
It never downloads or installs a validator, extracts archive entries, or modifies
input. The external check runs after the built-in checks pass; unsupported built-in
cases must be checked directly with the official tool.

External tileset roots retain the referring tile's bounds, geometric error and
hierarchy depth. A shared external tileset is checked at each placement; only
references on the active traversal path count as cycles. Local `schemaUri`
dependencies in tilesets and glTF metadata are resolved relative to their
document, checked for presence and valid JSON, and included in used entries.
Full metadata schema semantics remain part of the official validator's checks.

The schema bundle comes from Cesium GS's 3D Tiles specification at commit
`4d781014b52294759834018a931223b98ac1ce47`. Relative schema references were rewritten
to local `$defs`; source descriptions and requirements are retained. Attribution
and the upstream CC BY 4.0 notice are in `docs/schema/LICENSE.adoc`; schema loading
performs no network or filesystem reference resolution.

Validation also exposed two producer fixes: parent boxes now contain child minimum
thickness/rounding, and vector/point-cloud feature attributes use padded uint16
(or exact float32 for larger tables), with valid metadata schema identifiers.
The official 0.6.1 validator passes the standard mesh and uncompressed vector
fixtures with zero errors; it still warns about the draft vector extension it
does not implement. Runtime verification remains necessary for those extensions.

### Reproducible builds

For byte-identical vector archives, add `vector --reproducible`. This omits the
entire diagnostic `conversion.json.performance` section (timings and worker
utilization). With the same input, conversion options, encoder binary and
GDAL/GEOS/NumPy/codec dependencies, repeated conversions produce the same archive
bytes. Worker count can change without changing those bytes. Point-cloud and
plain archive packing have no timing section and are reproducible by default.
Source filesystem timestamps and the caller's archive-member order do not affect
packing: `tileset.json` is first, remaining names are sorted, the index is last,
and all members have fixed 1980-01-01 timestamps and 0644 permissions.

Without `--reproducible`, vector payloads, manifests, build state, geometry reports
and the conversion report excluding `performance` remain identical. Only that
section is volatile; its length/CRC also changes the ZIP container's offsets,
central-directory records and 3TZ index. Excluding those container records is
necessary when comparing ordinary diagnostic archives. Do not compare their raw
archive hashes for reproducibility.

The guarantee does not equate a fresh build with a reuse build: ingestion history,
reuse statistics and which geometry reports were produced differ. Different source
paths/layer identities, dependency versions, options or externally generated input
bytes can also change output. Mesh texture conversion depends on the selected
external codec/build; byte equality here is exercised for vector, point-cloud and
plain packing rather than promised across all external texture encoders.

## Native terrain validation and benchmarking

Native terrain acceptance compares every supported grid size with the original Python/GDAL development oracle in `tests/fixtures/terrain_oracle.py`, then independently decodes quantized-mesh triangles, shared edges, height endpoints and bounds. Simplification fixtures check elevation and ECEF displacement at grid nodes, edge midpoints and cell centres, preserved coverage sidecars, and reductions for flat/hill terrain. The runtime validates triangle intersections too, and falls back to the original grid when its added-error limit cannot be met. `maxMeshoptEstimateMetres` is the simplifier's estimate; the measured `maxAdded*ErrorMetres` fields come from decoded-surface validation. These limits exclude source sampling and height quantization error.

`test_native_terrain.py` and `test_terrain_overlay.py` invoke the actual CLI with an empty PATH. After building with `native-geospatial`, run the independent audit with:

```sh
RUSTY_TILES_BIN="$PWD/target/debug/rusty-tiles" python3 -m unittest discover -s tests -p 'test_native_terrain.py'
```

`tests/fixtures/terrain.cjs` exercises the real preview in Chromium using Cesium's public terrain loading and height-sampling APIs, then repeats after a cache-disabled reload. It expects a 32 × 32 EPSG:4326 DEM at `[12, .01, 0, 42, 0, -.01]`, constant 123.5 metre heights with NoData in rows/columns 12–19, converted through zoom 9 with `--heightOffset 10.25 --fillHeight -999.125`. Pass the local preview URL and set `NODE_PATH` to an installed Playwright module directory. Software rendering confirms compatibility rather than performance. The terrain runtime requires the `native-geospatial` feature, never Python.

The repeatable conversion benchmark is `tests/fixtures/benchmark_terrain.py`. Build the original converter at `9a95862` in a separate worktree and this branch with `cargo build --release --locked --features native-geospatial`, using separate Cargo target directories. Put the development Python/GDAL environment on `PATH` for the original CLI, then run:

```sh
python3 tests/fixtures/benchmark_terrain.py \
  --old-bin /path/to/original/release/rusty-tiles \
  --new-bin target/release/rusty-tiles \
  --work target/terrain-benchmark --repeats 3
```

The harness creates deterministic flat, smooth regional and projected rugged DEMs, compares old Python output with native full-grid and 1 metre simplified output, and writes raw samples plus geometry/coverage checks to `results.json`. It measures serial release CLI runs after a warmup, including publication, with rotated method order. Linux `wait4` peak RSS records the largest process, rather than summed simultaneous process-tree memory. The original uses its default GDAL cache; the native converter uses its 64 MiB cache. Fixtures are synthetic, and filesystem caches remain warm; these results do not establish cold-storage throughput or rendering speed.

A recorded run of the initial native implementation at `62091ea`, including raw timing/RSS samples, input hashes, binary hashes, dependency versions and output checks, is in [`tests/fixtures/terrain_benchmark_results.json`](tests/fixtures/terrain_benchmark_results.json). That initial implementation substantially reduced smooth terrain, but incurred a large conversion-time cost on rugged terrain for a smaller size reduction. Use `--maxError 0` when preserving the original grid and conversion throughput matter more than reducing terrain payloads.

To compare 1 metre simplification performance with the initial native converter at `62091ea`, use the same harness with `--mode performance`. Build that revision in a separate worktree and pass its release binary as `--old-bin`:

```sh
python3 tests/fixtures/benchmark_terrain.py --mode performance --old-ref 62091ea \
  --old-bin /path/to/prior-native/release/rusty-tiles \
  --new-bin target/release/rusty-tiles \
  --work target/terrain-performance --repeats 3
```

This compares the prior native 1 metre path with the new single-worker and four-worker paths, and requires identical terrain payloads, coverage sidecars, manifests and geometry counts. Serial and parallel conversion reports must also match. Encoding batches hold at most four grids; native GDAL datasets and warps remain on the sampling thread. Retry pruning avoids identical meshopt candidates, and surface validation avoids rechecking unchanged source edges while still checking changed triangle intersections.

The optimized run is recorded in [`tests/fixtures/terrain_performance_results.json`](tests/fixtures/terrain_performance_results.json). With four encoders, median warm release times improved from 0.118 to 0.064 seconds (small flat), 7.161 to 5.332 seconds (regional smooth), and 27.224 to 5.355 seconds (projected rugged), with byte-identical terrain payloads and unchanged geometry counts/coverage. The same rugged case improved to 14.968 seconds with one worker; peak RSS with four workers rose from 99.3 to 104.2 MiB. An additional run with the default Rayon configuration completed in 5.282 seconds at 113.9 MiB. Sampling remains serial, and memory measurements retain the largest-process `wait4` definition above.

## Native raster validation

Native raster acceptance runs the real CLI with an empty executable `PATH`, checks decoded PNG pixels and source COG bands/masks/NoData/metadata, and verifies successful replacement, rollback, categorized JSON errors, progress and repeatability. `tests/fixtures/raster_oracle.py` preserves the former Python implementation for development comparisons on the same GDAL stack. RGB with explicit alpha, numeric grayscale, projected grayscale and byte grayscale fixtures compare every decoded tile and recipe/TileJSON output. Native GDAL 3.12/3.13 CI also creates and converts an invented raster without Python bindings. Display resampling follows the installed GDAL tile algorithm defaults, as before; native output baselines require the same GDAL/PROJ/codec versions.

For release performance comparisons, build the original Python CLI from `9a95862` (its raster path is unchanged through `3f0026c`) and the current native CLI against the same GDAL/PROJ stack. The development Python environment needs NumPy, GDAL bindings and the `gdal` executable for the baseline. Then run:

```sh
python3 tests/fixtures/benchmark_raster.py \
  --old-bin target/raster-old-build/release/rusty-tiles \
  --new-bin target/release/rusty-tiles \
  --old-ref 9a95862 --work target/raster-benchmark
```

The Linux benchmark uses one warmup and at least three full CLI repetitions per method in rotated order. It compares Python defaults, a one-worker Python control and the native converter with an empty executable `PATH`. Fixture generation and fidelity checks are outside the measured interval. Each case checks every decoded tile pixel and PNG byte, TileJSON/recipes, and all source-resolution COG values, masks and band metadata. CPU includes waited descendants; peak RSS is the largest process, not aggregate process-tree memory. Recorded measurements are in [`tests/fixtures/raster_benchmark_results.json`](tests/fixtures/raster_benchmark_results.json).

On the recorded i7-12700KF/GDAL 3.13.3 stack, median warm release times were:

| Fixture | Python default | Native | Python peak RSS | Native peak RSS |
| --- | ---: | ---: | ---: | ---: |
| 256² colour + alpha | 0.185 s | 0.069 s | 81.2 MiB | 58.5 MiB |
| 4096² regional colour + alpha | 9.116 s | 5.681 s | 313.1 MiB | 293.6 MiB |
| 4096² regional grayscale | 7.765 s | 4.470 s | 289.3 MiB | 282.1 MiB |
| 2048² projected colour + alpha | 3.540 s | 2.286 s | 168.9 MiB | 210.0 MiB |

Every PNG byte matched, and source samples/masks/metadata and TileJSON/recipes were preserved. Python defaults used all 20 logical CPUs for tiling; the native pools were capped at four. The one-worker Python control is included in the raw results. These synthetic, warm-cache measurements do not predict throughput on cold storage or larger inputs.

Native COG compression and tiling use at most four workers in separate stages. The GDAL block cache and explicit display warp budget remain 64 MiB each. Temporary TIFFs are uncompressed and deleted before publication; reserve scratch space for their full bands, including any reprojected display raster. This trades disk capacity for less repeated compression while retaining DEFLATE on the published COG and identical PNG output on the tested dependency stack.

## Native vector validation and comparison

Build with `native-geospatial` and run the real CLI acceptance tests:

```sh
cargo test --locked --features native-geospatial
RUSTY_TILES_BIN="$PWD/target/debug/rusty-tiles" python3 -m unittest discover -s tests -p 'test_vector*.py'
RUSTY_TILES_BIN="$PWD/target/debug/rusty-tiles" python3 -m unittest discover -s tests -p 'test_native_vector.py'
```

The original Python modules live unchanged in `tests/fixtures/vector_oracle/`. Integration tests select the native CLI through `RUSTY_TILES_BIN`; oracle math helpers remain independent reference checks. Native tests exercise conversion and readiness with an empty PATH, exact scalar/list/noData metadata, projected and three-axis placement, shared locks, holes, fragment coverage, budgets, repair policies, decoded quantization bounds, deterministic worker output and reuse. The migration rejects Python reuse baselines explicitly. External C++ geodiff 2.3.1 and the local Go driver passed indexed/unindexed cross-apply checks. The vector browser probes passed Cesium 1.143.0 and 1.144.0 rendering, refinement, styling and picking before and after cache-disabled reloads; see `docs/VECTOR.md` to reproduce.

The opt-in `--aggregatePoints` checks independently decode counts and metre error,
retain boolean/INT64 source properties in leaves, separate layers, preserve every
oversized MultiPoint coordinate, exercise tolerance/budget fallbacks, and compare
workers and edited-source reuse. The shared voxel grid also runs through the
existing point-cloud acceptance tests. The browser aggregate case renders and
picks seven count aggregates at distance, then all 64 source points with original
properties at close range on Cesium 1.143.0 and 1.144.0, including quantized meshopt
content and cache-disabled reloads. Top-level error exceeds root error so large
aggregate tolerances still leave a distance interval for coarsest content.

Build the Python baseline at `fada1d1` and the new CLI in separate release target directories against the same GDAL/PROJ/GEOS stack, then run:

```sh
python3 tests/fixtures/benchmark_vector.py \
  --old-bin /path/to/python-release/rusty-tiles \
  --new-bin target/release/rusty-tiles \
  --output target/vector-benchmark --repeats 3
```

The baseline needs development Python GDAL/NumPy on PATH; native measured runs use an empty PATH. `tests/fixtures/vector_benchmark_results.json` records raw samples, binary/source identities, decoded checks and output sizes. On the i7-12700KF/GDAL 3.13.3 stack, warm release medians were:

| Invented input | Python, 1 worker | Rust, 1 worker | Python, 4 workers | Rust, 4 workers | Speedup, 4 workers |
| --- | --- | --- | --- | --- | --- |
| small-lines | 0.213 s | 0.039 s | 0.319 s | 0.036 s | 8.8× |
| regional-lines | 9.740 s | 2.291 s | 7.908 s | 1.810 s | 4.4× |
| polygons-with-holes | 5.922 s | 0.739 s | 3.982 s | 0.512 s | 7.8× |
| projected-lines | 2.625 s | 0.530 s | 2.118 s | 0.408 s | 5.2× |
| dense-points | 1.558 s | 0.458 s | 1.381 s | 0.384 s | 3.6× |

All full-detail world positions and properties matched; maximum coordinate difference was 9.4e-10 metres. With aggregation off, tile counts matched, and native manifests, reports and payloads other than performance diagnostics were identical across worker counts. Peak RSS fell on four cases; for regional lines at four workers it was 118.2 MiB natively versus 112.1 MiB for the largest Python process. Python worker memory is not summed, so this is not an aggregate memory comparison. Native tile data and each worker's SQLite cache are bounded by the existing budgets and 32 MiB cache.

The invented dense fixture has 8,192 semantic points. At four workers,
`--aggregatePoints` took 0.520 s versus 0.384 s for native retention/routing and
1.381 s for Python retention/routing. Aggregation is 2.7× faster than the Python
baseline but adds 35% native conversion time and 48% archive bytes (1.565 MB versus
1.057 MB) to provide extra coarse content. It publishes 383 nodes instead of 255,
with 16 count aggregates at the root and every original point/property in leaves.
Peak largest-process RSS was 60.6 MiB, versus 56.5 MiB for native routing and
86.1 MiB for Python routing. These are conversion/output measurements; reduced
distant geometry is verified by decoded counts and browser refinement, without
a frame-rate claim. Use `--case dense-points` to repeat this comparison alone.

These are full CLI conversions including packing, after one warmup per method and three serial repetitions with rotated method order. Inputs are invented, content is uncompressed, filesystem caches are warm, and the workstation had unrelated background jobs. CPU and largest-process RSS come from Linux `wait4`. Fixture generation and decoded checks are untimed; these measurements do not establish cold-storage or rendering throughput.

## Python-free 0.3 runtime acceptance

Build the full CLI with `--locked --features native-geospatial`, or retain the
default mesh/archive/local point-cloud build. CI runs both; its native matrix
uses GDAL 3.12 and 3.13. The release floor is GDAL 3.12, PROJ 9.2, and GEOS 3.10
for vector constrained triangulation. Install matching headers, `pkg-config`
files, libclang and SQLite development files for the full build. Deploy the same
GDAL/PROJ/GEOS shared-library ABI used to compile it; `doctor` reports the actual
linked versions and checks the local EPSG database and command capabilities.

The checked-in Dockerfile builds GDAL 3.12.4, PROJ 9.8.1 and the required drivers
(GeoTIFF/COG, PNG, ASCII grid, GeoJSON, Shapefile, SQLite/GeoPackage), with a Debian
trixie build/runtime ABI. Its `acceptance` stage executes the native test binaries
in a runtime image with no Python installed, including actual converter/preview
CLI tests. Its `runtime` stage contains the installed release CLI and native
libraries/database. It does not contain a Cesium runtime, datasets, Basis Universal
encoder, or downloaded PROJ grids. Supply those explicitly when needed.

```sh
docker build --target acceptance -t rusty-tiles:native-acceptance .
docker build --target runtime -t rusty-tiles:native .
docker run --rm --network none rusty-tiles:native doctor --json
```

PROJ's database is not a substitute for source-specific datum/geoid grids. Mount
licensed local grids and set `PROJ_DATA` to include both grids and `proj.db`.
Conversion remains offline and fails if its only-best operation needs a missing
grid. The binary neither downloads grids nor infers vertical datums.

The locked encoder dependencies are `gdal-sys` 0.12.0, `las` 0.11.1 / `laz` 0.13.0,
and `meshopt` 0.6.2 (vendored meshoptimizer 0.25); the preview server uses
`tiny_http` 0.12.0. Fixture browser acceptance pins Cesium IIFE 1.143.0.
The native matrix was also checked locally with GDAL 3.12.4 and 3.13.3.

0.3 changes `--version`, so consumers must invalidate their converter/derivative
cache identity. Python vector archives require a fresh native conversion before
reuse. Different native encoder identities, schemas, linked library versions,
CRSs or conversion settings also require a fresh baseline; same-revision,
same-settings reproducibility remains covered. The terrain manifest retains
`heightOverlay` version 1 with south-to-north coverage rows. Consumer upgrades
must enable `native-geospatial`, carry the matching libraries, and review the
converter pin/cache changes together. Echidna still has its own Python raster
and height helpers; their dependencies are separate from this runtime migration.

Development acceptance never silently falls back to Python conversion. Set
`RUSTY_TILES_BIN` to the full native build; the adapter invokes it with an empty
executable path and keeps the frozen Python module untouched for explicitly
selected comparisons. Former helper-only tests now inspect native GLB encoding,
OGR placement, or native library geometry, warning restoration and streaming
budget guards. Independent decoded metadata/geometry/coverage checks remain.

For all five invented preview layers, generate an output directory, serve it,
and run the browser checks with installed Playwright/Chromium and the local
Cesium runtime:

```sh
RUSTY_TILES_BIN="$PWD/target/release/rusty-tiles" \
  python3 tests/fixtures/preview_layers.py target/preview-case
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --mesh target/preview-case/mesh --point-cloud target/preview-case/cloud \
  --annotations target/preview-case/annotations --imagery target/preview-case/imagery \
  --terrain target/preview-case/terrain --port 9279
# In another terminal; NODE_PATH must expose an installed Playwright module.
node tests/fixtures/preview_layers.cjs http://127.0.0.1:9279
node tests/fixtures/point_cloud.cjs http://127.0.0.1:9279
node tests/fixtures/terrain.cjs http://127.0.0.1:9279
node tests/fixtures/vector_compat.cjs http://127.0.0.1:9279 --require-native --require-aggregates
```

The five-layer probe checks mounts, toggles, mesh picking, imagery decoding,
cache-disabled hard refresh and zero external requests. The existing focused
probes additionally check source metadata, near/far refinement, holes/fragment
boundaries and terrain coverage heights. These use software rendering and make
no hardware frame-rate claim.

`tests/fixtures/benchmark_runtime.py BINARY RESULTS.json` compares readiness and
preview with the frozen helpers at `bf3346c`, using seven warm, rotated runs and
one warmup on invented local files. Recorded release medians in
`tests/fixtures/runtime_benchmark_results.json` are:

| Measurement | Python helper | Full native CLI |
| --- | ---: | ---: |
| All-command doctor | 126.9 ms | 23.9 ms |
| Preview startup + config fetch | 51.7 ms | 15.7 ms |
| Sampled server peak RSS | 24.1 MiB | 27.0 MiB |
| 32 sequential loopback 1 MiB fetches | 28.3 ms | 51.8 ms |

Startup improved 5.3× for doctor and 3.3× for preview. This small sequential HTTP
case is 1.83× slower natively and uses 2.9 MiB more server RSS. The native build
links GDAL even when serving a preview. These are local process/HTTP measurements,
not browser rendering, concurrent throughput, or cold-storage benchmarks.

Full 0.3 public-data audits are recorded in `tests/fixtures/public_runtime_audit.json`:
all 10,653,336 Autzen numeric records occur exactly once; all 56,600 Natural Earth
roads retain their scalar fields and 652,521 original segments exactly once.
Maximum decoded road-position error was 0.104 m, within each leaf's reported
rounding bound. These use the existing cached licensed sources, with their hashes
and attribution retained; conversions run without executable helpers, while
Python independently reads the finished archives. Repeat the roads audit with:

```sh
python3 scripts/public_data.py roads /path/to/cache
rusty-tiles vector -i /path/to/cache/natural-earth-roads.gpkg -o /path/to/roads.3tz \
  --layer roads --heightOffset 0 --maxFeatures 64 --maxVertices 4096 \
  --maxBytes 131072 --lodTolerance 100 --jobs 4
python3 scripts/audit_vector.py /path/to/cache/natural-earth-roads.gpkg /path/to/roads.3tz
```

The road audit expects this EPSG:4326 multiline source, zero ellipsoidal height,
uncompressed native content, and all scalar fields. It is an opt-in fidelity
check; audits and dataset preparation are development tools, never runtime
requirements. Run the external geodiff cross-apply suite documented above before
release promotion as well.
