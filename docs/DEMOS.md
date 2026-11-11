# Try each converter

These examples use invented data committed in this repository. Clone a [source checkout](INSTALL.md#build-from-source) and run the commands from its root with an [installed CLI](INSTALL.md). Conversion downloads nothing and refuses existing outputs; use a fresh output directory for another run.

## Standard package

These commands run with the standard CLI and need no GDAL, PROJ or GEOS. The native CLI runs them too. The Python package exposes functions instead of shell commands; see the [Python example](../bindings/python/README.md#example).

The [mesh quick start](../README.md#quick-start) converts, validates and previews `tests/fixtures/example.glb`. The same model can be packaged without generating level of detail:

```sh
mkdir -p output
rusty-tiles doctor --command mesh --command glb-to-3tz --command point-cloud --command vector
rusty-tiles glb-to-3tz -i tests/fixtures/example.glb -o output/packaged.3tz \
  --anchor 153.02 -27.47 0
rusty-tiles validate output/packaged.3tz
```

Generate sixteen invented points in WGS84 UTM zone 56S, near Brisbane, with Python 3's standard library, then convert them. Python only creates the input; the converter does not need it.

```sh
python3 tests/fixtures/local_points.py output/cloud.las --utm
rusty-tiles point-cloud -i output/cloud.las -o output/cloud.3tz \
  --source-crs EPSG:32756 --height-offset 0 --max-points 4
rusty-tiles validate output/cloud.3tz
```

The LAS header carries no CRS, so the command supplies one. A zero height offset is correct because these Z values are already ellipsoidal metres. For local XYZ instead, omit `--utm` when generating a fresh file and pass `--source-crs local` with no height offset.

The vector fixture contains a point, a line, a polygon with a hole and a vertical polygon, with invented longitude/latitude and ellipsoidal heights in metres:

```sh
rusty-tiles vector -i tests/fixtures/vector.geojson -o output/vector.3tz --max-features 2
rusty-tiles validate output/vector.3tz
```

To preview, install Cesium once (needs Node.js with npm), extract an archive and select its layer:

```sh
npm install --prefix target/preview-runtime --no-save --package-lock=false cesium@1.146.0
unzip output/cloud.3tz -d output/cloud
rusty-tiles preview --cesium target/preview-runtime/node_modules/cesium/Build/Cesium \
  --point-cloud output/cloud
```

Use `--mesh` for the packaged model and `--annotations` for vectors. Open the printed URL and use the matching extent button. A local cloud without placement does not align with georeferenced layers.

## Native imagery and terrain

This section needs the [native-geospatial CLI](INSTALL.md#native-geospatial-cli). Generating the invented inputs also needs a Python environment with GDAL 3.12+, NumPy, laspy and pyproj; these do not enable native conversion in a standard build, and the converter itself needs no Python or GDAL executable. The fixture script creates and converts all five preview layers, including a 32 × 32 RGB GeoTIFF and a 32 × 32 elevation GeoTIFF with a NoData hole:

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

The imagery recipe uses `raster --maxZoom 10`; its entrypoint is `imagery/tilejson.json`. The terrain recipe uses `terrain --cells-per-leaf 16 --height-offset 10.25`; its entrypoint is `terrain/tileset.json`. The elevations and offsets are invented. `validate` accepts `.3tz` archives, not imagery or terrain directories, so check those in the browser. Read the [terrain height rules](TERRAIN.md#source-and-height-meaning) before converting real elevations.

## Release and public-data checks

These checks are for contributors. The [release acceptance script](../scripts/release_acceptance.sh) runs the README route. With `ACCEPTANCE_REQUIRE_BROWSER=1` it also renders and picks the README pyramid before and after a cache-disabled reload, and requires all five layers and the four browser probes. Without that setting, missing browser tools are reported as skips, which do not count as browser acceptance.

The [public corpus manifest](../bench/demodata_manifest.json) records source URLs, licences, hashes and per-converter recipes for openly licensed real data. The [benchmark suite](../CONTRIBUTING.md#demo-data-acceptance-and-benchmarks) runs against an already provisioned corpus; some derived files need preparation scripts that are not in this repository.
