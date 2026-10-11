# T1: read-only tooling contracts for `doctor` and `preview`

Foundation slice for #126 checklist items 4 and 5. Baseline is `develop` at
`f2ed077`. This contract precedes implementation; line references are to that
baseline unless they name a function. It owns `src/doctor.rs`,
`src/preview.rs`, `preview/index.html`, the `doctor`/`preview` arms of
`src/main.rs`, their tests, release-harness steps 2, 5 and 6 and the matching
docs. The Python binding exposes neither tool (`bindings/python/README.md:20`).

Producers, validation and implicit tiling are not changed beyond two minimal
shared-check edits:

- `raster/native.rs` extracts `output_drivers` from `require_outputs`. Its
  doctor-only `capabilities` now registers drivers through the shared `init`.
- `vector::native_available` returns execution's error classification.

## Doctor invariants

1. **Status.** Each command's readiness has one status: `ready`,
   `unavailable` or `unsupported`. `ready` is exactly `status == "ready"`.
   - **Unsupported** means that this build or platform does not contain the
     capability, and a rebuild is required. Examples are the portable `raster`
     (`raster.rs:271-277`), `terrain` (`terrain.rs:425-431`) and D1
     (`raster_directory.rs:386-392`).
   - **Unavailable** means that the capability is compiled in but a runtime
     check failed: the GDAL/PROJ version, PROJ database, a GDAL driver or GEOS.
2. **Readiness comes from the check that execution runs.** Pure-Rust commands
   (`mesh-to-3tz`, `mesh-local-to-3tz`, `glb-to-3tz`, `createTilesetJson`,
   `convert`, `validate`, `preview`, local point clouds and portable vectors)
   have no runtime dependency. Their resolver is the build itself, so they are
   `ready`. The `match` over a typed `Command` is exhaustive, so adding a
   command cannot silently leave it out. That replaces the unsourced string
   list at `doctor.rs:44-53`. `convert-to-implicit` is retired for 0.4.0 and
   has no readiness entry. The clap-parity test exempts it only until its
   removal merges.
   - **Native vector** uses `vector::native_available`. That is execution's
     own `pipeline::available()` (`vector/pipeline/store.rs:765`) mapped
     through execution's `job_error`. It does not require the PROJ database,
     because CRS operations are validated per source.
   - **Native raster** uses `raster::output_capability`. That is the same
     `native::init` and `native::output_drivers` pair, in the same order, that
     source admission runs (`raster/source.rs:664-665`, through
     `require_outputs`). `raster::capabilities` supplies only the
     informational driver inventory.
   - **Terrain and D1** export no capability check. They run `native::init`,
     then open the source with only the GTiff driver allowed
     (`terrain/raster.rs:141`, `raster_directory/native.rs:44`). Doctor
     therefore composes their readiness from the GTiff entry of the raster
     driver inventory. Before this slice, those two checked only GDAL/PROJ
     (`doctor.rs:60-67,102-105`).
   - The **top-level PROJ database check** gates raster, terrain and D1. Those
     commands need it to admit their source CRS.
3. **Outcome.** Every not-ready command carries the failure kind that its own
   check returns for that cause: `refusal`, a `JobErrorKind`. The CLI presents
   it through execution's own category mapping (`job_category`) as
   `commands.<name>.refusal`. The first not-ready command in name order sets
   the process `error.code` and exit; all selected `ready` exits 0.
   - The top-level dependencies (`Error::Environment`) are classified as
     `unsupported`, as every native producer does: vector `job_error`, raster
     `init`, terrain and D1 `native_error`.
   - Measured on the native build with `GDAL_SKIP` and an empty `PROJ_DATA`
     (`logs/review-execution-categories.log`), every native refusal is
     `unsupported`/2, both in doctor and in execution.
   - The one recorded divergence: with no GTiff driver, terrain and D1 fail
     to open the source. `Dataset::adopt` (`geospatial/native.rs:99-103`)
     returns `Error::Data`, which their `native_error` maps to
     `invalid_input`/3 (`terrain/raster.rs:25-33,155`;
     `raster_directory/native.rs:11-25,59`). Doctor reports `unsupported`/2
     for the same cause. The producers own that classification. The test pins
     both sides.
   - JSON keeps the full inventory in its single result.
