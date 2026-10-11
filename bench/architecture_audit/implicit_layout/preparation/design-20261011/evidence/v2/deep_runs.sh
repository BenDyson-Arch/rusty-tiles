#!/bin/bash
# P-IMPL v2 deep-view Cesium A/B (one browser at a time, CPUs 13,15, nice 10, deterministic mode).
set -u
E=$(cd "$(dirname "$0")" && pwd); V=$HOME/.cache/perf-lane/work/pimpl/v2; O=$E/deep; mkdir -p $O
export PIMPL_DETERMINISTIC=1 PIMPL_STABLE=30
r() { # tag views cases...
  local tag=$1 views=$2; shift 2
  python3 -I $E/cpustat.py snap > $O/$tag.cpu0
  timeout 5400 taskset -c 13,15 nice -n 10 python3 -I $E/cesium/run_real.py $views $O/$tag.json "$@" > $O/$tag.log 2>&1
  python3 -I $E/cpustat.py diff $O/$tag.cpu0 > $O/$tag.cpuload
}
for b in 1 0; do
  export PIMPL_CHILD_BOUNDS=$b PIMPL_MSSE=0.0001
  r mf2-lod3-cb$b $V/views-mf2-lod3.json today=$V/today-mf2-lod3:today forest4=$V/forest-mf2-lod3-s4:forest
  r mf2-lod3-s6-cb$b $V/views-mf2-lod3-s6.json today=$V/today-mf2-lod3:today forest6=$V/forest-mf2-lod3-s6:forest
  r mf2-lod16-cb$b $V/views-mf2-lod16.json today=$V/today-mf2-lod16:today forest4=$V/forest-mf2-lod16-s4:forest
  r vec-perf-cb$b $V/views-vec-perf.json today=$V/today-vec-perf:today forest4=$V/forest-vec-perf-s4:forest
done
export PIMPL_CHILD_BOUNDS=1 PIMPL_MSSE=1
r pc-deep-cb1 $V/views-pc-deep.json today=$V/today-pc-deep:today forest4=$V/forest-pc-deep-s4:forest
echo done > $O/done
