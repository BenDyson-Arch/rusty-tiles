#!/bin/bash
# P-IMPL v2 perf-lane Cesium A/B: lane scripts from bb7d054 (cesium_cpus.py = affinity line only),
# ABBA x2 rounds, one case per invocation, per-CPU busy sampling around every invocation.
# CPUs 13,15 are SMT threads of P-cores 6 and 7; their siblings are 12 and 14 (lscpu -e).
set -u
E=/tmp/rusty-tiles-pimpl-design-20261011/evidence; L=$E/perf_lane_bb7d054; O=$E/v2/perf; W=$HOME/.cache/perf-lane/work/pimpl/v2
I=/tmp/rusty-tiles-i1-contract-20261011/samples
mkdir -p $O
export PIMPL_CPUS=13,15
declare -A BASE=( [vector]=$I/vec-perf.3tz [mf2-lod3]=$W/mf2-lod3-implicit.3tz [pc-deep]=$I/pc-deep.3tz )
declare -A HEAD=( [vector]=$W/forest-vec-perf-s4 [mf2-lod3]=$W/forest-mf2-lod3-s4 [pc-deep]=$W/forest-pc-deep-s4 )
run() { # tag case path [baseline]
  local tag=$1 c=$2 p=$3 b=${4:-}
  python3 -I $E/v2/cpustat.py snap > $O/$tag-$c.cpu0
  if [ -n "$b" ]; then extra="--baseline $b"; else extra=""; fi
  nice -n 10 python3 -I $L/cesium_cpus.py --timeout 300 --work $W/perf-work/$tag-$c --out $O/$tag-$c.json $extra $c=$p > $O/$tag-$c.log 2>&1
  python3 -I $E/v2/cpustat.py diff $O/$tag-$c.cpu0 > $O/$tag-$c.cpuload
}
for c in vector mf2-lod3 pc-deep; do
  run base1 $c ${BASE[$c]}
  run head1 $c ${HEAD[$c]} $O/base1-$c.json
  run head2 $c ${HEAD[$c]} $O/base1-$c.json
  run base2 $c ${BASE[$c]} $O/base1-$c.json
  run base3 $c ${BASE[$c]} $O/base1-$c.json
  run head3 $c ${HEAD[$c]} $O/base1-$c.json
  run head4 $c ${HEAD[$c]} $O/base1-$c.json
  run base4 $c ${BASE[$c]} $O/base1-$c.json
done
echo done > $O/done
