# Contributing to rusty-tiles

This guide is for people who build, test or change rusty-tiles. It covers the crate layout, the test suites, fixtures, release checks and the evidence behind fidelity claims. For using the tool, start with the [README](README.md).

Bug reports, documentation fixes and focused code changes are welcome. For a large change or a new format, open an issue first so we can agree scope and fidelity requirements.

## Report a problem

Include the command, the expected and actual result, the rusty-tiles version, your operating system and the Rust, GDAL and Python versions. For rendering problems, add the viewer version and whether hardware acceleration is on. Share the smallest invented or openly licensed reproduction you can.

Do not upload private survey data, sensitive locations, personal metadata, tokens or data you may not redistribute. Redact paths and credentials from logs.

## Branches and pull requests

| Branch | Purpose | How changes arrive |
| --- | --- | --- |
| `main` | Stable code. Version tags are cut here. | Release PR from `develop`, or a hotfix PR |
| `develop` | Integration branch for the next release | Feature, fix, test and documentation PRs |
| `feat/`, `fix/`, `docs/`, `chore/` + description | Short-lived work | Branch from `develop` and open a PR back to it |
| `hotfix/` + description | Urgent fix to a release | Branch from `main`, PR to `main`, then merge `main` back into `develop` |

External contributors work from a fork:

```sh
git fetch upstream
git switch -c fix/describe-the-change upstream/develop
git push -u origin fix/describe-the-change
```

Open the PR against `develop`. Describe the problem, the new behaviour, how you tested it and any fidelity or performance tradeoff. Keep unrelated changes apart. There is no commit-message convention or contributor agreement. Maintainers squash short-lived PRs. Release promotions from `develop` to `main` use a merge commit. After a release or hotfix, `main` is merged back into `develop` through a PR. Nobody rebases or force-pushes a shared branch.

### Review rules

Both shared branches need a PR, passing `Rust` and `Python` checks, an up-to-date branch and resolved discussions. Direct pushes, force-pushes and branch deletion are blocked, including for administrators. CI workflow tokens are read-only and CI receives no deployment secrets. The separate tag-triggered release workflow has write access to release assets and GHCR packages.

`main` also needs one code-owner approval, and stale approvals are dismissed. CODEOWNERS requests `@BenDyson-Arch`. While there is one maintainer, administrators may bypass only this approval, only through a PR, for their own changes. Review this exception when more maintainers join. Maintainers cut releases from verified commits on `main`. Contributions are licensed under MIT.

## Crate layout

| Module | Responsibility |
| --- | --- |
| `main.rs`, `lib.rs` | CLI parsing, summaries, `--json` results and exit codes; public library exports |
| `report.rs` | `Reporter`, `Event`, `EventSink` and `ConversionResult` |
| `output.rs` | The conversion `Job`: preflight, private work directory, no-clobber publication |
| `error.rs` | `Error` and its stable exit-code categories |
| `pack.rs` | Stored ZIP/ZIP64 with the 3TZ index, and `convert` |
| `validate.rs` | Read-only `.3tz` validation |
| `tileset.rs` | `createTilesetJson` and `glb-to-3tz` |
| `tileset_node.rs` | Shared 3D Tiles node pieces for tilers |
| `implicit.rs`, `implicit/tileset.rs` | Quadtree/octree availability, semantic metadata, converter emission and bounded expansion |
| `tile.rs` | Mesh spatial leaves and replacement LODs |
| `mesh.rs`, `hlod.rs`, `grid.rs` | Indexed mesh model, parent proxies built from children, and nearest-triangle grid |
| `texture.rs`, `jpeg.rs`, `gpu_texture.rs` | Texture baking, JPEG delivery and UASTC encoding |
| `glb.rs`, `glb_write.rs` | Shared GLB framing and meshopt view rewriting, and single-mesh GLB authoring |
| `lossless.rs` | Tile serialization with meshopt as a byte codec |
| `bbox.rs` | Bounding boxes from glTF positions |
| `georef.rs` | Mesh placement and source CRS adapters |
| `vec3.rs` | Small `[f64; 3]` helpers |
| `point_cloud.rs`, `point_cloud/` | LAS/LAZ reading and disk-backed tiling |
| `point_sampling.rs` | Voxel grid shared by point clouds and vector aggregation |
| `vector.rs`, `vector/native/` | OGR ingestion, SQLite store, LOD, encoding and reuse |
| `vector_encoding.rs` | Lossless vector buffer compression |
| `raster.rs`, `raster/native.rs` | COG and XYZ imagery |
| `terrain.rs`, `terrain/` | DEM sampling, quantized-mesh encoding and simplification |
| `geospatial.rs`, `geospatial/native.rs` | Shared GDAL and PROJ layer for all native converters |
| `doctor.rs`, `preview.rs` | Readiness report and local preview server |
| `fixtures.rs` | Tiny GLB used by tests |
| `bindings/python/` | PyO3 stable ABI extension, Python API guide and installed-wheel acceptance tests |