4. **Probes are named, bounded and read-only.** The report lists every probe
   it ran under `probes`.
   - In native builds every report checks linked versions, sets the
     process-wide PROJ offline policy and looks up EPSG:4326 in the database.
   - Every build lists PROJ directories when any are configured: `PROJ_DATA`,
     `PROJ_LIB` and, natively, GDAL's search paths. A portable report without
     those variables lists nothing.
   - Per-command probes (GDAL driver registry, GEOS triangulation, the Cesium
     stat) run only for selected commands. Before this slice,
     `doctor.rs:41-68` resolved every command before filtering.
   - Each listing is non-recursive and capped at 4096 entries per directory;
     a truncated listing is reported in `inventoryErrors`.
   - Doctor launches no subprocess, enables no network and creates no files:
     no PROJ user cache, no HOME/XDG/cwd entries and no grid downloads.
   - The only state changes are the PROJ offline policy, plus GDAL driver
     registration and the GDAL cache limit set once by
     `geospatial::native::init`. Every native operation shares these changes.
   - Baseline `raster::native::capabilities` re-ran `GDALAllRegister` outside
     that `Once`, and concurrent reports aborted inside GDAL. It now calls
     `init`, so doctor needs no lock of its own.
5. **Typed report.** A typed `doctor::Report` replaces `Result<Value, Error>`
   (`doctor.rs:40`). Selection is the typed `Command`, so `report` is
   infallible. Descriptive per-command details (CRS classes, limitations,
   drivers) remain JSON data. Status, readiness, reason, refusal and probes are
   typed.

## Preview invariants

1. **Roots.** Each selected root and the Cesium root are canonicalized once at
   startup. Each must be a directory, and its manifest (`tileset.json`,
   `tilejson.json` or `Cesium.js`) must pass the same exact-name rule as a
   request. Otherwise there is a `data` error (exit 3) before binding.
