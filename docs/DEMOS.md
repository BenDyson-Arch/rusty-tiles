# Try each converter

These examples use invented data committed in this repository. Clone the source checkout and run the commands from its root with an [installed CLI](INSTALL.md). Conversion downloads nothing and refuses existing outputs; use a fresh output directory for another run.

## Standard package

These commands use the 0.4 standard CLI, with no GDAL/PROJ/GEOS installation. The native CLI also supports them. The Python wheel exposes conversion functions rather than these shell commands; use the [Python example](../bindings/python/README.md#install-and-convert) for that package.

The [mesh quick start](../README.md#quick-start) covers installation, conversion, archive validation and preview of `tests/fixtures/example.gltf`. The same tiny model can be packaged without generating LOD:

```sh
mkdir -p output
rusty-tiles doctor --command mesh-to-3tz --command glb-to-3tz --command point-cloud --command vector
rusty-tiles glb-to-3tz -i tests/fixtures/example.gltf -o output/packaged.3tz \
  --cartographic-position-degrees 153.02 -27.47 0
rusty-tiles validate output/packaged.3tz
```

Generate sixteen invented points in WGS84 UTM zone 56S, near Brisbane, with Python 3's standard library, then convert them with the standard CLI. Python creates the example input; the converter needs no Python runtime.

```sh
python3 tests/fixtures/local_points.py output/cloud.las --utm
rusty-tiles point-cloud -i output/cloud.las -o output/cloud.3tz \
  --source-crs EPSG:32756 --height-offset 0 --max-points 4
rusty-tiles validate output/cloud.3tz
```

The LAS header carries no CRS, so the command supplies it explicitly. Zero height offset is appropriate here because these invented Z values are ellipsoidal metres. For a local XYZ encoding example, omit `--utm` when generating a fresh file and use `--source-crs local` without a height offset.

The vector fixture contains a point, a line, a polygon with a hole and a vertical polygon, with invented longitude/latitude and ellipsoidal metre heights:

```sh
rusty-tiles vector -i tests/fixtures/vector.geojson -o output/vector.3tz --max-features 2
rusty-tiles validate output/vector.3tz
```

After [installing Cesium 1.146](../README.md#4-preview), extract an archive and select its layer:

```sh
unzip output/cloud.3tz -d output/cloud
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --point-cloud output/cloud
```

Use `--mesh` for the packaged model and `--annotations` for vectors. Open the printed URL and use the corresponding extent button. A local cloud without placement will not align with georeferenced layers.

## Native imagery and terrain

This section requires the [native-geospatial CLI](INSTALL.md#native-geospatial-cli); it cannot run with the standard CLI or Python wheel. Use a development Python environment containing GDAL 3.12+, NumPy, laspy and pyproj to generate the invented inputs. These fixture dependencies do not enable native conversion in a standard build; native conversion itself needs no Python or GDAL executable. The fixture script creates and converts all five preview layers, including a 32 × 32 RGB GeoTIFF and a 32 × 32 elevation GeoTIFF with a noData hole:

```sh
rusty-tiles doctor --command raster --command terrain
RUSTY_TILES_BIN="$(command -v rusty-tiles)" python3 tests/fixtures/preview_layers.py output/native-demo
rusty-tiles validate output/native-demo/mesh.3tz
rusty-tiles validate output/native-demo/cloud.3tz
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --mesh output/native-demo/mesh --point-cloud output/native-demo/cloud \
  --annotations output/native-demo/annotations --imagery output/native-demo/imagery \
  --terrain output/native-demo/terrain
```

The imagery recipe uses `raster --maxZoom 10`; its entrypoint is `imagery/tilejson.json`. The terrain recipe uses `terrain --cells-per-leaf 16 --height-offset 10.25 --fill-height -999.125`; its entrypoint is `terrain/tileset.json`. The elevations and offsets are invented test values. `validate` currently accepts `.3tz` archives, not imagery or terrain directories; the browser checks their decoded output. See [terrain height semantics](TERRAIN.md) before choosing settings for real elevations.

## Release and public-data checks

The [release acceptance script](../scripts/release_acceptance.sh) exercises the README route. Set `ACCEPTANCE_REQUIRE_BROWSER=1` to render and pick the actual README pyramid before and after a cache-disabled reload, then require all five layers and the four detailed browser probes. The manual CI release-acceptance job installs pinned Cesium and Playwright plus Chromium, enables this gate and retains its logs. Local standard-build checks can omit this setting; their printed skips are not evidence of browser acceptance.

The [public corpus manifest](../bench/demodata_manifest.json) records source URLs, licences, hashes and per-converter recipes for openly licensed real data. The [benchmark suite](../CONTRIBUTING.md#demo-data-acceptance-and-benchmarks) runs against an already provisioned corpus. Some derived files require external preparation scripts that are not distributed in this repository. Those benchmarks are separate from the self-contained first-use examples and release acceptance above.
