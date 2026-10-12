#!/bin/sh
# Final-binary (default cap 2^29) checks: precheck-only arms, phase-1 twins, and re-measured rows.
W=/home/bend/.cache/c2wb-int
B=$W/bin/rusty-tiles-final-cap2p29
O=$W/results/final
mkdir -p $O
cd $W/scripts
for round in 1 2 3 4; do
  for t in P E; do
    { date -Is; echo "round $round $t"; uptime; } >> $O/load.txt
    HOUSE=1 python3 -I gate.py $B $W/cases-final.json $O/final-$t.jsonl $t
  done
done
echo done
