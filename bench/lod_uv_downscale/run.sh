#!/usr/bin/env bash
set -euo pipefail
if [ "${UV_DOWNSCALE_IDLE:-}" != 1 ]; then
  exec chrt -i 0 nice -n 19 env UV_DOWNSCALE_IDLE=1 bash "$0" "$@"
fi
cd /home/bend/.cache/rt-wt/lod-uv-downscale
export CARGO_TARGET_DIR=/home/bend/.cache/rt-wt/target-lod-uv-downscale CARGO_BUILD_JOBS=3
export RAYON_NUM_THREADS=2 OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2
scratch=/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/uv-downscale
evidence=/home/bend/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/uv-downscale
py="$scratch/venv/bin/python"
mkdir -p "$scratch" "$evidence"
for name in tex3 crop3; do
  if [ ! -f "$scratch/$name/tree.json" ]; then "$py" bench/lod_uv_downscale/run.py prep "$name" >"$evidence/$name-prep.log" 2>&1; fi
  "$py" -c 'import shutil,sys;from pathlib import Path;r=Path(sys.argv[1]);shutil.rmtree(r/"output",ignore_errors=True);shutil.rmtree(r/"decoded",ignore_errors=True)' "$scratch/$name"
  "$py" bench/lod_uv_downscale/measure.py "$evidence/$name-process.json" "$py" bench/lod_uv_downscale/run.py build "$name" >"$evidence/$name-build.log" 2>&1
  "$py" bench/lod_uv_downscale/run.py controls "$name" >"$evidence/$name-controls.log" 2>&1
  "$py" bench/lod_uv_downscale/run.py metrics "$name" >"$evidence/$name-metrics.log" 2>&1
done
"$py" bench/lod_uv_downscale/validate.py >"$evidence/validation.log" 2>&1
"$py" bench/lod_uv_downscale/report.py
