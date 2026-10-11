#!/bin/bash
# R-10: a timed case that loads implicit roots and documents. Deep views on mf2-lod3 (16 views, MSSE 1e-4,
# overview at 16, deterministic, default child-bounds cull), ABBA x2 by case order within run_real
# invocations, CPUs 13,15, per-CPU busy sampled around each invocation.
set -u
E=$(cd "$(dirname "$0")" && pwd); V=$HOME/.cache/perf-lane/work/pimpl/v2; O=$E/deep-abba; mkdir -p $O
until [ -f $E/cesium/fixtures.done ]; do sleep 10; done
export PIMPL_DETERMINISTIC=1 PIMPL_STABLE=30 PIMPL_CHILD_BOUNDS=1 PIMPL_MSSE=0.0001
r() { local tag=$1; shift
  python3 -I $E/cpustat.py snap > $O/$tag.cpu0
  timeout 5400 taskset -c 13,15 nice -n 10 python3 -I $E/cesium/run_real.py $V/views-mf2-lod3.json $O/$tag.json "$@" > $O/$tag.log 2>&1
  python3 -I $E/cpustat.py diff $O/$tag.cpu0 > $O/$tag.cpuload; }
r round1-AB today=$V/today-mf2-lod3:today forest4=$V/forest-mf2-lod3-s4:forest
r round1-BA forest4=$V/forest-mf2-lod3-s4:forest today=$V/today-mf2-lod3:today
r round2-AB today=$V/today-mf2-lod3:today forest4=$V/forest-mf2-lod3-s4:forest
r round2-BA forest4=$V/forest-mf2-lod3-s4:forest today=$V/today-mf2-lod3:today
echo done > $O/done
