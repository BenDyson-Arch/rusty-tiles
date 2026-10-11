#!/bin/bash
# Follow-up: today vs --explicit vs forest on the LOD-chain vector case (default cullWithChildrenBounds).
set -u
E=$(cd "$(dirname "$0")" && pwd); V=$HOME/.cache/perf-lane/work/pimpl/v2; O=$E/deep
until [ -f $O/done ]; do sleep 10; done
export PIMPL_DETERMINISTIC=1 PIMPL_STABLE=30 PIMPL_CHILD_BOUNDS=1 PIMPL_MSSE=0.0001
python3 -I $E/cpustat.py snap > $O/mf2-lod3-explicit-cb1.cpu0
timeout 5400 taskset -c 13,15 nice -n 10 python3 -I $E/cesium/run_real.py $V/views-mf2-lod3-x.json $O/mf2-lod3-explicit-cb1.json \
  today=$V/today-mf2-lod3:today explicit=$V/explicit-mf2-lod3:explicit forest4=$V/forest-mf2-lod3-s4:forest > $O/mf2-lod3-explicit-cb1.log 2>&1
python3 -I $E/cpustat.py diff $O/mf2-lod3-explicit-cb1.cpu0 > $O/mf2-lod3-explicit-cb1.cpuload
echo done > $O/done-explicit
