# Which converter do I need?

Use this table for the 0.4.0 development build. For a published binary, check the documentation at its release tag. The [installation guide](INSTALL.md) explains default and native builds.

| Input and goal | Command | Build and limits |
| --- | --- | --- |
| Static textured GLB/glTF needing spatial tiles and coarser geometry | `mesh-to-3tz` | Default; supported triangle geometry and base-colour materials only |
| Existing GLB/glTF to preserve, without creating LOD | `glb-to-3tz` | Default; bundles supported local resources and preserves source content |
| Existing tileset directory to package | `convert` | Default; packages existing content without generating geometry or LOD |
| Eligible rusty-tiles explicit point/vector archive to migrate | `convert-to-implicit` | Default; requires a regular tree and valid existing content, retains payload bytes; see [eligibility](CLI.md#convert-to-implicit) |
| GLB/glTF needing an explicit tileset manifest | `createTilesetJson` | Default; writes `tileset.json` |
| LAS/LAZ with local metre XYZ or a verified grid-free CRS | `point-cloud` | Default; native build required for other supported CRS operations |
| GeoJSON or GeoPackage points, lines and polygons | `vector` | Default; native build required for other supported CRS operations |
| PostGIS, GeoParquet or other OGR vector inputs | `vector` | Native geospatial |
| GeoTIFF or other GDAL imagery | `raster` | Native geospatial; outputs COG and PNG XYZ tiles with TileJSON |
| Elevation raster | `terrain` | Native geospatial; prototype quantized-mesh output with height sidecars |

The three spatial tilers write 3D Tiles 1.1 implicit hierarchies by default. Their `.3tz` archives contain the tileset and content. Vector output uses pinned experimental glTF extensions; consult [viewer compatibility](VECTOR.md#compatibility) before delivery.

## Choose placement before conversion

- **Meshes:** place local model coordinates with `--cartographic-position-degrees`, or select a general horizontal CRS with explicit source axes, height offset and optional shift. Geographic and EPSG:3857 adapters retain their existing conventions. General placement follows the shared grid-free/native CRS policy; see [mesh placement](FORMATS.md#mesh-placement).
- **Point clouds:** choose `--source-crs local` for metre XYZ, `header` for a LAS CRS, or an explicit supported CRS. Georeferenced conversion requires an explicit offset to ellipsoidal metre heights. Use zero only if the source already has those heights.
- **Vectors:** GeoJSON defaults to longitude/latitude; GeoPackage uses its layer CRS. Select layers and decide height handling explicitly. The default build refuses CRS definitions outside its verified grid-free tier. See [vector requirements](VECTOR.md#requirements).
- **Imagery and terrain:** use the native build and locally installed CRS resources. Terrain additionally requires metre heights, an explicit height offset and a fill height; see the [terrain guide](TERRAIN.md).

The mesh adapters and point/vector CRS options have different supported inputs. A CRS accepted by `vector` is not automatically accepted by `mesh-to-3tz`.

## Preserve or generate content

Choose `glb-to-3tz` for a richer model whose animation, skins or material data must remain intact. It does not create coarser geometry, and not every glTF extension can currently be inspected or packaged. Gaussian splat packaging remains tracked in [#91](https://github.com/BenDyson-Arch/rusty-tiles/issues/91).

Choose `mesh-to-3tz` when its [geometry and material contract](FORMATS.md#fidelity-and-input-limits) matches the source and you need LOD. Unsupported attributes or materials are rejected with guidance rather than silently dropped.

For updated vector sources, `vector --reuse-tileset previous.3tz` can retain compatible unchanged content. It still scans the source and repacks the archive. Encoder or backend changes require a fresh conversion baseline; see [reuse after edits](VECTOR.md#reuse-after-edits).

## Inspect and deliver

Run `doctor --command COMMAND` for build readiness and `COMMAND --help` for options. After conversion, run `validate output.3tz`; this validator does not yet accept raster or terrain directories. Use `preview` with extracted archives to check appearance, refinement and picking before publishing.

The [quick start](../README.md#quick-start) uses invented geometry. The [opt-in demo-data suite](../CONTRIBUTING.md#demo-data-acceptance-and-benchmarks) supplies hash-pinned, openly licensed examples for the broader converter set. Conversion never downloads those assets implicitly.
