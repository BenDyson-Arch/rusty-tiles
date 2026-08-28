# tinyowl-tiles

Rust CLI that turns geospatial sources into **3D Tiles** packages (`.3tz`). The TinyOwl / echidna hub owns buckets, jobs, and Cesium serving. This crate only **transforms**.

**Oracle (v0):** Cesium [`3d-tiles-tools@0.5.4`](https://github.com/CesiumGS/3d-tiles-tools) (`createTilesetJson` + `convert`). CLI names and flags match the tools (`createTilesetJson`, `--cartographicPositionDegrees`, `--rotationDegrees`). Golden tests compare semantic `tileset.json` fields and Cesium `root.transform` to `npx` when Node is present. Bounding volumes are Z-up AABBs; the oracle may emit a tighter dito.ts OBB.

**Horizon:** [3D Tiles 2.0 vector content](https://cesium.com/blog/2026/06/29/help-shape-vector-data-support-in-3d-tiles/) — see [`docs/VECTOR.md`](docs/VECTOR.md). DEM/terrain later — [`docs/TERRAIN.md`](docs/TERRAIN.md). COG/PMTiles stay in `tinyowl-server` + GDAL. COLMAP is out of scope.

Hub worker integration: [`docs/HUB_SEAM.md`](docs/HUB_SEAM.md).

## Install

```bash
cargo install --path .
```

Building needs a C++ compiler (`g++` or `clang++`) for the `meshopt` crate (`cc`, not CMake).

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

# Split into a REPLACE HLOD tree when over 20k triangles
# Metashape geographic (lon° / height m / −lat°) and world EPSG:3857
# (easting / height / −northing) GLBs are baked to ENU metres automatically.
# Prefer an explicit --sourceCrs; auto-detect warns on stderr.
tinyowl-tiles mesh-to-3tz -i model.glb -o model.3tz --sourceCrs geographic

# Metashape Shift export: keep local metres; E/N/A pins root ECEF only
tinyowl-tiles mesh-to-3tz -i mgal_detail_offset.glb -o model.3tz \
  --sourceCrs epsg:3857 \
  --sourceOffsetFile offset.txt
# or: --sourceOffset 14812000 -1384000 100
# Diagnostic: float32 tiles (default is quantized meshopt)
tinyowl-tiles mesh-to-3tz -i mgal_detail_offset.glb -o model.3tz -f \
  --sourceCrs epsg:3857 --sourceOffsetFile offset.txt --noMeshopt

# Stubs (exit 2) until spec / pipeline land
tinyowl-tiles vector -i features.geojson -o features.3tz
tinyowl-tiles terrain -i dem.tif -o terrain.3tz
```

`-f` / `--force` overwrites the output path.

## Conventions (lossy + placement)

`mesh-to-3tz` is a **photogrammetry** pipe, not a full glTF round-trip:

- Kept: `POSITION`, `NORMAL`, `TEXCOORD_0`, base-color JPEG.
- Dropped silently: vertex colors, PBR beyond base color, skins, morphs, animations, instancing, extras.
- Default tile GLBs use `KHR_mesh_quantization` + `EXT_meshopt_compression` as **required** extensions (no empty fallback buffer). Consumers need a meshopt decoder (CesiumJS ships one). Use `--noMeshopt` for float32 GLBs.
- Local-metre meshes (no CRS bake, no `--cartographicPositionDegrees`) get **no** `root.transform` — content sits in a local ENU-like frame at the origin. Placement is the hub/viewer’s job ([`docs/HUB_SEAM.md`](docs/HUB_SEAM.md)). Geographic / offset bakes set `root.transform` to ENU→ECEF.

## Layout

| Path | Role |
|------|------|
| `src/tileset.rs` | `create-tileset-json`, `glb-to-3tz` |
| `src/bbox.rs` | glTF POSITION → `boundingVolume.box` |
| `src/georef.rs` | Cesium ENU→ECEF, `--rotationDegrees` HPR, Metashape geographic→ENU |
| `src/pack.rs` | `convert` → stored `.3tz` + `@3dtilesIndex1@` |
| `src/tile.rs` | `mesh-to-3tz` wrap-or-split orchestrator |
| `src/hlod.rs` | parent simplify + sampled Hausdorff GE |
| `src/compress.rs` | quantized meshopt GLB writer |
| `src/mesh.rs` | glTF IR (mmap, skip image decode) |
| `src/split.rs` | k-d centroid split to 20k-triangle leaves |
| `src/texture.rs` | UV crop, resize, JPEG; parent LOD atlas bake |
| `src/glb_write.rs` | per-tile GLB authoring |
| `src/vector.rs` | v1 stub + spec URLs |
| `src/terrain.rs` | later stub |
| `oracle/package.json` | pinned `3d-tiles-tools` |
| `tests/golden.rs` | semantic + transform compare vs `npx` |
| `tests/demo_glb.rs` | ignored parity on `mgal_detail.glb` |
| `tests/mesh_tile.rs` | wrap-or-split, HLOD GE, meshopt size, texture crop |

## Tests

```bash
cargo test
# optional oracle pin:
cd oracle && npm install
```

`tests/demo_glb.rs` runs wrap parity against `../tinyowl-demodata/glb/mgal_detail.glb` when that file is present (override with `TINYOWL_DEMO_GLB`). `tests/demo_mesh.rs` is ignored: `cargo test --release --test demo_mesh -- --ignored` runs `mesh-to-3tz` on the same file.
