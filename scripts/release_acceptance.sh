#!/usr/bin/env bash
# Release acceptance: run the documented route end to end with the release CLI.
#
# usage: scripts/release_acceptance.sh
#
# Required steps (any failure exits non-zero):
#   1. build    cargo build --release --locked --features native-geospatial
#               (skipped when RUSTY_TILES_BIN names an existing binary)
#   2. doctor   README mesh/validate/preview commands report ready
#   3. convert  mesh-to-3tz -i tests/fixtures/example.gltf (README example)
#   4. validate validate --json on the archive
#   5. preview  preview --json starts on a free port, serves config.json and
#               the extracted tileset manifest (curl), then stops on SIGTERM
#               (strict browser mode also renders/picks this README pyramid
#               before and after a cache-disabled reload)
# Optional browser steps, run only when everything they need is present:
#   6. layers   tests/fixtures/preview_layers.py writes all five preview layers
#   7. probes   preview_layers.cjs, point_cloud.cjs, terrain.cjs and
#               vector_compat.cjs (each repeats its checks
#               after a cache-disabled reload)
# Skipped steps are listed at the end; they never fail the run.
#
# Environment:
#   RUSTY_TILES_BIN  use this default or native binary instead of building
#   CESIUM_DIR       Cesium 1.146 Build/Cesium directory (default
#                    target/preview-runtime/node_modules/cesium/Build/Cesium)
#   NODE_PATH        directory exposing the playwright module to node
#   CHROMIUM         Chromium executable for the probes (default /usr/bin/chromium)
#   PYTHON           Python with GDAL, NumPy, laspy and pyproj (default python3)
#   ACCEPTANCE_WORK  scratch directory (default: a new temporary directory)
#   ACCEPTANCE_REQUIRE_BROWSER  1 requires all five layers and browser probes;
#                              0 (default) permits missing prerequisites locally
set -uo pipefail

REPO=$(cd "$(dirname "$0")/.." && pwd)
WORK=${ACCEPTANCE_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/rusty-tiles-acceptance.XXXXXX")}
mkdir -p "$WORK"
PYTHON=${PYTHON:-python3}
CESIUM_DIR=${CESIUM_DIR:-$REPO/target/preview-runtime/node_modules/cesium/Build/Cesium}
export CHROMIUM=${CHROMIUM:-/usr/bin/chromium}
PASSED=() SKIPPED=()
SERVER=
REQUIRE_BROWSER=${ACCEPTANCE_REQUIRE_BROWSER:-0}

fail() {
  echo "FAIL: $1" >&2
  [[ -n ${2:-} && -f $2 ]] && tail -20 "$2" >&2
  exit 1
}
pass() { PASSED+=("$1"); echo "ok: $1"; }
skip() { SKIPPED+=("$1 ($2)"); echo "skipped: $1 ($2)"; }
stop_server() {
  if [[ -n $SERVER ]]; then
    kill "$SERVER" 2>/dev/null
    wait "$SERVER" 2>/dev/null
    SERVER=
  fi
}
trap stop_server EXIT

case $REQUIRE_BROWSER in
  0|1) ;;
  *) fail "ACCEPTANCE_REQUIRE_BROWSER must be 0 or 1" ;;
esac

browser_prerequisites() {
  missing=()
  command -v node >/dev/null || missing+=("node")
  [[ -f $CESIUM_DIR/Cesium.js ]] || missing+=("Cesium runtime at CESIUM_DIR=$CESIUM_DIR")
  node -e "require('playwright')" >/dev/null 2>&1 || missing+=("playwright on NODE_PATH")
  [[ -x $CHROMIUM ]] || missing+=("Chromium at CHROMIUM=$CHROMIUM")
  "$PYTHON" -c "import osgeo, numpy, laspy, pyproj" >/dev/null 2>&1 \
    || missing+=("$PYTHON with GDAL, NumPy, laspy and pyproj")
}

missing_browser_reason() {
  reason="missing: $(printf '%s; ' "${missing[@]}")"
  reason=${reason%; }
}

