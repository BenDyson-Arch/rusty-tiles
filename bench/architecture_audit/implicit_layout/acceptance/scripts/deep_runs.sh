#!/bin/bash
# Deep-view A/B (P A-5.2): develop vs head, PIMPL_CHILD_BOUNDS 0 and 1, one browser at a time.
P=/tmp/rusty-tiles-forest/bench/architecture_audit/implicit_layout/cesium
W=/home/bend/.cache/forest-acc/work/deep
export PIMPL_DETERMINISTIC=1 PIMPL_STABLE=30
for name in vec-perf mf2-lod3 mesh-deep pc-deep; do
  R=$W/real-$name
  if [ $name = pc-deep ]; then export PIMPL_MSSE=1; else export PIMPL_MSSE=0.0001; fi
  for b in 0 1; do
    python3 -I $P/cpustat.py snap > $R/cb$b.cpu0
    PIMPL_CHILD_BOUNDS=$b taskset -c 13,15 nice -n 10 python3 -I $P/run_real.py $R/views.json $R/res-cb$b.json \
      develop=$R/develop:develop head=$R/head:head > $R/cb$b.log 2>&1
    echo "$name cb$b exit=$?"
    python3 -I $P/cpustat.py diff $R/cb$b.cpu0 > $R/cb$b.cpuload
  done
  python3 -I $P/judge_deep.py --cb0 $R/res-cb0.json --cb1 $R/res-cb1.json --parents $R/parents.json --errors $R/errors.json > $R/judge.json 2> $R/judge.err
  echo "$name judge exit=$?"
done
