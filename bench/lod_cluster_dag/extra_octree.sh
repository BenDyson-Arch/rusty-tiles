#!/usr/bin/env bash
set -euo pipefail
cd /home/bend/.cache/rt-wt/lod-cluster-dag
work=/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag
name=${1:?};out=$work/hlod-octree-$name
base=/home/bend/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/cluster-dag/octree-$name
p=$work/venv/bin/python
"$p" bench/lod_cluster_dag/density_octree.py "$out" /home/bend/scratch/rusty-tiles-scratch/proxy-density/$name/view "$base/density.json"
"$p" bench/lod_cluster_dag/check_octree_tree.py "$out" /home/bend/scratch/rusty-tiles-scratch/proxy-density/$name/view "$base/tree-check.json"
touch "$base/extras-complete"
