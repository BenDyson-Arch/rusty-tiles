# T1: read-only tooling contracts for `doctor` and `preview`

Foundation slice for #126 checklist items 4 and 5. Baseline is `develop` at
`f2ed077`. This contract precedes implementation; line references are to that
baseline. It owns `src/doctor.rs`, `src/preview.rs`, the `doctor`/`preview` arms
of `src/main.rs`, their tests, release-harness steps 2 and 5 and the matching
docs. Producers, validation and implicit tiling are not changed. The Python
binding exposes neither tool (`bindings/python/README.md:20`).

## Doctor invariants

1. Each command's readiness has one status: `ready`, `unavailable` or
   `unsupported`. **Unsupported** means that this build or platform does not
   contain the capability. A rebuild is required, and execution refuses the
   command as `unsupported`; for example, the portable `raster`
   (`raster.rs:271-277`), `terrain` (`terrain.rs:425-431`) and D1
   (`raster_directory.rs:386-392`). **Unavailable** means that the capability is
   compiled in but a runtime probe failed: GDAL/PROJ version, PROJ database, GDAL
   driver or GEOS. `ready` is exactly `status == "ready"`.
2. Readiness comes from the resolver that execution uses. Pure-Rust commands
   (`mesh-to-3tz`, `mesh-local-to-3tz`, `glb-to-3tz`, `createTilesetJson`,
   `convert`, `validate`, `preview`, local point clouds and
   portable vectors) have no runtime dependency. Their resolver is the build
   itself, so they are `ready`. The `match` over a typed `Command` is
   exhaustive, so adding a command cannot silently leave it out. That replaces
   the unsourced string list at `doctor.rs:44-53`. `convert-to-implicit` is
   retired for 0.4.0 and has no readiness entry. Native vector uses
   `vector::native_available` (`vector.rs:16`) and native raster uses
   `raster::capabilities` (`raster.rs:264`). Terrain and D1 open only the GDAL
   GTiff driver (`terrain/raster.rs:141`, `raster_directory/native.rs:44`), so
   their native readiness also requires the GTiff driver that
   `raster::capabilities` reports. Before this slice, those two checked
   only GDAL/PROJ (`doctor.rs:60-67,102-105`).
3. The process outcome follows from the statuses. All selected `ready` exits 0.
   Any `unavailable` gives `environment`, exit 4. Otherwise, any `unsupported`
   gives `unsupported`, exit 2, which matches execution's category for the same
   build. JSON keeps the full inventory in its single result.
4. Probes are named, bounded and read-only. The report lists every probe it ran
   under `probes`. The top-level probes always run: linked versions, the
   process-wide PROJ offline policy, an EPSG:4326 database lookup and a
   top-level PROJ directory listing. Per-command probes (GDAL driver registry,
   GEOS triangulation) run only for selected commands. Before this slice,
   `doctor.rs:41-68` resolved every command before filtering. Each listing is
   non-recursive and capped at 4096 entries per directory; a truncated listing
   is reported in `inventoryErrors`. Doctor launches no subprocess, enables no
   network and creates no files: no PROJ user cache, no HOME/XDG/cwd entries
   and no grid downloads. The only state changes are the process-wide offline
   policy, GDAL driver registration and the GDAL cache limit set by
   `geospatial::native::init`. All three are shared with every native
   operation. A process-wide mutex serializes native probes across concurrent
   reports, because `raster::capabilities` re-runs `GDALAllRegister` outside
   that `Once` (`raster/native.rs:1674`).
5. A typed `doctor::Report` replaces `Result<Value, Error>` (`doctor.rs:40`).
   Selection is the typed `Command`, so `report` is infallible. Descriptive
   per-command details (CRS classes, limitations, drivers) remain JSON data;
   status, readiness, reason and probes are typed.

## Preview invariants

1. Roots: each selected root and the Cesium root are canonicalized once at
   startup. Each must be a directory whose canonical manifest (`tileset.json`,
   `tilejson.json` or `Cesium.js`) is a regular file inside that canonical root
   (`preview.rs:156-170`). Otherwise there is a `data` error (exit 3) before
   binding.
