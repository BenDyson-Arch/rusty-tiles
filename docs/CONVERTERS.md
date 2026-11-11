# Which converter do I need?

Most conversions use the standard build: the released CLI, a default Cargo build or the Python wheel. It needs no GDAL, PROJ or GEOS. The native-geospatial build is needed only for imagery, terrain, OGR-only vector formats and CRS operations outside the grid-free tier; see [native-geospatial CLI](INSTALL.md#native-geospatial-cli). Python wheels contain the standard converters only.

| Input and goal | Command | Build and notes |
| --- | --- | --- |
| Static textured GLB/glTF needing spatial tiles and coarser levels | `mesh` | Standard. See the [mesh guide](MESH.md) |
| Existing GLB/glTF to package unchanged, without LOD | `glb-to-3tz` | Standard; bounded static model profile, exact source and resource bytes; [limits](architecture/model-wrapping-contract.md) |
| Existing tileset directory to package | `convert` | Standard; packages existing content without generating geometry or LOD |
| GLB/glTF needing an explicit tileset manifest | `createTilesetJson` | Standard; one static model; writes a sibling `tileset.json` that references the source |
| LAS/LAZ point clouds | `point-cloud` | Standard |
| GeoJSON or GeoPackage points, lines and polygons | `vector` | Standard |
| PostGIS, GeoParquet or other OGR vector inputs | `vector` | Native |
| GeoTIFF or other GDAL imagery | `raster` | Native; writes a COG and PNG XYZ tiles with TileJSON |
| Elevation raster | `terrain` | Native; writes 3D Tiles terrain meshes |

The spatial tilers (`mesh`, `point-cloud`, `vector`) write 3D Tiles 1.1 implicit tiling. Their `.3tz` archives contain the tileset and its content. Vector output uses pinned experimental glTF extensions; check [viewer compatibility](VECTOR.md#compatibility) before delivery.

## Choose placement before conversion

- **Meshes:** `mesh` keeps local output unless `--anchor LON LAT ELLIPSOIDAL_HEIGHT` is given. `--orientation-xyzw X Y Z W` and `--scene-offset X Y Z` require the anchor. Orientation rotates cartographic ENU vectors. The offset is in source Y-up metres after node transforms and before orientation; it is not a projected coordinate or a geoid correction. A projected or geographic source instead declares `--source-crs`, with `--source-axes`, `--source-offset` or `--source-offset-file`, and `--height-offset`; this excludes the anchor options. See [placement](MESH.md#source-and-placement), [source CRS](MESH.md#source-crs) and the [placement frame details](architecture/f1c1-contract.md).
- **Point clouds:** choose `--source-crs local` for metre XYZ, `header` for the LAS CRS, or an explicit supported CRS. Georeferenced conversion requires an explicit offset to ellipsoidal metre heights. Use zero only if the source already has those heights.
- **Vectors:** GeoJSON defaults to longitude/latitude; GeoPackage uses its layer CRS. Select layers and decide height handling explicitly. See [vector requirements](VECTOR.md#requirements).
- **Imagery and terrain:** use the native build with locally installed CRS resources. Terrain also requires metre heights and an explicit height offset; see the [terrain guide](TERRAIN.md).

Mesh `--source-crs` and point-cloud placement share one horizontal CRS resolver, and standard vector conversion supports the same grid-free tier of [CRS definitions](FORMATS.md#point-clouds). Each converter keeps its own height and input rules: native vector support for compound or three-axis CRSs does not extend to mesh or point-cloud input. The [Python API](../bindings/python/README.md) lists the functions the wheel exposes; it does not include every CLI command.

## Preserve or generate content

Choose `mesh` when the source is a static glTF/GLB inside its [admitted domain](MESH.md#source-and-placement) and you need LOD. It keeps core PBR textures, UV0/UV1, authored tangents and colours. Leaves are exact, with per-triangle source identity for picking. Coarse levels carry a sampled, measured geometric error and a representative source identity, and the whole run stays within a declared `--memory-budget`. Unsupported attributes, materials and extensions are refused rather than silently dropped.

Choose `glb-to-3tz` to preserve source bytes within the bounded static local metre/Y-up model profile. It shares the core PBR resource profile with `mesh` and creates no coarser geometry. Animation, skins, morph targets, imported extensions/extras, embedded data URIs and larger models are outside this profile. `convert` can package caller-authored content as opaque bytes, without checking those semantics. Gaussian splats are not supported; see [#91](https://github.com/BenDyson-Arch/rusty-tiles/issues/91).

For updated vector sources, `vector --reuse-tileset previous.3tz` can retain compatible unchanged content. It still scans the source and repacks the archive. Encoder or backend changes require a fresh conversion; see [reuse after edits](VECTOR.md#reuse-after-edits).

## Inspect and deliver

Run `doctor --command COMMAND` to check build readiness and `COMMAND --help` for options. A command listed in `--help` may still lack its native capability; `doctor` reports that. After conversion, run `validate output.3tz`; it does not yet accept raster or terrain directories. Use `preview` with extracted archives to check appearance, refinement and picking before publishing.

Both builds include validation and preview, so a standard CLI can preview imagery and terrain outputs made elsewhere.

The [quick start](../README.md#quick-start) uses invented geometry. [Try each converter](DEMOS.md) adds self-contained point, vector, imagery and terrain examples. Conversion never downloads data. Contributors can run the [demo-data suite](../CONTRIBUTING.md#demo-data-acceptance-and-benchmarks) against a separately provisioned, hash-pinned public corpus.

## Upgrading from 0.3

- `mesh-to-3tz` and `mesh-local-to-3tz` were removed. Use `mesh`; its options differ, so follow the [mesh migration notes](MESH.md#migration-from-older-mesh-converters).
- `point-cloud` and `vector` write implicit tiling only. There is no archive-to-archive rewrite: re-run older explicit point-cloud and vector archives from the original source.