Native GDAL handles are created inside each worker. Spatial references and transformations cannot be sent between threads. Native CRS operations use GDAL's process-wide offline policy, so never re-enable PROJ networking while they run.

## Build

Use current stable Rust with rustfmt and a C++ compiler. The default build uses portable JPEG. Native features need `pkg-config` and their system libraries.

| Build | Command | Extra system packages |
| --- | --- | --- |
| Default | `cargo build --locked` | None |
| Native geospatial | `cargo build --locked --features native-geospatial` | GDAL 3.12+, PROJ 9.2+, GEOS 3.10+, their headers and `pkg-config` files, SQLite dev files, libclang |
| Native JPEG | `cargo build --locked --features native-jpeg` | libjpeg-turbo headers/libraries and `pkg-config` |

`RUSTY_TILES_DISABLE_NATIVE_JPEG=1` overrides `native-jpeg` for portable packaging. Python wheels never enable native features.

Native builds require `LIBSQLITE3_SYS_USE_PKG_CONFIG=1` in the Cargo environment
to share system SQLite with GDAL/PROJ. For example:
`LIBSQLITE3_SYS_USE_PKG_CONFIG=1 cargo build --locked --features native-geospatial`.
Apply the same override to native tests, Clippy and examples below. Leave it unset
for portable builds and wheels; they use bundled SQLite. Docker and the native
acceptance scripts set it automatically.
Static SQLite selectors (`SQLITE3_STATIC`, `PKG_CONFIG_ALL_STATIC`) and
`SQLITE3_LIB_DIR` overrides are rejected for native builds; select the matching
SDK through `PKG_CONFIG_PATH`. MSVC native builds also need `VCPKGRS_DYNAMIC=1`.

## Run the tests

### Rust

```sh
cargo fmt --all --check
cargo test --locked
cargo test --locked --features native-jpeg
LIBSQLITE3_SYS_USE_PKG_CONFIG=1 cargo test --locked --features native-geospatial
RUSTY_TILES_DISABLE_NATIVE_JPEG=1 cargo test --locked --lib jpeg::tests
```

The default suite runs real local LAS/LAZ conversions. The native suite adds CLI acceptance for doctor, preview, point cloud, raster, terrain and vector. Native CRS tests use independent references and synthetic local grids, with no Python or downloads. `cargo clippy --locked --all-targets --features native-geospatial -- -D warnings` is a CI gate in the native GDAL matrix.

CI and release builds cache Cargo dependencies by job, Rust toolchain and lockfile, with separate platform and GDAL keys. The Python-free job exports all Docker build layers to the GitHub Actions `python-free-acceptance` cache, including the pinned PROJ/GDAL SDK and Rust build outputs. Its acceptance stage always runs the full installed suite through `no-cache-filters: acceptance`; only the packaged runtime image is loaded into Docker. Runtime builds read that export without overwriting it; the release workflow uses the same cache scope. The Dockerfile's Cargo cache mounts remain local to a builder and are not exported by the GitHub cache backend, so source changes can still require Rust dependency compilation on a fresh runner. A cold SDK build remains slow; measure later runs after the cache has been populated.

`tests/implicit_subtree.rs` checks binary subtree availability independently of the writer. `implicit_metadata.rs` checks available-tile metadata rank and malformed binary input. `native_implicit.rs` checks complete implicit mesh/vector archives, boundary roots, corruption and reuse after edits. The point-cloud suite audits every LAS/LAZ leaf record and its placement across subtree boundaries. The mesh fidelity suite expands implicit tilesets before auditing leaf membership.

To retain invented linked quadtree/octree archives, run `RUSTY_TILES_IMPLICIT_FIXTURES=target/implicit-fixtures cargo test --locked --test implicit_subtree linked_subtrees`. Each scheme has single-content and multiple-content cases with two subtrees. Check Cesium's availability reader with `node tests/fixtures/implicit_subtree.cjs target/implicit-fixtures target/preview-runtime/node_modules/cesium`.