2. Requests: only GET/HEAD, only `/`, `/index.html`, `/config.json` and
   `/<selected-layer>/<relative>`. The relative path is percent-decoded once and
   must consist only of normal components, with no `\` or `:`. Its canonical
   target must be a regular file under the canonical root (`preview.rs:63-81`).
   Everything else is 404 with no body: `..`, encoded traversal, absolute paths,
   symlinks resolving outside the root, directories, unselected layers. Symlinks
   that resolve inside the root are served.
3. Network: `--host` is an IP address, so no name resolution occurs; the default
   is `127.0.0.1` (`main.rs:211`). On a loopback bind, the server answers only
   requests whose `Host` is `localhost`, `127.0.0.1`, `[::1]` or the bound
   address. Any other `Host` receives 421 and no body. This defends against DNS
   rebinding, which the baseline does not check. A non-loopback address is the
   explicit opt-in. It accepts any `Host`, is announced on stderr and appears as
   `"loopback": false` in the JSON startup line.
4. Lifetime: the URL is announced only after every root is validated and the
   listener is bound. Each connection owns its own task
   ([preview transport](preview-transport.md)). The process serves until it is
   signalled. SIGTERM/SIGINT end it with no cleanup obligation, because it
   writes nothing. Bind and listener failures fail the process.

## Public-surface decisions

| Item | Decision |
| --- | --- |
| `doctor::COMMANDS`, `doctor::canonical` (`doctor.rs:10-35`) | **Replace** with `doctor::Command` (`ALL`, `name`, `aliases`, `from_name`). The CLI parser and the clap-parity unit test consume it. |
| `doctor::DEFAULT_CESIUM` (`doctor.rs:27`) | **Remove** from the library. The cwd-relative README location becomes the CLI's `--cesium` default. The library checks a runtime only when given one. |
| `doctor::report`, `doctor::display` (`doctor.rs:40,244`) | **Rework** to `report(&[Command], Option<&Path>) -> Report`, with `Display` for human output. |
| Hard-coded ready list (`doctor.rs:44-53`) | **Remove**; replaced by the exhaustive resolver match. |
| Terrain/D1 readiness (`doctor.rs:60-67,102-105`) | **Rework** to require the GTiff driver. |
| PROJ inventory (`doctor.rs:188-242`) | **Retain** with a per-directory cap. |
| `preview::Preview::new`, path fence, MIME, no-cache, HTTP/1 transport | **Retain**. |
| `Preview::serve(&str host, ...)` (`preview.rs:122`) | **Rework** to take a `SocketAddr` and enforce loopback `Host`. |
| `preview::MANIFESTS` | **Retain** as provisional public data. |

## Acceptance plan

Doctor tests run in both builds. A portable `raster`/`terrain`/D1 selection is
`unsupported` with exit 2, and its execution also refuses as `unsupported`.
A native `GDAL_SKIP=GTiff` makes raster, terrain and D1 `unavailable` with exit
4, and D1/terrain execution fails on a real GTiff under the same environment.
The named probes for a pure-Rust selection exclude per-command native probes.
Doctor leaves HOME, XDG, cwd and PROJ data directories unchanged. A unit test
covers the listing cap. Preview tests attack a live server with `..`, encoded
and double-encoded traversal, backslashes, absolute paths, NUL, outside
symlinks (file, directory and manifest), unselected layers and foreign `Host`
headers. They also cover a non-loopback opt-in bind, SIGTERM shutdown with the
port closed, and a symlinked root. Library unit tests cover the fence without
a socket. Release steps 2 and 5 assert the status/exit contract and the fence
against the release binary.

## Findings and their tests

Static observations above were confirmed by execution:

- Baseline native `GDAL_SKIP=GTiff doctor --json --command terrain --command
  raster-tile-to-directory` reported both ready, while conversion of real
  GeoTIFF fixtures failed (`cannot open … GTiff`). Test:
  `native_doctor::missing_gtiff_driver_makes_gtiff_commands_unavailable_like_their_execution`.
  Removing the GTiff term makes it fail.
- Baseline portable `doctor --command raster` exited 4 (`environment`), while
  `raster` itself exited 2 (`unsupported`). Test:
  `native_doctor::portable_native_commands_are_unsupported_like_their_execution`.
- Concurrent `doctor::report` calls in one native process aborted in a GDAL
  C++ assertion inside `raster::native::capabilities`. On four threads this
  happened in 39 of 40 runs before serialization and 0 of 40 after. Test:
  `doctor::tests::concurrent_reports_complete`.
- The baseline preview answered `Host: evil.example` with 200 on loopback.
  Test: `native_preview::loopback_listener_refuses_foreign_host_authorities`.
  Removing the check makes it fail.
- Baseline `preview --host localhost` resolved the name and bound `[::1]`.
  Now it is a usage error, and no name is resolved.

The baseline traversal, symlink and unselected-layer fence held against every
added attack; that is retained evidence, not a defect.

## Proof limits

There is no defence against a concurrent local writer racing canonicalization
and open, against hard links, or against case-folding filesystems. The served
roots are trusted local selections. Terrain and D1 export no capability
resolver of their own, so doctor composes their native readiness from
`raster::capabilities`. A producer-owned resolver remains a follow-up. So does
guarding `raster::native::capabilities` with the shared registration `Once`.
Doctor's mutex does not protect a library caller that runs doctor alongside
native conversions on other threads. Under a missing GTiff driver, terrain
and D1 conversion report `invalid_input`; their categories belong to their
producers. A ready
status does not prove that a source-specific CRS or height operation succeeds;
conversion still validates it. Platforms without directory publication
(`directory_platform.rs:351`) are not release targets and are not modelled.
Accept-time errors other than per-connection aborts end the process; resource
exhaustion is not retried.
