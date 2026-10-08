# Mesh, point cloud and imagery guide

This page explains what `mesh-to-3tz`, `point-cloud` and `raster` produce, and where their limits are. It is for users choosing settings for their own data. Every option and default is in the [command reference](CLI.md). Vector and terrain have their own guides: [vector](VECTOR.md) and [terrain](TERRAIN.md).

The commands below use placeholder file names. Replace them with your own data.

## Meshes

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

Use `glb-to-3tz` to wrap a richer model unchanged. Referenced local buffers, images and structural metadata schemas are bundled alongside the original glTF or GLB. Resources must be inside the model's directory (subdirectories are supported); data URIs stay embedded. Remote, absolute, escaping and percent-encoded resource URIs are refused before publication.

### Mesh placement

| Option | Use |
| --- | --- |
| `--source-crs auto` | The default. Detects the position convention from coordinate values. |
| `--source-crs geographic` | Positions are longitude, height and negative latitude |
| `--source-crs epsg:3857` | Positions are easting, height and negative northing |
| `--source-offset E N [A]` | Restore a Metashape shifted export |
| `--source-offset-file offset.txt` | Read that shift from a Metashape `offset.txt` |
| `--cartographic-position-degrees lon lat [height]` | Place a local model on the globe |
| `--rotation-degrees heading pitch roll` | Orient a placed model |

Choose an explicit `--source-crs` when you know the export's reference. These adapters cover Metashape-style exports only. They do not read arbitrary CRS metadata. Float32 degree coordinates may already have lost precision that conversion cannot recover.

Add `--node-features` to `mesh-to-3tz` to pick and style source glTF nodes by `name` and `node_index`. Instances with the same name remain separate features; unnamed nodes use `node_<index>`. The option authors new tile content even for small inputs and keeps node boundaries through parent simplification. It may increase primitive counts and metadata size.

## Point clouds

Local metre XYZ data needs no globe placement and works in the default build:

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

The converter automatically uses native fallback when that feature is built. Otherwise it refuses the input with `pure-Rust point-cloud CRS transform unavailable: …; use a build with --features native-geospatial (GDAL >= 3.12, PROJ >= 9.2) and the required local PROJ database/grids`. Compound CRS headers declaring a geoid are refused with this message by the default build; point-cloud conversion still requires a 2D horizontal CRS and explicitly established ellipsoidal heights even in a native build. No datum shift or grid requirement is silently dropped. `doctor --command point-cloud` lists the tier, CRS classes and native fallback readiness.

### What the output keeps

The default output uses a disk-backed midpoint octree and binary implicit subtrees. Coincident points are distributed deterministically, with overlapping actual bounds retained in tile metadata. `--explicit` keeps the earlier binary partition and explicit manifest.

- Points stream through disk-backed partitions, so memory stays bounded.
- Parents hold voxel samples. Leaves keep every point, including coincident points.
- Metadata keeps source coordinates, source record indices, original RGB and supported numeric LAS fields.
- Rendered positions are float32, and their rounding is reported.
- Scalar metadata keeps source values. Scaled extra dimensions are stored as decoded float64.

Waveforms, array extra dimensions, unknown VLR preservation and compound vertical CRSs are unsupported.

`point-cloud --metadata-attributes` exposes `vertex_classification`, `vertex_intensity` and `vertex_return_number` as property attributes for shader access. The existing table properties (`classification`, `intensity`, `return_number`) remain available for feature picking and table styling. The distinct attribute names prevent shader field collisions in CesiumJS 1.146.

## Imagery

`raster` needs the `native-geospatial` build. It uses GDAL's COG, display and tiling APIs directly, so no Python or `gdal` executable is involved.

For byte imagery:

```sh
rusty-tiles raster -i orthophoto.tif -o output/imagery \
  --min-zoom 10 --max-zoom 18 --display image
```

For numeric data, choose a band and an explicit display range:

```sh
rusty-tiles raster -i measurements.tif -o output/measurements \
  --min-zoom 10 --max-zoom 18 --display gray --band 1 \
  --display-min -10 --display-max 10
```

### What the output contains

| Path | Contents |
| --- | --- |
| `source.cog.tif` | Original values, bands, masks and NoData in a DEFLATE COG |
| `tiles/{z}/{x}/{y}.png` | Reprojected display tiles |
| `tilejson.json` | TileJSON manifest for the display tiles |
| `conversion.json` | Settings and source details |

Display tiles are a derivative. The COG is the faithful copy.

### Transparency

Masks and alpha are intersected in 256 by 256 source windows before resampling. `--alpha-band` selects a band holding opacity values from 0 (transparent) to 255 (opaque), for either `image` or `gray` display. Gray display also accepts alpha stored in numeric bands such as Int16: alpha is converted to Byte without scaling, clamping values outside 0–255. The selected data band's mask/NoData and the display alpha still apply, so masked pixels stay transparent. Image display continues to require byte imagery.

For rendered greyscale stored in Int16 bands with alpha in band 4:

```sh
rusty-tiles raster -i survey.tif -o output/survey \
  --min-zoom 10 --max-zoom 18 --display gray --band 1 \
  --display-min 0 --display-max 255 --alpha-band 4
```

### Limits and resources

- A job over 100,000 display tiles fails with advice to narrow the zoom range.
- Polar and antimeridian coverage needs preprocessing.
- The GDAL block cache and display warp budget are 64 MiB each.
- COG compression and tiling each use up to four workers, capped by available CPUs.
- Temporary display TIFFs are uncompressed for speed. Allow scratch space for source-resolution and reprojected bands.
- Display resampling follows the installed GDAL defaults, so output can differ across GDAL versions.
- There is no PMTiles writer.

## Archives

A `.3tz` is an indexed, stored ZIP or ZIP64 file. `tileset.json` comes first, other members are sorted, and the index is last. Members have fixed 1980-01-01 timestamps and 0644 permissions. Source timestamps and member order do not affect the bytes.

Point-cloud and `convert` archives are reproducible by default. Vector archives need `--reproducible`. Mesh texture bytes depend on the selected codec build. See [byte-identity checking](../CONTRIBUTING.md#check-byte-identity).
