# tinyowl-tiles

Rust CLI that turns geospatial sources into **3D Tiles** packages (`.3tz`). The TinyOwl / echidna hub owns buckets, jobs, and Cesium serving. This crate only **transforms**.

**Oracle (v0):** Cesium [`3d-tiles-tools@0.5.4`](https://github.com/CesiumGS/3d-tiles-tools) (`createTilesetJson` + `convert`). CLI names and flags match the tools (`createTilesetJson`, `--cartographicPositionDegrees`, `--rotationDegrees`). Golden tests compare semantic `tileset.json` fields and Cesium `root.transform` to `npx` when Node is present. Bounding volumes are Z-up AABBs; the oracle may emit a tighter dito.ts OBB.

**Horizon:** [3D Tiles 2.0 vector content](https://cesium.com/blog/2026/06/29/help-shape-vector-data-support-in-3d-tiles/) — see [`docs/VECTOR.md`](docs/VECTOR.md). DEM/terrain later — [`docs/TERRAIN.md`](docs/TERRAIN.md). COG/PMTiles stay in `tinyowl-server` + GDAL. COLMAP is out of scope.

Hub worker integration: [`docs/HUB_SEAM.md`](docs/HUB_SEAM.md).

## Install

```bash
cargo install --path .
```

## Commands

```bash
# GLB/glTF → tileset.json (real boundingVolume.box, 3D Tiles 1.1)
tinyowl-tiles createTilesetJson -i model.glb -o tileset.json

# Place on the globe (lon lat [height_m]; optional heading pitch roll degrees)
tinyowl-tiles createTilesetJson -i model.glb -o tileset.json \
  --cartographicPositionDegrees 151.2 -33.9 10 \
  --rotationDegrees 45 0 0

# Directory with tileset.json → .3tz (ZIP; unzippable by tinyowl-server ExtractZip)
tinyowl-tiles convert -i ./tileset-dir -o model.3tz

# One shot wrap (no split): GLB → .3tz
tinyowl-tiles glb-to-3tz -i model.glb -o model.3tz \
  --cartographicPositionDegrees 151.2 -33.9

# Split into a REPLACE tree when over 20k triangles / ~200 KB
tinyowl-tiles mesh-to-3tz -i model.glb -o model.3tz \
  --cartographicPositionDegrees 151.2 -33.9

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
| `src/georef.rs` | Cesium ENU→ECEF + `--rotationDegrees` HPR |
| `src/pack.rs` | `convert` → stored `.3tz` + `@3dtilesIndex1@` |
| `src/tile.rs` | `mesh-to-3tz` wrap-or-split orchestrator |
| `src/mesh.rs` | glTF IR (mmap, skip image decode) |
| `src/split.rs` | k-d centroid split to 20k / 200 KB leaves |
| `src/texture.rs` | UV crop, resize, JPEG |
| `src/glb_write.rs` | per-tile GLB authoring |
| `src/vector.rs` | v1 stub + spec URLs |
| `src/terrain.rs` | later stub |
| `oracle/package.json` | pinned `3d-tiles-tools` |
| `tests/golden.rs` | semantic + transform compare vs `npx` |
| `tests/demo_glb.rs` | ignored parity on `mgal_detail.glb` |
| `tests/mesh_tile.rs` | wrap-or-split, 80k grid, texture crop |

## Tests

```bash
cargo test
# optional oracle pin:
cd oracle && npm install
```

`tests/demo_glb.rs` runs against `../tinyowl-demodata/glb/mgal_detail.glb` when that file is present (override with `TINYOWL_DEMO_GLB`).
