# R2 source raster and imagery pyramid

R2 replaces the legacy `raster.rs` producer and its `output::Job` consumer.
It consumes the accepted F0 attempt and D1/D2 directory lifecycle. This is an
authoring contract; preparation and selected native facts do not establish
production acceptance or close #113, #124 or #126.

The source owner admits a local GTiff, PNG, JPEG or AAIGrid, subject to actual
driver availability. Its finite inventory contains at most five regular,
nonsymlink leaves. Named PAM, mask, worldfile and AAIGrid projection companions
have explicit per-driver authority; external overviews, RPC, executable source
references and unrecognized authority are refused. Canonical aliases and source
ancestors cannot be replaced by the output. Source identity and stable external
configuration remain caller preconditions through explicit source close and
dependency recheck; this is not a snapshot or compare-and-swap guarantee.

The standalone COG preserves full-resolution decoded typed samples, Boolean
validity, critical affine/CRS, Area/Point, NoData, roles, scale, offset, units and
normalized opaque palette. Positive combined input uses homogeneous real
types, representable common NoData and shared ordinary masks. Source JPEG
entropy and raw 16-bit TIFF palette entries are not preservation authorities.
A single ordinary real band with Undefined color interpretation receives a
named Gray interpretation in the COG; defined roles, palette/alpha, multiband
and wide/complex inputs do not receive this migration. The report retains the
original role and names the migration. The original exact-role failing probe
remains a failure under its original contract.

Strict COG creation regenerates nearest overviews, uses DEFLATE without a
predictor, closes the artifact and reopens without PAM. Full windowed all-band
sample and Boolean-mask coherence precedes display. Original source handles
close and every admitted dependency is rechecked. Display then reads the
standalone COG, resolving roles once. Image selects Byte RGB or gray; explicit
gray styling has a finite increasing raw-value range. Exactly one opacity
source applies. Ordinary nonzero masks mean valid, not opacity. An explicit
alpha overrides the native alpha-mask association while independent NoData and
ordinary masks retain authority. Invalid pixels and zero opacity become zero
RGBA. Unmasked selected nonfinite samples fail.

The grid admits static EPSG:4326 longitude/latitude within ±180/±85 or
EPSG:3857 within represented H0 = `20037508.342789244`. Finite nonsingular
source affines receive a bounded outward corner enclosure. Source dimensions
are at most 65536 each. Checked target dimensions fit native i32 and at most
2 GiB finest RGBA; the source ceiling does not impose another target ceiling.
Finest coverage is calculated once and coarse addresses are integer ancestors.
An exact supplied target geotransform is created before Warp into the existing
destination, avoiding extent/size resolution recomputation. Each requested zoom
is sampled directly from the closed finest raster with nearest, east/south ties
selecting the greater fine index. Padding is zero RGBA; recursive nearest
overviews do not define the coarse sample phase.

One `RasterLimits` owns six positive caps: aggregate source 512 MiB, source
pixels 268435456, decoded source 2 GiB, tiles 100000, completed encoded output
8 GiB and logical working allowance 16 GiB. Workers default to two, supported
one through four. Fixed ceilings include 32 bands, five leaves, 1 MiB companion
text, XML depth 64 and 65536 visited nodes, native blocks of 1048576 pixels and
16 MiB decoded, zoom 24 and 16 KiB each for TileJSON/report. Post-open native
metadata/block refusal does not imply zero allocation. Completed-file caps are
checked after close; they are not filesystem quotas.

The concrete ledger uses actual type sizes, admitted counts and observed Rust
capacities. It charges source and target logical pyramids, requested RGBA tiles,
finite source facts/paths, sequential coherence or display scratch, exactly
N+3 member tuples, native argument/config payload and fixed operation owners.
Logical work, physical staged file inventory and native opaque allocations are
distinct; this is not an RSS bound. The ledger charges output path lengths,
so it gates admission but is not published: `report.json` bytes do not depend
on the output location. Actual epoch08 retained grid sizes are GridPlan608 and TargetGrid56 bytes;
source/member capacities and small-input controls are recorded with their exact
source and path identities. These observations do not establish an RSS bound.

R2 scopes thread-local temporary directory, PAM and worker configuration and
restores prior values after native close. Effective `COG_TMP_COMPRESSION` must
be absent or supported ZSTD/LZW; explicit empty/other values are Unsupported.
`COG_DELETE_TEMP_FILES` must be absent or YES. Matching is ASCII case
insensitive. Both codec closures are admitted; the preparation trace does not
identify the actual temporary compression tag. Temporary creation, read-only
reopening and unlink were observed; payload reads were not traced.

One attempt owns original failure/cancellation/observer causes. ABI callbacks
catch panic. Algorithm Run attempts always finish with Finalize and exactly one
Release; utility options and returned datasets have explicit close owners.
Workers, native handles, intermediates, required members, bounded reports and
events finish before D2 seal and publication. The accepted inventory is exactly
`source.cog.tif`, `tiles/{z}/{x}/{y}.png`, `tilejson.json` and `report.json`.

Root owns the public request/result, attempt/publication, inventory/report,
CLI/doctor/preview and migration. Source/native/display/tile have one author;
grid mathematics has another. A separate nonauthor inspects final source and
independent literal raster/PNG meaning, causal faults, capacity controls and
applicable CI/wheel/Blender results. Old Python converter parity, broad
projected CRS and historical report schema are explicitly retired authorities.
Accepted D1 single-tile Python behavior remains a separate consumer.

Preparation, actual facts, failed/superseded epochs and review identities are
retained [losslessly](../../bench/architecture_audit/raster_source/README.md).
No production, cross-platform, publication or release acceptance follows from
this document or its preparation evidence.
