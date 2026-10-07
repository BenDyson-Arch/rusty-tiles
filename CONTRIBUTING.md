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

For native geospatial work, install GDAL >= 3.11, PROJ >= 9.2, their headers and `pkg-config` files, and libclang, then run `cargo test --locked --features native-geospatial`. CI tests the minimum GDAL 3.11 and the GDAL 3.13 stack separately from the existing Rust and Python jobs. Native CRS tests use independent coordinate references and synthetic local grids; they require no Python or grid downloads. Create native handles within each worker: spatial references and transformations deliberately cannot be sent or shared between threads. Native CRS operations use GDAL's process-wide offline policy; do not re-enable PROJ networking while they are running.

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
python3 scripts/preview.py --point-cloud target/cloud-browser-case/tiles \
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

## Check and reproduce the Python environment

Run `rusty-tiles doctor` before starting a job, or select converters explicitly:

```sh
rusty-tiles doctor --command vector --command terrain
rusty-tiles doctor --command point-cloud --json
```

The check inventories the actual Python interpreter, module versions/locations,
GEOS triangulation, PROJ database availability/data directories/local grids, and
readiness for each converter. It exits unsuccessfully if a selected converter is
missing a required capability. Native point-cloud/mesh/packing commands can be checked without
Python. It does not install dependencies or fetch grids; an inventory is not proof
that every requested CRS/height operation is supported.

`doctor --command point-cloud` runs entirely in Rust and reports local XYZ readiness separately from native geospatial placement. A mesh-only/default build supports local point clouds; enable `native-geospatial` for header/explicit CRS placement. An all-command doctor still inventories Python dependencies for the converters that have not yet migrated.

Known-good, exact Python profiles tested on 2026-10-05 are in
`scripts/gdal-requirements.txt` (vector/raster runtime and development terrain oracles) and
`scripts/point-cloud-requirements.txt` (development LAS/LAZ fixtures and independent audits). Both require Python 3.12 or newer. Point-cloud and terrain conversion themselves have no Python dependency.
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

## Python conversion diagnostics

Data/conversion errors print concise messages; dependency installation guidance
appears only when an actual Python import fails. Set
`RUSTY_TILES_PYTHON_TRACEBACK=1` to include a traceback for debugging. Embedded
modules use named synthetic filenames so tracebacks never inline the helper
source. Failed conversions still publish nothing.

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
