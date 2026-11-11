# Mesh, point cloud and imagery guide

This page explains what the model-packaging, point-cloud and raster commands produce and where their limits are. Every option and default is in the [command reference](CLI.md). Meshes, vectors and terrain have their own guides: [mesh](MESH.md), [vector](VECTOR.md) and [terrain](TERRAIN.md).

Model packaging and point-cloud conversion use the standard build, which also covers the grid-free CRS tier described [below](#coordinate-reference-systems). Imagery conversion and CRS definitions outside that tier need the [native-geospatial build](INSTALL.md#native-geospatial-cli).

The commands below use placeholder file names. Replace them with your own data.

## Meshes

Convert meshes with `rusty-tiles mesh`; the [mesh guide](MESH.md) covers its profiles, placement, source CRS and limits.

`glb-to-3tz` packages a model without changing it. It preserves exact source and resource bytes within the [bounded static model profile](architecture/model-wrapping-contract.md). Local buffers and PNG/JPEG images must stay inside the model directory. Admitted percent escapes and dot segments keep their original URI spellings. Animation, skins, morph targets, imported extensions/extras and embedded data URIs are excluded. `convert` packages content as opaque bytes; it does not validate or repair excluded model semantics.

## Point clouds

Local metre XYZ data needs no globe placement:

```sh
rusty-tiles point-cloud -i cloud.laz -o output/cloud.3tz \
  --source-crs local --max-points 8192 --chunk-points 100000
```

Georeferenced data needs a CRS and an explicit height offset:

```sh
rusty-tiles point-cloud -i cloud.laz -o output/cloud.3tz --source-crs header --height-offset 0
```

`--source-crs header` reads the LAS CRS as WKT or GeoTIFF EPSG keys. A custom GeoTIFF definition needs an explicit CRS such as `EPSG:32632`. Only a 2D horizontal CRS is accepted. An offset of 0 is right only when source Z is already ellipsoidal metres.

### Coordinate reference systems

The standard build transforms to ECEF with `proj4rs`, using only verified grid-free operations. This tier is shared by mesh `--source-crs` and standard vector conversion. It accepts:

- WGS84 geographic (`EPSG:4326`), UTM north and south (`EPSG:32601–32660`, `EPSG:32701–32760`), Web Mercator (`EPSG:3857`) and World Mercator (`EPSG:3395`). EPSG codes may contain whitespace after the colon, such as `EPSG: 32632`.
- About 3,100 further EPSG codes from a [GDAL-verified allow-list](#gdal-verified-epsg-allow-list), including GDA2020 and GDA94 MGA zones, GDA94/GDA2020 geographic, GDA94 Lambert and Albers, NZTM2000, ETRS89 UTM and TM35FIN.
- WKT1/WKT2 and PROJ definitions using transverse Mercator, Mercator, Lambert conformal conic, Albers equal area, stereographic or oblique stereographic, when the datum is explicitly WGS84 or the definition supplies a three- or seven-parameter Helmert shift (`TOWGS84`/`+towgs84`). The [parameter rules](#projection-parameter-rules) below apply.

Horizontal feet and other supported units leave source Z unscaled: heights stay source Z plus the offset, in metres.

### GDAL-verified EPSG allow-list

The standard build embeds a table of EPSG codes whose definition, run through this resolver, reproduces GDAL/PROJ's best operation. Each listed code maps to an explicit PROJ or WKT1 definition with a three- or seven-parameter Helmert shift or a null shift, never a grid. A code is listed only when all 18 samples, a 3×3 lattice over its area of use at heights 0 m and 1,000 m, land within 1 mm in ECEF of GDAL's only-best, no-ballpark, network-off result, and GDAL answers at every sample. The table records the EPSG dataset, PROJ and GDAL versions it was generated with (EPSG v13.102, PROJ 9.9.0 and GDAL 3.13.3), and CI regenerates it to catch drift. Sampling is evidence over the area of use, not a proof for every coordinate.

Covered families include GDA2020 MGA (`EPSG:7846–7859`), GDA94 MGA (`EPSG:28348–28358`), GDA94 Lambert and Albers (`EPSG:3112`, `EPSG:3577`), GDA94 and GDA2020 geographic (`EPSG:4283`, `EPSG:7844`), NZTM2000 (`EPSG:2193`), ETRS89 UTM (`EPSG:25829–25835`), TM35FIN (`EPSG:3067`), RGF93 Lambert-93, SWEREF99 TM, the JGD2000/JGD2011 plane rectangular systems, NAD83(2011) and NAD83(CSRS) codes, and many national grids defined by Helmert transformations.

`conversion.json` names the datum operation each listed code uses, with EPSG's stated accuracy, as `crsOperation` (point clouds), per layer (vectors) or `source_crs.datum_operation` (meshes); the CLI prints it after the conversion. A null transformation, for example GDA2020 to WGS 84 (2), is reported with its stated accuracy of 3 m, never as exact: the numbers match GDAL, but the datums still differ by up to that amount. `rusty-tiles doctor --sourceCrs EPSG:7856` resolves one definition without converting anything and reports its operation.

These stay refused in the standard build and need the native-geospatial build:

- grid-based operations: British National Grid (`EPSG:27700`, OSTN15), NAD27, the original NAD83 State Plane and UTM codes, NAD83 BC and Canada Albers and similar, about 740 codes;
- CRSs with no non-ballpark operation offline, such as CGCS2000;
- methods outside the portable set, such as ETRS89 LAEA Europe (`EPSG:3035`), Swiss Oblique Mercator and Krovak;
- coordinate epochs (`EPSG:7856@2020`) and compound CRSs;
- CRSs on a dynamic reference frame, such as ITRF2014 (`EPSG:9000`), WGS 84 (G1762) (`EPSG:9057`) and WGS 72 (`EPSG:4322` and its UTM zones), even where GDAL answers without an epoch. Datum ensembles such as WGS 84 and ETRS89 are not dynamic frames and are covered.

A native build keeps resolving every EPSG code other than the WGS84 cases with strict native PROJ; the table serves the standard build only.

These need strict native GDAL/PROJ instead:

- EPSG codes outside the allow-list, unknown datums, non-Greenwich prime meridians and geographic `+lon_0` offsets;
- non-decimal PROJ angles, quoted PROJ values, PROJ `+init` references or spaced assignments, and unsupported WKT syntax, including mixed delimiters;
- spherical transverse Mercator, Lambert azimuthal equal area, projection parameters outside the portable domain and unusual axes;
- coordinate epochs, including `EPSG:32632@2020`, and grid parameters.

Lambert azimuthal equal area uses native PROJ because the portable implementation exceeds the 1 mm ECEF accuracy threshold for some definitions. An ellipsoid alone does not establish a datum.

A native build falls back to native PROJ automatically. The standard build refuses the input with `pure-Rust CRS transform unavailable: …; use a build with --features native-geospatial (GDAL >= 3.12, PROJ >= 9.2) and the required local PROJ database/grids`. It also refuses compound CRS headers that declare a geoid with this message. Even a native build requires a 2D horizontal CRS and explicitly established ellipsoidal heights for point clouds. No datum shift or grid requirement is silently dropped. `doctor --command point-cloud` lists the tier, CRS classes, the allow-list's size and versions, and native fallback readiness.

### What the output keeps

Output is a disk-backed midpoint octree written as a bounded implicit forest, with one binary subtree per implicit root. Coincident points are distributed deterministically, and their overlapping actual bounds are kept in tile metadata.

- Points stream through disk-backed partitions. Chunk and leaf budgets limit record counts. Schema, hierarchy and encoded metadata still use memory, and there is no whole-job memory bound.
- Parents keep the first point per cubic voxel, on the finest grid that fits `--max-points`; a child's voxels are never coarser than its parent's. Leaves keep every point, including coincident points.
- Tiles hold only position and colour unless `--metadata` adds the property table below.
- Positions are 16-bit per tile axis (`KHR_mesh_quantization`). Their error, at most half a step per axis, is reported and included in parent geometric error. A leaf whose step would be coarser than one source coordinate step keeps float32. The report gives that source step in output metres as `sourcePrecisionMetres` and counts such leaves as `float32Tiles`. For local input the step is the smallest LAS scale; otherwise it is the smallest transformed step over the source box, each axis at its minimum, maximum and spanned zero.
- `COLOR_0` is 8-bit, round(v/257).
- Metadata keeps source coordinates, source record indices, original 16-bit RGB and supported numeric LAS fields.
- A tile holds at most `--max-points` points, at most 8192 (the default). With metadata, each point is one row of the tile's single `points` property table. CesiumJS 1.146 cannot build the GPU texture for a property table wider than the WebGL maximum texture size (8192 on the pinned target; the device limit varies). That disables metadata styling through custom shaders for the tile.
- Scalar metadata keeps exact raw values, including unscaled 64-bit integers, and no-data/min/max declarations in the property schema. Admitted scaled fields are stored as decoded float64. Scaled INT64/UINT64 Extra Bytes are unsupported, to prevent silent integer rounding.

`--metadata-attributes` exposes `vertex_classification`, `vertex_intensity` and `vertex_return_number` as property attributes for shader access. The table properties (`classification`, `intensity`, `return_number`) remain available for feature picking and table styling. The distinct attribute names prevent shader field collisions in CesiumJS 1.146.

Waveforms, array, untyped or undocumented extra dimensions, unknown VLR preservation and compound vertical CRSs are unsupported. Source and Extra Bytes scales must be positive and finite. Source metadata is limited to 65,536 VLR/EVLR records and 16 MiB; point data streams separately. Nonfinite input XYZ fails as invalid data. The [point-cloud design notes](architecture/point-acceptance-contract.md) give the exact source, coordinate, precision and lifecycle boundaries.

### Projection parameter rules

Most users can skip this section. It lists the definitions the standard build refuses, or passes to native PROJ, because the portable and native parsers could disagree.

**Parameter names.** Original WKT parameter names and any EPSG method or parameter identities are checked against the declared projection method and WKT version before conversion to PROJ parameters. Unverified spellings use native interpretation. Mercator's first standard parallel is distinct from polar stereographic's latitude of standard parallel. Missing variant parameters, parameters belonging to another method and colliding aliases, including native-exported Mercator origin aliases, require native interpretation. Shared zero origin/offset and unit scale defaults are kept only for methods where both parsers agree.

**Required parameters.**

- LCC and Albers 2SP require both standard parallels; LCC 1SP requires its origin. Mercator and polar stereographic parallel variants require their standard parallel.
- PROJ LCC definitions that omit `lat_0` or `lat_2` require native interpretation, because the two parsers infer different defaults.
- PROJ UTM definitions require an explicit `+zone` from 1 to 60.
- Projected PROJ definitions require linear units such as `m` or `ft`; angular unit names such as `degrees` are rejected.

**Value ranges.**

- Latitude parameters must lie in `[-90, 90]` degrees. Mercator standard parallels must lie strictly inside that range. LCC standard parallels must also stay at least `1e-10` radians from either pole.
- Projection scales (`+k`, `+k_0` and their WKT equivalents) must be finite and positive.
- Polar stereographic rejects a non-unit scale combined with an explicit standard parallel different from its origin, including the opposite pole. Opposite-sign polar origins and standard parallels, or a zero standard parallel, require native interpretation to preserve the hemisphere.

**Conic conditioning.** Portable conic standard parallels must lie between −80° and +80°, and must be equal or at least one degree apart. Their signed sum must have a magnitude of at least one degree. Nearly opposite parallels and tangent conics too close to the equator are refused by **both standard and native builds**, because native output can also be inaccurate. Native checks read canonical method and parameter identities and angular units from PROJJSON, including Michigan, Belgium and the natural origin of LCC 1SP variant B; its false-origin latitude does not determine cone conditioning.

**Latitude domain.**

- Ordinary stereographic origins must be strictly between −80° and +80°, or exactly at either pole. Oblique stereographic origins must be strictly between −80° and +80°.
- For oblique and nonpolar ordinary stereographic, each point's source latitude must also lie strictly within that range. When a point is outside it, or the portable inverse cannot establish its latitude, a native build retries the whole batch through strict native PROJ, which decides whether the coordinates are valid. The standard build refuses the conversion before publication.
- Albers (spherical or ellipsoidal) points at or beyond ±80° source latitude are refused by **both builds**, because portable and native inverses can both clamp nearby latitudes to a pole. Native parsing applies this guard before datum shifts, including quoted definitions and WKT.

## Imagery

`raster` needs the native-geospatial build. It calls GDAL directly, without Python or executable lookup. It reads GTiff, PNG, JPEG and AAIGrid sources when the build provides that driver. Sources need:

- static EPSG:4326 or EPSG:3857 coordinates;
- a finite, nonsingular affine transform;
- for EPSG:4326, coverage within longitude ±180° and latitude ±85°; for EPSG:3857, coverage within ±20,037,508.342789244 metres on each axis.

Other projected CRSs, polar coverage and wrapped coverage require preprocessing. Keep sources, their companion files and native configuration unchanged during the call.

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

There is no PMTiles writer.

### Source preservation

The COG is checked against every source sample and independent Boolean mask before display tiles are made. Bands must share a supported real sample type and a common NoData declaration. Critical affine, CRS, Area/Point, roles, NoData, scale, offset, units and normalized opaque palettes are preserved. A single otherwise ordinary Undefined band is migrated to the Gray role, and the report says so.

Original encoded bytes, JPEG entropy, arbitrary metadata and 16-bit palette precision are not preserved. External overviews, reference-bearing companion files, remote sources and ambiguous authority are unsupported. Sources may have internal overviews; output overviews are rebuilt.

### Display

Display styling uses raw samples; scale and offset remain source metadata.

- `gray` maps the finite increasing range to 0–255 with nearest-integer rounding and clamps values outside it.
- Byte image display selects RGB, gray or opaque palette colours.
- `--alpha-band` selects opacity explicitly. Otherwise the unique applicable declared alpha band is used, or opacity 255. Numeric alpha clamps to 0–255 and rounds without scaling.
- Independent masks are Boolean validity: any nonzero mask value is valid. NoData and masks still apply with an explicit alpha band. Invalid samples become transparent black; valid black keeps its opacity.

The finest zoom uses an exact grid and nearest-neighbour warp. Each coarser zoom samples that finest image directly by exact integer pixel index, with east/south tie selection and transparent padding. Tiles are sampled and PNG-encoded sequentially in Rust. XYZ consumers such as CesiumJS and Leaflet accept the mix of RGB and RGBA tiles. The report gives the exact target transform, dimensions and tile rectangle. Output may vary across native dependency versions; byte-identical output requires the same build.

### Limits

| Limit | Default |
| --- | --- |
| Source bytes | 512 MiB |
| Source pixels | 268,435,456 |
| Decoded bytes | 2 GiB |
| Tile count | 100,000 |
| Completed output bytes | 8 GiB |
| Logical work | 16 GiB |

All six limits must be positive. The finest RGBA derivative has a fixed 2 GiB ceiling. Source dimensions are limited to 65,536 on each axis; target dimensions must fit positive native 32-bit integers. Native block cache and warp budgets are 64 MiB each.

Workers default to two; `--jobs` accepts one to four, capped by available CPUs. Native thresholds may choose fewer workers. Logical work counts Rust-owned buffers and temporary derivatives; it is not a process RSS or filesystem quota. Completed member bytes are checked after the files are closed. On failure the previous output is kept.

The [raster design notes](architecture/r2-raster-contract.md) give the detailed source and ownership boundaries.

## Archives

A `.3tz` is an indexed, stored ZIP or ZIP64 file. `tileset.json` comes first, other members are sorted, and the index is last. Members have fixed 1980-01-01 timestamps and 0644 permissions. Source timestamps and member order do not affect the bytes.

Point-cloud and `convert` archives are reproducible by default. Vector archives need `--reproducible`. Lossy mesh texture bytes depend on the codec build and CPU architecture. See [byte-identity checking](../CONTRIBUTING.md#check-byte-identity).
