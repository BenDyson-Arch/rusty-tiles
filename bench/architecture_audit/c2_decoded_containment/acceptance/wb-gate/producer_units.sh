#!/bin/sh
# Oracle (O-WB c2o) precheck-only units of every b51f83d explicit producer archive.
F=/home/bend/.cache/forest-acc/work/head-b51f83d
O=/home/bend/.cache/c2wb-int/producer-units
cd /tmp/rusty-tiles-c2wb/bench/architecture_audit/c2_decoded_containment
for a in $F/*-explicit.3tz; do
  n=$(basename $a .3tz)
  python3 -I oracle/c2_oracle.py $a --json --precheck-only --no-detail --limit containmentWorkUnits=4611686018427387904 > $O/$n.json 2> $O/$n.err
  echo "$n $?"
done
