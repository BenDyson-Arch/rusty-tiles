# Mesh, point cloud and imagery guide

This page explains what the mesh, point-cloud and raster commands produce and where their limits are. It is for users choosing settings for their own data. Every option and default is in the [command reference](CLI.md). Vector and terrain have their own guides: [vector](VECTOR.md) and [terrain](TERRAIN.md).

**Build choice:** mesh and point-cloud conversion work in the standard package for local/manual placement and verified grid-free CRS operations. Other eligible horizontal CRS operations need the native-geospatial build. Imagery conversion always needs native-geospatial. The standard CLI downloads and Python wheels do not include that feature; use the [native container or source build](INSTALL.md#native-geospatial-cli).

Standard GeoJSON/GeoPackage vector conversion shares the grid-free CRS tier and bundles SQLite. Other OGR formats require native-geospatial. See [vector requirements](VECTOR.md#requirements) for reader and height limits.

The commands below use placeholder file names. Replace them with your own data.

## Meshes

Convert meshes with `rusty-tiles mesh`; the [mesh guide](MESH.md) covers its
profiles, placement, declared source CRS and limits. The legacy `mesh-to-3tz`
and `mesh-local-to-3tz` routes were removed in 0.4.0; see the
[migration notes](MESH.md#migration-from-older-mesh-converters).

`glb-to-3tz` preserves exact source and resource bytes within the [bounded static model profile](architecture/model-wrapping-contract.md). Local buffers and PNG/JPEG images must remain inside the model directory; admitted percent escapes and dot segments retain their original URI spellings. Animation, skins, morph targets, imported extensions/extras and embedded data URIs are excluded. The former richer-model wrapping escape hatch is retired. Opaque `convert` packaging does not validate or repair excluded model semantics.

## Point clouds

Local metre XYZ data needs no globe placement and works in the standard build:

```sh
rusty-tiles point-cloud -i cloud.laz -o output/cloud.3tz \
  --source-crs local --max-points 8192 --chunk-points 100000
```

Georeferenced data needs a CRS and an explicit height offset:

```sh
rusty-tiles point-cloud -i cloud.laz -o output/cloud.3tz --source-crs header --height-offset 0
```

`--source-crs header` reads the LAS CRS as WKT or GeoTIFF EPSG keys. A custom GeoTIFF definition needs an explicit CRS such as `EPSG:32632`. Only a 2D horizontal CRS is accepted. An offset of 0 is right only when source Z is already ellipsoidal metres.

The default binary and Python wheel use `proj4rs` for verified grid-free transforms to ECEF. They accept WGS84 geographic (`EPSG:4326`), UTM north/south (`EPSG:32601–32660`, `EPSG:32701–32760`), Web Mercator (`EPSG:3857`) and World Mercator (`EPSG:3395`). WKT1/WKT2 and PROJ definitions support transverse Mercator, Mercator, Lambert conformal conic, Albers equal area, stereographic and oblique stereographic when their datum is explicitly WGS84 or they supply a three- or seven-parameter Helmert shift (`TOWGS84`/`+towgs84`). Horizontal feet and other supported units retain source Z plus the offset in metres. EPSG codes may contain whitespace after the colon, such as `EPSG: 32632`.

Original WKT parameter names and any EPSG method/parameter identities are checked against the declared projection method and WKT version before conversion to PROJ parameters. Mercator’s first standard parallel is distinct from polar stereographic’s latitude of standard parallel; unverified spellings retain native interpretation. LCC/Albers 2SP require both parallels; LCC 1SP requires its origin, and Mercator/polar stereographic parallel variants require their standard parallel. Missing variant parameters, parameters belonging to another method, and colliding aliases (including native-exported Mercator origin aliases) require native interpretation. Shared zero origin/offset and unit scale defaults are retained only for methods where both parsers agree.

Portable conic standard parallels must lie between −80° and +80°, and must be equal or at least one degree apart. Their signed sum must have a magnitude of at least one degree: nearly opposite parallels and tangent conics too close to the equator are refused in **both portable and native builds**, because native output can also be inaccurate. Native checks read canonical method and parameter identities and angular units from PROJJSON, including Michigan, Belgium and the natural origin of LCC 1SP variant B; its false-origin latitude does not determine cone conditioning. PROJ LCC definitions that omit `lat_0` or `lat_2` require native CRS interpretation; native and portable parsers infer different defaults. Ordinary stereographic origins must be strictly between −80° and +80° or exactly at either pole. Oblique stereographic requires its origin to lie strictly between −80° and +80°. Both oblique and nonpolar ordinary stereographic require each point's source latitude to lie strictly within that range. When a point exceeds this coordinate domain, or the portable inverse cannot establish its latitude, native builds retry its whole batch through strict native PROJ, which determines whether the coordinates are valid; default builds refuse the conversion before publication. Albers (spherical or ellipsoidal) points at or beyond ±80° source latitude are refused in **both builds**, because portable and native inverses can both clamp nearby latitudes to a pole. Native parsing applies this source-latitude guard before datum shifts, including quoted definitions and WKT. Nonfinite input XYZ still fails as invalid data.

PROJ UTM definitions require an explicit `+zone` from 1 to 60. Projection scales (`+k`, `+k_0` and their WKT equivalents) must be finite and positive. Polar stereographic rejects a non-unit scale combined with an explicit standard parallel different from its origin, including the opposite pole. Opposite-sign polar origins and standard parallels, or a zero standard parallel, require native CRS interpretation to preserve the hemisphere. Latitude parameters must lie in `[-90, 90]` degrees; Mercator standard parallels must lie strictly inside that range, and Lambert conformal conic standard parallels must also remain at least `1e-10` radians from either pole. Projected PROJ definitions require linear units such as `m` or `ft`; angular unit names such as `degrees` are rejected.

Other EPSG codes, unknown datums, non-Greenwich prime meridians, geographic `+lon_0` offsets, non-decimal PROJ angles, quoted PROJ values, PROJ `+init` references or spaced assignments, unsupported WKT syntax (including mixed delimiters), spherical transverse Mercator, projection parameters outside the portable domain, Lambert azimuthal equal area, unusual axes, coordinate epochs (including `EPSG:32632@2020`) and grid parameters require strict native GDAL/PROJ. LAEA uses native PROJ because the portable implementation exceeds the 1 mm ECEF accuracy threshold for some definitions. An ellipsoid alone does not establish a datum.

The converter automatically uses native fallback when that feature is built. Otherwise it refuses the input with `pure-Rust point-cloud CRS transform unavailable: …; use a build with --features native-geospatial (GDAL >= 3.12, PROJ >= 9.2) and the required local PROJ database/grids`. Compound CRS headers declaring a geoid are refused with this message by the standard build; point-cloud conversion still requires a 2D horizontal CRS and explicitly established ellipsoidal heights even in a native build. No datum shift or grid requirement is silently dropped. `doctor --command point-cloud` lists the tier, CRS classes and native fallback readiness.

### What the output keeps

The default output uses a disk-backed midpoint octree written as the bounded implicit forest, with one binary subtree per implicit root. Coincident points are distributed deterministically, with overlapping actual bounds retained in tile metadata. The earlier `--explicit` binary partition and explicit manifest were removed in 0.4.0.

- Points stream through disk-backed partitions. Chunk/leaf budgets limit record counts; schema, hierarchy and encoded metadata still contribute memory. P1 records measured RSS/scratch/descriptor growth rather than promising a whole-job bound.
- Parents keep the first point per cubic voxel, on the finest grid that fits `--maxPoints`; a child's voxels are never coarser than its parent's. Leaves keep every point, including coincident points.
- Owner amendment, 2026-10-29: tiles hold only position and colour unless `--metadata` adds the table below. Positions are 16-bit per tile axis (`KHR_mesh_quantization`); their error, at most half a step per axis, is reported and in parent geometric error. A leaf whose step would be coarser than one source coordinate step keeps float32. The report gives that step in output metres as `sourcePrecisionMetres` (the smallest LAS scale for local input; otherwise the smallest transformed step over the source box, each axis at its minimum, maximum and spanned zero) and the count of such leaves as `float32Tiles`. `COLOR_0` is 8-bit, round(v/257).
- Metadata keeps source coordinates, source record indices, original 16-bit RGB and supported numeric LAS fields.
- A tile holds at most `--maxPoints` points, at most 8192 (the default), each one row of the tile's single `points` property table when metadata is kept. CesiumJS 1.146 cannot build the GPU texture of a property table wider than the WebGL maximum texture size (8192 on the pinned target; the device limit varies), which disables metadata styling through custom shaders for that tile.
- Scalar metadata keeps exact raw values, including unscaled 64-bit integers, and no-data/min/max declarations in the property schema. Admitted scaled fields are stored as decoded float64; scaled INT64/UINT64 Extra Bytes are explicitly unsupported to prevent silent integer rounding.

Waveforms, array/untyped or undocumented extra dimensions, unknown VLR preservation and compound vertical CRSs are unsupported. Source and Extra Bytes scales must be positive finite. Source metadata is limited to 65,536 VLR/EVLR records and 16 MiB; point data streams separately. The [P1 contract](architecture/point-acceptance-contract.md) records the exact source, coordinate, precision and lifecycle boundaries.

`point-cloud --metadata-attributes` exposes `vertex_classification`, `vertex_intensity` and `vertex_return_number` as property attributes for shader access. The existing table properties (`classification`, `intensity`, `return_number`) remain available for feature picking and table styling. The distinct attribute names prevent shader field collisions in CesiumJS 1.146.

## Imagery

`raster` needs the `native-geospatial` build and calls GDAL directly, without
Python or executable lookup. Its local source profile admits GTiff, PNG, JPEG
and AAIGrid when the selected build provides that driver. Sources need static
EPSG:4326 or EPSG:3857 coordinates, a finite nonsingular affine transform and
coverage within longitude ±180° and latitude ±85° for EPSG:4326.
EPSG:3857 admits its separate represented square, ±20,037,508.342789244 metres
on each axis. Other projected CRSs, polar and wrapped coverage require
preprocessing. Keep sources, their companions and native configuration stable
during the call.

For byte imagery:

```sh
rusty-tiles raster -i orthophoto.tif -o output/imagery \
  --min-zoom 10 --max-zoom 18 --display image
```

For numeric data, choose a band and a raw display range:

```sh
rusty-tiles raster -i measurements.tif -o output/measurements \
  --min-zoom 10 --max-zoom 18 --display gray --band 1 \
  --display-min -10 --display-max 10
```

| Path | Contents |
| --- | --- |
| `source.cog.tif` | Standalone DEFLATE COG with all admitted decoded bands and critical source facts |
| `tiles/{z}/{x}/{y}.png` | 256×256 display tiles: RGB when fully opaque, otherwise RGBA |
| `tilejson.json` | Relative XYZ tile addresses and approximate descriptive geographic source bounds |
| `report.json` | Typed source, grid, display, limits and resource report |

The COG is checked against every source sample and independent Boolean mask
before display production. Bands must share a supported real sample type and
common NoData declaration. Critical affine, CRS, Area/Point, roles, NoData,
scale, offset, units and normalized opaque palettes are preserved. A single
otherwise ordinary Undefined band has the named Undefined→Gray role migration
reported explicitly. This does not promise original encoded bytes, JPEG
entropy, arbitrary metadata or 16-bit palette precision. External overviews,
reference-bearing companions, remote sources and ambiguous authority are
unsupported. Source internal overviews may exist; output overviews are rebuilt.

Display styling uses raw samples; scale and offset remain source metadata.
Gray maps the finite increasing range to 0–255 with nearest integer rounding
and clamps values outside it. Byte image display selects RGB, gray or opaque
palette colors. `--alpha-band` selects opacity explicitly; otherwise the
unique applicable declared alpha is used, or opacity 255. Numeric alpha clamps
to 0–255 and rounds without scaling. Independent masks are Boolean validity,
so any nonzero mask value is valid. NoData and independent masks still apply
with an explicit alpha override. Invalid samples become transparent black;
valid black retains its opacity.

The finest display uses an exact admitted grid and explicit nearest-neighbour
warp. Every requested coarser zoom samples that closed finest image directly
by exact integer pixel index, with east/south tie selection and transparent
padding. Tiles are sampled and PNG-encoded sequentially in Rust. XYZ consumers
such as CesiumJS and Leaflet accept the resulting mix of RGB and RGBA PNG
tiles. Reports expose the
exact target transform, dimensions and tile rectangle. Output may vary across
native dependency versions; reproducibility claims require the same build.

Six positive limits cover source bytes, source pixels, decoded bytes, tile
count, completed output bytes and logical work. Defaults are 512 MiB,
268,435,456 pixels, 2 GiB, 100,000 tiles, 8 GiB and 16 GiB respectively.
The finest RGBA derivative has a fixed 2 GiB ceiling. Source dimensions are
limited to 65,536 on each axis; target dimensions must fit positive native
32-bit integers. Native block cache and warp budgets are 64 MiB each. Workers
default to two; `--jobs` permits one through four, capped by available CPUs.
Native thresholds may choose fewer workers. Logical work includes concrete
Rust owners and temporary derivatives; it is not a process RSS or in-call
filesystem quota. Completed member bytes are checked after close. Failure
preserves the previous output through the completed-directory publisher.

There is no PMTiles writer. The [R2 contract](architecture/r2-raster-contract.md)
records the detailed source and ownership boundaries and acceptance status.

## Archives

A `.3tz` is an indexed, stored ZIP or ZIP64 file. `tileset.json` comes first, other members are sorted, and the index is last. Members have fixed 1980-01-01 timestamps and 0644 permissions. Source timestamps and member order do not affect the bytes.

Point-cloud and `convert` archives are reproducible by default. Vector archives need `--reproducible`. Mesh texture bytes depend on the selected codec build. See [byte-identity checking](../CONTRIBUTING.md#check-byte-identity).