Upstream validator 0.6.1 validates native single-content cloud, mesh and vector fixtures with zero errors. Draft vector declarations produce a warning. Its pinned `3d-tiles-tools` 0.5.0 [traverser](https://github.com/CesiumGS/3d-tiles-tools/blob/v0.5.0/src/tilesets/traversal/ImplicitTraversedTile.ts) reads only `root.content.uri`, so stock traversal fails on multiple implicit content templates. For complete multiple-content checks, use `node tests/fixtures/implicit_validator.cjs target/validator/node_modules/3d-tiles-validator PATH/tileset.json --fix-multiple-content-traversal`. This explicitly patches only template selection in that pinned traverser; schema, subtree, metadata and content validators remain unchanged. It reports the workaround and fails on validation errors. Deep mesh and fragmented vector fixtures pass with zero errors under this workaround.

`tests/shared_metadata.rs` checks the public metadata API and node identities through instancing, mesh compression and LODs. `native_point_cloud.rs` checks property attributes against every LAS/LAZ tile's source table. The committed output digests cover unchanged defaults and vector payloads.

For the optional issue #79 browser acceptance, install CesiumJS 1.146.0 and Playwright once, then export the invented fixtures and serve them:

```sh
npm install --prefix target/metadata-browser --no-save --package-lock=false cesium@1.146.0 playwright
RUSTY_TILES_METADATA_ACCEPTANCE_DIR=target/metadata-fixtures cargo test --locked --features native-geospatial --test shared_metadata --test native_point_cloud
cargo run --features native-geospatial -- preview --cesium target/metadata-browser/node_modules/cesium/Build/Cesium \
  --mesh target/metadata-fixtures/mesh-false-true --point-cloud target/metadata-fixtures/point-false --port 9279
```

In another terminal, run `NODE_PATH="$PWD/target/metadata-browser/node_modules" node tests/fixtures/shared_metadata.cjs http://127.0.0.1:9279`. The probe uses installed Chromium (`CHROMIUM` overrides its path), picks separately instanced buildings, checks building and point style colors and picks a point with its original LAS metadata. Both explicit and implicit fixtures, including compressed meshes, pass upstream `3d-tiles-validator` 0.6.1 with zero errors and warnings using the validator command above.

Implicit subtrees have four levels. Regular boundaries use child-subtree availability. Boundaries whose actual boxes exceed their regular cells use standard external tileset roots containing bounded implicit subtrees. This preserves tight bounds, measured errors and picking: Cesium otherwise culls an unloaded subtree using its regular cell, even when later tile metadata enlarges it. The coordinate limit is 31 refinements; coincident point/centroid buckets use deterministic assignment with actual bounds retained.

Doctor, machine results, native diagnostics, preview, force replacement and archive validation now run in Rust. Their inputs are generated locally, and the CLI runs with an empty executable `PATH`. The remaining Python tests use independent readers or frozen converter oracles.

### Python acceptance

The Python extension has its own dependency-free suite. It installs an actual
wheel into a fresh virtual environment and runs the README example, all five
entry points, callbacks, concurrent calls, errors and force replacement with an
empty executable `PATH`:

```sh
python3 -m pip install 'maturin==1.15.0'
cargo clippy --locked -p rusty-tiles-python -- -D warnings
RUSTY_TILES_DISABLE_NATIVE_JPEG=1 maturin build --release --locked --out target/wheels
python3 scripts/test_python_wheel.py target/wheels/*.whl
```

Normal CI checks the wheel on CPython 3.10 and 3.14. Tag builds produce and
smoke-test one `cp310-abi3` wheel per release platform; the stable ABI covers
CPython 3.10 and newer. See [the Python API guide](bindings/python/README.md).

The Python suite drives the built CLI and checks its output with independent readers. It needs GDAL Python bindings that match your native GDAL, NumPy, laspy and pyproj.

```sh
cargo build --locked --features native-geospatial
python3 -m venv --system-site-packages target/pyenv
target/pyenv/bin/pip install -r scripts/point-cloud-requirements.txt
RUSTY_TILES_BIN="$PWD/target/debug/rusty-tiles" \
  target/pyenv/bin/python -m unittest discover -s tests -p 'test_*.py'
```

`--system-site-packages` reuses the system GDAL bindings. Otherwise install `scripts/gdal-requirements.txt` into the venv too. Both profiles need Python 3.12 or newer. Without `RUSTY_TILES_BIN`, CLI tests skip locally and print one warning. CI sets `RUSTY_TILES_REQUIRE_BIN=1`, which makes a missing binary fail the suite. With the binary set, only the two geodiff tests skip unless their tools are configured.

### Optional tests

These are ignored by default and run only when asked.

| Test | Command | Needs |
| --- | --- | --- |
| Wrapper conventions against `3d-tiles-tools@0.5.4` | `cargo test --test golden -- --ignored` | npx, with network or a cached install |
| Leaf triangle audit of a real model | `cargo test --release --test fidelity full_model_leaf_triangle_audit -- --ignored --nocapture` | `RUSTY_TILES_FIDELITY_SOURCE`, `RUSTY_TILES_FIDELITY_ARCHIVE`, optional `RUSTY_TILES_FIDELITY_OFFSET` |
| Checks on your own GLB | `cargo test --release --test demo_mesh -- --ignored` | `RUSTY_TILES_DEMO_GLB` |
| GeoPackage diff cross-apply | `python3 -m unittest discover -s tests -p 'test_geodiff_compat.py'` | `GEODIFF_CPP_BIN`, `GO_GEODIFF_DRIVER` |

The fidelity audit checks leaf triangle position and winding membership. It is not a CRS oracle or a bound on parent error. Omit the offset variable for an unprojected local source. For the geodiff test, build the Go driver from a go-geodiff v0.4.3 or newer checkout:

```sh
go build -o /tmp/go-geodiff-driver /path/to/rusty-tiles/tests/fixtures/geodiff_driver.go
GEODIFF_CPP_BIN=/path/to/geodiff GO_GEODIFF_DRIVER=/tmp/go-geodiff-driver \
  python3 -m unittest discover -s tests -p 'test_geodiff_compat.py'
```

It generates invented GeoPackages, checks byte-identical changesets, cross-applies them and compares reuse output with a fresh build. It includes GDAL spatial-index triggers. Older Go versions without the index functions fail rather than skip.

### Demo-data acceptance and benchmarks

The native Rust `demodata-suite` example exercises the openly licensed corpus assembled on 2026-10-08, normally in the sibling `../demodata` directory. Keep the approximately 1.5 GB of assets outside this checkout. [The manifest](bench/demodata_manifest.json) pins 416 files by size and SHA-256 and retains their source URLs, licences and attribution. Use the corpus's own README and `fetch.py` to provision it separately; provisioning can require Python/GDAL tools and Blender. The suite itself downloads nothing and runs every converter with an empty executable `PATH` and `PROJ_NETWORK=OFF`. Missing or changed required files fail a run.

```sh
cargo build --locked --release --features native-geospatial \
  --bin rusty-tiles --example demodata-suite
target/release/examples/demodata-suite --data-root ../demodata --verify
target/release/examples/demodata-suite --data-root ../demodata --list
target/release/examples/demodata-suite --data-root ../demodata \
  --bin target/release/rusty-tiles --profile smoke --output target/demodata-smoke
target/release/examples/demodata-suite --data-root ../demodata \
  --bin target/release/rusty-tiles --profile all --benchmark --repeats 3 \
  --output target/demodata-benchmark
```

Choose a fresh output directory for each invocation. Outputs must not overlap the corpus; selected source hashes are checked before and after a run. On Windows, append `.exe` to the executable paths. `--case ID` selects a named recipe and can be repeated. `--baseline-bin PATH` compares another CLI using the same inputs and options, rotating the method order between repetitions. Baselines must support the selected command options; an unsupported option fails the run.

| Profile | Cases | Coverage |
| --- | ---: | --- |
| `smoke` | 8 | Milk truck mesh; trimmed Autzen and USGS root points; Natural Earth places/countries; Australian imagery; Brisbane terrain; safe splat rejection |
| `core` | 21, including smoke | Lantern, Corset, 999,999-triangle Floreat and Delft meshes; 56,600 roads; 3DBAG building geometry; FlightHelmet glTF/resource wrapping and mesh-converter refusal; safe global imagery rejection; Cesium sparse quadtree/octree, multiple contents and metadata fixtures |
| `scale` | 7 | Full Autzen; all 144 USGS EPT nodes; Sentinel-2 imagery; terrain mosaic/projected DEM; dense places; 3D roads |
| `all` | 28 | All profiles |

Audits run outside conversion timing. Mesh leaves must retain every oriented float32 position triangle exactly once. Point leaves must retain every staged LAS record exactly once and every declared scalar metadata column byte-for-byte, including flags, RGB and Extra Bytes. Archive validation checks the hierarchies, placement and declared budgets. Vectors account for accepted/skipped source features; this corpus check does not independently compare every feature property or triangulated polygon. Imagery audits decode every 256-square PNG and check geographic XYZ coverage. Terrain audits independently decode quantized attributes and triangle/edge indices and check finite height overlays and tile counts. Packed Cesium reference resources must remain byte-identical. Expected failures must return the structured data-error category, exit 3 and publish nothing.

LAS decompression and sorted EPT-node merging happen in a private directory before timing. Raw integer records, flags and attributes survive preparation unchanged. Full Autzen converts horizontal international feet and vertical US survey feet through its staged header into local metres, with no datum transformation or globe placement. Trimmed Autzen remains in its original numeric foot units for a local encoding test. 3DBAG vectors use local numeric RD New/NAP coordinates to avoid optional datum grids. Copernicus and draped-road heights remain orthometric numeric values; `--heightOffset 0` is an encoding benchmark setting. These recipes do not assert correct ellipsoidal heights or survey placement. Recipe notes retain the individual coordinate assumptions.

Benchmark mode runs one audited warmup and at least three audited serial repetitions per method. `results.json` records the binary hashes, doctor/library information, machine information, input hashes, preparation hashes, exact commands, individual samples, medians, output sizes, fidelity checks and payload fingerprints. Wall time includes CLI startup and publication; fixture preparation and audits are excluded. On Unix, an isolated worker measures conversion CPU time and peak RSS with `wait4`, avoiding the audit process's inherited memory floor. Other platforms report unavailable CPU/RSS as null. Inputs use the warm filesystem cache. Repeated runs must produce the same member/file payload hashes, excluding `conversion.json` timing diagnostics and the derived ZIP index. This checks repeatability on the same codec/GDAL stack, not byte identity between different converter versions.

Fast manifest, path, input integrity and process-measurement tests run in normal native CI and the Python-free Docker suite. The large corpus is opt-in and adds no downloads to normal CI:

```sh
RUSTY_TILES_DEMODATA="$PWD/../demodata" \
  cargo test --locked --features native-geospatial --test demodata_suite \
  demodata_smoke -- --ignored --nocapture
```

The glTF extension, voxel, SPZ, CityJSON and unused 3D Tiles fixtures remain a hash-pinned reference inventory for future work. Listing or verifying them is not a passing conversion/conformance claim. [Recorded demo-data results](bench/demodata_benchmark_results.json) contain the full acceptance and benchmark evidence; timings describe that machine and recipe only.

Recorded on Linux x86_64, Intel i7-12700KF, 20 available threads, approximately 64 GB RAM, GDAL 3.13.3 and PROJ 9.8.1. That 27-case run predates the additional FlightHelmet wrapping case in the current 28-case manifest. All recorded cases passed an audited warmup and three measured repetitions, with identical payload fingerprints per case. Representative conversion medians:

| Recipe | Source size | Wall seconds | Peak RSS MiB |
| --- | ---: | ---: | ---: |
| Floreat mesh | 999,999 triangles | 3.540 | 584.8 |
| Full Autzen, staged local metres | 10,653,336 points | 14.883 | 56.0 |
| Merged USGS EPT nodes | 11,553,258 points | 21.015 | 58.8 |
| Sentinel-2 imagery | 954 decoded tiles | 11.839 | 227.3 |
| Copernicus terrain mosaic | 822 decoded tiles | 6.679 | 136.2 |

[The paired smoke run](bench/demodata_baseline_results.json) also passed all eight cases for current and baseline `99b9605`, with identical payloads within each method's three repetitions. The baseline defaults to explicit tiling and current defaults to implicit tiling. For the 512-pixel atlas milk-truck recipe, current emits 270 leaves versus 132 in the baseline: medians are 0.376 versus 0.207 seconds, with archives of 5,494,680 versus 4,200,493 bytes. This exposes the cost of the changed partition/layout on that recipe; it is not a general speedup claim. Both methods retain all 3,624 source triangles exactly once.

[The additional FlightHelmet wrapping run](bench/demodata_gltf_wrap_results.json) passed an audited warmup and three repetitions on the same machine. Its 48,397,031-byte archive validates and preserves the glTF, one buffer and fifteen PNGs byte-for-byte, with identical repeated payload fingerprints. Median conversion time was 0.067 seconds and peak RSS 30.1 MiB; these are local warm-cache measurements.


## Fixtures and oracles

Core tests use invented fixtures only. Private data and credentials are never bundled.

| Path | Role |
| --- | --- |
| `tests/fixtures/example.gltf`, `tests/fixtures/vector.geojson` | The README mesh and vector examples |
| `tests/fixtures/vector_oracle/` | The original Python vector modules, kept unchanged for comparison |
| `tests/fixtures/terrain_oracle.py` | The original Python terrain converter, compared at every grid size |
| `tests/fixtures/raster_oracle.py` | The original Python raster converter, compared tile by tile |
| `tests/fixtures/*.cjs` | Browser probes, described below |
| `bench/benchmark_*.py` | Repeatable benchmark harnesses |
| `bench/*_results.json`, `bench/public_runtime_audit.json` | Recorded evidence, listed below |
| `docs/schema/` | Bundled tileset schema used by `validate`. Do not edit. |

Oracles are development tools. They are never embedded and conversion never falls back to Python. Acceptance tests run the native CLI with an empty executable `PATH`. laspy and pyproj act as independent readers for point clouds.

For a regression, check the meaningful output. Examples are triangle membership and winding, texels, independent coordinate references, metadata values, archive indexing and failure publication. For preview changes, check rendering and picking with invented fixtures. For performance changes, compare the same source, settings, camera and hardware, and report memory and fidelity with timing. Do not claim hardware frame rates from a software-rendered browser.

### Browser probes

The probes need Node, Playwright, Chromium and a pinned Cesium runtime. They download nothing, repeat their checks after a cache-disabled reload and exit non-zero on failure. Software rendering confirms behaviour, not performance.

```sh
npm install --prefix target/browser-probe --no-save --package-lock=false playwright
RUSTY_TILES_BIN="$PWD/target/release/rusty-tiles" python3 tests/fixtures/preview_layers.py target/preview-case
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --mesh target/preview-case/mesh --point-cloud target/preview-case/cloud \
  --annotations target/preview-case/annotations --imagery target/preview-case/imagery \
  --terrain target/preview-case/terrain --port 9279
# In another terminal:
export NODE_PATH="$PWD/target/browser-probe/node_modules"
node tests/fixtures/preview_layers.cjs http://127.0.0.1:9279
node tests/fixtures/point_cloud.cjs http://127.0.0.1:9279
node tests/fixtures/terrain.cjs http://127.0.0.1:9279
node tests/fixtures/vector_compat.cjs http://127.0.0.1:9279 --require-native --require-aggregates
```

| Probe | Checks |
| --- | --- |
| `preview_layers.cjs` | Mounts, toggles, mesh picking, imagery decoding and zero external requests |
| `point_cloud.cjs` | Coarse-to-full refinement and picked LAS properties |
| `terrain.cjs` | Cesium terrain loading and sampled heights. Expects a 32 by 32 EPSG:4326 DEM at 123.5 m, NoData rows 12 to 19, `--height-offset 10.25 --fill-height -999.125`, zoom 9. |
| `vector_compat.cjs` | Native vector rendering, LOD, holes, fragment boundaries, picking and aggregates |
| `vector_metadata.cjs` | Property styling, visibility, source identity, exact INT64 and missing values |
| `implicit_levels.cjs` | Deep implicit mesh/cloud traversal across every level and boundary, full leaf counts and picking |

Generate standalone vector cases with `tests/fixtures/vector_compat.py OUTPUT`. Add `--batch`, `--quantize`, `--meshopt-helper PATH` or `--aggregate-points` to match the option under test. `tests/fixtures/vector_metadata.py OUTPUT` builds the metadata cases. Omit `--require-native` to inspect fallback behaviour in older Cesium releases.

For the deep probe, export the invented 24×24 textured grid using `RUSTY_TILES_DIGEST_EXPORT=target/implicit-inputs cargo test --locked --features native-geospatial --test output_digests export_recipes -- --ignored --exact`. Convert its `inputs/mesh.glb` with `--max-triangles 2 --max-bytes 0 --tile-size 64 --cartographic-position-degrees 12.1 41.9 200`. Reconvert the preview fixture's `cloud.las` with `--source-crs header --height-offset 10 --max-points 1`. Extract both archives and serve them with a generated annotations layer. Run `node tests/fixtures/implicit_levels.cjs URL`. Cesium 1.143.0 traverses levels 0–5, retains all 1,152 leaf triangles and 257 points, and picks both. The vector probe also checks contiguous instantiated levels, including routing nodes.

### Plain fill GLB regression

Issue #75 retains `b3dm` because CesiumJS 1.146.0 fails on plain triangle fill GLBs inside a vector tileset. The paired fixture preserves GLB bytes, source rings and metadata while removing only the wrapper and changing content URI suffixes. It includes batched features, a hole, fragmented source boundaries, LOD and the unchanged Sudan/Antarctica fixture rings. The probe checks picking, property color styling and visibility on initial load and a hard refresh.

```sh
RUSTY_TILES_BIN="$PWD/target/debug/rusty-tiles" python3 tests/fixtures/vector_fill_glb.py target/fill-glb-fixtures
rusty-tiles preview --annotations target/fill-glb-fixtures --cesium target/metadata-browser/node_modules/cesium/Build/Cesium --port 9377
# In another terminal:
NODE_PATH="$PWD/target/metadata-browser/node_modules" node tests/fixtures/vector_fill_glb.cjs http://127.0.0.1:9377 --expect-current-failure
```

Generate a second directory with `--compressed` to exercise `--quantize --meshopt`; repeat with portable and native binaries. `--expect-current-failure` requires wrapped fills and unaffected vector contents to pass while plain fragmented fills report the exact known `loopIndices` failure. The invented fragmented polygon and Sudan cannot be picked; Antarctica can retain a pickable coarser polygon while descendants fail. Omit that flag when checking a future runtime: every plain and wrapped case must then pass before changing the default. This regression does not alter converter output, cache identity or the frozen Python oracle. The measured four-build/encoding matrix is in [vector_fill_glb_results.json](bench/vector_fill_glb_results.json).

## Check byte-identity

Point-cloud and `convert` archives are reproducible by default. For vector archives, add `--reproducible`. It omits `conversion.json.performance`, the only volatile section. Then the same input, options, binary and GDAL, GEOS, PROJ and codec versions give identical bytes, whatever the worker count.

```sh
rusty-tiles vector -i tests/fixtures/vector.geojson -o output/a.3tz --reproducible
rusty-tiles vector -i tests/fixtures/vector.geojson -o output/b.3tz --reproducible --jobs 1
cmp output/a.3tz output/b.3tz
```

Without `--reproducible`, payloads, manifests, build state and reports still match. The performance section changes the ZIP offsets and index, so raw archive hashes differ. Compare members, not the archive. A fresh build and a reuse build are not byte-equal, because their ingestion history and reuse statistics differ. Mesh texture bytes depend on the codec build and are not promised equal.

The fixed recipe test `tests/output_digests.rs` runs every enabled converter twice on every platform. On the baseline platform recorded in `tests/fixtures/output_digests.json` (currently Linux x86_64), it also checks committed SHA-256 digests for the five recipes independent of GDAL and PROJ: two mesh recipes, `glb-to-3tz`, `createTilesetJson` and `convert`. Codec output can differ across CPU architectures, so other platforms check repeatability on their own stack. Native recipes also check repeatability on the current library stack. The digest test takes about 1.3 seconds with native geospatial enabled.

Regenerate the baseline digests only after reviewing an intended output change. Regeneration records the current OS and CPU architecture:

```sh
UPDATE_OUTPUT_DIGESTS=1 cargo test --locked --test output_digests
scripts/compare_outputs.sh develop
```

The comparison script builds each revision in its own release target directory. It runs 12 recipes with an empty executable `PATH` and compares every archive member and directory file. It normalizes only vector encoder and build-state fingerprints at their known paths. `COMPARE_WORK` chooses the scratch directory. `COMPARE_TARGET_ROOT` chooses the build cache. Pass a second revision to compare two committed revisions; otherwise it compares the base with the current working tree.

These recipes use `--explicit` for the three spatial converters; the script omits the flag for baseline binaries that predate it. Implicit reproducibility, subtree metadata, reuse and fidelity are checked separately. The implicit vector fingerprint includes the subtree emitter and uses a different tiling configuration. The explicit fingerprint retains the reviewed 0.3.0 baseline for byte identity; changes to explicit encoding must update that identity deliberately.

## Release checklist

1. CI passes on `develop`. Its jobs are `Rust`, `Native geospatial` for GDAL 3.12 and 3.13, `Python` and `Python-free runtime`. The manual workflow also runs `Release acceptance`.

   ```sh
   scripts/release_acceptance.sh
   ```

   This builds the release CLI and checks doctor, the README mesh conversion, validation, preview startup, served manifests and shutdown. Set `RUSTY_TILES_BIN` to check an existing build. Browser probes run when `CESIUM_DIR`, Playwright on `NODE_PATH`, Chromium and the development Python dependencies are available. `PYTHON` chooses that interpreter. Missing optional steps are printed. A failed required step or enabled browser probe fails the script.
2. The Docker acceptance stage passes. It runs every native test binary in a runtime image with no Python.

   ```sh
   docker build --target acceptance -t rusty-tiles:native-acceptance .
   docker build --target runtime -t rusty-tiles:native .
   docker run --rm --network none rusty-tiles:native doctor --json
   ```

3. `cargo package --locked --no-verify` includes the docs listed in `Cargo.toml`.
4. The geodiff cross-apply test passes with real tools.
5. The browser probes pass on the pinned Cesium release.
6. The public-data audits below are repeated when encoders change.
7. `CHANGELOG.md` describes every user-visible change.
8. Before the first Python release, create the GitHub environment `pypi` and configure a PyPI trusted publisher for owner `BenDyson-Arch`, repository `rusty-tiles`, workflow `release.yml`, environment `pypi`. This uses OIDC and needs no API token. Configure a pending publisher if the PyPI project does not exist yet; see [PyPI's trusted publishing guide](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/).
9. Update `[workspace.package].version` in `Cargo.toml` to the release version (both Rust crates inherit it), merge to `main`, then push the matching `vVERSION` tag. [The release workflow](.github/workflows/release.yml) checks that the tag matches the crate version and belongs to `main`, tests and packages default binaries and Python wheels for Linux and macOS (x86_64 and ARM64) and Windows x64, then tests and publishes the Linux amd64 native image to GHCR. After all builds pass, it publishes the wheels to PyPI and the GitHub release with `SHA256SUMS`. Linux wheels use manylinux_2_28; no system geospatial libraries or libjpeg-turbo are required. Prereleases do not update the image's `latest` tag.
10. Confirm the GHCR package is public in its package settings ([new packages start private](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry#pushing-container-images)), then check the installer against the published release and pull the image without authentication:

   ```sh
   sh scripts/install.sh --version VERSION --prefix /tmp/rusty-tiles-release/bin
   /tmp/rusty-tiles-release/bin/rusty-tiles --version
   docker run --rm --platform linux/amd64 --network none ghcr.io/bendyson-arch/rusty-tiles:vVERSION doctor --json
   ```

The standalone downloads are default builds using the portable JPEG encoder. Linux builds use Ubuntu 22.04 (glibc 2.35+ and libstdc++); macOS builds use macOS 15. The installer checks the release checksum before running or installing a binary. `cargo-binstall` uses the same archive layout via the manifest metadata; publishing to crates.io is a separate maintainer action.

A new version changes converter and cache identity. Consumers must invalidate derivative caches. Vector archives from another encoder identity, schema, library version, CRS or setting need a fresh baseline before reuse. The terrain manifest keeps `heightOverlay` version 1 with south-to-north rows. Echidna keeps its own Python raster and height helpers, separate from this runtime.

## Verification evidence

These files record measured runs. Read them for numbers. They describe one machine and run, not guarantees.

| Evidence | What it covers | Reproduce with |
| --- | --- | --- |
| [`public_runtime_audit.json`](bench/public_runtime_audit.json) | Full audits of Autzen points and Natural Earth roads, with source hashes and attribution | `bench/public_data.py`, `bench/audit_point_cloud.py`, `bench/audit_vector.py` |
| [`vector_benchmark_results.json`](bench/vector_benchmark_results.json) | Native vector against the Python baseline at `fada1d1`, including `--aggregate-points` | `bench/benchmark_vector.py` |
| [`raster_benchmark_results.json`](bench/raster_benchmark_results.json) | Native raster against the Python baseline at `9a95862`, with every PNG byte compared | `bench/benchmark_raster.py` |
| [`terrain_benchmark_results.json`](bench/terrain_benchmark_results.json) | First native terrain at `62091ea` against Python | `bench/benchmark_terrain.py` |
| [`terrain_performance_results.json`](bench/terrain_performance_results.json) | Parallel terrain encoding, with byte-identical payloads | `bench/benchmark_terrain.py --mode performance` |
| [`runtime_benchmark_results.json`](bench/runtime_benchmark_results.json) | Native doctor and preview against the Python helpers at `bf3346c` | `bench/benchmark_runtime.py BINARY RESULTS.json` |

Benchmarks build the old and new binaries in separate worktrees and target directories against the same GDAL stack. They use warm caches, one warmup and rotated method order. Peak RSS is the largest process from Linux `wait4`, not a process-tree sum. Run each script with `--help` for its arguments.

Public data downloads are opt-in and stay outside the repository. The helper records licence, attribution and SHA-256 provenance.

```sh
python3 bench/public_data.py autzen /path/to/cache
rusty-tiles point-cloud -i /path/to/cache/autzen-local-metres.las \
  -o /path/to/cache/autzen.3tz --source-crs local
python3 bench/audit_point_cloud.py /path/to/cache/autzen-local-metres.las /path/to/cache/autzen.3tz
python3 bench/public_data.py roads /path/to/cache
rusty-tiles vector -i /path/to/cache/natural-earth-roads.gpkg -o /path/to/roads.3tz \
  --layer roads --height-offset 0 --max-features 64 --max-vertices 4096 \
  --max-bytes 131072 --lod-tolerance 100 --jobs 4
python3 bench/audit_vector.py /path/to/cache/natural-earth-roads.gpkg /path/to/roads.3tz
```

The Autzen helper converts international feet and US survey feet to local metres and removes CRS declarations. That audit validates local conversion and metadata, not a datum transform. Sources are [PDAL Autzen data](https://github.com/PDAL/data/tree/main/autzen) under [CC BY 4.0](https://github.com/PDAL/data/blob/main/LICENSE) and [Natural Earth roads](https://www.naturalearthdata.com/downloads/10m-cultural-vectors/roads/) under [public-domain terms](https://www.naturalearthdata.com/about/terms-of-use/).

Other durable facts:

- Geodiff compatibility was tested with upstream geodiff 2.3.0 `e71dfe1` and 2.3.1, and go-geodiff v0.4.3 `ec6a8d3`. v0.4.3 fixes [go-geodiff issue #3](https://github.com/tinyowl-labs/go-geodiff/issues/3).
- The official validator 0.6.1 passes the standard mesh and uncompressed vector fixtures with zero errors. It warns about the draft vector extension it does not implement.
- Validation led to two producer fixes. Parent boxes now contain child thickness and rounding. Feature attributes use padded uint16, or float32 for larger tables.
- The issue #54 fixture of 3,000 twelve-vertex polygons measured the effect of geometry batching.
- Locked encoder dependencies are `gdal-sys` 0.12.0, `las` 0.11.1 with `laz` 0.13.0, `meshopt` 0.6.2 with meshoptimizer 0.25, and `tiny_http` 0.12.0. The native matrix was also checked locally with GDAL 3.12.4 and 3.13.3.
- Vector compression runs in process. The library field `VectorOptions.meshopt_encoder` is ignored.
