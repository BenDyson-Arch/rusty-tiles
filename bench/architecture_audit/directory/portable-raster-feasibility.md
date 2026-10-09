# Portable raster / terrain feasibility (#82)

Evidence checked 2026-10-09. This is a bounded source audit for the #118
directory publication work and #124 raster pilot; it does not implement #82,
select a dependency, or claim tested portable raster support. No Cargo commands
were run for this audit.

## Observed repository behavior

`Cargo.toml` defaults to no native geospatial feature. Its `image` dependency
enables JPEG/PNG/WebP, not TIFF. `src/raster.rs` and `src/terrain/raster.rs`
currently require native GDAL for raster conversion and DEM reading/warping.
The existing `tests/fixtures/raster_oracle.py` imports GDAL and compares against
a frozen historical implementation: useful regression evidence, but not an
independent analytic oracle. `proj4rs = 0.2.0` and `proj4wkt` already exist in
the dependency graph. `src/crs.rs` has explicit restrictions and accuracy
comments; that vector/ECEF transform path is not a raster warp engine.

## Current upstream evidence (documentation/source, not local execution)

| Question | Evidence and limit |
| --- | --- |
| Does Rust `tiff` still lack JPEG and ZSTD decoding? | No. [image-rs supported-format table](https://github.com/image-rs/image-tiff#compressions) lists both decoding methods; JPEG/ZSTD encoding is absent. [Published API](https://docs.rs/tiff/0.11.3/tiff/) identifies 0.11.3. LERC is absent from that support table; do not infer universal TIFF/GeoTIFF support. |
| Is ZSTD necessarily native-free? | No. [Current upstream manifest](https://raw.githubusercontent.com/image-rs/image-tiff/main/Cargo.toml) exposes both `zstd` and `zstd-safe-rust`; the latter uses `zrip-decode`, and `zstd` takes precedence when both are enabled. This is a moving main-branch manifest, not a verified release/dependency selection. Audit a pinned crate and its entire selected feature graph before claiming native-free packaging. |
| Are tiled/range readers available? | [async-tiff 0.3.0](https://docs.rs/async-tiff/0.3.0/async_tiff/) documents read-only tiled TIFF, object-store access, GeoTIFF metadata, request merging and separate IO/decoding. These are useful building blocks, not a drop-in GDAL conversion contract. |
| Is LERC unavailable throughout Rust? | No. [async-tiff decoder source](https://docs.rs/async-tiff/0.3.0/src/async_tiff/decoder.rs.html) registers feature-gated LERC, unwraps Deflate/ZSTD and dispatches numeric types. It currently discards the decoded LERC mask (`_mask`) and uses `zstd::Decoder` for the ZSTD wrapper. Coverage semantics and native transitive dependencies need separate evaluation. |
| Does a Rust COG/XYZ implementation exist? | [cogrs 0.0.4](https://docs.rs/cogrs/0.0.4/cogrs/) documents local/HTTP/S3 sources, point queries, XYZ extraction, reprojection and DEFLATE/LZW/ZSTD/JPEG/WebP. Its [reader docs](https://docs.rs/cogrs/0.0.4/cogrs/cog_reader/) describe range access, overview metadata and tile caching. This establishes availability; correctness, dependency portability, memory bounds and production fitness remain untested here. |
| Can existing Rust transforms replace PROJ wholesale? | [proj4rs 0.2.0](https://docs.rs/proj4rs/0.2.0/proj4rs/) describes lightweight CRS transforms, radians for angular coordinates, optional EPSG definitions, no default WKT support and experimental grid shifts. Supported CRS transforms are feasible; full PROJ/GDAL parity is not established. Raster inverse mapping, pixel semantics and resampling still require implementation. |

The old broad codec/reader absence rationale should therefore be refreshed.
The remaining work is chiefly a defined format/CRS/coverage contract and
evidence of correctness and resource bounds, rather than proof that no Rust
reader exists. “GDAL-free” and “no native transitive dependencies” are distinct
acceptance criteria.

## Proposed pilot constraints

Keep #124 explicit and finite: native-only RGB byte GeoTIFF, exactly 256 by 256,
north-up EPSG:3857, already aligned to one requested XYZ tile, one PNG plus
TileJSON/report through the directory transaction. Refuse unsupported inputs
before publication. No implicit warp, interpolation, COG copy, terrain,
overviews, palette, alpha/mask/NoData, band scaling or general CRS support.
Report capabilities truthfully in the default build. This pilot validates
directory publication and source-to-image correctness, not #82 portability.

Keep the directory sink independent of reader choice. A future portable path
can produce bounded encoded tile artifacts using the same publication API,
without exposing GDAL handles or making a generic raster framework part of
this change.

## Proposed independent analytic oracle

Use Python's standard-library `struct` and `pathlib` to write classic,
little-endian, uncompressed TIFF with one interleaved RGB strip. Populate
baseline width/height, bits-per-sample, compression=1, photometric=RGB,
strip offset/count, samples-per-pixel, rows-per-strip and chunky planar tags;
write GeoTIFF ModelPixelScale, ModelTiepoint and GeoKeyDirectory (projected
model, PixelIsArea, EPSG:3857, metre units). This needs neither GDAL nor a TIFF
writer package. It is a test fixture generator, not a production TIFF writer.

Choose asymmetric channel patterns such as R=column, G=row,
B=(3*column+5*row)%256, including valid black. Independently derive the tile
affine from H=pi*6378137, span=2*H/2**z, left=-H+x*span,
top=H-y*span and pixel size=span/256. Check decoded PNG pixels against these
formulas, XYZ path and independently calculated longitude/latitude bounds.
Avoid production helpers in the expected-value calculation. PNG compression
bytes are not the oracle; decoded pixels are.

Later #82 evidence should add analytic numeric constant/plane rasters,
pixel-center/edge samples, explicit NoData/mask intersections, block/strip and
codec fixtures, rotated transforms, CRS refusals, bounded decode allocation,
overview choice and range-IO accounting. Terrain additionally needs metre
height/vertical-datum policy and seam/error checks. Those are future work,
not implied by the aligned RGB pilot.

## Observed native pilot oracle evidence

The independent script was run against the copied native candidate
`/home/bend/.cache/rusty-tiles-118-native-candidate`. Its SHA-256, source fixture
hashes, report facts and child RSS are recorded in `oracle.json`; the associated
uncommitted source snapshot hashes are in `oracle-source-manifest.json`.
All eight positives passed (classic TIFF and BigTIFF in both byte orders;
zoom 0, 3 and 24; a valid TIFF padded to exactly
32 MiB; and a 30 MiB ASCII ImageDescription). Fifteen refusal/preservation
controls and three oracle sensitivity controls passed. These results have no
silent native-feature skip. Successful CLI rasterReport values matched the
disk reports; refusals required their expected structured error kinds and no
hidden directory staging residue. Truncated headers, invalid table offsets
and an oversized first-IFD entry count were tested explicitly.

The initial `/usr/bin/time` approach could not run because that executable is
absent. The script instead compiles the existing small `measure_child.c`
launcher using `cc`; exec into that launcher precedes fork/wait4, avoiding
inherited Python fixture-generator RSS. Observed peak child RSS was 65,872–
66,660 KiB for ordinary/padded fixtures and 152,720 KiB for the metadata-heavy
fixture. This measures these native processes; it establishes no universal
memory ceiling. Scratch and descriptor peaks were not sampled by this script.

Source review confirms GTiff-only admission, internal georeferencing, restricted
sibling discovery and dataset-file-list checking, plus RGB/byte/size/CRS/affine,
NoData/mask/scale/offset checks. Review identified that 32 MiB input and 256 by 256 raster dimensions alone
do not bound native TIFF codec allocation. The final pilot now rejects storage
blocks exceeding 256 by 256 and explicitly rejects PixelIsPoint and embedded
color/transfer profiles. A bounded first-IFD scan rejects standalone TIFF
TransferFunction tag 301, orientation other than default/top-left, additional
image IFDs and SubIFDs. The TransferFunction control originally demonstrated
that GDAL COLOR_PROFILE metadata alone did not reveal that tag; this source
policy now checks it directly. Native codecs and metadata parsing still require
separate resource evidence; this is not a general allocation proof. The metadata-heavy measurement also demonstrates native metadata
parsing costs substantially more than the 196,608-byte RGB sample buffer.
No full #82 portability or arbitrary-TIFF resource claim follows from this run.
