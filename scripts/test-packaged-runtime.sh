#!/bin/sh
# Focused installed-release acceptance in the offline, Python-free runtime image.
set -eu

fixtures=${1:-/fixtures}
for interpreter in python python3; do
    if command -v "$interpreter" >/dev/null 2>&1; then
        printf 'packaged runtime must not contain %s\n' "$interpreter" >&2
        exit 1
    fi
done
command -v rusty-tiles >/dev/null 2>&1
[ -s "$fixtures/vector.geojson" ]
[ -s "$fixtures/d1-rgb.tif" ]

# The CI container also disables networking; this prevents PROJ grid downloads.
PROJ_NETWORK=OFF
export PROJ_NETWORK
work=$(mktemp -d "${TMPDIR:-/tmp}/rusty-packaged-runtime.XXXXXX")
trap 'rm -rf "$work"' 0
trap 'exit 1' HUP INT TERM

rusty-tiles doctor --command vector --json
rusty-tiles doctor --command raster --json
rusty-tiles vector -i "$fixtures/vector.geojson" -o "$work/vector.3tz" \
    --max-features 2 --reproducible --json
[ -s "$work/vector.3tz" ]
rusty-tiles validate "$work/vector.3tz" --json

rusty-tiles raster -i "$fixtures/d1-rgb.tif" -o "$work/imagery" \
    --min-zoom 0 --max-zoom 0 --json
[ -s "$work/imagery/tilejson.json" ]
[ -s "$work/imagery/report.json" ]
[ -s "$work/imagery/source.cog.tif" ]
[ -s "$work/imagery/tiles/0/0/0.png" ]
# validate accepts .3tz archives; raster directory readiness is checked above.
printf 'packaged runtime acceptance passed (vector archive and native raster)\n'
