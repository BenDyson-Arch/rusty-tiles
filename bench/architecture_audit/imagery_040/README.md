# Finite 0.4.0 imagery bar — 2026-10-24

This receipt covers the raster/imagery half of #124 only. The owner-approved
non-mesh rescope is read from `origin/docs/foundation-rescope-040` at
`986b1cf49a861435f04299ffffdfd1439e8f15d4` (section 2 and replacement #124);
`origin/docs/hlod-direction-040` at
`a5ce03e6d3d93f3c3500b14af9d2fd5425636992` repeats the finite non-mesh bar.
The base producer source is `cda7ca6110c714f8eaceea04df8f5945eb6443c8`.
This change adds tests, capability documentation and this receipt, with no
producer changes. It does not accept terrain, platform/consumer checks, the
combined release candidate, or the deferred 0.4.x audit programme.

## Retained domain and acceptance checks

Retain [native R2](../../../docs/architecture/r2-raster-contract.md): admitted
local source closure, sample-preserving standalone COG, Byte RGB/gray or explicit
raw numeric-gray display, one opacity source intersected with independent
NoData/ordinary masks, nearest PNG XYZ pyramid, TileJSON and a finalized report.
The display CRS domain is static EPSG:4326/3857, zooms 0–24; this is imagery,
with no terrain-height or new portable/projected capability promise.

- `tests/test_raster_display.py` adds literal constant RGB (including valid
  black), analytic RGB ramps, three isolated impulses, selected-band numeric
  gray with literal clipping/rounding values, and intersected NoData, ordinary
  mask and zero/partial alpha. Each case checks every emitted PNG texel at
  finest and two coarse levels, including zero padding. The independently
  implemented stdlib D1 PNG reader supplies decoded bytes; GDAL never supplies
  expected display values. COG samples and masks are read back against literal
  arrays, with GDAL as the reader (not an independent TIFF decoder).
- The same fixtures check literal EPSG:3857 TileJSON bounds and complete sample
  counts. `raster_exact_tiler.rs` retains the full z16/z24 source-index census;
  `test_raster_import.py` and `d1_raster_oracle.py` retain EPSG:4326 coverage and
  independent XYZ/TileJSON checks.
- `raster_contract.rs` retains typed/palette/affine/CRS preservation, six-cap
  equality/one-below refusal and selected nonfinite checks. `native_raster.rs`,
  `raster_pyramid.rs`, `raster_directory.rs`, raster module units and the D1/D2
  oracles retain native ownership, physical faults, cancellation, old-output
  preservation, inventory/report finalization and Rust/CLI behavior.
- `bindings/python/tests/test_api.py` asserts no R2 `raster_to_pyramid`,
  `RasterRequest` or `RasterDisplay` export. Its existing D1 Unsupported,
  invalid-request precedence and old-output preservation tests remain. The
  Python README explicitly names this boundary; no Python imagery API is added.

## Resource receipt

The successful native CLI run used the existing local Injalak orthophoto
(`image_data/inj/injalak_orth.tif`, original SHA-256
`e9188f2115602b3e09add329ddbc0e8fb7944b634d921c987068abc1d39cf577`).
Preparation outside measurement normalized 7683×8192 to 8192² RGBA at the same
EPSG:4326 extent and added a central 1024² zero-alpha hole. Prepared-input hash:
`5cafc0ef1ec02ec7abd007b191baf5161ed0d5023951235e57104555612c5c39`.

At z9–14, two requested/resolved workers and `GDAL_CACHEMAX=64` MiB:

- **RSS 302,172 KiB (295.09 MiB), below 768 MiB.** A fresh isolated Python
  wrapper records the converter child's exact `getrusage` high-water mark;
  fixture creation is excluded. The required slot-2 `measure.py` independently
  reports 0.29 GiB (rounded).
- **Sampled owned disk 415,825,040 bytes (396.56 MiB), below 3 GiB**,
  including staging, native temporaries and published output; allocated-block
  peak 308,617,216 bytes. Sampling is every 20 ms, not an exact filesystem quota.
  The explicit logical/completed-workspace cap is 3 GiB; other caps retain R2
  defaults and pass. Input preparation is outside converter-owned scratch.
- Final output is 146,912,514 bytes, including 146,910,311 artifact bytes and
  the two JSON members; ten PNG tiles, finest grid 512×256. This real source's
  small geographic extent makes this a source/COG-heavy downsampling workload,
  not a universal fine-grid stress claim.
- Wall 14.7 s, CPUs `0-3,16-19`, slot 2, no wait in the successful attempt.
  Peak sampled descriptors nine, process threads four, observed swap zero.
  The two-worker operation policy is distinct from total process thread count.
- GDAL 3.13.3, PROJ 9.9.0, SQLite 3.54.0; Linux 7.2.11, i7-12700KF.
  Source and prepared-input hashes are unchanged; returned/stored reports agree,
  exact four-role output inventory is complete, no cleanup diagnostics, no
  private staging and empty temporary directory after publication.

Measured binary SHA-256:
`0ad6a692e59a8e32567a731fe653bbbc6077516ccf497f56fd82d4819393632f`.
The successful run is the only converter resource execution. An earlier queued
wrapper could not launch an absent optional `/usr/bin/time`; no converter ran.
That failed setup is retained separately, and the Python RSS wrapper replaces it.

## Execution and evidence

Passed on this worktree: portable/native `cargo clippy --all-targets -- -D
warnings` (with `--features native-geospatial` for native), portable/native
raster integration suites, native raster module units (31 tests), portable
raster units, all 13 Python raster tests, D1 (40 positives, 38 refusals, three
sensitivity controls), D2 (six replacements, six preservation checks, six
overlap refusals, two unfollowed-link positives and eight observed races), and
`python3 scripts/changelog.py check`. Four selected imagery boundary tests also
pass against a freshly built and installed standard wheel with an empty PATH
(the new no-R2 surface, D1 Unsupported, Replace preservation and invalid-request
precedence). D2 races are observations, not a universal
concurrency proof; retained deterministic fault tests establish their named
schedules.

Raw logs, input/binary identities, preparation/measurement scripts, artifact
inventory and the independent review are retained at
`~/.cache/rusty-tiles-evidence/040-bar-imagery/README.md`.
Technical evidence `MANIFEST.sha256` SHA-256:
`8f19238ee9d8b9a2fed304ee5e73228b74de0d8c563b3861afad1eafcdc6e4c3`.
The manifest includes the raw receipts, executable harnesses and first review;
its mutable external README and later final review are indexed separately.
`cleanup.json` records Cargo cleanup and deletion of the complete task scratch
folder after the artifact inventory, source checks and wheel tests finished.

All Cargo commands and converter runs use `chrt -i 0 nice -n 19`; Cargo uses the
isolated `target-bar-imagery` target, three build jobs and debug info disabled.
Native builds use system SQLite (`LIBSQLITE3_SYS_USE_PKG_CONFIG=1`), two default
Rayon/test workers, and `native-geospatial`. Resource execution uses the required
slot-2 `measure.py` wrapper; preparation is outside converter measurement.
Logical working allowance, sampled owned disk and process RSS are distinct.

No imagery production defect was found or fixed. An initial wheel invocation
also exposed a test-command mistake: that harness ignores requested test names
and ran its broader small smoke suite, including mesh cases; its clean-PATH
precondition failed. The corrected focused four-test run passed. Both the
mistaken invocation and corrected result remain in external evidence. Deeper codec,
native-lifetime, hostile-input/worst-case resources and exhaustive display
matrices remain deferred; this finite run is not a universal memory guarantee.

Final independent nonauthor review: no findings; finite imagery technical bar
accepted with the limits above. External `review-final.json` SHA-256:
`58b36ad15b70c35cdc40bd9684b56c6a91dc8fb4a38a372cb31683e37c42894b`. Only this review-receipt metadata was appended after that review.
