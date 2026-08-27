# tinyowl-tiles

Rust CLI that turns geospatial sources into **3D Tiles** packages (`.3tz`). The TinyOwl / echidna hub owns buckets, jobs, and Cesium serving. This crate only **transforms**.

**Oracle (v0):** Cesium [`3d-tiles-tools@0.5.4`](https://github.com/CesiumGS/3d-tiles-tools) (`createTilesetJson` + `convert`). Golden tests compare semantic `tileset.json` fields (asset version, URIs, geometricError) to `npx` when Node is present. Bounding volumes are Z-up AABBs; the oracle may emit a tighter OBB.

**Horizon:** [3D Tiles 2.0 vector content](https://cesium.com/blog/2026/06/29/help-shape-vector-data-support-in-3d-tiles/) — see [`docs/VECTOR.md`](docs/VECTOR.md). DEM/terrain later — [`docs/TERRAIN.md`](docs/TERRAIN.md). COG/PMTiles stay in `tinyowl-server` + GDAL. COLMAP is out of scope.

Hub worker integration (when v0 is golden): [`docs/HUB_SEAM.md`](docs/HUB_SEAM.md).

## Install

```bash
cargo install --path .
```

## Commands

```bash
# GLB/glTF → tileset.json (real boundingVolume.box, 3D Tiles 1.1)
tinyowl-tiles create-tileset-json -i model.glb -o tileset.json

# Place on the globe (lon lat [height_m]; optional heading pitch roll degrees)
tinyowl-tiles create-tileset-json -i model.glb -o tileset.json \
  --cartographic-position-degrees 151.2 -33.9 10 \
  --rotation-degrees 45 0 0

# Directory with tileset.json → .3tz (ZIP; unzippable by tinyowl-server ExtractZip)
tinyowl-tiles convert -i ./tileset-dir -o model.3tz

# One shot: GLB → .3tz  (what model-worker should exec)
tinyowl-tiles glb-to-3tz -i model.glb -o model.3tz \
  --cartographic-position-degrees 151.2 -33.9

# Stubs (exit 2) until spec / pipeline land
tinyowl-tiles vector -i features.geojson -o features.3tz
tinyowl-tiles terrain -i dem.tif -o terrain.3tz
```

`-f` / `--force` overwrites the output path.

## Layout

| Path | Role |
|------|------|
| `src/tileset.rs` | `create-tileset-json`, `glb-to-3tz` |
| `src/bbox.rs` | glTF POSITION → `boundingVolume.box` |
| `src/georef.rs` | WGS84 ENU→ECEF + HPR (TinyOwl / Cesium flags) |
| `src/pack.rs` | `convert` → `.3tz` |
| `src/vector.rs` | v1 stub + spec URLs |
| `src/terrain.rs` | later stub |
| `oracle/package.json` | pinned `3d-tiles-tools` |
| `tests/golden.rs` | semantic compare vs `npx` when available |

## Tests

```bash
cargo test
# optional oracle (needs Node):
cd oracle && npm install
```
