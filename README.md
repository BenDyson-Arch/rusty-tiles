# rusty-tiles

An MIT-licensed, standalone tool for textured GLB/glTF → 3D Tiles 1.1 → `.3tz`, raster imagery pyramids, DEM terrain tiles and experimental glTF vector tiles. Bring your own data; no survey datasets, access tokens or hosted services are bundled.

## Build and run

Rust and a C++ compiler are required. If `pkg-config` finds `libturbojpeg` at build time, JPEG encoding uses its SIMD implementation. Otherwise the portable Rust encoder is used. Native builds require the corresponding shared library at runtime. `RUSTY_TILES_DISABLE_NATIVE_JPEG=1` forces the portable build; cross-compilation uses it automatically.

```sh
cargo build --release

target/release/rusty-tiles mesh-to-3tz \
  -i /path/to/your/model.glb \
  -o model.3tz \
  --sourceCrs epsg:3857 \
  --sourceOffsetFile /path/to/your/offset.txt
```

The CLI defaults to `--textureFormat jpeg`: quality 95 with full chroma resolution, with PNG for materials that use transparency. Fidelity here means perceptual detail, not identical encoded bytes.

| Format | Purpose | Additional requirement |
| --- | --- | --- |
| `jpeg` | Fast generation and image decoding; full-chroma quality 95 | libjpeg-turbo recommended |
| `webp` | Smaller downloads; quality 95, lossless alpha | None |
| `uastc` | GPU-compressed KTX2 with mipmaps; less texture memory, slower generation | Native Basis Universal encoder |
| `lossless` | Exact decoded leaf RGBA texels in PNG | None |

The library's `MeshTo3tzOptions::default()` retains portable `Lossless` output. Set `texture_format` explicitly for a delivery codec.

