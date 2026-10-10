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
40 positives, 38 typed refusals/preservation controls and three
sensitivity controls. The fixture writer and PNG reader use only Python's
standard library, with no production or GDAL format helpers. Positives cover
all 32 raw/AdobeDeflate × contiguous/separate × strip/tile × classic/BigTIFF ×
byte-order combinations, zoom boundaries, a 32 MiB source,
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
61 MiB and the metadata-heavy case roughly146 MiB. These are observed process
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
Linux CLI remains byte-for-byte identical to the tested binary. Windows and both macOS architecture directory tests passed on `129001c`.
Final review-fix CI remains required.

The review at `129001c` reproduced extreme BigTIFF offsets classified as I/O
failures. The reader now checks each requested byte range against source length
before seeking, including checked addition. The oracle requires exit 3 and
`invalid_input` for both endian forms of the extreme offsets, EOF and truncated
count boundaries, with no output or staging residue. The original reproduction
is retained in `review-large-ifd-offset-before-fix.json`.

Admission is now limited to uncompressed or AdobeDeflate, predictor 1, and
contiguous or separate storage. Other codec/predictor values are refused before
GDAL open. This matches the independently proved storage combinations rather
than admitting every installed GDAL codec. The review-fix native suite passes
376 tests (9 existing ignored), including the new public offset regression.
The Docker test fixture now uses the same supported-filesystem selector as the
runtime tests; overlay filesystem admission remains unchanged.

## D2 replacement evidence

The [D2 contract](../../../docs/architecture/d2-directory-contract.md) declares
current-at-hold replacement, an absent-output interval, conditional restoration,
and native-path recovery transport. The independent review is in
[d2-review.md](d2-review.md). `d2-oracle.json` proves six replacement leaf cases,
six preservation checks and eight unscheduled subprocess races through the real
CLI. Deterministic fault and cross-process schedules are separate runtime tests;
unscheduled CLI successes do not prove a failed-restoration schedule.

`d2-raster-regression.json` replays the complete 40-positive/38-refusal raster
oracle on the D2 binary. `d2-source-manifest.json` binds that binary and production
sources to their commit. `d2-implementation-results.json` records check hashes
and distinguishes full suites from later focused controls. Installed wheel and
local distribution-Blender receipts are separate. Platform CI remains required
before D2 acceptance; no Linux result substitutes for Windows/macOS execution.

Replay the real-consumer replacement oracle after a native build:

```sh
python3 -B tests/d2_directory_oracle.py target/debug/rusty-tiles \
  --json-output bench/architecture_audit/directory/d2-oracle.json
```

### D2 overlap correction

The review at `3d17e4` reproduced source deletion when the output identified the
source or contained it. That candidate is not accepted. The raster consumer now
compares filesystem identities before decoding, events or staging, including
hard links and ancestor aliases, while leaving the final output symlink
unfollowed. Corrected evidence is in `d2-overlap-oracle.json`,
`d2-overlap-raster-regression.json`, `d2-overlap-results.json`, and
`d2-overlap-source-manifest.json`. Earlier D2 receipts remain historical evidence
and do not override this correction. #130 remains held for corrected review.
