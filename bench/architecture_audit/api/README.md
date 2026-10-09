# API contract observations for architecture audit #113

These executable probes observe the portable Rust library, CLI and Python binding
on develop `8dfd74bd87dd23c86278e98d736c3f5912c246cf`. They do not add regression
tests that require the baseline's undesirable behavior to remain. A later fix
can change an observation without failing the probe; build/fixture/driver failures
still fail because they prevent a valid audit.

`results.json` contains statuses, CLI errors, root transforms, image MIME types,
logical archive hashes, callback snapshots and validation results. It records
source and checkout revisions, lockfile and executable hashes, exact build
commands and toolchains. Production snapshot metadata records Git blob-tree
digests, a digest of working file contents, and the production diff from the
baseline. Baseline means the last commit changing production paths, so adding
audit artifacts does not change that identifier.

## Reproduce

From the repository root:

```sh
python3 bench/architecture_audit/api/probe.py
```

The runner executes the following build with dependencies from `Cargo.lock`:

```sh
CARGO_TARGET_DIR=/home/bend/.cache/rusty-tiles-113-api-target \
CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 \
CARGO_PROFILE_DEV_INCREMENTAL=false \
cargo build --locked --offline -p rusty-tiles -p rusty-tiles-python \
  --features pyo3/extension-module
```

Then it compiles `core_probe.rs` directly with `rustc` against that build's
library and serde_json rlibs, loads the just-built Python shared library by
absolute path, and invokes the just-built CLI. This avoids an extra dependency
resolution/lockfile and avoids relying on an installed Python wheel. The full
`rustc` command is recorded in JSON. Use `--target-dir PATH` to choose another
dedicated target directory; use `--results PATH` to preserve the checked-in
baseline results when rerunning. Offline mode requires Cargo's dependency cache
to be populated. Fixture/output directories live on the target-directory disk
and are removed when the run finishes. No production source files are changed.

The runner currently loads `debug/librusty_tiles.so` and is Linux-specific.
It exercises a directly built binding, not an installed wheel. macOS/Windows
library names/loading and packaged-wheel installation are outside this probe.

The recorded run used Rust/Cargo 1.98.0, Python 3.14.7 and Linux x86_64, with
portable default features and PyO3's extension-module feature. The clean build
completed in 39.86 seconds with two Cargo jobs. No applicable `AGENTS.md` was
found in the worktree or relevant ancestors.

## Observed versus desired

| Case | Baseline observations | Desired contract to decide |
| --- | --- | --- |
| Legacy geographic mesh plus source offset `[1000, 2000, 300]` | Rust/Python return success and produce identical uncompressed archive payloads to no-offset controls. CLI rejects with data error, exit 3, before output creation. | Uniformly reject an unsupported offset or apply documented semantics; avoid silently discarding it. |
| Rotation `[37, 11, 5]` without cartographic placement | Rust/Python mesh and GLB APIs return success and produce identical payloads to no-rotation controls; root transform is absent. CLI rejects both with data error, exit 3, before output creation. | Uniform validation or explicit placement-independent rotation semantics. |
| Texture format omitted while forcing a texture rebuild | Rust/Python output `image/png`; CLI outputs `image/jpeg`. | Intentionally shared default, or clearly documented frontend-specific fidelity/codec defaults. |
| Python callback raises on first event | Callback observes no output yet. Conversion continues and publishes an archive, then Python raises the callback's RuntimeError. CLI validation of that output succeeds. | Decide cancellation versus observer-failure semantics, and expose publication status unambiguously. |
| Python callback raises on final explicit-mesh event | Callback observes the output already exists; Python raises RuntimeError. The published archive passes CLI validation. | Same publication contract; rollback is not assumed by this audit. |

The defaults concern is an observable frontend difference, not a claim that
either codec is inherently wrong. The callback behavior is reproducible, not
a claim that cancellation is currently promised or rollback is required.

## Scope and evidence

The standard-library fixture generator writes three tiny, self-contained GLBs:
a local triangle, a triangle in legacy geographic Y-up coordinates, and a local
triangle with a 4x4 opaque PNG texture. The geographic Y-up mapping is
`X=longitude degrees, Y=height metres, Z=-latitude degrees`. Fixture SHA-256s
are recorded. Geographic comparisons explicitly choose legacy geographic
coordinates and meshopt off; rotation comparisons use local coordinates with
no placement. Mesh comparisons request explicit hierarchy, and GLB comparisons
exercise the direct wrapping API. Codec and callback probes set `max_bytes=1`
and `tile_size=64` to force the mesh/texture pipeline rather than the unchanged
small-model wrapper. They disable meshopt and request explicit hierarchy.

Pair comparisons hash each sorted member name, byte length and uncompressed
member data, excluding ZIP metadata. This observes the complete logical
package, including geometry, transforms and the 3TZ index; two missing outputs
are never classified as identical. The callback probes capture output existence
at each delivered event and run the CLI validator on the resulting archive.
The post-publication case selects the explicit pipeline's final
`mesh-to-3tz: done ...` note; the first-event case is independent of that note.

Supporting source locations at the recorded baseline:

- `src/main.rs:916` and `src/main.rs:967`: frontend-only rotation/offset rejection.
- `src/mesh.rs:334`: legacy geographic bake ignores its offset argument.
- `src/tileset.rs:64`: placement gates application of rotation in the wrap path.
- `src/tile.rs:67`, `src/main.rs:438`, `bindings/python/src/lib.rs:184`: codec defaults.
- `bindings/python/src/lib.rs:137`: callback exception returned after conversion.
- `src/tile.rs:548`: explicit pipeline publishes before the final callback note.

Not tested: general geographic CRS with explicit axes/height, invalid placement
numbers, force replacement of an existing output during callback failure,
signal cancellation, native JPEG/geospatial features, other operating systems,
or larger mesh workloads. There are no performance or memory claims from these
tiny fixtures.