browser_converter_prerequisites() {
  env PATH="" "$BIN" doctor --command vector --command point-cloud --command raster --command terrain --json \
    >"$WORK/browser-readiness.json" 2>"$WORK/browser-readiness.err" \
    || missing+=("selected binary's vector/point-cloud/raster/terrain capabilities")
}

# Fail before a potentially expensive build when a release gate cannot run.
if [[ $REQUIRE_BROWSER == 1 ]]; then
  browser_prerequisites
  if [[ ${#missing[@]} -gt 0 ]]; then
    missing_browser_reason
    fail "required browser acceptance prerequisites: $reason"
  fi
fi

# json_field FILE EXPRESSION: evaluate a Python expression on the JSON `d`.
json_field() {
  "$PYTHON" -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$1" "$2"
}

# start_preview LOG ARGS...: start preview --json on a free port; sets URL.
start_preview() {
  local log=$1
  shift
  env PATH="" "$BIN" preview --json --port 0 "$@" >"$log" 2>"$log.err" &
  SERVER=$!
  for _ in $(seq 100); do
    [[ -s $log ]] && break
    kill -0 "$SERVER" 2>/dev/null || fail "preview exited at startup" "$log.err"
    sleep 0.1
  done
  [[ $(json_field "$log" 'd["ok"]') == True ]] || fail "preview did not report ok" "$log"
  URL=$(json_field "$log" 'd["url"]')
}

# 1. build
if [[ -n ${RUSTY_TILES_BIN:-} ]]; then
  [[ -x $RUSTY_TILES_BIN ]] || fail "RUSTY_TILES_BIN is not an executable: $RUSTY_TILES_BIN"
  BIN=$RUSTY_TILES_BIN
  skip build "using RUSTY_TILES_BIN=$BIN"
else
  LIBSQLITE3_SYS_USE_PKG_CONFIG=1 cargo build --manifest-path "$REPO/Cargo.toml" --release --locked \
    --features native-geospatial >"$WORK/build.log" 2>&1 || fail build "$WORK/build.log"
  BIN=$REPO/target/release/rusty-tiles
  pass build
fi

if [[ $REQUIRE_BROWSER == 1 ]]; then
  browser_converter_prerequisites
  if [[ ${#missing[@]} -gt 0 ]]; then
    missing_browser_reason
    fail "required browser acceptance prerequisites: $reason" "$WORK/browser-readiness.json"
  fi
fi

# 2. doctor
env PATH="" "$BIN" doctor --command mesh-to-3tz --command validate --command preview --json \
  >"$WORK/doctor.json" 2>"$WORK/doctor.err" \
  || fail "doctor for README commands (exit $?)" "$WORK/doctor.json"
[[ $(json_field "$WORK/doctor.json" 'd["ok"]') == True ]] || fail "doctor not ok" "$WORK/doctor.json"
pass doctor

# 3. convert (README example)
ARCHIVE=$WORK/output/example.3tz
mkdir -p "$WORK/output"
env PATH="" "$BIN" mesh-to-3tz -i "$REPO/tests/fixtures/example.gltf" -o "$ARCHIVE" \
  --cartographic-position-degrees 153.02 -27.47 0 >"$WORK/convert.log" 2>&1 || fail "convert" "$WORK/convert.log"
grep -q "^next: rusty-tiles validate " "$WORK/convert.log" || fail "convert summary has no next: line" "$WORK/convert.log"
pass convert

# 4. validate
env PATH="" "$BIN" validate "$ARCHIVE" --json >"$WORK/validate.json" 2>&1 \
  || fail "validate (exit $?)" "$WORK/validate.json"
[[ $(json_field "$WORK/validate.json" 'd["ok"]') == True ]] || fail "validate not ok" "$WORK/validate.json"
pass validate

# 5. preview startup, manifest and shutdown (a stand-in runtime unless Cesium is present)
"$PYTHON" -m zipfile -e "$ARCHIVE" "$WORK/output/example" || fail "extract archive"
RUNTIME=$CESIUM_DIR
if [[ ! -f $RUNTIME/Cesium.js ]]; then
  RUNTIME=$WORK/stand-in-cesium
  mkdir -p "$RUNTIME"
  echo "// stand-in runtime for the startup check" >"$RUNTIME/Cesium.js"
fi
start_preview "$WORK/preview.json" --cesium "$RUNTIME" --mesh "$WORK/output/example"
curl -fsS "${URL}config.json" -o "$WORK/config.json" || fail "curl config.json"
[[ $(json_field "$WORK/config.json" 'd["mesh"]') == /mesh/tileset.json ]] \
  || fail "config.json does not name the mesh manifest" "$WORK/config.json"
curl -fsS "${URL}mesh/tileset.json" -o "$WORK/served-tileset.json" || fail "curl tileset.json"
cmp -s "$WORK/served-tileset.json" "$WORK/output/example/tileset.json" || fail "served manifest differs"
if [[ $REQUIRE_BROWSER == 1 ]]; then
  node "$REPO/tests/fixtures/quick_start.cjs" "$URL" >"$WORK/quick-start-browser.log" 2>&1 \
    || fail "README quick-start browser probe" "$WORK/quick-start-browser.log"
fi
PREVIEW_PID=$SERVER
stop_server
kill -0 "$PREVIEW_PID" 2>/dev/null && fail "preview still running after SIGTERM"
curl -fsS --max-time 2 "${URL}config.json" -o /dev/null 2>/dev/null && fail "preview port still answering"
pass preview

# 6-7. optional browser acceptance
browser_prerequisites
browser_converter_prerequisites
if [[ ${#missing[@]} -gt 0 ]]; then
  missing_browser_reason
  [[ $REQUIRE_BROWSER == 0 ]] || fail "required browser acceptance prerequisites: $reason"
  skip "preview layers" "$reason"
  skip "browser probes" "$reason"
else
  CASE=$WORK/preview-case
  RUSTY_TILES_BIN=$BIN "$PYTHON" "$REPO/tests/fixtures/preview_layers.py" "$CASE" \
    >"$WORK/layers.log" 2>&1 || fail "preview_layers.py" "$WORK/layers.log"
  pass "preview layers"
  "$PYTHON" "$REPO/tests/t1_terrain_oracle.py" "$CASE/terrain" --browser-oracle "$CASE/terrain-oracle.json" --browser-imagery \
    >"$WORK/terrain-oracle.log" 2>&1 || fail "terrain GLB oracle" "$WORK/terrain-oracle.log"
  for probe in preview_layers point_cloud terrain "vector_compat --require-native --require-aggregates"; do
    read -r script flags <<<"$probe"
    case $script in
      preview_layers) layers=(--mesh "$CASE/mesh" --point-cloud "$CASE/cloud"
        --annotations "$CASE/annotations" --imagery "$CASE/imagery" --terrain "$CASE/terrain") ;;
      point_cloud) layers=(--point-cloud "$CASE/cloud") ;;
      terrain) layers=(--terrain "$CASE/terrain" --imagery "$CASE/imagery") ;;
      vector_compat) layers=(--annotations "$CASE/annotations") ;;
    esac
    start_preview "$WORK/preview-$script.json" --cesium "$CESIUM_DIR" "${layers[@]}"
    read -r -a browser_flags <<<"$flags"
    if [[ $script == terrain ]]; then browser_flags=("$CASE/terrain-oracle.json"); fi
    node "$REPO/tests/fixtures/$script.cjs" "$URL" "${browser_flags[@]}" >"$WORK/$script.log" 2>&1 \
      || fail "browser probe $script.cjs" "$WORK/$script.log"
    pass "browser probe $script.cjs"
    stop_server
  done
fi

echo
echo "passed: ${#PASSED[@]} steps; work: $WORK"
if [[ ${#SKIPPED[@]} -gt 0 ]]; then
  echo "skipped:"
  printf '  %s\n' "${SKIPPED[@]}"
fi
exit 0
