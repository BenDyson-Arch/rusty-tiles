#!/usr/bin/env bash
set -euo pipefail
cd /home/bend/.cache/rt-wt/lod-cluster-dag
work=/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag
name=${1:?tex3 or m2048};case "$name" in tex3) source=/home/bend/.cache/rusty-tiles-evidence/h17-20261013/crop/tex3.glb;; m2048) source=/home/bend/.cache/rusty-tiles-preview/input/mgal_detail_offset_2048.glb;; *) exit 2;; esac
out=$work/hlod-octree-$name
evidence=/home/bend/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/cluster-dag/octree-$name
mkdir -p "$out/.run" "$evidence"
( set -o noclobber; date -Is > "$out/.run/launched-once" )
exec systemd-run --user --scope --unit=rt-cluster-octree-$name-20261024 \
 -p MemoryMax=20G -p MemorySwapMax=0 \
 chrt -i 0 nice -n 19 env RT_MEASURE_SLOT=2 OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 \
 python3 -I /home/bend/.cache/rt-scripts/measure.py "$evidence/build.log" \
 "$work/venv/bin/python" bench/lod_cluster_dag/watch_disk.py "$out/.run" \
 "$work/venv/bin/python" bench/lod_cluster_dag/octree_hlod.py "$source" \
 /home/bend/scratch/rusty-tiles-scratch/proxy-density/$name/view "$out" \
 "$work/libmeshopt.so" "$work/libcharts-octree.so"
