# Changelog

All notable changes to rusty-tiles are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## 0.4.1 - 2026-11-13

### Added

- Python `list_tiles(archive)` lists the content tiles of a `.3tz` archive or
  unpacked tileset directory with their depth, `leaf`/`coarse` role, geometric
  error and accumulated transform.
- Python `decode_tile(archive, member, *, format="glb")` returns one tile as a
  plain glTF 2.0 GLB (meshopt decoded, KTX2/Basis pages transcoded to PNG at
  level 0, feature IDs and property tables kept) or, with `format="arrays"`,
  as NumPy arrays in the tile frame. It is a view for display and inspection,
  such as in Blender's bundled glTF importer, not a reversible round trip.
- `mesh` capture progress: `capture_scan`, `capture_validate`, `capture_corners`,
  `capture_positions`, `capture_triangles`, `capture_sequence`, `capture_images`,
  `capture_tree`, `capture_corner_depths`, `capture_leaf_plans` and
  `capture_merge` phases with totals, repeated at least every 250 ms, and
  `hlod_incidence_items`, `hlod_incidence_reduce`, `hlod_incidence_ends` and
  `hlod_incidence_merge` inside `hlod_incidence`. They are cancellation
  checkpoints, so a cancel or Python callback exception during capture or the
  incidence pass lands within about a second.
- `validate` reports coarse HLOD tiles whose corner `NORMAL` points more than
  90° from its face (the face shades against its winding) as the
  `coarseNormalInversion` warning, with counts by cell depth in
  `interfaceCensus.coarseNormals`. Warnings appear in a new report `warnings`
  list, never fail validation (`ok` stays `true`), and the CLI prints them on
  stderr. Archives written before this release usually carry the warning.
- The standard build (CLI and Python wheel) resolves about 3,100 more EPSG codes from a
  GDAL-verified allow-list, including GDA2020/GDA94 MGA, GDA94/GDA2020 geographic, GDA94
  Lambert/Albers, NZTM2000, ETRS89 UTM and TM35FIN, for mesh, point-cloud and vector
  conversion. A code is listed only when its generated Helmert or null-shift definition
  lands within 1 mm of GDAL/PROJ's best operation at all 18 sampled points of its area of
  use (EPSG v13.102, PROJ 9.9.0, GDAL 3.13.3). Grid-based codes such as British National
  Grid (OSTN15), NAD27 and the original NAD83, coordinate epochs and dynamic reference
  frames (ITRF, WGS 84 (Gxxxx), WGS 72) stay refused; native builds keep using strict PROJ.
- `conversion.json` and the CLI summary name the EPSG datum operation an allow-listed code
  uses and EPSG's stated accuracy; a null transformation such as GDA2020 to WGS 84 is
  reported with its accuracy (3 m), not as exact. `doctor` reports the allow-list's size
  and versions, and `doctor --source-crs <definition>` resolves one CRS without converting.
- `validate --memory-budget BYTES` and `--jobs N|auto` (Python
  `validate(..., memory_budget_bytes=, jobs=)`, Rust `ValidationRequest::memory_budget` and
  `::jobs`), with `mesh`'s default policy: the lesser of 4 GiB and half the detected
  headroom. When either is given, the report's new `resources` member states the resolved
  budget and its source, the workers, the store sizes derived from the budget and the
  scratch-disk bound.
  An explicit budget below 512 MiB is refused as `resource_limit` before the archive is
  opened.
- `validate` progress for the HLOD delivery stage: `validate.delivery` (passes done of
  the total) and `validate.delivery.leaves`, `.edges` and `.coarse` (placements replayed
  of the total). Every event, every charged delivery row and every 10,000 SQLite
  operations is a cancellation checkpoint.
- A start-of-run scratch-disk check for `validate`: before traversal, free space is
  compared with the catalog's terms plus, for an HLOD archive, a per-triangle term from the
  triangles its GLB payloads declare. Below 16 bytes per triangle the volume is refused as
  typed insufficient space (`io`); below 256 a `scratch_space` warning event is emitted.
- Ctrl+C and Ctrl+Break in a Windows console cancel the running command and
  clean up as SIGINT does on Unix (exit 130). A second press ends the process
  at once.
- Python: `resolve_source_crs(definition)` resolves one source CRS exactly as the
  converters do, without converting, and returns `accepted`, `kind`, `definition`,
  `crsOperation` (the reports' shape), `summary` and `refusal`; an unsupported definition
  is returned, not raised. `supported_epsg_codes()` lists every EPSG code the package
  resolves, and `crs_table_info()` gives the EPSG table's EPSG, PROJ and GDAL versions and
  counts. The Rust library exports the same as `resolve_source_crs`,
  `supported_epsg_codes` and `crs_table_info`.
- Python wheels are also tested inside official Blender 5.2.4 LTS (Python 3.13) on Linux x64, Windows x64 and Apple Silicon macOS, alongside Blender 4.5.14 LTS.
- A rolling development build of `develop`, the GitHub pre-release `dev`, is rebuilt after a nightly full CI pass when `develop` has changed. It has the CLI archives, Python wheels and `SHA256SUMS`, plus the container tag `ghcr.io/bendyson-arch/rusty-tiles:dev`, and uses a development version such as `0.4.1.dev202611121734`. It is not published to PyPI or crates.io. `scripts/install.sh --version dev` installs its CLI; see "Development builds" in `docs/INSTALL.md`.

### Changed

- `mesh` capture sizes its external sorts from the memory budget: runs grow until
  every capture sort merges once (fan-in 64, 128 or 256), and full runs are sorted
  and written by background writers while the next fills. The plan is charged in
  capture's declared memory bound and reported as `diagnostics.capture.sort`;
  the fixed configuration stays the floor, so the minimum budget and refusals are
  unchanged. Archives are byte-identical for every budget and `--jobs`.
- The coarse stage's `hlod_incidence` pass plans its item and endpoint sorts the same
  way within the coarse stage's admission (floor fan-in, so no new scratch refusals),
  reported as `diagnostics.memory.hlod.incidence_sort`.
- The CRS refusal message reads `pure-Rust CRS transform unavailable` for every converter,
  instead of naming point clouds.
- `mesh` no longer refuses source images over 32 MiB encoded. Each image is
  charged against the memory budget at its actual encoded length plus its
  decoder working memory, in capture, the leaf stage and the coarse stage.
  Capture reads every image's header (PNG `IHDR` or the first JPEG frame
  header) before it writes any scratch, and an image whose decode the budget
  cannot hold is `resource_limit` there; each decode is checked again against
  the decoder's own header, which must declare the same dimensions
  (`invalid_input` otherwise). A 1 GiB encoded sanity ceiling (`unsupported`)
  and the 16,384 px edge remain. Malformed or unsupported image headers are now
  refused at that preflight, before capture. The error kinds are the
  decoder's, except that an image both over a header limit (edge, 16-bit) and
  corrupt later in its stream now reports the header refusal (`unsupported`)
  instead of the corruption (`invalid_input`).
- Breaking error type: a `mesh` source image over 32 MiB encoded (and at most
  1 GiB) that the memory budget cannot hold is now `resource_limit` (exit 1;
  Python `ResourceLimitError`), refused with "image capture: decoding a source
  image needs N bytes ... but the memory budget leaves B for image decoding".
  It was `unsupported` (exit 2; `UnsupportedError`) "encoded image exceeds
  32 MiB admission ceiling" before, whatever the budget. One that fits is now
  converted. Over 1 GiB stays `unsupported`, now "encoded image exceeds 1 GiB
  admission ceiling".
- Archives for sources admitted before are unchanged at the budgets the digest
  tests use, with two planning caveats. An image of at most 32 MiB with more
  than 4 encoded bytes per texel (an incompressible RGBA PNG, or one inflated
  by ancillary chunks) now gets a higher, correct leaf-stage charge, which can
  move a leaf's density step at a budget right at that threshold. The coarse
  stage's decode charge now uses the largest actual encoded image instead of a
  fixed 32 MiB, so for every source that declares images (textured, or with
  images no material samples) the coarse stage's job count, image cache and
  reported memory minima can change; coarse planning does not change archive
  bytes, and image-free sources never had this term. The output digest tests
  run at default budgets, not at these thresholds.
