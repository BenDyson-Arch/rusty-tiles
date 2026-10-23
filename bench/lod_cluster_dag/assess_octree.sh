#!/usr/bin/env bash
set -euo pipefail
cd /home/bend/.cache/rt-wt/lod-cluster-dag
work=/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag
name=${1:?};out=$work/hlod-octree-$name
evidence=/home/bend/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/cluster-dag/octree-$name
case "$name" in tex3) source=/home/bend/.cache/rusty-tiles-evidence/h17-20261013/crop/tex3.glb;;m2048) source=/home/bend/.cache/rusty-tiles-preview/input/mgal_detail_offset_2048.glb;;*) exit 2;;esac
p=$work/venv/bin/python
"$p" bench/lod_cluster_dag/inventory_full.py "$out" "$evidence" > "$evidence/inventory.log"
"$p" bench/lod_cluster_dag/inventory_full.py /home/bend/scratch/rusty-tiles-scratch/proxy-density/$name/view "$evidence/baseline" > "$evidence/baseline-inventory.log"
"$p" bench/lod_cluster_dag/evaluate_octree.py "$source" "$out" "$evidence" > "$evidence/evaluate.log" 2>&1
"$p" bench/lod_cluster_dag/check.py "$out" "$source" "$evidence/check.json" > "$evidence/check.log"
"$p" - "$out" "$evidence" <<'PY'
from pathlib import Path
import sys,json,hashlib
root=Path(sys.argv[1]);ev=Path(sys.argv[2]);sample=root/'.run/validation';(sample/'content').mkdir(parents=True,exist_ok=True)
(sample/'textures').symlink_to(root/'textures',target_is_directory=True)
nodes=json.loads((root/'build.json').read_text())['nodes'];ids={0}
for d in sorted({n['depth'] for n in nodes}):
 for leaf in [True,False]:
  choice=[n for n in nodes if n['depth']==d and (not n['children'])==leaf]
  if choice:ids.add(choice[len(choice)//2]['id'])
rows=[]
for i in sorted(ids):
 p=root/nodes[i]['file'];(sample/'content'/p.name).symlink_to(p)
 rows.append({'file':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()})
(ev/'validation-sample.json').write_text(json.dumps(rows,indent=2))
PY
node bench/lod_cluster_dag/validate.mjs "$out/.run/validation" "$evidence/validation.json"
node bench/lod_cluster_dag/decode.mjs "$out/.run/validation" "$out/.run/decoded-sample" > "$evidence/decode.log" 2>&1
"$p" bench/lod_cluster_dag/decode_agreement.py "$out/.run/validation" "$out/.run/decoded-sample" "$evidence/decode-agreement.json"
touch "$evidence/complete"