For GPU output, `./scripts/build-basisu.sh` downloads and builds a checksum-pinned [Basis Universal 1.60](https://github.com/BinomialLLC/basis_universal/tree/v1_60) release (CMake, curl and tar required). Pass `--textureFormat uastc --basisu target/tools/basisu`. Conversion never downloads an encoder implicitly.

`-f` / `--force` replaces an existing output only after successful conversion. Concurrent jobs get independent work directories. Temporary data is created beside the output and removed on ordinary success/error, so allow sufficient scratch disk space. `RAYON_NUM_THREADS` controls conversion parallelism; encoder processes each use one thread.

## Pipeline and fidelity

- **Partitioning:** balanced spatial median splits assign each original triangle to one leaf. Triangles are not clipped, duplicated or simplified in leaves. A leaf must also pack its source charts at their original pixel resolution. Ordinary atlases are at most `--tileSize` (default 2048); an individual triangle requiring a larger chart gets an exceptional larger atlas instead of downsampling. Tree fan-out is at most eight, with compaction balanced by descendant leaf count to avoid unnecessary replacement levels.
- **Geometry:** lossless `EXT_meshopt_compression` over float32 attributes, plus vertex-cache and fetch ordering for coarse meshes. There is no leaf position, normal or UV quantization. `--noMeshopt` disables the byte codec. Bounds enclose complete triangles, including ones crossing a partition boundary.
- **Textures:** chart regions and gutters are copied on the source pixel grid. Source images are decoded in bounded batches; chart bytes and intermediate textures are spilled to disk. Delivery encoding is separate from the smaller, losslessly stored working images used by parents. UASTC starts at effort 0 and retries at effort 1 when needed, checking source-level RGB PSNR ≥42 dB and, for non-opaque materials, alpha PSNR ≥50 dB. A texture failing that gate stays PNG. This is a per-atlas quality check, not a guarantee about every rendered pixel. `lossless` retains exact decoded RGBA texels.
- **Coarse levels:** a hierarchy balanced by descendant leaf count groups up to eight children per parent. Parent meshes are welded and simplified independently of source UV seams, keeping boundary edges fixed and material groups separate. New charts reject overlapping interiors. Colour is re-baked from compatible-facing child surfaces using linear-light, premultiplied-alpha filtering. Upper levels receive larger atlases to delay costly refinement. Parent construction never decodes delivery JPEG/WebP/UASTC as its source. Simplification may exceed its triangle target to preserve boundaries; chart-heavy parents may grow their atlas up to 4096. If simplification produces a surface that cannot be safely reprojected, that parent retains the textured child surfaces and reports the fallback. Other texture failures propagate instead of silently producing an untextured tile.
- **Appearance:** base-colour factors, metallic/roughness factors, emissive factors, alpha settings, double-sidedness and authored normals are retained for supported sources. Missing normals remain missing, preserving glTF flat shading. Node reflections reverse triangle winding; nonuniform scales use inverse-transpose normal transforms. Only the active scene is loaded.
- **Packaging:** stored ZIP/ZIP64, a complete sorted `@3dtilesIndex1@`, single-pass file reads/CRC calculation, and atomic output publication. Archives can exceed the old 4 GiB / 65,535-member limits. `validate_3tz` cross-checks the index against the ZIP directory.

Leaf source resolution is retained; the compatibility option `--maxTexelDensity` must be `0`. Refinement error includes accumulated sampled surface distance and a finite-face texture footprint. These estimates and the viewer’s screen-space-error setting control refinement; they are not a mathematical guarantee of perceptual equality from every camera. Source-resolution textures can produce larger archives than the former density-capped pipeline. Compare both quality and speed when benchmarking.

## Supported sources and placement

Spatial conversion currently supports static triangle meshes with `POSITION`, optional `NORMAL`, optional `TEXCOORD_0`, and base-colour JPEG/PNG/WebP images. UVs must lie in `[0,1]`. Source wrapping is respected in copied gutters; atlas sampling is trilinear. Additional vertex attributes, skinning, morph targets, animation, source glTF extensions, additional PBR texture channels and incompatible samplers are rejected explicitly instead of silently discarded. This is not a general glTF round-trip converter.

For these richer assets, `glb-to-3tz` packages the original model unchanged. Small supported inputs under both the triangle and byte thresholds also use this wrap path.

- `--sourceCrs geographic`: Metashape longitude / height / −latitude positions are converted to local ENU metres.
- `--sourceCrs epsg:3857`: world easting / height / −northing, or local Metashape Shift export with `--sourceOffsetFile` / `--sourceOffset E N A`. The offset is restored in float64 before projection into the local ENU frame; shifted Mercator metres are not ground metres. The root places that frame in Cesium ECEF coordinates.
- `--cartographicPositionDegrees lon lat [height]`: explicit globe placement; optional `--rotationDegrees heading pitch roll`.
- Local metre data receives no root transform unless placement is supplied. Preserve the original file; float32 degree coordinates have already lost precision that no tiler can recover.

## Other commands

```sh
# Wrap an original model, preserving its content
target/release/rusty-tiles glb-to-3tz -i model.glb -o model.3tz

# Generate only a manifest
target/release/rusty-tiles createTilesetJson -i model.glb -o tileset.json

# Package an existing tileset directory
target/release/rusty-tiles convert -i ./tileset-directory -o model.3tz
```

Cesium consumes `tileset.json` and tile resources through a server capable of serving archive members, or from an extracted archive; the converter does not add direct `.3tz` support to Cesium.

## Verification

```sh
cargo test --offline
python3 -m unittest discover -s tests -p 'test_*.py'
# Optional external wrapper comparisons (requires npx and network/cache):
cargo test --test golden -- --ignored
```

Fidelity tests decode the actual meshopt streams and compare source triangle membership, float attributes, remapped RGBA texels and materials. They also exercise active-scene selection, reflected/nonuniform transforms, invalid-input publication safety and ZIP64 member counts. Additional tests cover hierarchy depth, preserved boundary edges, chart overlap, face selection, alpha handling and full-chroma JPEG output. The existing golden suite compares wrapper conventions with the pinned `3d-tiles-tools@0.5.4` oracle when explicitly invoked with `--ignored`; it reports skips if the oracle cannot run. Default tests do not download external tools or discover neighboring datasets.

To audit every leaf triangle of your shifted Web Mercator model after reprojection (omit `RUSTY_TILES_FIDELITY_OFFSET` for an unprojected local-coordinate source):

```sh
RUSTY_TILES_FIDELITY_SOURCE=/path/to/your/model.glb \
RUSTY_TILES_FIDELITY_ARCHIVE=model.3tz \
RUSTY_TILES_FIDELITY_OFFSET=/path/to/your/offset.txt \
cargo test --release --test fidelity full_model_leaf_triangle_audit -- --ignored --nocapture
```

This compares the complete multiset of position-and-winding signatures and checks that a source without normals does not acquire fabricated normals. The optional offset applies the same source reprojection before comparing leaf geometry. Independent PROJ reference coordinates cover the projection regression; this multiset check alone is not an independent CRS oracle or a coarse-level error bound.

### Validation and known limits

The 0.1.0 closeout passed 67 Rust tests and 12 Python tests from both the working tree and the extracted source package. Seven Rust tests are opt-in for external source data, full-model audits or the external oracle. Native and portable JPEG checks pass. Browser checks cover the invented combined scene and annotation-only example, including layer toggles and property picking.

The full-scale mesh validation used a privately supplied 10-million-triangle model: the complete leaf position/winding multiset passed, and missing normals remained missing. The data owner accepted interior views, near/far transitions, texture detail and annotation alignment on 2026-09-11. This is visual acceptance of that tested scene, not a guarantee for arbitrary inputs. Private source data and its location are not included.

A matched conversion benchmark took 101.26 s before and 101.20 s after the rewrite. Separate full runs measured peak memory falling from 4.83 GiB to 2.26 GiB. Output grew from 1.44 GB to 3.79 GB because leaf textures retained source resolution. These measurements do not claim faster conversion or predict other machines. The placement fix also passed an independent PROJ coordinate regression; its test coordinates are invented.

One intermediate PNG checksum failure did not recur in three subsequent full conversions; its cause remains unconfirmed. Set `RUSTY_TILES_DEBUG_IMAGES` to a diagnostic directory to retain a parent image if delivery decoding fails. Failed conversions do not publish an incomplete archive. The external wrapper oracle was unavailable during the full-model validation; this is not an oracle-parity claim. Clippy warnings are tracked as non-blocking style cleanup.

## Raster, terrain and glTF vector lab commands

These commands remain standalone lab tools. Python helpers are embedded in the Rust executable; terrain/vector need Python 3, NumPy and GDAL with GEOS. Raster needs the GDAL CLI (`gdal raster tile`, tested on 3.13.3). Existing output paths are rejected. Failed jobs clean their staging directory without publishing a partial result.

```sh
# Preserve the source pixels in a COG and generate PNG XYZ display tiles.
target/release/rusty-tiles raster \
  -i /path/to/your/orthophoto.tif \
  -o output/imagery --minZoom 10 --maxZoom 19

# Source heights must be metres; supply the conversion to ellipsoidal height.
# Zero below is only an explicit unchanged-height lab experiment, not a datum claim.
target/release/rusty-tiles terrain \
  -i /path/to/your/dem.tif \
  -o output/terrain --maxZoom 20 --heightOffset 0 --fillHeight 0

# GeoPackage -> glTF vector content, with layer identity and typed scalar properties.
target/release/rusty-tiles vector \
  -i /path/to/your/mapping.gpkg -o output/mapping.3tz --layer roads \
  --maxFeatures 64 --maxVertices 65536 --maxBytes 4194304

python3 -m unittest discover -s tests -p 'test_*.py'
```

**Raster:** creates `source.cog.tif`, `tilejson.json` and `tiles/{z}/{x}/{y}.png`. The source COG retains original bands and georeferencing; PNG tiles are a reprojected display derivative. Numeric rasters need an explicit styling/scaling step for display; the tool does not invent a stretch. No PMTiles converter is bundled yet. A complete source/COG pixel-and-channel comparison passed during validation.

**Terrain:** creates `layer.json`, TMS `{z}/{x}/{y}.terrain` files and `conversion.json`. The regular grid defaults to 65 samples per edge; shared boundaries and a common height quantization interval avoid mismatched edge heights. GDAL samples bounded raster windows. Availability starts at both hemisphere roots; Cesium can sample heights with `sampleTerrainMostDetailed`. `--fillHeight` explicitly defines missing/outside data. `--heightOffset` is a constant, not a spatial geoid transformation. Source datum and metre units must be established separately. Grid resolution and quantization are reported; there is no certified maximum surface-error bound or adaptive mesh simplifier yet. The files are uncompressed; serving HTTP gzip is optional for this preview. Terrain normals are not encoded in this prototype; the preview uses elevation colouring.

**Vectors:** targets `3DTILES_content_gltf_vector` on 3D Tiles 1.1, using `EXT_mesh_polygon`, `EXT_mesh_features` and `EXT_structural_metadata`. Polygon schema checked against Khronos glTF PR 2570 revision `c1a035499b70aeb5d8281470101423e5e285dfe3`; renderer tested with Cesium 1.143.0. This is a draft-format prototype, not a claim that 3D Tiles 2.0 is finalized. OGR layers (including GeoPackage) are streamed into a disk-backed spatial hierarchy with full-detail leaves and simplified parent content. Actual encoded feature, vertex and byte budgets are enforced. Oversized lines and multi-geometries fragment; oversized polygons retain their triangulated surface and encode source outlines separately to avoid internal seams. Parents that cannot retain all identities within budget route without content. Buffered clipping, implicit tiling and batched line restart encoding are not implemented. See [vector LOD](docs/VECTOR.md) for simplification, fallback and memory limits. Points, lines, polygons and their Multi forms are supported. Polygon triangulation uses a best-fit plane but retains original 3D vertex positions, including nonplanar and vertical polygons. Holes are retained. Heterogeneous scalar schemas are supported; missing string/numeric values use explicit metadata noData values. Nullable booleans and complex/mixed property values still require an explicit schema. Unsupported inputs fail explicitly. `_source_id` preserves the JSON-encoded source ID, or the OGR FID when absent; `_source_layer` disambiguates layer identity. The four-feature fixture rendered and picked all four property records in Cesium, including its polygon with a hole and vertical polygon.

Terrain validation found zero encoded edge-height mismatch across 7,251 neighboring tile pairs. At 23 sampled source pixels, rendered heights differed by a mean 0.0232 m and maximum 0.0873 m. This is a sampled check, not a dataset-wide bound. That private test source had an unverified vertical datum; the terrain command remains a prototype requiring users to establish their own height datum.

`--repair` permits GDAL/GEOS MakeValid for invalid polygon outlines and records changes in `geometry-reports.jsonl` with a bounded sample in `conversion.json`. With `--ambiguousOutlines`, crossings that disagree by more than 2 cm in 3D retain their original closed 3D outlines and feature properties. Without that flag they fail explicitly. Real-data validation retained every input feature and checked all retained scalar properties; no real-data fixtures are distributed.

## Preview your own scene

The preview uses Cesium's IIFE build. It reads only the output directories you explicitly select, defaults to loopback, and needs no ion token or external basemap. Do not point it at a source-data parent directory: selected directories and their file contents are served to anyone able to reach the chosen interface. Directory listings and paths escaping those directories are rejected.

Install the tested viewer runtime (Node/npm required for this step):

```sh
npm install --prefix target/preview-runtime --no-save --package-lock=false cesium@1.143.0
```

For a small smoke test, convert the **invented** point, line, polygon-with-hole and vertical-polygon fixture:

```sh
mkdir -p output
./target/release/rusty-tiles vector \
  -i tests/fixtures/vector.geojson -o output/example.3tz --maxFeatures 2
python3 -m zipfile -e output/example.3tz output/example
python3 scripts/preview.py \
  --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --annotations output/example
```

Open [the local preview](http://127.0.0.1:9227/). The fixture is a software test, not a surveyed location.

For your own full scene, first generate `model.3tz`, `output/annotations.3tz` and `output/imagery` with the commands above. Supply the actual source CRS/offset for the model; the shown Mercator settings are an example, not automatic CRS detection. Model and annotation heights must use compatible references. Then extract your generated archives and start the same preview:

```sh
python3 -m zipfile -e model.3tz output/mesh
python3 -m zipfile -e output/annotations.3tz output/annotations
python3 scripts/preview.py \
  --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --mesh output/mesh \
  --annotations output/annotations \
  --imagery output/imagery
```

Every data flag is optional; select at least one. Add `--terrain output/terrain` only for a DEM covering the same area and with the correct height reference. Layer toggles, extent buttons, annotation picking and an FPS display are included. Covered annotations retain their actual positions; hide the mesh to inspect them.

To serve on your own network, explicitly pass `--host YOUR_INTERFACE_IP --port 9227`. Only use data you intend to share there. Nothing is uploaded or automatically published. Restarting the same command reuses your output directories; no files under `/tmp` are required. Generated files under `output/` and `target/` are ignored by git.
# rusty-tiles

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for bug reports, local checks, the `develop` → `main` release workflow, review requirements and data-sharing rules. Contributions normally target `develop`; `main` is protected for releases.
