#!/usr/bin/env bash
set -euo pipefail
cd /home/bend/.cache/rt-wt/lod-cluster-dag
work=/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag
out=$work/hlod-final-m2048
evidence=/home/bend/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/cluster-dag
mkdir -p "$out/.run"
# This sentinel makes a second full-model build an explicit new decision.
( set -o noclobber; date -Is > "$out/.run/launched-once" )
exec systemd-run --user --scope --unit=rt-cluster-hlod-m2048-20261024 \
 -p MemoryMax=20G -p MemorySwapMax=0 \
 chrt -i 0 nice -n 19 env RT_MEASURE_SLOT=2 OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 \
 python3 -I /home/bend/.cache/rt-scripts/measure.py "$evidence/m2048.build.log" \
 "$work/venv/bin/python" bench/lod_cluster_dag/watch_disk.py "$out/.run" \
 "$work/venv/bin/python" bench/lod_cluster_dag/hlod.py \
 /home/bend/.cache/rusty-tiles-preview/input/mgal_detail_offset_2048.glb - "$out" \
 "$work/libmeshopt.so" "$work/libcharts.so" \
 /home/bend/scratch/rusty-tiles-scratch/proxy-density/m2048/view/tileset.json
