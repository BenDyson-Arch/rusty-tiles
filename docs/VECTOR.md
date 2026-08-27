# Vector tiling (v1)

Not implemented until [3d-tiles#838](https://github.com/CesiumGS/3d-tiles/pull/838) is stable enough to pin.

CLI already exists:

```bash
tinyowl-tiles vector -i features.geojson -o features.3tz
```

Exit code 2 with a pointer at the spec.

Planned encode (do not invent a private format):

- Tileset extension `3DTILES_content_gltf_vector` (1.1 now; 2.0 successor when published)
- glTF `KHR_mesh_primitive_restart`, `EXT_mesh_polygon`
- Feature IDs/properties: `EXT_mesh_features`, `EXT_structural_metadata`
- Inputs: GeoJSON first, then project GPKG layers, shapefile later
- Implicit quadtree (3D Tiles 1.1), each leaf a glTF vector tile, wrap `.3tz`
- Not MVT / tippecanoe / Web Mercator

See `src/vector.rs`.
