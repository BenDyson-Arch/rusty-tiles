# D1 evidence

The [directory contract](../../../docs/architecture/d1-directory-contract.md)
and [raster contract](../../../docs/architecture/d1-raster-contract.md) define
this slice. D2 replacement remains open in #118. [Platform sources](platforms.md)
and the [#82 feasibility review](portable-raster-feasibility.md) distinguish
implementation decisions from executed proof.

Production source `a299e6c` passes 340 portable and 375 native Rust tests
(8/9 existing ignored), portable workspace/all-target and native all-target
Clippy with warnings denied, formatting and source packaging. The rebuilt
standard wheel passes 29 tests in isolated CPython and 29 in local distribution
Blender 5.2.2. This wheel explicitly refuses native raster capability; the
native Rust/CLI pilot is separately tested on Linux. These local results do
not claim native GDAL raster execution on Windows or macOS. Platform CI is
required to qualify the actual exclusive directory primitives there.

`oracle.json` records the independently generated/decoded TIFF-to-PNG cases:
eight positives, fifteen typed refusals/preservation controls and three
sensitivity controls. The fixture writer and PNG reader use only Python's
standard library, with no production or GDAL format helpers. Positives cover
classic TIFF and BigTIFF in both byte orders, zoom boundaries, a 32 MiB source,
and 30 MiB of ASCII metadata. Every positive checks 65,536 RGB samples, exact
output closure, geographic bounds and disk/CLI report parity. Refusals include
compensated PixelIsPoint, standalone transfer functions, embedded ICC profiles,
nondefault orientation, oversized native storage blocks and malformed IFDs.

The transfer-function control exposed an executed defect in GDAL metadata
inspection: tag 301 alone was accepted because GDAL did not expose it in the
COLOR_PROFILE domain. The final bounded tag scan rejects it before GDAL open.
The fixture remains a required refusal, not a waived unsupported test.

Peak RSS is observed through a fresh small child launcher using Linux wait4,
not inherited Python fixture-generation memory. Ordinary cases used roughly
65 MiB and the metadata-heavy case roughly149 MiB. These are observed process
high-water marks, not universal native-allocation bounds; descriptor/thread
and scratch peaks are not measured by this oracle. Native blocks are limited
to256×256, but GDAL metadata/codec allocations remain an explicit limitation.
The final inventory and cleanup tests establish ownership separately from RSS.

`oracle-source-manifest.json` verifies the initially uncommitted tested source
against its later commit, including production file hashes and build settings.
`implementation-results.json` hashes local check logs; wheel/interpreter records
are separate. No release artifact or publication acceptance is implied.

Replay from the repository root after building the native CLI:

```sh
cargo test --locked
cargo test --locked --features native-geospatial,native-jpeg
python3 -B tests/d1_raster_oracle.py target/debug/rusty-tiles \
  --json-output bench/architecture_audit/directory/oracle.json
```

The oracle uses an existing `cc` and the checked-in small RSS launcher where
`/usr/bin/time` is unavailable. It downloads nothing. The native suite includes
real directory collisions, commit/cancel ordering, retained cleanup paths,
postcommit staging-name reuse, source close/member write/finalizer failures,
concurrent facade attempts, CWD redirection and installed-interface behavior.

The first macOS CI compile found a libc `getattrlist` pointer-type mismatch.
The platform-only cast correction is included after `a299e6c`; a rebuilt native
Linux CLI remains byte-for-byte identical to the tested binary. The source
manifest records this equivalence. Actual macOS execution still requires CI.
