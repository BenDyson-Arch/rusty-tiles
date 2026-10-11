#!/bin/bash
# P-IMPL v3 scale: forest + documents at S = 4, vector without retained sources (PD-11). E-cores 16,18.
set -u
E=$(cd "$(dirname "$0")" && pwd); V=$HOME/.cache/perf-lane/work/pimpl/v2; I=/tmp/rusty-tiles-i1-contract-20261011/samples
OUT=$E/scale; mkdir -p $OUT
cases=(
 "vec-perf|explicit:$I/vec-perf-explicit.3tz"
 "vec-mf2-lod3|explicit:$V/mf2-lod3-explicit.3tz"
 "vec-mf2-lod16|explicit:$V/mf2-lod16-explicit.3tz"
 "pc-deep|implicit:$I/pc-deep.3tz"
 "synth-vec-100k-lod3-real|synth-vector:100000:3:real:$V/mf2-lod3-explicit.3tz"
 "synth-vec-100k-lod3-full|synth-vector:100000:3:full:$V/mf2-lod3-explicit.3tz"
 "synth-vec-100k-lod16-real|synth-vector:100000:16:real:$V/mf2-lod16-explicit.3tz"
 "synth-vec-100k-lod16-full|synth-vector:100000:16:full:$V/mf2-lod16-explicit.3tz"
 "synth-pt-100k-b4|synth-point:100000:4"
 "synth-pt-1m-b8|synth-point:1000000:8"
)
for c in "${cases[@]}"; do
  name=${c%%|*}; spec=${c#*|}
  taskset -c 16,18 nice -n 10 python3 -I $E/forest_model.py measure $OUT/$name-S4.json "$spec" 4 4 > /dev/null || echo "FAIL $name"
done
echo done > $OUT/done
