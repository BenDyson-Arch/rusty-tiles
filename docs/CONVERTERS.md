# Which converter do I need?

Use this table for the 0.4.0 development build. For a published binary, check the documentation at its release tag. **Standard** means the standard CLI/Rust build and the conversion APIs in the Python wheel, without system GDAL/PROJ/GEOS. **Native-geospatial** means the native container or a CLI/Rust build compiled with `--features native-geospatial`; it is not available as a Python wheel. See [installation](INSTALL.md#choose-a-build).

| Input and goal | Command | Build and limits |
| --- | --- | --- |
| Static textured GLB/glTF needing spatial tiles and coarser geometry | `mesh-to-3tz` | Standard for local/manual or grid-free placement; other eligible CRS operations need native-geospatial. Supported triangle geometry and base-colour materials only |
| Local static GLB/glTF with PNG/JPEG core PBR textures and full-detail leaves | `mesh-local-to-3tz` | Local source metres/Y-up; optional explicit WGS84 anchor/quaternion/metre offset, standard build. Exact shared resources, UV0/UV1, authored tangents/colors; normal maps require authored companions and conformal accumulated transforms. No coarse LOD. Source primitive/triangle picking follows the [F1c2 candidate contract](architecture/f1c2-contract.md); [placement](architecture/f1c1-contract.md); [source profile](architecture/f1b3-contract.md) |
| Existing GLB/glTF to preserve, without creating LOD | `glb-to-3tz` | Standard; bounded static local metre/Y-up profile, exact source/resource bytes, typed publication; [limits](architecture/model-wrapping-contract.md) |
| Existing tileset directory to package | `convert` | Standard; packages existing content without generating geometry or LOD |
| Eligible rusty-tiles explicit point/vector archive to migrate | `convert-to-implicit` | Standard; requires a regular tree and valid existing content, retains payload bytes; see [eligibility](CLI.md#convert-to-implicit) |
| GLB/glTF needing an explicit tileset manifest | `createTilesetJson` | Standard; one admitted static model, writes sibling `tileset.json` with live source references |
| LAS/LAZ with local metre XYZ or a verified grid-free CRS | `point-cloud` | Standard; native build required for other supported CRS operations |
| GeoJSON or GeoPackage points, lines and polygons | `vector` | Standard; native build required for other supported CRS operations |
| PostGIS, GeoParquet or other OGR vector inputs | `vector` | Native-geospatial |
| GeoTIFF or other GDAL imagery | `raster` | Native-geospatial; outputs COG and PNG XYZ tiles with TileJSON |
| Elevation raster | `terrain` | Native-geospatial; bounded 3D Tiles terrain meshes |

The three spatial tilers write 3D Tiles 1.1 implicit hierarchies by default. Their `.3tz` archives contain the tileset and content. Vector output uses pinned experimental glTF extensions; consult [viewer compatibility](VECTOR.md#compatibility) before delivery.

## Choose placement before conversion

- **Bounded mesh foundation:** `mesh-local-to-3tz` keeps local output unless `--anchor LON LAT ELLIPSOIDAL_HEIGHT` is explicit. Optional `--orientation-xyzw X Y Z W` and `--scene-offset X Y Z` require that anchor. Orientation rotates cartographic ENU vectors; offset is post-node source Y-up metres before orientation, not projected coordinates or a geoid correction. See [exact frame, limits and report](architecture/f1c1-contract.md).
- **Legacy broader meshes:** `mesh-to-3tz` uses `--cartographic-position-degrees` or a general horizontal CRS with explicit axes/height/shift. These existing routes remain pending migration; rigid local placement does not establish their numerical acceptance. See [mesh placement](FORMATS.md#mesh-placement).
- **Point clouds:** choose `--source-crs local` for metre XYZ, `header` for a LAS CRS, or an explicit supported CRS. Georeferenced conversion requires an explicit offset to ellipsoidal metre heights. Use zero only if the source already has those heights.
- **Vectors:** GeoJSON defaults to longitude/latitude; GeoPackage uses its layer CRS. Select layers and decide height handling explicitly. The standard build refuses CRS definitions outside its verified grid-free tier. See [vector requirements](VECTOR.md#requirements).
- **Imagery and terrain:** use the native build and locally installed CRS resources. Terrain additionally requires metre heights, an explicit height offset and a fill height; see the [terrain guide](TERRAIN.md).

General legacy mesh and point-cloud placement share the horizontal CRS resolver, but converter-specific height and input restrictions still apply. Native vector support for compound/three-axis CRS does not extend that support to mesh or point-cloud input. The [Python API](../bindings/python/README.md) lists the standard functions it exposes; the wheel does not include every CLI command.

## Preserve or generate content

Choose `glb-to-3tz` to preserve source bytes within the bounded static local metre/Y-up model profile. It shares the admitted core PBR resource profile with `mesh-local-to-3tz` and creates no coarser geometry. Animation, skins, morph targets, imported extensions/extras, embedded data URIs and larger models are explicitly outside this replacement. The former broader wrapping escape hatch is retired; `convert` can package caller-authored content as opaque bytes, without certifying those semantics. Gaussian splat packaging remains tracked in [#91](https://github.com/BenDyson-Arch/rusty-tiles/issues/91).

Choose `mesh-to-3tz` when its [geometry and material contract](FORMATS.md#fidelity-and-input-limits) matches the source and you need LOD. Unsupported attributes or materials are rejected with guidance rather than silently dropped.

For updated vector sources, `vector --reuse-tileset previous.3tz` can retain compatible unchanged content. It still scans the source and repacks the archive. Encoder or backend changes require a fresh conversion baseline; see [reuse after edits](VECTOR.md#reuse-after-edits).

## Inspect and deliver

Run `doctor --command COMMAND` for build readiness and `COMMAND --help` for options. After conversion, run `validate output.3tz`; this validator does not yet accept raster or terrain directories. Use `preview` with extracted archives to check appearance, refinement and picking before publishing.

Both CLI builds include validation and preview. A standard CLI can preview existing imagery and terrain outputs even though converting their source rasters requires native-geospatial. A command appearing in `--help` does not prove that its native capability is compiled in; use `doctor` for that check.

The [quick start](../README.md#quick-start) uses invented geometry. [Try each converter](DEMOS.md) adds self-contained point, vector, imagery and terrain examples. The [opt-in demo-data suite](../CONTRIBUTING.md#demo-data-acceptance-and-benchmarks) audits an already provisioned, hash-pinned public corpus; its external preparation tools are not distributed here. Conversion never downloads those assets implicitly.