2. **Requests.** Only GET/HEAD are answered, and only for `/`, `/index.html`,
   `/config.json` and `/<selected-layer>/<relative>`. Everything else is 404
   with no body. The relative path is percent-decoded once; invalid UTF-8,
   such as overlong `%c0%ae`, is 400. The path then passes two rules that make
   every platform agree:
   - **Lexical admission, before any filesystem access.** The path is one or
     more `/`-separated segments. Each segment is non-empty, so there is no
     trailing slash or `//`. No segment is `.` or `..`, and none contains `\`,
     `:` (Windows drives and streams) or a control character, including NUL.
     The baseline relied on the OS: `open("file/")` fails with ENOTDIR on
     Linux but resolves on macOS, which failed macOS CI.
   - **Exact names.** The canonical target must be a regular file below the
     canonical root, and its path there must equal the requested segments,
     compared component by component as bytes. A platform may resolve an
     alias to an existing file: a case or Unicode-normalization variant
     (APFS/HFS+, NTFS), a Windows 8.3 name or trailing dot/space, or any
     symlink. The canonical names then differ from the request, so the alias
     is refused everywhere. Symlinks below a root are not followed, even
     inside it; a selected root may itself be a symlink.
3. **Network.** `--host` is an IP address, so no name resolution occurs; the
   default is `127.0.0.1` (`main.rs:211`).
   - A loopback bind, including IPv4-mapped `::ffff:127.0.0.1`, answers only
     requests whose authority is `localhost`, `127.0.0.1`, `[::1]` or the bound
     address, with the bound port. Any other `Host`, or none (for example an
     HTTP/1.0 request without `Host`), receives 421 and no body. This defends
     against DNS rebinding, which the baseline does not check.
   - A non-loopback address is the explicit opt-in. It accepts any `Host`, is
     announced on stderr and appears as `"loopback": false` in the JSON startup
     line.
4. **Lifetime.** The URL is announced only after every root is validated and
   the listener is bound. Each connection owns its own task
   ([preview transport](preview-transport.md)). The process serves until it is
   signalled. SIGTERM/SIGINT end it with no cleanup obligation, because it
   writes nothing. Bind and listener failures fail the process.
5. **Imagery.** The page builds the R2 `UrlTemplateImageryProvider` from the
   exact TileJSON `bounds`, `minzoom` and `maxzoom` (`preview/index.html`).
   - **The Cesium defect.** In Cesium 1.146,
     `ImageryLayer._createTileImagerySkeletons` maps the south-east corner of
     a rectangle lying on imagery tile edges to the outside neighbour tile.
     The north-west corner can land there through rounding.
     `Rectangle.intersection` of that neighbour with the bounds is
     `undefined`, the next read of `.west` throws, and rendering stops. R2
     bounds are tile-aligned, so every R2 pyramid failed.
   - **The inset.** The provider rectangle is inset on all four edges by 1/1024
     of a finest-zoom pixel in longitude (`360 / 2^maxzoom / 256 / 1024`
     degrees). At z24 that is about 8e-11°, three or more orders of magnitude
     above double rounding.
   - **Texel size.** A Mercator pixel spans `cos φ` times fewer degrees of
     latitude than of longitude, so the same inset is `1/(1024·cos φ)` of a
     pixel in latitude. That is still under 1/88 of a pixel at the Mercator
     limit of 85.05°. Texture coordinates come from the full imagery tile, so
     the inset only leaves that sliver of the outer boundary unpainted.
   - **Alternatives.** Insetting one edge does not suffice. Omitting the
     rectangle would request tiles outside the pyramid, and their 404s fail
     the page. Changing the tiling scheme does not alter this intersection.
   - The TileJSON written by R2 stays exact and still frames the camera.
   - **Pin.** The page applies the inset with any runtime. The pin to Cesium
     1.146 behaviour is in the regression check: `raster_preview.cjs` asserts
     `Cesium.VERSION === '1.146.0'`, so a runtime bump must re-prove it.

## Public-surface decisions

| Item | Decision |
| --- | --- |
| `doctor::COMMANDS`, `doctor::canonical` (`doctor.rs:10-35`) | **Replace** with `doctor::Command` (`ALL`, `name`, `aliases`, `from_name`). The CLI parser and the clap-parity unit test consume it. |
| `doctor::DEFAULT_CESIUM` (`doctor.rs:27`) | **Remove** from the library. The cwd-relative README location becomes the CLI's `--cesium` default. The library checks a runtime only when given one. |
| `doctor::report`, `doctor::display` (`doctor.rs:40,244`) | **Rework** to `report(&[Command], Option<&Path>) -> Report`, with `Display` for human output and `Report::refusal`. Supporting types are `Status`, `Probe`, `Capability` (with `refusal`), `NativeGeospatial`, `Readiness` and `ProjInventory`. |
| Hard-coded ready list (`doctor.rs:44-53`) | **Remove**; replaced by the exhaustive resolver match. |
| Raster readiness (`doctor.rs:107-126`) | **Rework** to execution's `init` plus `output_drivers`. |
| Vector readiness (`doctor.rs:128-139`) | **Rework** to execution's check and classification only; there is no PROJ-database gate. |
| Terrain/D1 readiness (`doctor.rs:60-67,102-105`) | **Rework** to require the GTiff driver. |
| PROJ inventory (`doctor.rs:188-242`) | **Retain** with a per-directory cap. |
| Environment exit 4 for doctor | **Remove**. Not-ready commands use their execution category. |
| `preview::Preview::new`, MIME, no-cache, HTTP/1 transport | **Retain**. |
| Path fence (`preview.rs:63-81,156-170`) | **Rework** to lexical admission plus exact canonical names. Symlinks below a root are no longer served. |
| `Preview::serve(&str host, ...)` (`preview.rs:122`) | **Rework** to take a `SocketAddr` and enforce the loopback `Host`. |
| `preview::MANIFESTS` | **Retain** as provisional public data. |

## Acceptance plan

**Doctor tests** run in both builds.
- A portable `raster`/`terrain`/D1 selection is `unsupported` with exit 2,
  and its execution also refuses as `unsupported`.
- A native test runs each not-ready case through both doctor and the command
  itself, and compares their category and exit. The cases are `GDAL_SKIP` of
  GTiff, COG, GPKG and GeoJSON, and an empty `PROJ_DATA`. It pins the terrain
  and D1 GTiff divergence, and checks that native vector is ready and converts
  without the PROJ database.
- The named probes for a pure-Rust selection exclude per-command native
  probes.
- Doctor leaves HOME, XDG, cwd and PROJ data directories unchanged.
- Unit tests cover the listing cap and concurrent reports.

**Preview tests** attack a live server and also run as library unit tests.
- Every lexical rule and every exact-name alias form has a unit test.
- The live server is attacked with `..`, encoded, double-encoded and overlong
  traversal, trailing slashes, empty segments, case variants, trailing
  dots/spaces, streams, 8.3 names, backslash, drive and UNC forms,
  absolute and absolute-form paths, NUL and control characters, symlinks
  (file, directory, sibling output, inner alias), unselected layers and
  foreign `Host` headers.
- They also cover an IPv4-mapped loopback bind, a non-loopback opt-in bind,
  SIGTERM/SIGINT shutdown with the port closed, and a symlinked root.

**Release harness.** Steps 2 and 5 assert the status/exit contract and the
fence against the release binary. Step 6 loads a tile-aligned R2 pyramid
through preview in Chromium.

## Findings and their tests

Static observations above were confirmed by execution:

- **Terrain/D1 reported ready without GTiff.** Baseline native
  `GDAL_SKIP=GTiff doctor --json --command terrain --command
  raster-tile-to-directory` reported both ready, while conversion of real
  GeoTIFF fixtures failed (`cannot open … GTiff`). Test:
  `native_doctor::native_refusals_report_the_category_and_exit_of_execution`.
- **Portable exit code disagreed with execution.** Baseline portable
  `doctor --command raster` exited 4 (`environment`), while `raster` itself
  exited 2 (`unsupported`). Test:
  `native_doctor::portable_native_commands_are_unsupported_like_their_execution`.
- **Native categories disagreed with execution** (nonauthor review). The first
  T1 head reported every native not-ready command as `environment`/4. The
  commands themselves return `unsupported`/2, apart from the terrain/D1
  divergence above. Test: the native refusal test above.
- **Vector false negative.** The first T1 head, like the baseline, reported
  native vector not ready without a PROJ database, although a WGS84 GeoJSON
  conversion succeeds. Test: the same.
- **Concurrent reports aborted.** Concurrent `doctor::report` calls in one
  native process aborted in a GDAL C++ assertion inside
  `raster::native::capabilities`. On four threads this happened in 39 of 40
  runs before the fix. After a doctor mutex it was 0 of 40. After routing
  registration through `init`, with no mutex, it was also 0 of 40. Test:
  `doctor::tests::concurrent_reports_complete`.
- **Foreign `Host` served.** The baseline preview answered `Host:
  evil.example` with 200 on loopback. Test:
  `native_preview::loopback_listener_refuses_foreign_host_authorities`.
  Removing the check makes it fail.
- **Mapped loopback mislabelled.** On the first T1 head, an IPv4-mapped
  loopback bind was reported as non-loopback and skipped the `Host` check.
  Tests: `ipv4_mapped_loopback_bind_keeps_the_host_fence` and the unit test.
- **Host names were resolved.** Baseline `preview --host localhost` resolved
  the name and bound `[::1]`. Now it is a usage error, and no name is
  resolved.
- **macOS served a trailing-slash path.** On macOS CI, `/mesh/tileset.json/`
  was served (200), while Linux returned 404. Tests:
  `preview::tests::lexical_admission_rules_need_no_filesystem`,
  `names_must_match_the_canonical_names_exactly`, and the live attack list.
- **No R2 pyramid loaded in preview.** Every R2 pyramid failed with
  `TypeError: Cannot read properties of undefined (reading 'west')`, found by
  the performance lane.
  - The regression check is `tests/fixtures/raster_preview.cjs`, run through
    the real preview server. It also runs as release-acceptance step 6, on a
    tile-aligned R2 output of `tests/fixtures/d1-rgb.tif`.
  - It fails on the pre-fix binary and passes after the fix. It samples 81
    points: all of them must show imagery, and none may with the layer hidden.
  - The same check passes on the performance lane's three 8192² outputs
    (z9–14), with their transparent hole rendering as bare globe.

## Proof limits

- **Filesystem races.** There is no defence against a concurrent local writer
  racing canonicalization and open, or against hard links. The served roots
  are trusted local selections.
- **Platform aliases unexecuted.** The rule against aliases on
  case-insensitive and normalizing filesystems depends on `canonicalize`
  returning stored names. That is `realpath` on macOS and
  `GetFinalPathNameByHandleW` on Windows. It was reasoned from those APIs and
  exercised only on Linux; on Linux the alias tests pass, because those names
  do not exist there.
- **Terrain/D1 resolver and category.** Terrain and D1 export no capability
  resolver of their own, so doctor composes their native readiness from the
  raster driver inventory. A producer-owned resolver and their GTiff category
  remain producer follow-ups.
- **Ready is not a full proof.** A ready status does not prove that a
  source-specific CRS or height operation succeeds; conversion still
  validates it.
- **Unmodelled platforms.** Platforms without directory publication
  (`directory_platform.rs:351`) are not release targets and are not modelled.
- **Accept errors.** Accept-time errors other than per-connection aborts end
  the process; resource exhaustion is not retried.
- **Draped imagery unchecked.** Imagery draped on terrain uses the same
  provider, but it was not browser-checked here, because that probe needs the
  Python GDAL fixtures.
- **Imagery check is release-only.** The imagery probe is a
  release-acceptance browser step, not a `cargo test`, because it needs node,
  Playwright, Chromium and Cesium.
