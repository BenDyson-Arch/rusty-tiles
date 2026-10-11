#!/bin/sh
# ABBA perf-lane Cesium A/B on CPUs 13,15 with per-CPU busy sampling (siblings 12,14 included).
L=/tmp/pimpl-precode-review-20261011/evidence-copy/perf_lane_bb7d054
R=/tmp/pimpl-precode-review-20261011/runs/perf
M=/tmp/pimpl-precode-review-20261011/models
S=/tmp/rusty-tiles-i1-contract-20261011/samples
mkdir -p $R
run() { # name args...
  n=$1; shift
  python3 -I /tmp/pimpl-precode-review-20261011/scripts/cpustat.py snap > $R/$n.cpu0
  PIMPL_CPUS=13,15 nice -n 10 python3 -I $L/cesium_cpus.py --timeout 300 --work $R/work-$n --out $R/$n.json "$@" > $R/$n.log 2>&1
  python3 -I /tmp/pimpl-precode-review-20261011/scripts/cpustat.py diff $R/$n.cpu0 > $R/$n.cpuload
}
run base1 vector=$S/vec-perf.3tz pc-deep=$S/pc-deep.3tz
run forest1 --baseline $R/base1.json vector=$M/vec-perf-s4 pc-deep=$M/pc-deep-s4
run forest2 --baseline $R/base1.json vector=$M/vec-perf-s4 pc-deep=$M/pc-deep-s4
run base2 --baseline $R/base1.json vector=$S/vec-perf.3tz pc-deep=$S/pc-deep.3tz
echo done > $R/done
