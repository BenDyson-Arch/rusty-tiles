#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} != --idle ]]; then exec chrt -i 0 nice -n 19 bash "$0" --idle; fi
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 CARGO_BUILD_JOBS=3
export CARGO_TARGET_DIR=/home/bend/.cache/rt-wt/target-cluster-dag
cd /home/bend/.cache/rt-wt/lod-cluster-dag
work=/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag
evidence=/home/bend/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/cluster-dag
source_dir=/home/bend/.cache/rusty-tiles-evidence/h17-20261013/crop
p=$work/venv/bin/python
test "$(df --output=avail -B1 / | tail -1)" -ge 64424509440
g++ -O2 -shared -fPIC bench/lod_cluster_dag/charts.cpp -o "$work/libcharts.so"
for input in tex3 crop3; do
  dest=$work/hlod-final-$input
  if [[ -e $dest ]]; then mv -- "$dest" "$dest.previous-$(date +%s)"; fi
  "$p" bench/lod_cluster_dag/measure.py "$evidence/hlod-final-$input.measure.json" "$dest" \
    "$p" bench/lod_cluster_dag/hlod.py "$source_dir/$input.glb" "$work/$input" "$dest" "$work/libmeshopt.so" "$work/libcharts.so" > "$evidence/hlod-final-$input.build.log" 2>&1
  "$p" bench/lod_cluster_dag/evaluate.py "$source_dir/$input.glb" "$dest" "$evidence/hlod-final-$input" > "$evidence/hlod-final-$input.evaluate.log" 2>&1
  "$p" bench/lod_cluster_dag/inventory.py "$dest" "$evidence/hlod-final-$input" > "$evidence/hlod-final-$input.inventory.log"
  "$p" bench/lod_cluster_dag/check.py "$dest" "$source_dir/$input.glb" "$evidence/hlod-final-$input/check.json"
  node bench/lod_cluster_dag/validate.mjs "$dest" "$evidence/hlod-final-$input/validation.json"
  node bench/lod_cluster_dag/decode.mjs "$dest" "$work/oracle-hlod-$input" > "$evidence/hlod-final-$input.decode.log" 2>&1
  "$p" bench/lod_cluster_dag/decode_agreement.py "$dest" "$work/oracle-hlod-$input" "$evidence/hlod-final-$input/decode-agreement.json"
  touch "$evidence/hlod-final-$input/complete"
done
"$p" bench/lod_cluster_dag/hlod_controls.py "$work/libmeshopt.so" "$work/libcharts.so" "$evidence/hlod-controls.json"
