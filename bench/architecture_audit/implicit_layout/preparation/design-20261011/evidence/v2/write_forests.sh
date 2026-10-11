#!/bin/bash
# Write servable forest models (S per scheme) with 16 leaf-geometry views each.
set -u
E=$(cd "$(dirname "$0")" && pwd); V=$HOME/.cache/perf-lane/work/pimpl/v2; I=/tmp/rusty-tiles-i1-contract-20261011/samples
w() { rm -rf $V/$1; taskset -c 16,18 nice -n 10 python3 -I $E/forest_model.py write $V/$1 "$2" $3 $4 16 > /dev/null || echo "FAIL $1"; }
w forest-vec-perf-s4 explicit:$I/vec-perf-explicit.3tz 4 4
w forest-mf2-lod3-s4 explicit:$V/mf2-lod3-explicit.3tz 4 4
w forest-mf2-lod3-s6 explicit:$V/mf2-lod3-explicit.3tz 4 6
w forest-mf2-lod16-s4 explicit:$V/mf2-lod16-explicit.3tz 4 4
w forest-pc-deep-s4 implicit:$I/pc-deep.3tz 4 4
python3 -I $E/today_map.py $I/vec-perf-explicit.3tz $I/vec-perf.3tz $V/forest-vec-perf-s4-views.json $V/views-vec-perf.json
python3 -I $E/today_map.py $V/mf2-lod3-explicit.3tz $V/mf2-lod3-implicit.3tz $V/forest-mf2-lod3-s4-views.json $V/views-mf2-lod3.json
python3 -I $E/today_map.py $V/mf2-lod16-explicit.3tz $V/mf2-lod16-implicit.3tz $V/forest-mf2-lod16-s4-views.json $V/views-mf2-lod16.json
echo done
