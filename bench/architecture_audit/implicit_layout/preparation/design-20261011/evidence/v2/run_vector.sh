#!/bin/bash
# P-IMPL v2: realistic deep vector hierarchies from the perf-lane input with the develop-equivalent binary.
B=/home/bend/.cache/c2-corpus/rusty-tiles-f2238cd
IN=/home/bend/.cache/perf-lane/work/inputs/vector.geojson
cd "$(dirname "$0")"
for spec in "mf2-lod3:--maxFeatures 2 --lodLevels 3" "mf2-lod16:--maxFeatures 2 --lodLevels 16"; do
  name=${spec%%:*}; opts=${spec#*:}
  for mode in explicit implicit; do
    flag=""; [ $mode = explicit ] && flag="--explicit"
    taskset -c 17,19 nice -n 10 env RAYON_NUM_THREADS=2 $B --json vector -i $IN --reproducible --jobs 2 $opts $flag -o $name-$mode.3tz > $name-$mode.json 2> $name-$mode.err
    echo "$name $mode exit $?"
  done
done
