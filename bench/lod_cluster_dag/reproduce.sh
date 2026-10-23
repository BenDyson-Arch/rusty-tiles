#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} != --idle ]]; then exec chrt -i 0 nice -n 19 bash "$0" --idle; fi
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 CARGO_BUILD_JOBS=3
export CARGO_TARGET_DIR=/home/bend/.cache/rt-wt/target-cluster-dag
export CLUSTER_COMPRESS=1 CLUSTER_TEXTURE_SIZE=1024 CLUSTER_CROP_TEXTURE=1
export CLUSTER_PERMISSIVE=0 CLUSTER_ATTRIBUTE_WEIGHT=1 CLUSTER_JPEG_QUALITY=85
cd /home/bend/.cache/rt-wt/lod-cluster-dag
work=/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag
evidence=/home/bend/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/cluster-dag
source_dir=/home/bend/.cache/rusty-tiles-evidence/h17-20261013/crop
baseline=/home/bend/scratch/rusty-tiles-scratch/proxy-density/tex3/view
meshopt=/home/bend/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/meshopt-0.6.2/vendor/src
mkdir -p "$work" "$evidence"
test "$(df --output=avail -B1 / | tail -1)" -ge 64424509440
if [[ ! -x $work/venv/bin/python ]]; then
  uv venv "$work/venv"
  uv pip install --python "$work/venv/bin/python" numpy==2.5.4 scipy==1.18.1 pillow==13.0.0 point-cloud-utils==0.34.0
fi
g++ -std=c++11 -O2 -shared -fPIC "$meshopt"/*.cpp -o "$work/libmeshopt.so"
p=$work/venv/bin/python
for input in tex3 crop3; do
  # These two directories are exclusively owned generated outputs. A clean
  # directory makes peak scratch independent of an earlier experimental variant.
  if [[ -e $work/$input ]]; then mv -- "$work/$input" "$work/$input.previous-$(date +%s)"; fi
  "$p" bench/lod_cluster_dag/measure.py "$evidence/$input.measure.json" "$work/$input" \
    "$p" bench/lod_cluster_dag/build.py "$source_dir/$input.glb" "$work/$input" "$work/libmeshopt.so" "$baseline/tileset.json" > "$evidence/$input.build.log" 2>&1
  "$p" bench/lod_cluster_dag/evaluate.py "$source_dir/$input.glb" "$work/$input" "$evidence/$input" > "$evidence/$input.evaluate.log" 2>&1
  "$p" bench/lod_cluster_dag/inventory.py "$work/$input" "$evidence/$input" > "$evidence/$input.inventory.log"
  "$p" bench/lod_cluster_dag/check.py "$work/$input" "$source_dir/$input.glb" "$evidence/$input/check.json"
  node bench/lod_cluster_dag/validate.mjs "$work/$input" "$evidence/$input/validation.json"
  node bench/lod_cluster_dag/decode.mjs "$work/$input" "$work/oracle-$input" > "$evidence/$input.decode.log" 2>&1
  "$p" bench/lod_cluster_dag/decode_agreement.py "$work/$input" "$work/oracle-$input" "$evidence/$input/decode-agreement.json"
done
node bench/lod_cluster_dag/decode.mjs "$baseline" "$work/baseline" > "$evidence/baseline.decode.log" 2>&1
"$p" bench/lod_cluster_dag/evaluate.py "$source_dir/tex3.glb" "$work/baseline" "$evidence/baseline" "$baseline" > "$evidence/baseline.evaluate.log" 2>&1
"$p" bench/lod_cluster_dag/inventory.py "$baseline" "$evidence/baseline" > "$evidence/baseline.inventory.log"
