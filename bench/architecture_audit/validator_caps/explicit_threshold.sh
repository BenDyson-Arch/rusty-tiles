#!/usr/bin/env bash
# Where --explicit vector output meets the JSON value-node ceiling.
# Usage: bash explicit_threshold.sh CONVERTER VALIDATOR DIR > results/explicit-threshold.jsonl
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
converter=$1
validator=$2
mkdir -p "$3"
cd "$3"
for n in 2000 2200 2300 2400 2600; do
  python3 -I "$here/gen_vector.py" "p$n.geojson" 0 --points "$n" > /dev/null
  rm -f "e$n.3tz"
  taskset -c 0,1 nice -n 10 "$converter" vector -i "p$n.geojson" -o "e$n.3tz" --jobs 2 --maxFeatures 2 --explicit > /dev/null 2>&1
  taskset -c 0,1 nice -n 10 "$validator" validate --json "e$n.3tz" > "e$n.out" || true
  python3 -I - "$here" "$n" <<'EOF'
import json, sys, zipfile
sys.path.insert(0, sys.argv[1])
from analyze import nodes
n = sys.argv[2]
raw = zipfile.ZipFile(f'e{n}.3tz').read('tileset.json')
tiles = sum(1 for _ in zipfile.ZipFile(f'e{n}.3tz').namelist() if _.endswith('.glb'))
report = json.load(open(f'e{n}.out'))
print(json.dumps({'points': int(n), 'tilesetJsonValueNodes': nodes(json.loads(raw)), 'tilesetJsonBytes': len(raw),
                  'contentTiles': tiles, 'ok': report.get('ok'), 'tiles': report.get('tiles'),
                  'error': (report.get('error') or {}).get('message')}))
EOF
done
