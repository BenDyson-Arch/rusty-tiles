#!/bin/sh
# C2-WB 9.D gate, resumable: alternate P-core and E-core passes over every case not yet decided.
W=/home/bend/.cache/c2wb-int
B=$W/bin/rusty-tiles-measure-e8cf53a-cap2p62
O=$W/results/gate
cd $W/scripts
for round in 1 2 3 4 5 6 7 8; do
  for t in P E; do
    { date -Is; echo "round $round $t"; uptime; } >> $O/load.txt
    HOUSE=1 python3 -I gate.py $B $W/cases-all.json $O/gate-$t.jsonl $t
  done
  python3 -I table.py $O/table.json $O/gate-P.jsonl -- $O/gate-E.jsonl > $O/table.txt
  head -1 $O/table.txt
  python3 -I -c "
import json,sys
t=json.load(open('$O/table.json'))
names={c['name'] for c in json.load(open('$W/cases-all.json'))}
left=[r['name'] for r in t['rows'] for k in 'PE' if r['name'] in names and r.get('status'+k) not in ('decided','unexpectedOutcome')]
left+= [n for n in names if n not in {r['name'] for r in t['rows']}]
print('left', len(left)); sys.exit(0 if not left else 1)" && break
done
{ date -Is; uptime; } >> $O/load.txt
echo done
