# Mesh, point cloud and imagery guide

This page explains what the mesh, point-cloud and raster commands produce and where their limits are. It is for users choosing settings for their own data. Every option and default is in the [command reference](CLI.md). Vector and terrain have their own guides: [vector](VECTOR.md) and [terrain](TERRAIN.md).

**Build choice:** mesh and point-cloud conversion work in the standard package for local/manual placement and verified grid-free CRS operations. Other eligible horizontal CRS operations need the native-geospatial build. Imagery conversion always needs native-geospatial. The standard CLI downloads and Python wheels do not include that feature; use the [native container or source build](INSTALL.md#native-geospatial-cli).

Standard GeoJSON/GeoPackage vector conversion shares the grid-free CRS tier and bundles SQLite. Other OGR formats require native-geospatial. See [vector requirements](VECTOR.md#requirements) for reader and height limits.

The commands below use placeholder file names. Replace them with your own data.

## Meshes

For the bounded core PBR source profile and full-detail leaves, use
`mesh-local-to-3tz --leaf-triangles N`. It interprets source geometry as local
metres/Y-up and preserves selected PNG/JPEG bytes, all five core PBR texture
bindings and admitted vertex companions. Its F1c1 rigid placement is a development
candidate pending independent review and final consumer/platform acceptance:
give `--anchor LON LAT ELLIPSOIDAL_HEIGHT` and optional ENU
`--orientation-xyzw X Y Z W` / post-node Y-up metre `--scene-offset X Y Z`.
The cartographic ENU frame uses declared latitude/longitude, including elevated
anchors and longitude-defined pole meridians. One f64 root matrix transforms
local geometry and conservative oriented boxes; no Earth-sized values are baked
into f32 positions. See the [exact frame/limits](architecture/f1c1-contract.md)
and [command/report reference](CLI.md#mesh-local-to-3tz). No arbitrary source CRS,
geoid correction, coarse approximation or picking is admitted by that placement.

The remaining mesh sections describe the broader legacy `mesh-to-3tz` route.
It remains advertised pending separate support/migration decisions; the bounded
placement candidate does not establish its CRS/approximation correctness.

```sh
rusty-tiles mesh-to-3tz -i model.glb -o output/model.3tz --texture-format jpeg
```

Large meshes are split into full-detail leaves with simplified, textured parents. An input within both `--max-triangles` and `--max-bytes` is wrapped without splitting.

The default hierarchy uses midpoint octree cells, assigning each triangle by its centroid. The atlas fit test and single oversized triangle escape remain in place. Standard `TILE_BOUNDING_BOX` and `TILE_GEOMETRIC_ERROR` metadata preserve the actual bounds and measured errors. If a subtree's content extends beyond its regular cell, an external implicit tileset root exposes its actual bounds before viewer culling. Coincident centroids are distributed in source order to keep splitting finite; their actual boxes remain authoritative.

`--explicit` keeps the earlier median partition, folded hierarchy and output bytes. It is intended for consumers that walk the explicit `children` array.

### Texture formats

| `--texture-format` | Choose it for |
| --- | --- |
| `jpeg`, the CLI default | Fast delivery with high-quality lossy colour. Quality 95 with full chroma resolution. |
| `webp` | Smaller downloads, with lossless alpha |
| `lossless` | Exact decoded leaf texels, stored as PNG |
| `uastc` | GPU-compressed KTX2 textures. Needs a Basis Universal encoder. |

Materials that need transparency use PNG. The library's `MeshTo3tzOptions::default()` uses `lossless`.

For `uastc`, build the pinned encoder first. The helper script downloads and builds it, and needs CMake, curl and tar. Conversion itself never downloads it.

```sh
./scripts/build-basisu.sh
rusty-tiles mesh-to-3tz -i model.glb -o output/model.3tz --texture-format uastc --basisu target/tools/basisu
```

### Advanced options

- `--max-triangles` sets the leaf split threshold. The default is 20,000 triangles.
- `--tile-size` sets the leaf atlas edge. The default is 2048. A single oversized chart keeps its texels in a larger atlas.
- `--no-meshopt` turns off lossless meshopt compression. Neither mode quantizes float32 leaf geometry.
- `--max-texel-density` must stay 0, so leaf texels keep source resolution.
- `RAYON_NUM_THREADS` limits conversion threads.

Allow scratch disk beside the output for staged geometry and textures.

### Fidelity and input limits

Leaves keep their original triangles and source-resolution texture charts. JPEG and WebP delivery are lossy, and so is parent simplification. Choose `lossless` for exact leaf texels. Coarse levels may keep child surfaces when safe simplification is not possible. Refinement estimates are not a universal visual-error guarantee.

Spatial splitting supports static triangle meshes with optional normals, UVs in `[0,1]` and supported base-colour images. These inputs are rejected:

- Animation, skinning and morph targets.
- Extra vertex attributes and additional PBR texture channels.
- Unsupported source extensions.

`glb-to-3tz` preserves exact source and resource bytes within the [bounded static model profile](architecture/model-wrapping-contract.md). Local buffers and PNG/JPEG images must remain inside the model directory; admitted percent escapes and dot segments retain their original URI spellings. Animation, skins, morph targets, imported extensions/extras and embedded data URIs are excluded. The former richer-model wrapping escape hatch is retired. Opaque `convert` packaging does not validate or repair excluded model semantics.

Small local meshes that fit the `mesh-to-3tz` budgets also retain their source bytes. In the default implicit layout, declared resources move alongside the content under `implicit-content/`, preserving nested relative URI bases. A resource name that collides with generated content is refused before publication, including with `--force`.

### Mesh placement

| Option | Use |
| --- | --- |
| `--source-crs auto` | The default. Detects the position convention from coordinate values. |
| `--source-crs geographic` | Positions are longitude, height and negative latitude |
| `--source-crs epsg:3857` | Positions are easting, height and negative northing |
| `--source-crs EPSG:32632 --source-axes xyz --height-offset 0` | General horizontal CRS: POSITION is easting, northing, metre height after node transforms |
| `--source-axes y-up` | General CRS with easting, metre height, negative northing after node transforms |
| `--source-offset E N [A]` | Restore a source shift: E/N in horizontal CRS units, A metres |
| `--source-offset-file offset.txt` | Read that shift from `E:`, `N:` and optional `A:` lines |
| `--cartographic-position-degrees lon lat [height]` | Place a local model on the globe |
| `--rotation-degrees heading pitch roll` | Orient a placed model |

Choose an explicit `--source-crs` when you know the export's reference. Existing `auto`, `geographic` and `epsg:3857` commands retain their original Y-up adapter behaviour when axes/height options are absent. Auto with an offset remains the EPSG:3857 shifted adapter; it does not infer another projected CRS. Mesh files carry no trusted CRS selection: supply a horizontal EPSG, WKT or PROJ definition for other systems. The general path requires explicit `--source-axes xyz` or `y-up` and `--height-offset`, including zero when heights are already ellipsoidal metres. Axes describe POSITION **after all glTF node transforms**, so inspect the exporter rather than assuming OBJ axes survived a glTF export unchanged.

General placement shares the [point-cloud CRS policy below](#point-clouds): verified grid-free operations work in the default binary/wheel; other definitions need the native build and its locally available best non-ballpark operation. Source E/N offsets use the horizontal CRS's units and A uses metres. Source height remains metres regardless of horizontal units; `height-offset` is added after A. Compound/vertical or geocentric CRS input is refused. For RD New/NAP (`EPSG:7415`), select the horizontal `EPSG:28992` and supply a justified metre offset to ellipsoidal height; the converter does not infer or apply the NAP geoid. GDA2020/MGA (`EPSG:7850`) and GDA94/MGA (`EPSG:28350`) require native CRS resolution and may fail if the best operation needs unavailable grids. No ballpark datum shift is substituted.

The general path projects every loaded vertex into ECEF, chooses a local metre ENU frame, and transforms authored normals by the local projection Jacobian's inverse transpose, preserving hard edges. This CRS bake preserves triangles, winding, UVs and source-node identity before the usual atlas/LOD pipeline. Output positions/normals are float32, so reprojection is not byte identity with source positions. Double-precision offsets avoid first rounding a large shifted coordinate to float32. Precision already lost in source POSITION values or node transforms cannot be recovered. Coordinate/normal working buffers are bounded to batches of 4096 vertices in addition to the existing in-memory mesh. General placement cannot be combined with manual cartographic placement or rotation. Authored normals at an exact geographic pole are refused because the source coordinate basis is singular; valid date-line and near-pole positions remain supported.

Add `--node-features` to `mesh-to-3tz` to pick and style source glTF nodes by `name` and `node_index`. Instances with the same name remain separate features; unnamed nodes use `node_<index>`. The option authors new tile content even for small inputs and keeps node boundaries through parent simplification. It may increase primitive counts and metadata size.

## Point clouds

Local metre XYZ data needs no globe placement and works in the standard build:

```sh
rusty-tiles point-cloud -i cloud.laz -o output/cloud.3tz \
  --source-crs local --max-points 50000 --chunk-points 100000
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

The default output uses a disk-backed midpoint octree and binary implicit subtrees. Coincident points are distributed deterministically, with overlapping actual bounds retained in tile metadata. `--explicit` keeps the earlier binary partition and explicit manifest.

- Points stream through disk-backed partitions. Chunk/leaf budgets limit record counts; schema, hierarchy and encoded metadata still contribute memory. P1 records measured RSS/scratch/descriptor growth rather than promising a whole-job bound.
- Parents hold voxel samples. Leaves keep every point, including coincident points.
- Metadata keeps source coordinates, source record indices, original RGB and supported numeric LAS fields.
- Rendered positions are float32, and their rounding is reported.
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
| `tiles/{z}/{x}/{y}.png` | 256×256 RGBA display tiles |
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
warp. Every requested coarser zoom samples that closed finest image directly,
with east/south tie selection and transparent padding. Reports expose the
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
