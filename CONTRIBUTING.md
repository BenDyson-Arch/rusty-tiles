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

Native GDAL handles are created inside each worker. Spatial references and transformations cannot be sent between threads. Native CRS operations use GDAL's process-wide offline policy, so never re-enable PROJ networking while they run.

## Build

Use current stable Rust with rustfmt, a C++ compiler and `pkg-config`. libjpeg-turbo is optional.

| Build | Command | Extra system packages |
| --- | --- | --- |
| Default | `cargo build --locked` | None |
| Native geospatial | `cargo build --locked --features native-geospatial` | GDAL 3.12+, PROJ 9.2+, GEOS 3.10+, their headers and `pkg-config` files, SQLite dev files, libclang |
| Portable JPEG | `RUSTY_TILES_DISABLE_NATIVE_JPEG=1 cargo build --locked` | None |

## Run the tests

### Rust

```sh
cargo fmt --check
cargo test --locked
cargo test --locked --features native-geospatial
RUSTY_TILES_DISABLE_NATIVE_JPEG=1 cargo test --locked --lib jpeg::tests
```

The default suite runs real local LAS/LAZ conversions. The native suite adds CLI acceptance for doctor, preview, point cloud, raster, terrain and vector. Native CRS tests use independent references and synthetic local grids, with no Python or downloads. `cargo clippy --locked --all-targets --features native-geospatial -- -D warnings` is a CI gate in the native GDAL matrix.

Doctor, machine results, native diagnostics, preview, force replacement and archive validation now run in Rust. Their inputs are generated locally, and the CLI runs with an empty executable `PATH`. The remaining Python tests use independent readers or frozen converter oracles.

### Python acceptance

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

Generate standalone vector cases with `tests/fixtures/vector_compat.py OUTPUT`. Add `--batch`, `--quantize`, `--meshopt-helper PATH` or `--aggregate-points` to match the option under test. `tests/fixtures/vector_metadata.py OUTPUT` builds the metadata cases. Omit `--require-native` to inspect fallback behaviour in older Cesium releases.

## Check byte-identity

Point-cloud and `convert` archives are reproducible by default. For vector archives, add `--reproducible`. It omits `conversion.json.performance`, the only volatile section. Then the same input, options, binary and GDAL, GEOS, PROJ and codec versions give identical bytes, whatever the worker count.

```sh
rusty-tiles vector -i tests/fixtures/vector.geojson -o output/a.3tz --reproducible
rusty-tiles vector -i tests/fixtures/vector.geojson -o output/b.3tz --reproducible --jobs 1
cmp output/a.3tz output/b.3tz
```

Without `--reproducible`, payloads, manifests, build state and reports still match. The performance section changes the ZIP offsets and index, so raw archive hashes differ. Compare members, not the archive. A fresh build and a reuse build are not byte-equal, because their ingestion history and reuse statistics differ. Mesh texture bytes depend on the codec build and are not promised equal.

The fixed recipe test `tests/output_digests.rs` runs every enabled converter twice. It checks committed SHA-256 digests for the five recipes independent of GDAL and PROJ: two mesh recipes, `glb-to-3tz`, `createTilesetJson` and `convert`. Native recipes check repeatability on the current library stack. The digest test takes about 1.3 seconds with native geospatial enabled.

Regenerate the portable digests only after reviewing an intended output change:

```sh
UPDATE_OUTPUT_DIGESTS=1 cargo test --locked --test output_digests
scripts/compare_outputs.sh develop
```

The comparison script builds each revision in its own release target directory. It runs 12 recipes with an empty executable `PATH` and compares every archive member and directory file. It normalizes only vector encoder and build-state fingerprints at their known paths. `COMPARE_WORK` chooses the scratch directory. `COMPARE_TARGET_ROOT` chooses the build cache. Pass a second revision to compare two committed revisions; otherwise it compares the base with the current working tree.

## Release checklist

1. CI passes on `develop`. Its jobs are `Rust`, `Native geospatial` for GDAL 3.12 and 3.13, `Python` and `Python-free runtime`. The manual workflow also runs `Release acceptance`.

   ```sh
   scripts/release_acceptance.sh
   ```

   This builds the release CLI and checks doctor, the README vector conversion, validation, preview startup, served manifests and shutdown. Set `RUSTY_TILES_BIN` to check an existing build. Browser probes run when `CESIUM_DIR`, Playwright on `NODE_PATH`, Chromium and the development Python dependencies are available. `PYTHON` chooses that interpreter. Missing optional steps are printed. A failed required step or enabled browser probe fails the script.
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
8. Update `Cargo.toml` to the release version, merge to `main`, then push the matching `vVERSION` tag. [The release workflow](.github/workflows/release.yml) checks that the tag matches the crate version and belongs to `main`, tests and packages default binaries for Linux and macOS (x86_64 and ARM64) and Windows x64, then tests and publishes the Linux amd64 native image to GHCR. After all builds pass, it publishes the GitHub release with `SHA256SUMS`. Prereleases do not update the image's `latest` tag.
9. Confirm the GHCR package is public in its package settings ([new packages start private](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry#pushing-container-images)), then check the installer against the published release and pull the image without authentication:

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
