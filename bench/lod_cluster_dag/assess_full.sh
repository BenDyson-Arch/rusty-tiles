#!/usr/bin/env bash
set -euo pipefail
cd /home/bend/.cache/rt-wt/lod-cluster-dag
work=/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag
out=$work/hlod-final-m2048
evidence=/home/bend/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/cluster-dag/m2048
source=/home/bend/.cache/rusty-tiles-preview/input/mgal_detail_offset_2048.glb
p=$work/venv/bin/python
mkdir -p "$evidence"
"$p" bench/lod_cluster_dag/inventory_full.py "$out" "$evidence" > "$evidence/inventory.log"
"$p" bench/lod_cluster_dag/inventory_full.py /home/bend/scratch/rusty-tiles-scratch/proxy-density/m2048/view "$evidence/baseline" > "$evidence/baseline-inventory.log"
"$p" bench/lod_cluster_dag/evaluate_full.py "$source" "$out" "$evidence" > "$evidence/evaluate.log" 2>&1
"$p" - "$out" "$evidence" <<'PY'
from pathlib import Path
import sys,json,hashlib
root=Path(sys.argv[1]);evidence=Path(sys.argv[2]);sample=root/'.run/validation';(sample/'content').mkdir(parents=True,exist_ok=True)
(sample/'textures').symlink_to(root/'textures',target_is_directory=True)
ids=[0,1,2,4,6,17,32,33,48,62];rows=[]
for i in ids:
 p=root/'content'/f'{i}.glb';(sample/'content'/p.name).symlink_to(p)
 rows.append({'file':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()})
(evidence/'validation-sample.json').write_text(json.dumps(rows,indent=2))
PY
node bench/lod_cluster_dag/validate.mjs "$out/.run/validation" "$evidence/validation.json"
node bench/lod_cluster_dag/decode.mjs "$out/.run/validation" "$out/.run/decoded-sample" > "$evidence/decode.log" 2>&1
"$p" bench/lod_cluster_dag/decode_agreement.py "$out/.run/validation" "$out/.run/decoded-sample" "$evidence/decode-agreement.json"
touch "$evidence/complete"