- `validate` sizes its external sort runs, merge fan-in and buffers, catalog cache,
  resident-store thresholds and the HLOD delivery store's SQLite page cache from the
  memory budget (SQLite's temporary store stays on disk unless its row bound fits), and
  replays HLOD payload geometry on `--jobs` workers in a bounded lookahead. Verdicts,
  work counts, refusals and every report member except `resources` and `scratch` are
  unchanged and identical for every budget and worker count.
- The vector encoder fingerprint changes, so `--reuse-tileset` refuses every vector archive
  made before this release, with or without `--meshopt` ("previous encoder, CRS or conversion
  settings differ"). Run one fresh conversion without `--reuse-tileset`.
- Release and development builds publish the CLI archives and Python wheels that the full CI run of the same commit built and tested, after smoke-testing each archive on its platform, instead of rebuilding them. They are rebuilt only when that run's artifacts have expired.
- The native container's `Dockerfile` takes PROJ and GDAL from a `GEOBASE` stage or image: `geospatial-source` builds them from their checked source tarballs as before, or a pinned `ghcr.io/bendyson-arch/rusty-tiles-geobase:<gdal>-<proj>@sha256:...` image provides them. The image contents are unchanged.
- While a release candidate is out, development builds are versioned between it and the next candidate (for example `0.4.1rc2.dev202611121734` after `0.4.1rc1`) instead of before every candidate, and replacing the `dev` pre-release no longer interrupts its downloads.

### Removed

- Intel macOS (`x86_64-apple-darwin`) is no longer supported: releases have no Intel macOS CLI archive or wheel, and CI no longer builds or tests it. 0.4.0 is the last release with them. On an Intel Mac, `pip install rusty-tiles` finds no wheel (there is no source distribution) and `scripts/install.sh` stops with a message for 0.4.1 and later; install 0.4.0 (`--version 0.4.0`, `pip install rusty-tiles==0.4.0`) or build from source with Cargo (`cargo install --locked --git https://github.com/BenDyson-Arch/rusty-tiles rusty-tiles`, or `maturin build --release` for the wheel). macOS builds are Apple Silicon only, and the installer gives an Apple Silicon Mac in a Rosetta terminal the native build.

### Fixed

- Mesh HLOD coarse tiles no longer shade surfaces with starburst gradients or
  inverted facets when the source carries normals. Coarse `NORMAL` is now
  crease-aware: a vertex splits its normal across edges sharper than 60°, and
  every corner normal lies within 60° of its face, so no face is shaded against
  its own winding. Leaves are unchanged; coarse tiles of sources with normals
  have more vertices (13–15% more coarse bytes on the measured meshes).
- `validate` creates SQLite's temporary files for the HLOD delivery store (sorter runs,
  temporary tables, statement journals) in its scratch directory instead of SQLite's own
  temporary directory (`/var/tmp` or `TMPDIR`), so `--scratch-dir` holds them.
- A stop request now ends the long phases within a few seconds (capture's
  own checkpoints are under Added). This covers SIGINT/SIGTERM,
  Ctrl+C/Ctrl+Break in a Windows console, and `KeyboardInterrupt` at a Python
  progress event. Before, such a run could keep working for minutes and a
  service manager would kill it, leaving its scratch on disk. The phases are:
  - `validate` archive admission (before its first progress event), hashing
    of a large member, its catalog, row and delivery external sorts and
    merges, and the HLOD index build and final commit;
  - the mesh archive's index sort, staging of each leaf payload, density mip
    chains and coarse texture halving;
  - vector GeoJSON ingestion, its SQLite partition, fingerprint, grid and
    encoding statements, and restoring a reused tileset.

  The run still removes its scratch, exits 130/143 and publishes nothing.
- A few calls cannot see a stop, such as one KTX2 page encode, which can
  take minutes at the highest settings of the lossy ladder. If a stopped CLI
  run has not finished 20 seconds after the signal, the CLI removes the run's
  spill, work and staging paths itself and exits with the same status. A
  systemd stop then ends after about 20 seconds plus the time to remove the
  run's temporary files, instead of reaching the 90-second SIGKILL.
- `mesh` and vector content no longer write `"filter": "NONE"` on `EXT_meshopt_compression`
  buffer views. NONE is the extension's default, but Blender 5.2's glTF importer refuses the
  explicit value, so untextured mesh tiles did not open in Blender. `OCTAHEDRAL` is still
  written. Only the glTF JSON changes; compressed and fallback payload bytes are the same, but
  `mesh` and vector content hashes and archive bytes change. `validate` accepts both forms.
- `rusty_tiles.__version__` uses the wheel's PEP 440 spelling for pre-release versions (`0.4.0rc1`, not `0.4.0-rc.1`), so it matches `importlib.metadata.version("rusty-tiles")`.
- Windows: the Python wheel no longer crashes Blender 4.5 (an access violation in
  `MSVCP140.dll`) when it encodes or decodes KTX2 textures, for example in `decode_tile` or a
  textured `mesh_to_tileset`. The bundled Basis Universal C++ was built with the newer runtime's
  constexpr `std::mutex`, which Blender 4.5's older bundled C++ runtime cannot lock; it now keeps
  the runtime-constructed mutex that every `MSVCP140.dll` supports.

## 0.4.0 - 2026-11-11

### Added

- Large-mesh tiler delivery (H16): `mesh_leaves_to_3tz` (`MeshTilesetRequest`, `MeshTilesetReport`, `MeshTilesetResult`) publishes an H8 full-detail leaf directory as a Tiles 1.1 implicit forest in one `.3tz`, streamed one document at a time with rigid anchor/orientation/offset placement on the root. Parents carry no content until coarse levels land. The 3TZ member ceiling refuses as `unsupported` before anything is staged. Hidden CLI: `mesh-leaves-to-3tz` and `mesh-large-to-3tz`. These exports and commands were replaced by `mesh` / `mesh_to_tileset` before release, whose parents carry certified, textured coarse content.

- Self-contained per-converter demo recipes, including an invented LAS generator. Strict release acceptance renders and picks the README example and exercises all five preview layers before and after a cache-disabled reload.
- Candidate and release wheels are gated on checksum-pinned official Blender 4.5 LTS bundles for Linux x64, Windows x64 and both macOS architectures, in addition to CPython 3.10/3.14 on all five wheel platforms.
- General mesh horizontal CRS placement for `mesh-to-3tz` through the shared grid-free/native resolver (`mesh` has its own declared `--source-crs`, which rejects the legacy keywords), with explicit `--source-axes` and `--height-offset`, CRS-unit E/N shifts and metre heights. Authored normals retain hard edges through inverse-transpose projection; legacy geographic/Web Mercator placement defaults remain compatible. Python adds `source_axes` and `height_offset` keywords, and `doctor` reports the shared CRS tier.
- GeoJSON and GeoPackage vector conversion in the default binary and Python wheel, using streaming Rust readers, bundled SQLite, grid-free CRS transforms and constrained polygon triangulation. Both builds share tiling, metadata, LOD, compression and reuse; `native-geospatial` retains OGR/GEOS/PROJ for all vector inputs. Python adds `vector_to_3tz`, and `doctor` reports supported inputs and CRS limits.
- Grid-free point-cloud CRS transforms in the default binary and Python wheel using `proj4rs`: WGS84 geographic, UTM, Mercator and supported WKT/PROJ local projections with explicit Helmert datum shifts. Unsupported grids, epochs or unverified datums require strict native GDAL/PROJ; shifts are never silently discarded. `doctor` lists CRS classes and native fallback readiness. Python point-cloud conversion adds `source_crs` and `height_offset` keywords.
- Shared public `metadata` types and aligned GLB/property-table authoring for `EXT_mesh_features`, `EXT_structural_metadata` and implicit tile bounds/error semantics. Vector payloads and default converter output bytes are preserved.
- `mesh-to-3tz --node-features` keeps source nodes pickable and styleable by `name` and `node_index` through all LODs, including small inputs and instanced meshes. Unnamed nodes use `node_<index>`.
- `point-cloud --metadata-attributes` also exposes classification, intensity and return number as property attributes, retaining existing lossless property tables.
- Python bindings via PyO3 and maturin: path-based mesh, glTF/GLB, local LAS/LAZ and tileset conversion, archive validation, progress callbacks and typed exceptions. Tag builds smoke-test CPython stable ABI wheels on the five release platforms and publish to PyPI with trusted publishing.
- Opt-in native `demodata-suite` acceptance and benchmarks: pinned public inputs, smoke/core/scale profiles, conversion timing and memory, payload repeatability, and geometry/metadata/tile audits. Normal CI runs only the fast harness checks.
- 3D Tiles 1.1 implicit tiling is the default for point clouds, vectors and meshes. Binary subtrees carry Morton-ordered availability and standard tile bounds/error metadata. Meshes and point clouds use midpoint octrees; vectors preserve padded boxes and reuse. The output is the bounded implicit forest: an explicit skeleton above implicit roots of at most four levels, each with one subtree whose rows carry every tile's actual bounds, so Cesium can refine and pick without culling by regular cells. Only the deprecated `mesh-to-3tz` keeps `--explicit`; point-cloud and vector `--explicit` were removed (see Removed).
- Library: `implicit::Subtree`, `TileMetadata` and `expand_tileset` support quadtree/octree availability, multiple contents, child-subtree links, semantic metadata and bounded expansion for audits. Built-in archive validation reads native implicit output through the I1 implicit reader.
- Tag-triggered release binaries for Linux and macOS (x86_64 and ARM64) and Windows x64, a native geospatial GHCR image, a SHA-256-verifying installer and `cargo-binstall` metadata.
- Fixed converter recipes and committed output digests, cross-commit comparison tooling and a manual release acceptance workflow. CI fails when the Python acceptance binary is missing and checks native Clippy warnings.
- Every multi-word camelCase option now also accepts a kebab-case alias, such as `--sourceCrs`/`--source-crs` and `--maxPoints`/`--max-points`. camelCase stays the primary spelling. The `create-tileset-json`, `glbTo3tz` and `meshTo3tz` subcommand aliases are now shown in help.
- `doctor` reports whether a Cesium runtime is present at the README location (relative to the working directory) or at `doctor --cesium DIR`. This check is informational only.
- `raster --progress json` reports `source_cog`, `aligned_warp` and `tiling` on a normalized 1,000,000 scale (`tiling` resets for each zoom), then `raster_complete` with the actual tile count.
- Library: new `rusty_tiles::report` module with `Reporter`, `Event` and `ConversionResult { output, archive, report }`. `Reporter` can be silent, human stderr, NDJSON stderr or a custom `EventSink`. `report` is the published `conversion.json` value.
- Library: legacy entry points return a `ConversionResult`: `vector::vector_to_3tz_reported`, `terrain::dem_to_terrain_reported`, `tile::mesh_to_3tz_reported`, `tileset::glb_to_3tz_reported` and `pack::convert_to_3tz_reported`. Remaining legacy entry points are wrappers using `Reporter::default()`, which writes warnings and notes on stderr and no progress.
- Add literal native imagery display checks for constant RGB, ramps, impulses,
  selected numeric gray, NoData, masks and partial alpha, with independent PNG
  decoding and raw COG readback; record the finite 0.4.0 imagery resource receipt.
- Point-cloud 0.4 acceptance checks pair CLI and Python decoded LAS/LAZ fields,
  placement, settings and failure preservation on the current implicit forest.
- `rusty-tiles mesh` and Python `mesh_to_tileset` run one streamed local-metre mesh pipeline through leaf encoding, HLOD coarse levels (sampled error, textured) and implicit `.3tz` delivery, with rigid WGS84 placement, provisional profiles, memory-capped maximum workers, typed errors and separate runtime diagnostics. `doctor --command mesh` describes the built-in capability.
- `mesh`: `conversion.json` reports `texel_ownership`: whether each leaf's
  decoded UVs give every page texel a single owning triangle (`unique`), checked
  exactly, or whether overlapping source UVs share texels (`shared`, editable
  in vertex space only). Coarse tiles are unique by construction.
- HLOD can recover a root above omitted ownership cells from a complete bounded descendant frontier, with explicit replacement delivery and independently validated coverage, ownership and geometric error.

- Recovery validation charges metadata, coverage and ownership work under the existing validator budget.
- Bounded HLOD identity, exact canonical interface and explicit/implicit REPLACE continuity validation, with independently counted source-open edges, above-cap observations, a permanent real-archive/source oracle and sensitive geometry, metadata and delivery controls; sampled errors are not certificates.
- `mesh --memory-budget BYTES` (`--memoryBudget`), Rust `TilerRequest::with_memory_budget`
  and Python `mesh_to_tileset(..., memory_budget_bytes=None)` declare the run's memory
  ceiling, with one rule on every surface: a positive byte count (the CLI also takes
  `B`, `KiB`, `MiB`, `GiB` and `TiB` suffixes). Zero, signs, fractions, exponents,
  decimal units, booleans (Python `TypeError`) and overflow are `invalid_request`
  (CLI usage error, exit 2), refused before any file is read. The default is the lesser
  of 4 GiB and half the detected headroom (2 GiB when undetected). A budget is a
  ceiling that is never raised; a run that cannot fit under it is a `resource_limit`
  refusal before the binding stage allocates, and the archive is identical for every
  budget that admits the run.
- Run diagnostics always carry `memory` (the resolved budget, its source and each
  stage's minimum) and `hlod` (tiles, workers, CPU, lease peak, sampled-error
  measurement and per-depth appearance), and `phase_seconds` with the phases
  `capture`, `leaf_build`, `hlod_incidence`, `hlod_geometry` and `archive`.
  `mesh --progress json` emits those phases and logs each phase's wall seconds.

- `validate --expectProfile hlod` (`--expect-profile`), Rust
  `ValidationRequest::expect_profile` and Python `validate(..., expect_profile="hlod")`
  require the HLOD profile whatever an archive's labels say: every payload must carry
  source identity and every HLOD check runs. An archive that declares HLOD itself is
  held to it anyway. Any other profile name is a usage error (`ValueError` in Python).
- Large-mesh leaves (H8 texture pipeline): an ID / class texture class for
  `EXT_mesh_features` feature ID textures and categorical
  `EXT_structural_metadata` property textures, kept for a
  future annotate pass. `mesh` does not publish ID textures (coarse tiles carry feature IDs
  as per-face vertex attributes only; see the tile batching entry). Any number of feature-ID sets (1..=4 bytes each) are
  packed into RGBA8 PNG pages. Each set gets its own `channels`. Gutters and
  padding hold the no-feature value, so each set's `nullFeatureId` is
  2^(8*bytes)-1. Mips use a support-weighted mode and never average IDs. Each
  mip level is a separate exact PNG. The sampler uses
  NEAREST/NEAREST, so viewers generate no mipmaps. Basis KTX2 is refused for
  this class. Leaves may carry extra `featureIds` entries after the two source
  sets. The writer refuses any ID texture whose sampler is not NEAREST.
- Mesh tiler (`rusty-tiles mesh` / `mesh_to_tileset`): `--source-crs` (`local` by default, or a
  2D horizontal EPSG/WKT/PROJ definition through the shared conservative resolver) with
  `--source-axes y-up|xyz` (default `y-up`), `--height-offset` (default 0), and either
  `--source-offset E N [A]` added in double precision or `--source-offset-file` holding exactly
  the lines `E:`/`N:`/`A:` (a file you write, for example with a shift typed into Metashape's export); Python takes `source_crs`, `source_axes`, `source_offset`,
  `source_offset_file` and `height_offset`. Capture maps every position exactly in f64 into one
  ENU frame at the mapped source-bounds centre; normals use the Jacobian inverse transpose and
  tangents the Jacobian followed by Gram-Schmidt against the normal, with Jacobians interpolated
  from an adaptively checked lattice. The root transform carries ENU to ECEF; later stages are
  unchanged. A source CRS excludes `--anchor`, `--orientation-xyzw` and `--scene-offset`. Legacy
  `auto`/`geographic`/Web Mercator keywords are not accepted; EPSG:4326/3857/3395 and UTM work
  in the standard build. The report gains `source_coordinates: "source-crs"` and a `source_crs`
  object.
- Every release artefact now ships the licence notices of the Rust crates it contains. `scripts/third_party_licenses.py` generates `licenses/THIRD_PARTY_LICENSES-{cli,native-geospatial,python}.md` from the resolved dependency graph of each artefact (259, 272 and 264 crates) and each crate's packaged licence files, with SPDX standard text for the 14 crates that publish none. The release archives carry `THIRD_PARTY_LICENSES.md`, the wheel carries the file in `.dist-info/licenses`, and the container image carries it in `/usr/share/doc/rusty-tiles/` together with the GDAL and PROJ licences and a table of the GDAL, PROJ, GEOS (LGPL-2.1-or-later, dynamically linked), SQLite and libtiff libraries it links. The generator fails on any licence outside an allow-list of permissive licences (and MPL-2.0); none is outside it today.
- CI checks in seconds, without building, that the committed licence files match a regeneration, and release, wheel acceptance and the crate package check require the file in each artefact.
- Add finite vector source-to-published-forest acceptance for analytic geometry,
  topology, exact metadata/identity, placement, bounds and reuse, plus paired
  CLI/Python readback and realistic resource receipts for the 0.4.0 bar.

- Verify explicit fill winding and missing triangles; derive finite-fixture
  position tolerance from physical encoding cells instead of producer error reports.
- Validation limits surface (VS-C): `rusty-tiles validate IN --maxVolume KEY=N` (repeatable; alias `--max-volume`) sets an opt-in volume cap (`sourceArchiveBytes`, `archiveEntries`, `centralDirectoryBytes`, `archiveStoredBytes`) or replaces an amplification budget (`hierarchyVisits`, `references`, `totalBytesRead`, `totalPayloadElements`, `containmentWorkUnits`, `containmentBytesRead`, `containmentDecodedBytes`); values below the defaults are allowed, per-item format limits cannot be changed. `--scratchDir DIR` (alias `--scratch-dir`) puts validation scratch inside an existing directory (a memory-backed one is used and reported as `scratch.memoryBacked`). Python `validate(input, *, lazy_payloads=False, max_volume=None, scratch_dir=None, callback=None)`: `max_volume` is a mapping of the same keys to ints, `callback` receives `validate.hash`, `validate.traverse`, `validate.stream` (when decoded content bounds are streamed) and `validate.rows` progress events, and raising from it (or Ctrl-C) stops the validation with that exception. Rust: `validate::LimitKey`, `ValidationRequest::max_volume`, `ValidationRequest::scratch_dir` and `validate::inspect_with(request, &RunControl)` (cancellation and progress; the CLI's `--progress json` emits the events). Bad keys or values and a missing scratch directory are `usage` (exit 2; Python `InvalidRequestError`) before the input is opened. The contract is [validator-limits-amendment.md](docs/architecture/validator-limits-amendment.md).
- The validation report gains `storedBytes` (S, the stored member bytes, index included), `limitDerivation` (`{"model": 1, "budgets": {KEY: {"floor", "perStoredByte": [p, q], "source": "floor" | "scaled" | "explicit"}}}`) and `limits.liveDocumentBytes`. These depend only on S and the overrides.

### Changed

- Breaking validation (H7): legacy 3D Tiles 1.0 content (b3dm, i3dm, pnts and cmpt) is `unsupported` (exit 2, Python `UnsupportedError`) and is never partially inspected: "legacy FMT content is unsupported (glTF 2.0 content only): MEMBER". A payload is legacy when its member name ends in `.b3dm`, `.i3dm`, `.pnts` or `.cmpt` (ASCII case-insensitive; decided before the payload is read) or when its first four bytes are one of those magics under any other suffix, including `.glb`, `.gltf` and `.bin` (the message adds "starts with FMT magic"). An implicit content template whose normalized last segment has a legacy suffix is refused at root admission, before any subtree is read: "implicit: unsupported legacy FMT content template in DOCUMENT". The rule applies to tileset content, implicit content, `conversion.json` `retainedContentUris` and `rustyTilesSourceContents`. Exit-status changes: archives with valid b3dm content, which validated (b3dm with `RTC_CENTER` reported `decodedContentBounds.rtcCenter`), are now `unsupported`; malformed b3dm, and `.glb`/`.gltf` members holding legacy bytes, move from `invalid_input` to `unsupported`; `.i3dm`, `.pnts` and `.cmpt` content stays `unsupported` with the new message. A missing legacy member is still `invalid_input` ("missing archive entry", checked first), and for a present member a tile's "content bounds escape tile" is still reported before the legacy verdict. Report change: the C2 reason `rtcCenter` is removed, so the frozen `decodedContentBounds.*` enum has 14 names; the C1 `notInspected` label "b3dm feature and batch table semantics" is gone. C2-WB `J(u)` no longer counts b3dm tables. glTF-only archives keep byte-identical reports. Producers have written only GLB since #185; convert older archives again.
- Breaking large-mesh leaf budget: every profile's leaf now holds at most 8,192 triangles (`fast` was 32,768, `balanced` 16,384), so each leaf's `source_triangle` and `source_primitive` property tables fit CesiumJS 1.146's 8,192-texel property texture and picking stays exact per triangle. `fast` and `balanced` trees have up to four and two times as many leaves; the leaf writer refuses a wider table as `invalid_state`. (The hidden `mesh-leaves-to-3tz` command that refused older leaf directories with wider leaves was replaced by `mesh` before release.)
- The streamed forest writer refuses an external tileset document named outside the archive root (`a/b.json`), because the member names it writes resolve relative to the document. Existing producers name documents at the root and are unaffected.
- The vector encoder identity covers `implicit/tileset.rs`, which changed with H16, so vector build state and `tileset.json` `asset.extras.vectorBuildStateSha256` change and `--reuse-tileset` rebuilds earlier archives once. Payloads and subtrees are byte-identical.
- Vector tilesets are written from typed tile records (M1 slice 1). The producer builds its hierarchy as owned records, and `tileset_node` is the only emitter of vector tileset and tile JSON; the implicit forest takes the records directly instead of re-reading producer JSON. Archive members are byte-identical to the output before this change except the encoder identity (`vector-build.json` `config.encoder`, the record signatures derived from it, and `tileset.json` `asset.extras.vectorBuildStateSha256`), so `--reuse-tileset` rebuilds earlier archives once. New refusals, each publishing nothing: a staged payload whose length or SHA-256 no longer matches its content record ("staged content … nothing was published"); full-detail leaves that do not hold every source fragment exactly once ("vector hierarchy accounting"); and a previous `explicitRoot` with members, number forms or content this producer does not write ("previous vector hierarchy: …; run a fresh conversion without reuseTileset"), which replaces "previous content URI is not an immutable vector content path".
- Breaking point-cloud tile budget: `--maxPoints` (Python `max_points`, Rust `PointCloudOptions::max_points`) now defaults to 8192 and must be at most 8192 (`point_cloud::MAX_POINTS_PER_TILE`); larger values are refused as `invalid_request` (exit 2) before any output work. The previous default was 50,000. Each tile holds one `points` property table with a row per point, and CesiumJS 1.146 uploads that table as one GPU texture `count` texels wide: for a table wider than the WebGL maximum texture size (8,192 on the pinned Chromium target; the device limit varies) it warned `Cannot create a texture for the property table "points"` and dropped GPU metadata access (custom-shader styling by point properties) for the tile. Splitting a tile into several primitives and tables does not help, because a CesiumJS model picks and styles through one property table. Sampled parents use the same cap, so their voxel grid is 20³ instead of 36³. Feature IDs are always 16-bit; the float feature IDs used above 65,536 points per tile are gone. Default archives have more, smaller tiles: the 200,000-point perf-lane cloud goes from 9 to 112 tile contents and from 19.6 to 21.2 MB (+8.4%), and its default CesiumJS view now requests 2.1 MB instead of all 19.5 MB. Known limitation: legacy `mesh-local-to-3tz` still writes one `source_triangle` table per leaf, so `--leaf-triangles` above 8192 gets the same warning; that writer retires with the new mesh tiler.
- `glb-to-3tz` and `createTilesetJson` write `tileset.json` through the shared typed tile records (M1 slice 2), as vector does. Values and every other member are unchanged; the structural constants are now JSON integers: the box's six zero lanes (`0.0` to `0`) and the transform's last row (`0.0` to `0`, `1.0` to `1`). The committed glb-to-3tz and createTilesetJson digests change accordingly. A box or transform that the typed records cannot reproduce bit for bit is `invalid_state`. Internally the typed records gain explicit implicit child slots (`TileTree::with_slots`; availability is derived from the slots) and a streamed forest writer that emits one caller-partitioned document at a time, for the large-mesh tiler; vector output is byte-identical.
- Vector polygon outlines: a partitioned (oversized) polygon's source boundary is now one locked line feature of closed rings, not one two-vertex strip per boundary edge. The line splitter cuts an oversized ring into contiguous runs, and each tile chains a source feature's runs into maximal strips that end only where the ring leaves the tile; a ring wholly inside a tile is one closed strip. Each ring opens and closes at the midpoint of its longest edge (one extra vertex), so a closed strip's seam is collinear. CesiumJS draws strips butt-ended without joins, so the per-edge outline rendered as a notched comb at wide line widths (Sudan: 80 two-vertex strips, now 2). With `--quantize`, the shared position range of a tile with line strips is padded so that CesiumJS 1.146's synthetic strip-end neighbour (2c−n, stored in the u16 position type) cannot wrap into a long spike; this costs precision, not bytes (the quantization step on a tile with line strips can be up to three times coarser; about 3 m on the Sudan fixture). Outlines keep the source feature's metadata and picking. Polygons that are not partitioned still get no outline line: their `EXT_mesh_polygon` loops are the source boundary. Archive bytes change; fragmented fixtures shrink 13–16% (payload bytes) but use more tiles, because outline runs are spool rows of their own.
- Breaking vector fills (#75): fragmented polygon fills are glTF vector content, no longer `b3dm`. Each fill triangle is one `EXT_mesh_polygon` polygon with its own loop, published in the same `.glb` content as the polygon's source boundary lines, so a tile has one content and one request. Polygon rings now follow the pinned draft winding: exteriors are counterclockwise and holes clockwise, seen from local up. Every loop keeps its polygon's triangles on its left, and a polygon with a vertex outside its loops is refused when its feature is read. A parent-tile simplification that the encoder would refuse keeps the source polygon (reported as "simplified polygon would not encode; source polygon retained") instead of failing the build. Previously, best-fit polygons faced an arbitrary side, and some thin geographic triangles had loops wound against them. Loop indices and polygon offsets are 16-bit when they fit. The encoder refuses a polygon primitive with more vertices than loop indices, which CesiumJS 1.146.0 would render with missing triangles. The implicit forest writer and its payload rebase accept GLB/glTF content only. `--reuse-tileset` refuses an earlier archive with `b3dm` fills ("previous archive has legacy b3dm polygon fills"); convert once without it. Archive bytes change. The issue #75 fixtures shrink 8–16% (−15 to −21% of payload bytes), and the perf-lane vector archive shrinks by 2.3%. `validate` reports `b3dm` content as `unsupported` (H7, above).
- Breaking `doctor` contract (T1): each command reports `status` `ready`, `unavailable` (runtime dependency failed) or `unsupported` (not in this build), with a `reason`. Each command that is not ready reports a `refusal` with the category and exit status the command itself returns. Its native readiness uses the command's own capability check where it has one. `doctor` exits with the first refusal in name order, which is currently always `unsupported` (exit 2) and no longer `environment` (exit 4). One exception is recorded: without a GTiff driver, terrain and `raster-tile-to-directory` report `invalid_input`. Native vector no longer requires the PROJ database. Terrain and `raster-tile-to-directory` now require the GDAL GTiff driver that they open; before this change they were reported ready without it. Reports list the `probes` that ran and run native probes only for the selected commands. PROJ directory listings are capped per directory, and concurrent library reports no longer abort inside GDAL. In Rust, `doctor::Command`, `Status`, `Probe`, `Capability`, `NativeGeospatial`, `Readiness`, `ProjInventory` and the typed `Report` (with `Report::refusal`) replace `COMMANDS`, `canonical`, `DEFAULT_CESIUM`, `display` and the `Value` result. JSON readiness errors move from `error` to `reason`. `doctor` no longer lists the retired `convert-to-implicit`.
- `preview` now loads R2 raster pyramids. Cesium 1.146 stopped rendering on their tile-aligned TileJSON bounds, so the imagery provider rectangle is inset by 1/1024 of a finest-zoom pixel in longitude. The TileJSON output is unchanged. Release acceptance checks this in a browser.
- Breaking `preview` network contract (T1): `--host` takes an IP address, and `Preview::serve` takes a `SocketAddr`. On a loopback address, requests whose `Host` is not `localhost`, `127.0.0.1`, `[::1]` or the bound address receive 421, which blocks DNS rebinding. IPv4-mapped `::ffff:127.0.0.1` counts as loopback. A non-loopback address is an explicit opt-in that prints a warning and reports `"loopback": false` in the JSON startup line. Request path rules no longer depend on the platform: trailing slashes, empty segments, Windows short names, trailing dots and every symlink below a selected directory return 404, and case or Unicode-normalization variants either name a different file or fail the exact stored-name comparison; symlinks inside the directory were previously served.
- Point-cloud conversion now uses one typed request and F0 lifecycle through Rust, CLI and Python. Rust callers migrate from the `point_cloud_to_3tz*` overloads to `point_cloud_to_archive` with explicit coordinate intent and request-owned output policy. Python `point_cloud_to_3tz` requires `source_crs` and returns `PointCloudResult` with cleanup diagnostics. Callback and cancellation failures abort before publication, and source/output aliases are refused.
- Point-cloud admission requires positive finite LAS/Extra Bytes scales, bounds metadata allocation before decoding, and preserves Extra Bytes no-data/min/max schema declarations. Raw 64-bit scalar integers remain exact; scaled 64-bit integer Extra Bytes are explicitly unsupported rather than silently rounded through binary64.
- Validation adds C2 decoded-content containment: `decodedContentContainment` proves, with exact arithmetic, that every drawn vertex of an explicit in-domain archive lies inside its tile, content, external-content and ancestor volumes. Out-of-domain archives (implicit tiling, regions, rotated boxes, RTC, skins, morph targets, animation, unlisted extensions, span or work caps) list `decodedContentBounds.<reason>` in `notInspected`. A proven escape is `invalid_input`. The C1 payload label "scene transforms, skins, animations, morph targets and rendered bounds" splits into "scene transforms" (removed only when containment is proven) and "skins, animations, morph targets and rendered extent". Breaking: `ValidationLimits` gains five `containment*` fields (default work cap 2^29 units since C2-WB), and `ValidationFailure::InvalidState` (`invalid_state`, exit 1) reports internal invariant breaches. glTF node graphs are now admitted strictly (cycles, shared or duplicate children and malformed transforms are `invalid_input`).
- Breaking C2 work bound (C2-WB): `containmentWorkUnits` now charges per operation (per-level floors for the base, step, box and sphere tests; a 256-unit node floor; per-reference charges for re-read and re-decoded bytes, 16 per JSON value node, 160 per external buffer or image entry, 4 per instance primitive, 1152 plus 8 per level), and the default rises from 2^28 to 2^29 units. On the measurement host every class of the C2-WB matrix costs at most 14.65 ns per unit on an E-core and 8.10 ns on a P-core, so C2 work at the default cap stays within about 7.9 s (E-core) in release builds; the slice-1 units allowed 193–776 ns per unit on cheap-unit shapes. Archives with many references to large or JSON-heavy payloads, or with many external resources, can now report `decodedContentBounds.workLimit` where they were Contained; large-span archives can move the other way. JSON admission is now a single O(bytes) pass and drawn-vertex marking is linear in accessors plus primitives; verdicts, failure categories and counts are unchanged. The serialized `limits` report `containmentWorkUnits: 536870912`.
- Breaking validation (C1 meshopt headers): every `EXT_meshopt_compression` stream must start with the one header byte the pinned extension text (KhronosGroup/glTF `5ccac1cb`, Appendix A) gives for its mode: `0xa0` for ATTRIBUTES, `0xe1` for TRIANGLES and `0xd1` for INDICES. The linked meshoptimizer decoder also reads vertex codec v1 (`0xa1`), index codec v0 (`0xe0`) and sequence v0 (`0xd0`), so archives holding such streams validated. They are now `invalid_input` (exit 3) with "meshopt stream header 0x.. is not the 0x.. EXT_meshopt_compression requires". The check runs after the filter, mode, stride and count checks and before decoding, so a non-NONE filter is still reported `unsupported` first. Other wrong header bytes stay `invalid_input` but now report this message instead of "invalid meshopt compressed stream"; an empty stream reports "meshopt stream header missing". `encode-vector-content` now refuses an input view that already carries a stream with a forbidden header (`invalid_input`) instead of copying it into output that `validate` rejects. Freshly encoded stream bytes are unchanged (`0xa0` and `0xe1`), but the vector encoder fingerprint covers the changed sources, so vector build state and archive bytes change, and `--reuse-tileset` rebuilds archives made before this change instead of reusing them.
- Validation ceilings are raised so default (implicit) vector output at the 100,000-tile `--maxTiles` default validates. Archive entries rise from 65,536 to 524,288, hierarchy visits from 65,536 to 262,144, references from 262,144 to 1,048,576, JSON value nodes per document from 65,536 to 131,072, the central directory from 16 to 32 MiB, bytes per member from 64 to 128 MiB, and decoded views per document (and, with the I1 implicit reader below, the live implicit subtree path) from 64 to 128 MiB. Such archives use about 2.5 entries per tile, and the vector build state is about 92 MB at 100,000 tiles. Content-volume ceilings are unchanged: 1 GiB archive, 2 GiB of reads and 16 million payload elements. Dense vector content can still reach them below 100,000 tiles. Breaking: the serialized `limits` report these values. The hierarchy-visit and reference refusal messages now state the configured ceiling. Measured time and peak RSS at the ceilings are in `bench/architecture_audit/validator_caps`. The absolute volume caps named here (archive entries, central directory, archive size) and the fixed hierarchy-visit, reference and read budgets become opt-in caps and archive-scaled floors in the VS-C and VS-B entries below.
- Breaking implicit layout (P): the bounded implicit forest replaces chunked external-JSON implicit output in `point-cloud`, `vector`, `mesh-to-3tz` and the small-model wrapper. Tiles with at least four levels of descendants, and their leaf children, stay explicit; every other child becomes an implicit root with exactly one binary subtree (`subtrees/r{n}-…`), constant-zero child-subtree availability and actual bounds and error rows on every tile, so no viewer culls by a derived regular cell. Implicit content is `implicit-content/r{n}-{level}-{x}-{y}[-{z}]-{slot}.{suffix}`, and no implicit content template names a JSON tileset. Large explicit skeletons are split into explicit external tileset documents `tileset-d{m}.json` (at most 16,384 JSON values, 2 MiB and JSON depth 32 each) referenced by explicit link tiles. Hierarchies with more than 32 levels are a typed error. Encoders become `…-implicit-v3`, so archive bytes and conversion reports change intentionally. `--explicit` kept the explicit layout and payloads (point-cloud and vector `--explicit` were later removed; see Removed); its files still changed slightly: point-cloud `tileset.json` is now compact JSON in every mode, and vector `vector-build.json` and the `tileset.json` state hash change with the encoder revision and the new (empty) payload map. Library: `implicit::write_forest`, `ForestOutput`, `WrittenTile`, `WrittenContent`, `TileRole`, `RebaseRecord`, `rebase_payload` and `restore_payload` are public; `expand_tileset` follows explicit children, link tiles and documents, expands every implicit root and no longer follows JSON content templates.
- Validation (3TZ index): the 128 MiB per-member limit no longer applies to the `@3dtilesIndex1@` index, which would refuse any archive with more than 5,592,406 index rows. The default 524,288-entry and 32 MiB central-directory limits still refuse such archives first (VS-C later makes them opt-in). The index is admitted at exactly 24 bytes per other member, fixed by the central-directory count, and is now read in 64 KiB chunks instead of whole. Every other member keeps the per-member limit. Breaking: `validate` enforces the producer's 178,956,970-member 3TZ ceiling, so the largest index is 4,294,967,256 bytes. An archive declaring more members is `unsupported` (exit 2) with "3TZ member ceiling: N members exceed 178956970", reported before the entry limit and any directory allocation; such archives were `resource_limit` before. Python reports `UnsupportedError`. The catalog, the index offset table and the per-entry flags remained resident at this point (about 130 bytes per entry measured at 5.6 million entries); the VS-A entry below bounds them.
- Default archives no longer retain duplicate raw sources. Explicit tiles publish raw payloads in place, and implicit tiles publish one copy whose glTF scene root carries the tile's translation; only that root node's text changes, so the raw bytes are exactly recoverable. `rustyTilesSourceContents` is no longer written. Implicit vector archives are about half their previous size, close to `--explicit` (vector perf model 37.1 MB to 17.8 MB of tiles; 12.5k-tile LOD chains 148 MB to 67 MB).
- Breaking vector reuse: `--reuse-tileset` recovers unchanged payloads byte for byte from the earlier archive's published tiles. `vector-build.json` gains a `payloads` map with each implicit copy's member, translation, raw root-node text and raw JSON length; every recovered payload must match its SHA-256 name, otherwise reuse fails with `previous content checksum failed` or `invalid previous payload record` and nothing is published. The reuse identity is `implicit-forest-v3`: archives from earlier builds are refused with "run a fresh conversion" and need one fresh conversion to become a baseline.
- Breaking validation (I1): `validate` reads implicit roots through an independent implicit reader, on any explicit tile including inside explicit external tilesets. Every reachable subtree, availability bit and available tile is checked, and each implicit tile receives the explicit-tile payload, `hierarchyBounds`, `geometricErrorOrder`, `resourceReferences` and report-level budget checks. Report changes for archives with an implicit root: `checks` gains `implicitAddressingAndAvailability` after `hierarchyBounds`; `notInspected` loses it and gains `recordedBudgets.implicitTileExtras` (per-tile `extras` budgets are not compared below an implicit root); `decodedContentBounds.implicitTiling` and `metadataSemantics` remain. Counts: `tiles` is explicit tiles plus available implicit tiles, with each implicit root counted once, and `contentReferences` counts every content entry read: explicit payloads, external-tileset (link) contents and emitted implicit contents; both are unchanged for non-chunked archives. `budgets.tiles` excludes link tiles. External tileset documents are not tile payload, so per-tile byte totals cover payload bytes only. This applies both to the byte budget and to the recorded `extras.encodedBytes` comparison: a tile that mixes payload and external-tileset contents is compared on its payload bytes, and a tile whose only contents are external tilesets has no recorded byte comparison. An explicit archive whose recorded value included the JSON size now fails with "recorded encodedBytes differs from payload". Older chunked default archives (most vector and deep point-cloud or mesh outputs, which used `implicit-tileset-*.json` documents) and multi-node archives written by the removed `convert-to-implicit` now validate as Unsupported: "implicit: unsupported external tileset content template in <member>"; convert them again. Other new Unsupported messages, each prefixed "implicit: unsupported": "JSON-format subtree <member>", "extensions in subtree JSON <member>", "repeated subtree member <member>", "region implicit root volume", "subdivisionScheme …", "subtreeLevels above 32"/"availableLevels above 32", "3DTILES_implicit_tiling (1.0) extension", "escape adjacent to template variable", "template variables eliminated by dot-segments", "normalized template exceeds 1 KiB", "derived geometric error underflow …", and TILE_*/CONTENT_* semantics such as `CONTENT_URI` in a used metadata class. New ResourceLimit messages: "availability scan exceeds total_bytes_read work limit" (availability scans share the 2 GiB read-work counter), "member exceeds admitted byte limit: <subtree member>" when reading a subtree would take the live subtree path past the 128 MiB `document_decoded_bytes` limit, "subtree content entries exceed remaining references" and "… exceeds remaining hierarchy visits". The implicit presentation-node and expansion budgets are gone. Sparse deep subtrees that previously hit those budgets now validate, empty `minItems` arrays such as `"contentAvailability": []` and `contentMetadata` count mismatches are `invalid_input`, and translated or non-root implicit roots are checked against their explicit parent.
- Breaking validation (C2 slice 3): `decodedContentContainment` now covers implicit archives. Every content the I1 reader emits is a reference, tested exactly against its tile's effective volume (the `TILE_BOUNDING_BOX` row, FLOAT32 widened exactly, or the root box for a rowless level-0 tile), every implicit ancestor's, the level-0 tile's, the implicit root's own JSON volume, the referencing external-content volume after the root's transform, and every explicit ancestor's. The `notInspected` reason `decodedContentBounds.implicitTiling` is removed; `decodedContentBounds.derivedImplicitVolume` is listed when a reference's chain passes through a rowless tile at level 1 or deeper, and a template header with `metadata` is `tileMetadata`. Default point-cloud, vector and `mesh-to-3tz` archives now validate with `decodedContentContainment` (subject to the 2^29-unit cap); implicit archives with a proven escape, such as content that skips the glTF y-up rotation, now fail as InvalidInput. No public Rust API change.
- Breaking raster foundation (R2): `raster_to_pyramid` with one typed `RasterRequest`/`RasterLimits` and directory publication replaces the public `raster` module. Output is exactly `source.cog.tif`, `tiles/{z}/{x}/{y}.png`, `tilejson.json` and `report.json` (replacing `conversion.json`). The COG preserves full-resolution typed samples, validity, georeferencing, NoData, scale/offset/units and palette; the XYZ pyramid samples every zoom directly from the closed finest raster with exact integer nearest indices, east/south ties selecting the greater fine index, and zero-RGBA padding. Tiles are encoded in Rust; GDAL's tile algorithm is no longer required. Report bytes do not depend on the output location. Refusals use the typed error categories (`unsupported` exits 2, limits exit 1), including a portable build without native raster support.
- Breaking terrain foundation: bounded EPSG:4326 Float32/Float64 DEMs produce 3D Tiles 1.1 GLB meshes through one typed request and directory publication attempt. Source-footprint cells replace zoom/simplification options and quantized-mesh output. The pinned Cesium preview includes mesh surface queries, clamping and imagery draping.
- Breaking terrain encoding: tile GLBs require `EXT_meshopt_compression` and store positions as unnormalized uint16 `KHR_mesh_quantization` on a power-of-two grid per level (leaves stay Float32 where that grid could move a vertex more than 0.05 m). Each tile's skirt now shares the surface primitive and material, so a tile is one draw call. Geometric errors, skirt depths and boxes are certified on the decoded quantized positions; coarse levels now round shared lattice nodes to their own grid rather than repeating leaf positions. On the perf-lane DEM the output is 4.3x smaller (38.1 to 8.8 MB) and fully refined Cesium issues half the draw calls. `conversion.json` is schema 3 with `quantizationStepMetres`; `skirtVertices` counts only skirt bottoms, and `positionErrorMetres` covers the leaves.
- Audited the existing draft vector contract against CesiumJS 1.146.0 and corrected its source references. Output bytes and converter identity are unchanged. Repeatable browser checks documented why fragmented fills needed b3dm wrappers (fills have since become glTF vector content, see the #75 entry), and validation evidence distinguishes unsupported primitive-restart checks from producer errors.
- JPEG uses the portable Rust encoder by default. System libjpeg-turbo now requires the explicit `native-jpeg` feature; `RUSTY_TILES_DISABLE_NATIVE_JPEG=1` still overrides it for portable builds.
- README installation starts with prebuilt downloads. A simpler header figure and an invented mesh example make the quick start work with the default build.
- Doctor, machine protocol, diagnostics, preview, force replacement and archive validation tests now run in Rust. The vector Python oracle loads only in tests that compare it with the native converter.
- Benchmark harnesses, public-data audits and recorded evidence now live in `bench/`.
- Converter help lists `-i`, `-o` and `-f` first.
- Installation and converter guides distinguish development capabilities from published releases, explain build selection, and retain the existing mesh offset options with application-neutral help.
- `doctor --command` accepts aliased subcommand spellings. It takes its list from the same table as the readiness report.
- `--json` conversion results add a `settings` object. `counts` now holds only genuine counts, such as `points`, `tiles` and `features`. Settings such as `heightOffset`, `grid` or `lodLevels` moved from `counts` to `settings`.
- Without `--json`, every converter prints a one-to-three-line stderr summary: output, counts, warnings and next command. It replaces the previous ad hoc point-cloud summary and the per-level terrain lines.
- Option errors name the offending `--flag` and its valid range, instead of grouping several rules in one message.
- `validate` explains that it checks `.3tz` archives only when given a directory or non-archive file, instead of reporting an OS error.
- `RUSTY_TILES_NATIVE_DIAGNOSTICS=1` enables raw GDAL/GEOS warnings.
- Every converter runs as one job. It uses a private `.tiles-work-*` directory beside the output, removed on success or failure, and no-clobber publication unless `--force`.
- `mesh-to-3tz --progress json` emits its timing lines as `{"event":"log","message":...}`, so stderr stays NDJSON. Other events, phases and exit codes are unchanged.
- Library: converters no longer read `RUSTY_TILES_PROGRESS_JSON`. The CLI no longer sets it or `RUSTY_TILES_JSON_STDOUT`. Library callers that relied on the variable for progress should pass `Reporter::ndjson_stderr()` to a `*_reported` entry point.
- Roadmap and public-surface inventory record two owner decisions: the `mesh`
  tiler's declared source CRS (#218) is part of the 0.4 surface, and retiring
  `mesh-to-3tz` / `mesh-local-to-3tz` is a 0.4.0 gate rather than 0.4.x work.
- Clarify and test that R2 imagery pyramids are native Rust/CLI capabilities;
  the standard Python wheel retains its explicit unsupported D1 boundary.
- `mesh`: capture stores only the vertex attributes that some selected
  primitive carries. Its sort records and its retained triangle sequence
  used to reserve 60 bytes per corner for every attribute; a source with
  normals and colours now stores 190 bytes per triangle instead of 286, and
  one with a single texture coordinate set stores 130.
- `mesh`: the cell tree is built over capture's retained triangle sequence,
  so capture no longer writes a second, 93-byte-per-triangle copy in rank
  order. On a 788k-triangle crop the measured whole-run scratch peak falls
  from 401 to 293 bytes per triangle (normals and colours) and from 524 to
  357 (texture coordinates and images). Archives are byte-identical.
- HLOD local closures are measured on changed oriented faces touching a closing
  boundary cluster, its original edges and projected cycle-centroid controls.
  Other changes in that component contribute through S2 and moved-boundary
  witnesses. Retained faces remain boundary-to-surface witnesses; bounded interior
  remeshing cannot hide an unchecked closure. N3 retains its broader drop/closure
  domain, with loss measured into G.
- CI: the dev and test profile builds the crate at opt-level 1 and its dependencies at opt-level 2, with debug assertions and overflow checks still on. The geometry-heavy large-mesh tests ran unoptimized and took most of the Rust job, which hit its 50-minute limit on #296. The library unit tests drop from 10–12 minutes to 1–1.5 on a 4-job host, and `target/debug/rusty-tiles`, which the oracle steps run, is faster too. Release builds are unchanged.
- CI: the Rust job runs the library unit tests in parallel. Running them on one thread took about 21 of the job's 37 minutes and pushed it past its 40-minute limit. The C1 context controls still run serially, because the payload-context receipt needs their stderr lines intact, as does the one library test that changes the working directory. Binaries, integration tests and doctests also stay serial.
- CI: the Rust, Portable CLI and Official Blender jobs allow 40 minutes instead of 25. macOS x64 runners repeatedly hit the old limit while tests were passing. The Rust job has since grown to about 37 minutes; the parallel unit tests entry covers that.
- CI: pull requests now run a fast Linux tier: `Rust`, `Python` and native geospatial for GDAL 3.13 only. The portable and encoder-link matrices, Python wheels, wheel acceptance, the Python-free runtime and GDAL 3.12 moved to a full tier that runs on pushes to `main` and `develop`, nightly and manually. A failing push or nightly run on `develop` opens or updates one "Full CI matrix failing on develop" issue, and the next green run closes it. The release workflow now requires the full tier's `Full matrix` and `Rust` jobs for the tagged commit, so a pull-request run cannot satisfy it.
- CI: `playwright install-deps` is bounded to 180 seconds, because a stalled apt mirror held browser jobs for minutes.
- `mesh` output layout: shallow subtrees (height 1 to 3) are now written as explicit
  tiles while their document has room, and only become implicit roots when a
  document exceeds its caps. Documents are packed to 480 tile objects, 4,096
  records and 12 levels instead of being split at 32 objects. This cuts the
  dependent requests a viewer needs before the first content. In a simulated
  CesiumJS first view of the 2048-texture output, requests fall from 102 to 87
  and the deepest dependency chain from 5 to 3 round trips. Content GLB bytes are
  unchanged. Inlined tiles' payloads move from `implicit-content/…` to
  `content/f<first>-d<depth>.glb`, and the JSON, subtree and member set of every
  mesh archive changes.
- User documentation now leads with `rusty-tiles mesh` and Python `mesh_to_tileset` as the one public mesh entry point. The README command table, quick start, conversion examples and library snippets, the converter guide, install and demo guides use `mesh`, and the legacy `mesh-to-3tz` and `mesh-local-to-3tz` commands are described as deprecated with a link to the mesh migration notes. The quick start converts the new `tests/fixtures/example.glb`, the README pyramid packed as one GLB, because `mesh` refuses the `data:` URI buffer in `example.gltf`. The README licence line states the Cargo licence expression `MIT AND Apache-2.0 AND BSD-3-Clause`, with the vendored Basis Universal and Zstandard terms explained, and the Cesium sRGB note in the mesh guide gives the measured worst case instead of a private evidence path.
- Release binaries and the native container now smoke-test `mesh` and validate its output, including a textured source that exercises the in-process KTX2 encoder.
- Documentation rewritten for the 0.4.0 release: the README leads with the standard package
  (installer, Windows ZIP, `pip install rusty-tiles`, Cargo) and a three-command quick start,
  and keeps the native-geospatial build to one section. The guides drop development-branch
  wording and internal design codes, gather 0.3 migration notes in one place per guide, and
  document `raster-tile-to-directory` options and every `validate` check name.
- Documentation: the mesh guide explains how CesiumJS 1.146 treats sRGB colour pages and lit versus unlit tiles, with what is verified and what is not. The roadmap records milestone-4 status from merged PRs and the work that remains. Unreleased changelog entries that later changes superseded (textured coarse tiles, hidden leaf-delivery commands, `--explicit`, validator caps, `b3dm` fills) are corrected, and the legacy mesh commands are listed as deprecated. The licence notes state that Rust dependency notices are not yet bundled in release artefacts.
- Mesh colour textures (leaf and coarse base colour/emissive pages) now default
  to KTX2 ETC1S: quality 50 in `fast`, 90 in `balanced`, effort 0 in both.
  `quality` keeps UASTC and `lossless` keeps PNG. ETC1S accepts more RGB error
  than the previous
  UASTC gates (24/20 dB for large/small): per-channel PSNR floors are now
  20/16/13 dB for large/middle/tiny supported levels, with wider local-error
  and decoded sampler caps. Alpha,
  MASK, data, strict authored correspondence and escalation remain active.
  Tiny levels additionally require combined RGB PSNR of 16 dB, rejecting
  correlated colour damage while admitting one-channel ETC1S palette tails.
  On tex3 the matched two-worker sweep costs 47.1/56.5 CPU-s for fast/balanced
  raw encoding, versus 16.1/18.2 CPU-s for the previous UASTC pack 0 codec.
  Effort 0 reduces the initial effort 3 cost substantially, but ETC1S still
  costs about 3× that previous codec's encode CPU in exchange for fewer bytes.
  Smooth wrong-mip content can pass these PSNR gates; gross wrong-mip,
  black-page, inverted-channel and light-grey wrong-transfer controls fail.
- Breaking: the preparatory leaf-directory/delivery Rust exports and hidden `mesh-leaf-payloads`, `mesh-leaves-to-3tz` and `mesh-large-to-3tz` commands are replaced by the single source-mesh facade.
- `mesh`: each leaf and coarse tile now publishes one glTF primitive (node, mesh,
  material and texture page set) per material class, instead of one per source
  primitive. Source materials with equal factors, alpha mode, sidedness, bound
  slots and source filters merge, even when they read different images. Coarse
  pages are packed per material class, and MASK leaf pages of one class share a
  page. A primitive still splits for a different vertex attribute set, or for a
  position window that `UNSIGNED_SHORT` cannot hold. Decoded positions and
  every leaf triangle's `source_primitive`/`source_triangle` feature IDs are
  unchanged. On a 0.79M-triangle photogrammetry crop, glTF primitives
  fall from 1,439 to 156 over the leaves and from 1,124 to 109 over the coarse
  tiles (the root from 58 primitives and 110 textures to 3 and 1). A mipmapped
  source sampler (also the default without a sampler) still keeps one leaf page,
  and so one leaf primitive, per source binding in lossy profiles.
- `mesh`: coarse tiles no longer carry `baked_feature_id_*` texture feature-ID
  sets. They repeated the exact per-face feature-ID attributes against the same
  property tables. The ID-texture machinery stays available internally for a
  later annotate pass.
- `preview` uses CesiumJS's default maximum screen-space error of 16 (was 8). This
  applies to every preview layer (meshes, terrain, annotations and point clouds), not
  only to the mesh tiles H28 batches.
- Record the 0.4 classic octree HLOD direction, implementation slices and proposed
  acceptance gates; replace the earlier certified-proxy coarse-mesh requirements and propose a
  focused non-mesh release bar.
- HLOD bakes spend less CPU after the S1b border guards: the nearest-source searches behind S2, D4 representatives and the moved-boundary witnesses avoid redundant heap traffic, node copies and NaN fix-ups, and fold checks group faces and edges more cheaply. Visit order, search budgets and every guard are unchanged, and output is byte-identical.
- Mesh coarse levels use classic octree HLOD with exact leaves and at most 8,192
  triangles per payload. Shared source-arc contracts have canonical same-depth
  borders; contracted arcs have bounded mixed-depth gaps, with junctions and leaf
  contacts fixed. Border strips retain their interior rings and exact collinear
  subdivisions; bounded sheet reduction and local reconstruction recover stalled
  contractions. Memory admission includes live certificates and contract buffers.
  Per-depth diagnostics report topology, borders and omissions.
- After ordinary reduction, reconstruction and staged strategies fail, constructive
  last resorts run in order: N1 admits new shared-point faces/edges only in their
  deterministic seam owner; N2 admits folds (reorienting reversed faces and removing
  excess coincident copies); N3 clusters each input component with interface points,
  opening pins and border-touching open boundaries fixed. N3 permits border-free
  components and holes to drop or close at any size, measured into geometric error G
  rather than bounded by E; validation exempts a verified stage-3 tile's drops from cap
  pressure and size order. Outside N3, coarse drops require cap pressure and drops
  at or above child G go smallest first. All component drops are declared in
  `asset.extras.rustyTiles.droppedComponents` and measured into G.
- Constructive tiles declare strict `asset.extras.rustyTiles.lastResort`
  `{stage, foldFaces}` metadata, recounted by validation; inherited folds use stage 0.
  Omission remains only when the contract alone exceeds the cap or the last resort
  cannot preserve border-touching open boundaries, and propagates to all ancestors.
  Published descendants form the capped frontier; omitted frontiers perform no
  appearance work. Delivery checks total and per-depth omission counts.
- `mesh`: HLOD coarse tiles are indexed. Bit-identical corners share one vertex, so each tile decodes to the same triangles in fewer bytes: tex3 coarse tiles are 0.44–0.76× their former size and crop3's 0.25–0.45×. Leaves and geometric errors are unchanged.
- Compare ownership roots against one additional capped, source-measured candidate; require at least 1% lower geometric error to replace a successful root, while preserving own-build recovery.
- Measure HLOD coarse geometric error in both directions against the captured source
  surface after position decode, with deterministic sampling, a documented 5%
  sampling margin, monotone child floors and versioned distance summaries. Select coarse
  primitive representatives from the nearest source surface and provide bounded
  source queries for subsequent appearance baking. Exact leaves are unchanged.
- Bound every source-to-coarse nearest query with a per-query work budget and a guaranteed
  greedy witness; exhausted searches keep their best (overestimated) distance and are
  counted in `hlod.errorMeasurement.nearest`.
- Admit the source index as walk scratch before writing it, skip omitted cells (they
  hold no content), and keep the sampling density over published source at depths
  with omissions.
- `mesh`: coarse tiles are textured (slice S3 of the HLOD decision).
  Each material class of a coarse tile gets fresh planar charts, each turned to its
  minimum-area rectangle, with small charts merged into their neighbours. The charts
  are packed into one block-aligned page sized to its content: multiples of four from
  32 to 1024 per side, at `max(G/16, 1e-5 m)` per texel but never finer than the
  source's own texels. Pages that can't fit use the 1024 cap at the finest spacing
  that fits. Chart rectangles sit on a 2-texel grid with 2-texel gutters. That keeps
  charts isolated through mip level 1 of each complete authored chain; deeper levels
  average the whole page.
- Every present PBR slot is baked from the exact nearest point of the captured source
  surface. Base colour is multiplied by `COLOR_0` in linear light, and normal maps are
  re-expressed in the coarse tangent frame. Pages go through the same delivery encoder
  as leaves. MASK base colour keeps its coverage. A coarse page that no codec step can
  deliver within its gates is published as its exact level-0 PNG, sampled without
  mips. These pages are counted.
- Untextured input keeps flat class factors, plus per-vertex `COLOR_0` from the source
  when it has vertex colours. The manifest reports requested and achieved spacing, page
  texels and cap hits per depth.
- `mesh`: lossy leaves with mipmapped (or unspecified) samplers now use cropped island
  pages instead of a dedicated page carrying the whole source image plus one period of
  padding. Island interiors sit on the 8-texel source grid with 8-texel gutters. Each
  page keeps a complete authored chain whose first three levels are the source's own
  and never mix islands; deeper levels average the whole page. Lossless leaves and
  leaves with non-mipmapped samplers are byte-identical.
- `mesh`: every level of an authored mip chain is now the area-weighted average of
  level 0 over that texel's footprint, computed directly rather than from the previous
  level. Pages whose sides are not powers of two no longer blur through earlier levels.
- Mesh archives declare themselves HLOD deliveries in `tileset.json`
  (`asset.extras.rustyTiles.lod` is `"hlod"`), so `validate` applies every HLOD
  check without `--expectProfile hlod` and refuses a copy stripped of its
  identity labels. This changes `tileset.json` bytes.
- Leaf texture pages whose source sampler does not mipmap (minification
  `NEAREST` or `LINEAR`) are delivered as level-0-only KTX2 instead of a complete
  chain the sampler never read: less encode work and fewer bytes, unchanged
  rendering.
- The mesh memory budget now charges memory-backed (tmpfs) output and
  intermediate storage: the private work directory and the staged archive each
  take one copy of the output reservation plus 32 MiB from the budget when they
  live on tmpfs (or on an overlay whose upper layer is tmpfs or cannot be
  resolved, as inside most containers; `RUSTY_TILES_STORAGE_DISK=1` declares
  such unresolvable storage disk-backed). Every write into them is admitted
  before it lands, in huge pages on a tmpfs that uses them; one that would pass
  the charge is a `resource_limit` refusal. `diagnostics.memory.memory_backed_storage` reports the charges and
  measurements; `budget.memory_backed_storage_bytes` is their sum.
- `mesh` builds its coarse tiles with the 0.4 classic octree HLOD only. Leaves stay
  exact. Coarse tiles use shared contracted border interfaces, carry
  a **sampled, measured** geometric error rather than a certificate, and carry one
  representative `source_primitive` feature ID per triangle (exact provenance stays on
  leaves, so exact annotation maps to leaves). Coarse textures are fresh
  content-sized charts, and a profile picks one colour codec for leaves and coarse
  pages (ETC1S for `fast` and `balanced`, UASTC for `quality`, PNG for `lossless`).
  Omission follows the residual HLOD policy; a coarse page that no codec step can deliver within its gates falls back to
  its exact level-0 PNG, sampled without mips. The mesh guide, CLI reference and Python
  README describe this; the H16 and H28 contracts carry HLOD notes. Where an earlier
  entry in this release (the H16 mesh delivery) says coarse tiles are certified, read
  it as superseded by this one.
- Reduce repeated HLOD source-support and topology work while preserving coarse geometry and fold checks.
- Mesh HLOD: coarse tiles in `balanced`, `quality` and `lossless` publish a
  texture-aware geometric error. Where a coarse page is much coarser than its
  children's (the 1,024-texel page cap on dense inputs), the tile's error rises to
  `16 · texel spacing / t` (`t` = 2 balanced, 1.25 quality and lossless), so CesiumJS
  refines to the sharper children instead of drawing a blurry coarse tile beside
  sharp leaves. `fast` keeps the measured error. Every HLOD archive declares the rule
  in `asset.extras.rustyTiles.textureError` (`pxPerTexel` null in `fast`); `validate`
  refuses an HLOD tileset without it, refuses textured HLOD payloads in any archive
  that lacks it, checks `pxPerTexel` against the profile recorded in
  `conversion.json`, and recomputes the term from the archive's pages, UVs and
  hierarchy. In `quality` and `lossless` the root's error can rise.
  Leaf/HLOD stage directories written before this change have no texture rule and
  cannot be published; re-run the bake.
- Mesh: coarse HLOD charts and leaf crop islands now pack by their covered shape instead of their bounding rectangles. Shapes interlock without sharing a cell, so mip isolation, gutters and exact leaf sampling are unchanged. Leaf island pages are sized in whole 4×4 blocks instead of powers of two. On the Walem photogrammetry model, coarse page occupancy rises from 0.39 to 0.47 for about 2.2× the (small) chart CPU. Leaf page occupancy rises from 0.18.
- Mesh: coarse HLOD texels are spaced at 1.25·G/16 instead of G/16 (D5 amendment). With D10's error target, this keeps first views within the Metashape byte floor at the measured packing occupancy, with texel sharpness parity where it fits.
- Validation (C1 KTX2): `KHR_texture_basisu` is admitted, including in `extensionsRequired`, so archives with H8's default KTX2/Basis Universal textures validate; they were `unsupported` ("required extension KHR_texture_basisu"). The image a texture's extension `source` references must be declared `image/ktx2` (bufferView images must declare it; a URI image without `mimeType` is judged by its bytes, with no file-name suffix required) and must be a valid KTX 2.0 container for the extension: 2D for material maps, `VK_FORMAT_UNDEFINED`, width and height multiples of 4, a `levelCount` the dimensions allow, ETC1S with BasisLZ or UASTC with no or Zstandard supercompression, the extension's DFD channel sets, no sRGB for Red and Red-Green textures, no premultiplied alpha for material textures, `KTXswizzle` `rgba` and `KTXorientation` `rd` when present, consistent BasisLZ global data, and levels packed inside the file with consistent lengths. A core fallback `source` may not be a KTX2 image, and a texture without a fallback requires the extension in `extensionsRequired`. Failures are `invalid_input`; a valid non-2D KTX2 image not used by a material, and any image data URI, are `unsupported`. Each bufferView is inspected once however many images share it. Only the container is read: level data is not inflated, transcoded or decoded, and the report adds `KTX2 level data and transcoding` to `notInspected`. Texture, sampler and material references are now checked before image resources are read. Contract: `docs/architecture/c1-ktx2-basisu-amendment.md`.
- Balanced mesh leaves now use UASTC level 0 with Zstd for colour textures, improving fine detail at a larger download size. Coarse tiles and MASK/data texture policies are unchanged.
- Vector reuse: upgrading from 0.3 requires one fresh `--reuse-tileset` cache/baseline. Encoder fingerprints cover the vector modules and their dependencies, including source files previously omitted; the first reuse run rebuilds every tile.

- Release binary smoke tests, `scripts/release_acceptance.sh`, the preview-layer fixture script, the runtime benchmark, the perf lane's `mesh` case and the scale-gate examples now run `rusty-tiles mesh` instead of the deprecated commands. The perf lane drops its `mesh-local` and `mesh-explicit` cases. The committed output digests replace the two `mesh-to-3tz` recipes with one `mesh` recipe (lossless profile, one worker); the other recipes' digests are unchanged.
- Mesh tiler (`rusty-tiles mesh` / `mesh_to_tileset`): archives are about 2.5x
  smaller (pohnpei, 24.4M triangles: 2.90 GB to 1.14 GB; 119 to 47 bytes per
  source triangle). The per-triangle provenance tables of leaves and coarse tiles are
  losslessly meshopt-compressed (`EXT_meshopt_compression`, ATTRIBUTES/NONE); their
  schema, column types and decoded values are unchanged. Leaf normals in `fast`,
  `balanced` and `quality` are 12-bit octahedral codes behind the meshopt
  OCTAHEDRAL filter, decoded to normalized SHORT `NORMAL` (at most about 0.06
  degrees from the source normal); `lossless` keeps f32 normals. Profile constants
  remain provisional.
- Validation decodes the `EXT_meshopt_compression` OCTAHEDRAL filter: invalid
  octahedral codes, non-ATTRIBUTES modes and strides other than 4 or 8 are
  invalid input; declared bounds on filtered accessors may differ by one unit (the
  filter's specified precision); QUATERNION and EXPONENTIAL remain unsupported.
- Mesh tiler: a non-UTF-8 `--source-offset-file` is an invalid request rather than an I/O
  error. The output-side scratch reservation is rechecked once image dimensions are known,
  with 4 bytes per source texel for `lossless` PNG pages and 1 for KTX2 profiles (4/3 mips),
  so textured lossless runs are no longer under-reserved. The HLOD incidence pass is measured and
  bounded like the capture stages, and diagnostics add `run_scratch_peak_bytes`.
- Mesh tiler scratch admission: the upfront scratch need is now the largest declared stage
  working set (each external sort charged its streamed runs plus at most one intermediate merge
  group, `S + min(S, F × run)`, and the HLOD incidence pass) with a 10% model margin and
  per-file allocation rounding, instead of the sum of all stages with the whole logical need
  added again as extent overhead. On a 24.4M-triangle mesh the need falls from about
  4,300 to about 1,040 bytes per triangle. Each capture stage rechecks free space for its own
  working set on entry and fails closed if its measured peak exceeds its declared bound.
  Admission also reserves output-side space (leaf staging, archive staging and a replaced
  destination, estimated at 3 × 119 bytes per triangle plus image bytes; 119 was measured before the archive-size reduction, which brings the archive to about 47 bytes per triangle, and is kept as a conservative estimate), summed with scratch
  when they share a filesystem. Diagnostics `capture.scratch_need_bytes` and
  `scratch_logical_need_bytes` change meaning accordingly; new `scratch_stage_bytes`,
  `scratch_stage_peak_bytes` and `output_reserve_bytes` report the model and measurements.
- Spill sort runs are written as chunk files of at most 8 MiB, and readers remove each chunk
  as soon as it is consumed, so a streamed merge and the output it feeds share scratch space
  and intermediate merges free their inputs while writing. Record order and content are
  unchanged. The mesh tiler also releases its rank-ordered triangle projection (93 bytes per
  triangle) once capture has joined corner depths, instead of retaining it through leaves and
  coarse levels. The scratch stage model charges these lifetimes, including the coarse incidence pass.
- Breaking point-cloud format: tiles hold only position and colour by default.
  POSITION is 16-bit per tile axis (`KHR_mesh_quantization`), `COLOR_0` 8-bit;
  the per-point property table (all source fields, 16-bit RGB) and feature IDs
  need `--metadata` / `metadata=True`. About 12 instead of 98 bytes per point.
  A leaf whose 16-bit step would be coarser than the source coordinates keeps
  float32 positions.
- Point parents fill to `--maxPoints`: the first point per cubic voxel on the
  finest grid that fits, instead of a fixed 20×20×20 grid over the tile extent.
- Release workflow: a tag publishes only when CI succeeded for the tagged commit.
  Release-candidate tags (`v*-*`) no longer upload to PyPI, whose files cannot be
  replaced; they still publish GitHub prerelease binaries and the container image.
  The release test run skips the load-sensitive
  `spill::tests::spill_memory_is_flat_in_input_size`, which CI runs for the same
  commit.
- The deep implicit-forest validator test now reads a committed witness archive written by the removed `mesh-to-3tz`, pinned by sha256. The F1a–F1d2 oracle, viewer and wheel drivers that exercised `mesh-local-to-3tz`, and their CI steps, are retired; the six demo-corpus `mesh-to-3tz` cases are dropped. The CI `native-jpeg` test legs now run the default build.
- **Breaking:** `terrain` leaves NoData out of the mesh; `--fill-height`,
  `fill_height_metres` and the report's `fillHeightMetres` are removed.
- `terrain` skirts are sized by the tile's and its neighbours' errors, not
  the root error, so boxes follow the surface. `--cells-per-leaf` defaults
  to 128.
- `validate`: a tile without content may state a larger geometric error than
  its parent; tiles with content are checked against their nearest ancestor
  with content.
- `preview` loads the terrain tileset with `skipLevelOfDetail: true`, and `docs/TERRAIN.md` recommends it. Terrain levels are REPLACE, so a fully refined view fetched every ancestor. On the perf-lane DEM (289 leaves) that is 300 requests instead of 419 and 0.73 s instead of 0.87-1.03 s on a GPU, with a pixel-identical final frame. The output, its geometric errors and the default camera load (2 requests) are unchanged. `bench/perf_lane/cesium.py` gains `--tileset-options NAME=JSON` for such viewer settings.
- `validate`: the default `totalPayloadElements` budget is now 16 million + 224 × stored bytes, up from max(16 million, 16 × stored bytes). The report's `limitDerivation` entry for it gains `"additive": true`. Indexed HLOD coarse tiles take fewer stored bytes for the same validation work. The native HLOD delivery check now charges its scratch-store rows (128 units), original vertex rows (16) and placed original keys (32) at their measured cost. The ratio gives every legitimate producer output measured a 1.5× margin with 3% headroom: the densest, tens of thousands of untextured primitives of tiny repeated triangles whose coarse tiles are stage-3 last resorts, approaches about 144 units per stored byte. At the worst measured cost per unit, the admitted hostile work is at most 0.19 s plus 2,616 s of CPU per stored GB on a P-core and 0.25 s plus 3,476 s on an E-core (see the scratch-row entry), above the C2 statement's 1,037 and 1,875 s.
- `validate`: the HLOD original-vertex alias check uses a spill sequence and one bounded external sort instead of the SQLite scratch store. This is about 3× faster on leaf-heavy archives, and the verdicts are unchanged, with one exception. An archive with both an alias conflict and a non-finite placed position is now always refused for the non-finite position (`interfaceSegments`). Before, the refusal came from whichever the scan reached first. Both are invalid verdicts.
- `validate`: the native HLOD delivery check loads its scratch rows in key order. Leaf points, segments and faces, ancestor requirements, coarse border images, coincident-face twins and multi-edge faces go through one bounded external sort at a time and are appended to the SQLite store, instead of being inserted at random keys. The cost per charged unit no longer grows with the archive: on the random-leaf class (`leafrand`) it stays between 6.0 and 7.5 ns on a P-core from 6 MB to 152 MB stored, where it rose from 7.8 to 26.7 ns at 61 MB. The payload-element time statement now holds for every measured class, including extrapolation to 1 GB stored: at most 0.19 s plus 2,616 s of CPU per stored GB on a P-core and 0.25 s plus 3,476 s on an E-core, at the additive budget. Verdicts are unchanged. Reports are unchanged too, except the running count in a budget refusal (`observed`) and the scratch peak, which now includes the sort runs. tex3 validation takes 25% less CPU, and Walem 42% less.
- `validate`: the HLOD delivery check charges the keyed lookups that coarse ancestors repeat at their measured cost. These are a coarse tile's descendant incidences, edge faces and coincident faces, and a coarse drop's border contacts: 320 units per seek and 32 per further row. Before, only the seek was charged (128 units), or one unit per row. Edge faces are indexed by direction, so such a lookup reads only rows it charges; before, every coarse ancestor read the other direction's rows uncharged. Small archives dominated by these lookups charge up to about 54% more units. All such archives measured stay far under the 16-million-unit floor, so only an explicit `totalPayloadElements` budget can change their verdict. A coarse drop's border-contact check now seeks each side of the tile's range. The D4 coarse-identity check reads each reference once instead of once per ancestor reference.
- `validate`: legitimate mesh output with many tiny primitives validates at the default budget. Before, it could be refused on `totalPayloadElements`. The coarse-drop recount tries first the coarse face that answered its previous distance query, which gives the same answers with far less work: 46 million units became 7 million on the PR #306 review fixture, and its validation takes 0.25 s instead of 0.65 s. The open-segment census is charged 32 units per open edge, its measured cost, instead of 256. With the additive budget above, every dense tiny-triangle output measured (75 outputs from 152 KB to 84 MB: isolated, repeated, jittered, quad and double-sided layouts) validates with at least a 1.5× margin.
- `validate`: the HLOD delivery check charges the D11 coarse-drop recount and the last-resort checks at their measured cost, so hostile archives built to exercise them (a few big coarse faces entered in every distance-grid cell; dropped faces scanning a stack of coarse planes) stay within the time statement. A distance-grid insert costs 4 units, a distance query or face test 2, a child face read 97, a seam-owner screen of a coarse face or edge 32, a stage-3 certificate region or subdivision 10, a certificate distance test 9 (795 more on its interval path), a coarse tile's recount lookups 320, and each seam-owner or fold-stage lookup a 320-unit seek; all were one unit or uncharged. The recount reads child faces through a covering index, the distance grid sizes its cells per axis, the seam-owner screen and the recount's key index use a per-run randomly keyed hash (crafted coordinates cannot collide them), the recount's components are a vector and its grid a dense array, and the fold-stage check is one seek per declared tile instead of a scan of every descendant reference at every declared ancestor. Verdicts are unchanged.
- Point-only parents whose points exceed their budgets publish voxel aggregates
  by default instead of routing, coarsening the grid until every layer fits;
  `--aggregate-points` aggregates every point-only parent.
- Breaking validation (VS-A, archive catalog at scale): the admitted ZIP catalog and the report's payload rows no longer grow resident memory with the archive. Each member is one fixed-width row sorted by the `@3dtilesIndex1@` MD5 order (plus a secondary name hash), with names in an append-only log. A catalog that fits 96 MiB (every archive within the default 2^19-entry and 32 MiB directory limits) stays in memory; a larger one moves to a scratch directory, created only at the first spilled file, and is read through a fence table and a 16 MiB page cache. Only one bit per member (the used set, 21.3 MiB at the 178,956,970-member ceiling) still scales with the entry count. Spill is never put in RAM silently: the scratch parent is the first disk-backed directory that has room, in the order `$TMPDIR`, the user cache directory plus `rusty-tiles/scratch` (`$XDG_CACHE_HOME` or `~/.cache` on Linux, `~/Library/Caches` on macOS, `%LOCALAPPDATA%` on Windows; created if missing) and the system temporary directory, skipping memory-backed (tmpfs) ones. When only memory-backed scratch is available, `validate` uses it and the report says so in a new `scratch` member (`memoryBacked`, `peakBytes`), present whenever anything spilled and absent otherwise; the `validate_3tz` facade instead refuses with an I/O error "no disk-backed scratch directory". The index is checked by one streamed merge against the catalog instead of a sort and a binary search per row, name lookups no longer binary-search string names, and the central directory is read in 64 KiB chunks. Verdicts, categories, messages, report counts and payload order are unchanged, with two exceptions. (1) With more than 16 unreferenced members, the message lists the first 16 names in byte order followed by " and K more"; it used to join every name. (2) A failure of the validator's own scratch storage is `io` (exit 1; I/O and free-space refusals), `resource_limit` (size overflow) or `invalid_state` (corrupt scratch), with the prefix "validation scratch:". Breaking Rust API: `ValidationReport::payloads` is `PayloadRows` (length, `get`, iteration and streamed `Serialize`) instead of `Vec<PayloadReport>`; `ValidationReport::summary_json` and `ValidationReport::write_json` are new. The CLI writes the JSON report streamed, byte-identical to before. Python `validate()` keeps `report["payloads"]` a list of the same dicts (the report stays JSON-serializable) and gains `lazy_payloads=True`, which returns a lazy read-only sequence (`ValidationPayloads`) instead: it supports `len()`, indexing, slicing and iteration but is not a `list`, so `== [...]` is False and `json.dumps` raises `TypeError` until it is passed through `list()`. Rust `ValidationReport` gains `scratch: Option<ScratchReport>`. The contract is [c1-archive-catalog-scale-amendment.md](docs/architecture/c1-archive-catalog-scale-amendment.md).
- Breaking validation (VS-B, traversal and decoded containment at scale): validation memory no longer grows with the tile, reference or payload count. The C2 tile arena, reference list and per-URI table are replaced by a path stack of the current traversal path; decoded containment still streams only after the whole archive passed C1 with no exclusion and no precheck stop, reading a log of the in-domain tiles and references (in memory up to 32 MiB, then scratch) instead of the tables. Per-URI facts are one 199-byte CRC-checked record per member, kept in memory up to 100 MiB (every archive within the default entry limit) and otherwise in a rank-indexed scratch file. Each record holds the payload's replay receipt: the containment stream no longer re-runs the full C1 inspection of every reference but replays the payload against digests of its own successful first admission, skipping the accessor-value and KTX2 container passes (verdicts unchanged; if the archive changes during validation, a changed member fails with "source archive changed during inspection" before parsing, and a changed external resource can instead fail an earlier size, layout, decoder or I/O check). The external-tileset memo keeps at most 64 MiB of summaries and falls back to full traversal beyond it, with identical results. The tileset schema's `uniqueItems` on `children` is now linear in the document instead of `O(bytes × depth)`, with the same verdicts and messages. Default validation (CLI, Python and Rust `inspect`) scales the seven amplification budgets with the archive's stored member bytes S: `hierarchyVisits` and `references` are at least S/8, `totalBytesRead` and `containmentBytesRead` at least 4·S, `totalPayloadElements` and `containmentDecodedBytes` at least 16·S, and `containmentWorkUnits` at least 128·S; today's values are floors, so archives up to 1 MB of stored bytes keep exactly today's limits, and the report's `limits` shows the effective values. New per-item limit `liveDocumentBytes` (default 256 MiB, Rust `ValidationLimits::live_document_bytes`; not yet listed in the report's `limits`): the tileset documents on the external-tileset path, charged as raw bytes plus 128 bytes per JSON value node, over which validation is `resource_limit` "live tileset documents exceed N bytes". The I1 availability index tables are now charged to `documentDecodedBytes` with the subtree that holds them ("subtree availability index exceeds the live subtree byte limit"). `notInspected` keeps at most 64 archive-named glTF extension labels (the byte-smallest, each at most 256 bytes); when any is dropped the report gains `notInspectedTruncated: true` (absent otherwise; Rust `ValidationReport::not_inspected_truncated`). The contract is [validator-traversal-scale-amendment.md](docs/architecture/validator-traversal-scale-amendment.md).
- Breaking validation (VS-C): the four absolute volume caps are opt-in. By default `limits.sourceArchiveBytes`, `archiveEntries`, `centralDirectoryBytes` and `archiveStoredBytes` are `null` and archives over 1 GiB, 524,288 entries or a 32 MiB central directory are accepted, bounded by the file and the format (the 3TZ member ceiling, 46 directory bytes per entry, members inside the file). A ZIP64 directory claiming more entries than its length can hold is now `invalid_input` "ZIP entry count exceeds directory length" instead of `resource_limit` (with `--maxVolume archiveEntries=524288` it keeps the old refusal). Rust `ValidationLimits`'s four cap fields are `Option<u64>`; `ValidationReport` gains `stored_bytes` and `limit_derivation`.
- Breaking validation (VS-C): a refused cap or budget is still `resource_limit`, and its message gains the suffix ` (limit KEY=VALUE, SOURCE)` with `SOURCE` one of `floor`, `scaled`, `explicit` or `cap`; the CLI JSON error gains `error.limit` (`{"limit", "value", "observed", "derivation"}`), Python `ResourceLimitError.limit` holds the same dict (else `None`) and Rust `ValidationFailure::ResourceLimit` holds a `LimitRefusal` (`ValidationFailure::exceeded()`). "member reads exceed 2 GiB work limit" is now "member reads exceed work limit" and "payload elements exceed 16 million" is now "payload elements exceed archive budget". Re-pinned exact-message expectations: the I1 refusal pins and VS-B controls compare the message without the suffix.
- Breaking (VS-C): `ValidationFailure` gains `Scratch(ScratchRefusal)` (scratch failures inside validation, `io`/1 as before), `InvalidRequest` (`usage`/2), `Cancelled` (`cancelled`/1) and `Observer` (`observer_failure`/1). `rusty_tiles::Error::Scratch(ScratchRefusal)` is new: when every scratch candidate is memory-backed, `validate_3tz` now fails with `ScratchRefusal::MemoryBackedOnly` in the `environment` class (exit 4) instead of `io` (exit 1); set `TMPDIR` to a disk-backed directory.
- The stated C2 time bound at default limits is `(2^29 + 128·S)·τ`: about 7.9 s plus 1,875 s per stored GB on the measured E-core.

### Deprecated

- `RUSTY_TILES_PYTHON_TRACEBACK` is deprecated in favour of `RUSTY_TILES_NATIVE_DIAGNOSTICS`, but still honoured.

### Removed

- Breaking: removed explicit point-cloud and vector output (H6). `point-cloud --explicit` and `vector --explicit` are now unknown arguments (usage error, exit 2), the Python `explicit=` keyword of `point_cloud_to_3tz` and `vector_to_3tz` raises `TypeError`, and the Rust `PointCloudOptions::explicit` and `VectorOptions::explicit` fields are gone. Use the default implicit output; to move an explicit archive to implicit tiling, re-run the converter from the original source. Implicit output bytes are unchanged except the vector encoder fingerprint (`vector-build.json` `config.encoder` and the `tileset.json` state hash), which covers the edited sources. `--reuse-tileset` refuses an explicit-layout vector archive, written by the removed `--explicit` mode or by the explicit default of 0.3.0 and earlier ("previous archive uses the retired explicit vector layout; … Run a fresh conversion without reuseTileset"); convert once without it to make a baseline. `validate` still reads explicit archives, including earlier explicit point-cloud and vector archives. `mesh-to-3tz --explicit` is unchanged; it retires with `mesh-to-3tz`.
- Breaking: removed the `convert-to-implicit` subcommand, the Rust `convert_to_implicit`, `convert_to_implicit_reported` and `ConvertToImplicitOptions` exports, and the Python `convert_to_implicit` function. They appeared only in 0.4.0 development builds. Its replacement design stopped at an unresolved 3D Tiles implicit-root question. To get implicit tiling for an explicit archive, re-run `point-cloud` or `vector` (Python `point_cloud_to_3tz` or `vector_to_3tz`) from the original source without `--explicit`/`explicit=True`; implicit tiling is their default. `validate` still accepts single-node archives that the command already wrote; multi-node ones carry JSON content templates and are reported Unsupported.
- Breaking library API, CLI unaffected: removed the unused `rusty_tiles::split` and `rusty_tiles::compress` modules. This includes `compress::write_glb_compressed`, `compress::write_glb_compressed_with_scale` and the `rusty_tiles::write_glb_compressed` re-export. Mesh tiling and its meshopt output are unchanged.
- Unreleased development builds had certified coarse proxies with a separate texture
  bake, and a hidden `--lod` switch. They are removed outright, with no legacy mode,
  and no release publishes their output: coarse tiles carry no `proxy_source`,
  `lod_vanished` or `lod_filled` tables, the diagnostics carry no `proxy_bake`, and the
  `lod_prepass`, `lod_simplify`, `seam_resolution`, `lod_finish` and `proxy_bake`
  phase seconds are gone. The scratch diagnostics `lod_prepass` and
  `lod_prepass_scratch_peak_bytes` are now `hlod_incidence` and
  `hlod_incidence_scratch_peak_bytes`. The migration table in `docs/MESH.md` maps them.
- Breaking: removed the legacy mesh converters. The `mesh-to-3tz` command (alias `meshTo3tz`) and `mesh-local-to-3tz` are now unknown subcommands (usage error, exit 2), and `doctor` no longer lists them. The Python `mesh_to_3tz` (with its `texture_format`, `explicit` and `node_features` keywords), `mesh_local_to_3tz`, `MeshResult` and `ConversionResult` are gone. Use `rusty-tiles mesh` / `mesh_to_tileset`; the [migration notes](docs/MESH.md#migration-from-older-mesh-converters) map the old options. Automatic mesh CRS detection (`--source-crs auto`, `geographic`, the Web Mercator adapter keyword), Euler placement (`--cartographic-position-degrees`, `--rotation-degrees`), `--explicit`, `--node-features`, `--no-meshopt`, the atlas/leaf/proxy size options and the JPEG/WebP/UASTC output selectors (`--texture-format`, `--basisu`) have no replacement. Choose a `mesh` profile and declare the source CRS or a rigid `--anchor` instead. Archives written by the removed commands still validate.
- Breaking library API: removed `tile::{mesh_to_3tz, mesh_to_3tz_reported, MeshTo3tzOptions, TextureFormat}` and the root `MeshTo3tzOptions`/`DEFAULT_MAX_TRIANGLES`/`DEFAULT_MAX_BYTES`/`DEFAULT_TILE_SIZE`/`DEFAULT_MAX_TEXEL_DENSITY` exports; `mesh_to_archive`, `MeshRequest`, `MeshApproximation`, `MeshApproximationReport`, `MeshCertificateReport`, `MeshReport` and `MeshResult`; the `report` module (`Reporter`, `Event`, `EventSink`, `ConversionResult`, `ndjson`) and its root exports; the public `mesh`, `texture`, `hlod`, `grid` and `glb_write` modules; `georef::{SourceCrs, CrsKind, parse_metashape_offset}` (and its root exports) with the legacy geographic/Web Mercator adapter functions; `bbox::{bounding_box_from_gltf, bounding_box_from_gltf_path}` and the `bbox` AABB helpers; `fixtures::geographic_glb`; and the `Error::{TextureProjection, InputNotFound, NoContent}` variants. Observe runs with `RunControl` and `Observer`. `MeshPlacement`, `model_to_archive`, `model_to_manifest` and `bbox::{BoundingBox, Obb}` remain.
- Removed the `native-jpeg` Cargo feature, its libjpeg-turbo build probe and the `RUSTY_TILES_DISABLE_NATIVE_JPEG` variable; only the legacy mesh writer encoded JPEG. JPEG and PNG source decoding are unchanged.
- Removed the AGPL-licensed `zenwebp` dependency, which only the legacy WebP texture output used, together with `jpeg-decoder`, the direct `bytemuck` dependency and the `image` crate's WebP feature (18 fewer packages in `Cargo.lock`). The legacy external `basisu` process runner is gone; KTX2 encoding stays in-process.
- Removed the deprecated `RUSTY_TILES_PYTHON_TRACEBACK` alias; set `RUSTY_TILES_NATIVE_DIAGNOSTICS=1` to see raw GDAL/GEOS warnings.

### Fixed

- Point-cloud tile bounds are rounded outward. A tile's own half extent is now at least the largest written float32 position on each axis, and a parent's enclosure of a child (`|offset| + child half`, shared with vector) is rounded up instead of to nearest. On degenerate inputs such as a 1 µm-thin cloud, a box could sit up to an ulp inside its own content or its child, which C2 decoded-content containment reports as an escape. The implicit forest writer also publishes each implicit tile at its translation from the implicit root summed in f64, and rebases its payload in f64, so content could drift a few ulps past an ancestor's row. Each row, the implicit root's box and its explicit ancestors now grow outward by an upward-rounded bound on that drift. Boxes change only where a sum was inexact (rows, implicit root boxes and their explicit ancestors) or a rounded-down enclosure was binding. Point, vector and mesh bounds can therefore grow by a few ulps. Thin point-cloud archives built before this change can now fail `validate` (C2 containment, half-ulp bounds, `invalid_input`); rebuilding them with `point-cloud` fixes it. On the digest recipes and vector fixtures, vector, terrain and mesh output is unchanged.
- Published output is now durable across a crash on filesystems that do not order namespace updates (#170). All paths share one helper that calls `File::sync_all` on a directory, which is `F_FULLFSYNC` on macOS. Directory outputs sync every staged directory when sealed (which covers the terrain and raster pruning of emptied directories) and the destination parent after the install rename and after retiring a replaced entry. Single-file and legacy `.3tz` outputs sync the destination parent after the rename, and any ancestor directories created for the output are synced up to the first existing one. Windows has no directory fsync through std, so it is skipped there; filesystems that reject a directory fsync are tolerated. A parent sync that fails after the rename is reported as a cleanup diagnostic by the typed publishers; the legacy `.3tz` path has no diagnostics channel and ignores it, since the archive is already installed.
- Mesh CRS normal sampling now respects longitude/latitude boundaries and declared angular units, uses valid one-sided derivatives at supported coordinate-domain edges, and recomputes a complete batch if sampling selects native fallback. Actual invalid coordinates and singular geographic-pole normals remain refused.
- Portable GeoJSON filters tolerate case-distinct properties such as `A` and `a` when filtering unrelated fields or constants. Direct references to colliding SQLite identifiers are refused explicitly; source/output property names are preserved. The portable reader fingerprint changes, requiring a fresh vector reuse baseline.
- Small local `mesh-to-3tz` inputs with external glTF buffers, images or structural metadata schemas now include those resources beside unchanged implicit content. Nested URI bases are preserved, and generated-name collisions or unsafe dependencies fail before replacing the output.
- Retiling untextured mesh materials without a PBR object preserves that omission instead of inserting an invalid `pbrMetallicRoughness: null` member. Representative textured converter digests remain unchanged.
- Release acceptance checks the README route on default binaries as well as native builds. Missing optional raster/terrain support is reported separately from required mesh, validation and preview readiness.
- Georeferenced polygons with constant source height now retain their source XY topology through globe placement, LOD and fragmentation. This restores Sudan and Antarctica fills in country conversions without changing source vertices or relaxing repair safeguards. Polar seam fragments retain original boundaries and conservative bounds. Previous vector archives need a fresh conversion to use the corrected encoder.
- Native placement normalizes projected horizontal units and projection offsets to metres before adding an explicit metre height axis. This preserves feet, US survey feet and kilometre inputs under PROJ 9.9, including Albers and Helmert definitions, without scaling source Z.
- Point-cloud WKT parameters retain their declared projection method: original names and EPSG identities are validated before flattening aliases; incomplete or mismatched sets, unverified spellings and colliding native-exported aliases use native interpretation. Albers (spherical or ellipsoidal) refuses source latitudes at or beyond ±80° in both builds because portable and native inverses can share a polar clamp. Native conic conditioning inspects canonical method/parameter identities and angular units, including Michigan and Belgium variants. Polar-point regressions verify native refusal as well as successful fallback across platforms.
- Point-cloud polar stereographic preserves native hemisphere interpretation for conflicting origins/parallels and rejects opposite-pole scale conflicts. Failed stereographic latitude probes now permit native retry, and nonpolar ordinary stereographic also checks each point’s latitude. Native LCC 1SP variant B conditioning uses its natural origin, including quoted definitions and exported WKT.
- Point-cloud LCC definitions with omitted origins or second parallels retain native CRS interpretation. Polar stereographic rejects conflicting scale/standard-parallel parameters. Ill-conditioned nearly opposite conic parallels are refused in both builds; near-pole conic parallels use native PROJ. Oblique stereographic checks each point’s source latitude and retries the whole batch through native PROJ when needed.
- Point-cloud conic projections with distinct parallels less than one degree apart and ordinary stereographic origins from 80 degrees to below 90 degrees latitude use strict native PROJ for accuracy. Exact polar stereographic remains portable. PROJ `+init`, spaced assignments and additional WKT syntax retain native fallback; spaced EPSG codes are normalized.
- Point-cloud CRS validation rejects invalid latitude parameters and angular projected PROJ units. Oblique stereographic origins at or beyond 80 degrees latitude use strict native PROJ to avoid near-pole numerical instability. EPSG coordinate epochs and quoted PROJ values retain native fallback.
- Point-cloud CRS validation rejects nonpositive projection scales and missing UTM zones before publishing. DMS prime meridians, spherical transverse Mercator and polar oblique stereographic retain strict native fallback. Lambert azimuthal equal area uses native PROJ to meet the 1 mm accuracy threshold.
- Point-cloud geographic `+lon_0` offsets use strict native PROJ instead of being silently ignored by the portable tier. Valid DMS angular parameters retain native fallback; default builds name the required feature.
- `glb-to-3tz` bundles referenced local buffers, images and structural metadata schemas from glTF/GLB inputs without rewriting source bytes. Missing, unsupported or escaping resource URIs fail before publication.
- `raster --display gray --alphaBand` preserves explicit transparency from numeric alpha bands, intersected with the selected band's mask/NoData before resampling.
- Default implicit mesh output accepts uppercase and mixed-case glTF/GLB extensions consistently with the mesh loader.
- Archive validation traverses nested external tilesets with a work queue, retaining cycle/depth checks without recursive stack growth.
- Published `.3tz` files honor the process umask instead of retaining temporary-file mode 0600.
- `doctor --command` limits both human and JSON command inventories to the selected commands.
- Empty triangle grids allocate one cell and return immediately from nearest-surface searches.
- `glb-to-3tz`, and small `mesh-to-3tz` inputs, no longer stage in a fixed `<output>.tileset-work` folder, which deleted any existing folder of that name.
- An output created by another process during a conversion is reported as an output conflict, exit 5, instead of an I/O error.
- Point-cloud CLI height-decision errors now report `invalid_request`, matching
  Python's `InvalidRequestError`, rather than generic data errors.
- Charge leaf island masks, planning, packing and page spans to memory admission; bound and admit coarse mask workspace before allocation. Derive coarse worker admission from the separate mask-construction and cell-grid packing peaks to preserve automatic worker capacity.
- CI: the pull-request GDAL job named the `mesh_crs` integration test, which was deleted with the legacy mesh code, so every run failed before testing. It now runs `h26_mesh_frontends`, which covers `mesh --source-crs`.
- CLI: a closed or broken stderr (for example `rusty-tiles ... 2>&1 | head -1`) no longer panics with exit 101. Progress, warning and summary writes ignore stderr errors, a broken stdout does not change the exit status, and the exit code reflects the conversion result. With `--json --progress json` the JSON result is no longer lost. Success means the conversion completed; delivery of the JSON is not guaranteed when the reader of stdout closes the pipe. Any other stdout write error (for example a full disk) exits with the I/O failure status 1 unless the conversion itself failed. `preview` treats a failed URL banner on a non-pipe stdout as a startup error.
- Documentation: `doctor` exits with code 2, not 4, when a selected command is not ready. The README `validate` line is now the real `Inspected ...: N tiles, M content references` output. The command reference lists `validate --maxVolume` and `--scratchDir`. The statements that `mesh` parent tiles have no coarse content (command reference, Python guide, `doctor --command mesh` limitations and the mesh changelog entry) now describe the HLOD coarse levels and their textures. Removed the orphaned F1d2 paragraph from the README and stale `--explicit` recipe and `tiny_http` notes from the contributor guide.
- The `uniqueItems` unit tests no longer fail about one run in 128 to 256: past 15 items the
  schema library's hashed path finds `0` and `-0.0` as duplicates only when its random hash
  seed makes their hashes collide, so the tests stopped asserting that path on such inputs.
  Validator output is unchanged.
- CI: Playwright's system-library install is bounded to three minutes and no longer fails the job. The runner image already provides Chromium's libraries, and the Azure Ubuntu mirror has stalled on the extra font packages long enough to time out the Python and release-acceptance jobs.
- CI: on pull requests the GDAL job runs only the GDAL-gated modules and integration suites (it was re-running the whole suite and often hit its 25-minute limit at 20–22 minutes). The full tier still runs the complete suite with GDAL.
- A lossy texture page whose encode misses the codec's own verification floor no longer fails the run. This is the decoded-level error or detail correlation check, for example "detail does not follow the input" on sharp synthetic patterns. The page escalates through the same ladder as a gate miss. Every step still runs the full codec verification and the gates. Structural verification failures stay hard errors. The report counts escalated pages by trigger (`escalation_reasons`: `gate`, `codec_verify`), and each listed miss names its reason.
- `rusty-tiles mesh` no longer fails a lossy run when one texture page misses its decoded-image gate. That page is re-encoded at UASTC pack level 2, then level 3. If both miss, it falls back to an exact level-0 PNG, but only when its sampler never reads coarser mip levels. Every step is checked by the same gates, which are not loosened. The tiler report counts escalated pages by step (`escalated_pages`, `lossless_fallback_pages`) and lists the first 32, naming leaf, slot, source image and material, and each missed gate with its mip level. Any remaining hard failure names the leaf, page, slot and sources.
- Relieve the shared borders of mesh cells whose exact border alone fills much of the 8,192
  triangle cap: above 85%, their shared arcs are
  certified at a coarser ladder step, identical for every owner, and every tile on those
  arcs publishes at least eight times its certified displacement as geometric error.
  Relieved tiles contract by collapse along each chord; a contraction that stalls is handed
  to the last resort. The archive validator's border displacement rule
  is now G/8 per tile. Cells below the trigger with no inherited relief are unchanged.
- Recovery validation rejects invalid explicit ownership behind external document links; only generated implicit root cells inherit their header's ownership.
- HLOD can recover an omitted cell with a coarser guarded cluster when its finest capped grid violates shared-edge incidence, preserving every previously successful cell build.
- Certify the continuous dropped surface within geometricError for N3 tiles with
  budgeted subdivision and outward numerical enclosures; refuse inconsistent last-resort payload declarations and
  declarations on leaves before granting exemptions.
- Large-mesh bakes decode each source texture once per stage for each image and
  sampling class. The HLOD and leaf texture caches held whole decoded images
  under a memory budget and decoded a source again on every eviction (an 8192²
  Walem bake spent 65,871 of 68,657 HLOD CPU seconds re-decoding). Images and
  chains now stay resident while they fit the cache; the rest spill to declared
  scratch as 128×128 texel tiles read through a bounded cache. Output is
  unchanged. Diagnostics add leaf source decode counts, image scratch, leaf
  stage timing over the job's interval (decode, encode) and per-phase CPU
  seconds.
- HLOD: a bake no longer stops with `invalid_state` ("children of … lost … interface vertices") when a remesh round would remove a border point contact (a shared vertex with no shared edge in that cell). The round now keeps the point, or is refused so the next strategy runs. First reached by the full Walem model.
- HLOD: a tile touching another tile's shared border chain at one vertex only now keeps that point in its border contract. Before, it could close a T-junction edge onto the neighbour's edge, and an ancestor then failed `--expect-profile hlod` validation (`interfaceWinding`).
- `mesh`: image-free input no longer reserves a leaf texture workspace (about
  3 GiB for `balanced`), and the leaf stage refuses a plan whose single job does
  not fit available memory even when it builds no page.
- `mesh`: the leaf stage now charges each leaf's actual largest page rather than a
  profile-wide maximum page. It also releases its decoded source images before the
  HLOD stage, so textured inputs fit the default memory budget.
- HLOD: a cell whose only child is a coarse tile it cannot collapse onto (their shared-border contracts differ between the two depths) no longer fails the bake with `invalid_state` "coarse tiles or interface rows differ from the tree". The child's published depth is final once the cell closes and builds its own tile; before, the child's row was never written. First reached by the 10M-triangle mesh-1e7 scale input.
- `bench/benchmark_vector.py` no longer fails on the removed `src/vector/native.rs`. It now records the hashes of the encoder fingerprint's own source list.
- `mesh`: a lossy leaf texture page that misses its decoded gates at every codec step is now delivered as its exact level-0 PNG even when its sampler mipmaps, instead of failing the run (`unsupported`). The viewer generates that page's coarser levels, outside the decoded guarantee, as for `lossless` pages. The report and the archive's `conversion.json` declare such pages in `viewer_mip_fallback_pages` and list every one in `viewer_mip_fallbacks` (`first`, `depth`, `page`, `slot`); `lossless_fallback_pages` counts only level-0-sampled fallbacks. The textured synthetic city (`texture-1e5-1024`) now converts at `balanced`.
- `mesh`: tile boxes have a half length of at least 0.1 mm, so a flat tile (one planar triangle) no longer writes a zero axis.
- `mesh`: a UASTC escalation step whose earlier-missed blocks, or (before UASTC level 3) coarser levels, already miss when encoded alone is decided without encoding the whole chain. UASTC encodes each 4×4 block independently, so the decision is exact and output is unchanged; texture-1e5-1024 leaf-ladder encode work drops from 188 to 136 full-chain encodes.
- `mesh`: leaf textures whose UVs repeat a source image (REPEAT or MIRRORED_REPEAT, one or more whole periods on an axis) keep that axis whole on a page of the source's own size, published with the source wrap, instead of unrolling one copy per period into the atlas. Such leaves no longer lose resolution to the leaf density reduction: texture-1e5-1024 publishes 147.8 M leaf texels at full source resolution instead of 579 M at 1/2 to 1/8.
- `mesh`: a period page is used only where it holds no more texels than the island crops of its triangles, so sparse triangles a period apart keep crops. A MASK material on a period page keeps its coverage (the MASK evaluation samples with the page's wrap), and texel ownership counts triangles a period apart as sharing texels. Each period page adds one primitive (draw call) per leaf.
- `mesh`: when the leaf stage could not fit its largest leaf page beside its largest leaf closure in the declared memory budget (explicit, else the 4 GiB default ceiling), the leaves whose pages do not fit take the next texture density step instead of the run refusing; the report and `conversion.json` count them in `memory_reduced_leaves`. The memory refusal message now names the explicit `--memory-budget` remedy.
- `mesh`: a source of many isolated tiny triangles is no longer refused with "mesh run spill peak … exceeded the declared scratch need". The whole-run spill check now holds the run to the larger of capture's declared need and the HLOD stage's own bound, whose phases are each declared and admitted as they start; isolated triangles reach the HLOD incidence pass's worst case of six interface items per triangle.
- `mesh`: under a cgroup v2 memory limit, reclaimable page cache and slab charged to the cgroup (for example a preceding compile's object files) count as available memory, as the kernel reclaims them before an out-of-memory kill. Anonymous memory and shmem/tmpfs still count as used.
- Reduce stage-3 dropped-surface validation cost with conservative distance filters and decoded-face reuse that preserves candidate order and effort-limit verdicts.
- Skip impossible corner queries in stage-3 surface certificates after conservative centroid/face box separation, preserving candidate order and the advertised distance bound.
- Point parents no longer have an identical one-child duplicate with the same
  geometric error when all their points lie in one octant. A child holding
  nearly all of its parent's points can still repeat its parent's error.
- ARM builds (Linux and macOS) produce the same mesh simplification as x86-64: the bundled
  C/C++ libraries are now compiled without fused multiply-add contraction, which GCC and Clang
  enable by default on aarch64 and which changed meshoptimizer's collapse choices. x86-64
  output is unchanged.
- CI: the Windows library tests compile again (a Unix-only CPU-time probe is now gated to Unix),
  and the Python HLOD-expectation test declares the texture rule an HLOD tileset now requires.
- CI: the installed-wheel tests read repository text as UTF-8, so they pass on Windows, whose
  default encoding is not UTF-8.
- Windows: scratch-volume checks on a file (such as the HLOD audit sidecar) query its
  directory, because `GetDiskFreeSpaceExW` refuses a file path.
- CI: the independent HLOD oracle's Node decoder runs on Windows (its entry-point check compared
  a Windows path with a file URL path), and the native-geospatial and portable CLI jobs get
  40 and 60 minutes, which the slower runners need for the full suite.
- CI: release-acceptance browser probes render until the mesh reports loaded and view the
  point-aggregate case from 400 rather than 600 radii, where its published geometric error
  still selects the tileset; HLOD acceptance digests name members with `/` on Windows too.
- The CLI now handles SIGINT and SIGTERM (Unix) as a cancellation of the running command. `mesh` and the other spilling commands remove their `rusty-tiles-spill-*`, `.tiles-mesh-*` and `.tiles-stage-*` directories before exiting with status 130 (SIGINT) or 143 (SIGTERM) and a cancelled error, instead of leaving them on disk. A second signal, or a signal to a command with nothing to cancel (such as `preview`), terminates the process by that signal at once, as without a handler. `validate` reports its own cancellation type and exits the same way. The handler is built by the new default `cli` feature (`signal-hook`, Unix only), so the Python bindings, which turn default features off, do not build it. A signal that arrives once cancellation is no longer possible prints "publishing; signal again to force exit".
- The `spill_memory_is_flat_in_input_size` test no longer fails on process-to-process VmHWM noise: it compares the smallest peak of three fresh subprocesses per input size against a documented allowance.
- A test now pins that `escalation_reasons`, the escalation diagnostics and the published bytes of codec-verification escalations are identical for one worker and many.
- Terrain now reports a missing native GTiff driver as Unsupported before opening the DEM, consistently with doctor.
- `validate`: region containment measured a whole-circle longitude span
  (`west = -π`, `east = π`) as empty. A child region spanning every longitude
  passed inside a narrower parent, and a whole-circle parent rejected children
  crossing the antimeridian. The rounding tolerance now also applies at the
  parent's west edge.
- Vector coarse tiles omit or reduce whole features no larger than their error
  instead of keeping near-full copies; a ring within tolerance of its chord
  reduces to a triangle. Parent tolerances scale with their extent and errors
  shrink at every level. On the walem rock-art layer, 2560×1600 views at 0.75 radii load
  3.1 MB instead of 11.7 MB, and the opening view 0.4 MB instead of 3.6 MB.
- Point aggregation no longer refuses aggregates farther than the tolerance: the
  tile's tolerance and error rise to the measured distance instead.
- A parent over budget retries at its own diagonal before routing. Routing tiles
  without content get a geometric error of 16 times their box diagonal, as empty
  mesh parents do, so CesiumJS no longer stops at an empty root and draws
  nothing; `validate` refuses rusty-tiles vector archives below this. Archives
  from earlier releases with content-less parents now fail `validate`; rebuild them.

### Security

- Native `vector`: a source file can no longer select GDAL drivers that open
  other datasets, query services, download schemas, run algorithms or start
  processes named inside it (`OGR_VRT`, `GDALG`, `GPSBabel`, `WFS`, `OAPIF`, `CSW`,
  `GMLAS`). A `.vrt` could otherwise pull any readable local file, or
  a URL where GDAL has network drivers, into the published tiles. Every other
  registered vector driver is still accepted.

## 0.3.0 - 2026-10-07

### Added

- `preview --host` and `--port` choose the listen address. The default is `127.0.0.1:9227`. `preview --json` emits one startup object with the bound URL and layers.
- `doctor --command` also accepts `validate` and `preview`.
- A Dockerfile builds a Python-free runtime image linked against GDAL 3.12.4 and PROJ 9.8.1 built from source.
- Opt-in `vector --aggregatePoints` uses the Rust point-cloud voxel grid for per-layer count aggregates in point-only parents. It has metre-bounded placement, explicit aggregate metadata and original full-detail leaves.
- Terrain `--maxError` defaults to 1 metre of added elevation and 3D surface error relative to the quantized regular grid. `0` retains the full grid.

### Changed

- Readiness and local preview now run entirely in Rust. Preview embeds its HTML and serves only explicitly selected outputs and a locally installed Cesium IIFE.
- Native geospatial builds require GDAL >= 3.12, PROJ >= 9.2 and GEOS >= 3.10 for vectors. Default builds retain mesh/archive and local LAS/LAZ conversion.
- Python remains for independent development fixtures and audits. CI runs the native acceptance binaries in a matching-library container with no Python installed.
- The converter version changes derivative and cache identity. Prior Python vector archives and different native encoder identities require a fresh reuse baseline.
- Native Rust vector ingestion, disk-backed SQLite LOD and reuse, and in-process meshopt compression under `native-geospatial`. Python archives require a fresh native baseline.
- Native GDAL raster COG, image/grayscale display and XYZ tiling APIs under `native-geospatial`, with no Python or GDAL executable dependency. Source-resolution mask/alpha intersection and safe publication are retained. Compression and tiling use up to four workers, with uncompressed temporary TIFFs to avoid repeated compression.
- Native GDAL terrain sampling and Rust quantized-mesh encoding, with border-preserving mesh simplification.

## 0.2.0

### Added

- LAS/LAZ point-cloud conversion to 3D Tiles 1.1, with streamed disk-backed spatial partitioning, voxel-sampled parent LOD, full-detail leaves and per-point numeric metadata. Local metre XYZ and explicit horizontal CRS/height-offset placement are supported.
- GeoPackage, GeoJSON and Shapefile vector input through OGR, explicit layer selection, source layer/feature identity and typed scalar metadata.
- Vector parent LOD with metre-based simplification error, disk-backed indexing, configurable feature/vertex/byte budgets and full-detail leaves. Oversized polygons retain filled surfaces and original outlines without internal fragment edges.
- `vector --reuseTileset previous.3tz` reuses unchanged subtrees and immutable content after an externally applied GeoPackage diff. Spatial moves, insertions, deletions and shared boundaries invalidate affected branches. Incompatible settings rebuild from source.
- Explicit raster image/grayscale recipes that preserve source COG pixels and masks, with transparent display derivatives for missing coverage.
- Terrain height sidecars and a custom `heightOverlay` manifest entry that retain missing coverage separately from filled terrain heights.
- Opt-in public dataset download/audit helpers and compatibility tests for upstream geodiff 2.3.0 and go-geodiff v0.4.3, including indexed GeoPackage moves, inserts and deletes. Diff creation and application remain external to rusty-tiles.
- Compatibility: vector content uses draft glTF vector extensions tested with Cesium 1.143.0. Fragmented polygon fills use standard glTF inside b3dm wrappers alongside vector outlines. This is experimental content, not a finalized 3D Tiles 2.0 format.
- Compatibility: point-cloud conversion requires Python, NumPy, `laspy[lazrs]` and pyproj. Raster, terrain and vector conversion require Python, NumPy and GDAL. Vector also needs GEOS. Helpers are embedded in the executable, but these Python dependencies must be installed separately.
- Limit: point-cloud compound vertical CRS, waveform payloads and array extra dimensions are unsupported. Height offsets are constant conversions. No vertical datum is guessed.
- Limit: vector reuse scans the updated source and repacks the output archive. It avoids encoding unchanged content. It does not avoid every source-processing step. Buffered grid clipping and implicit tiling remain unimplemented.

## 0.1.0 - 2026-09-11

### Added

- Initial standalone release: textured mesh conversion and `.3tz` packaging, raster imagery, regular-grid terrain, experimental vector content and a local Cesium preview.
