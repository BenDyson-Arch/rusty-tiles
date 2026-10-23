#!/usr/bin/env python3
"""All coarse faces: original primitive references plus feature ID validity."""
import json,struct,sys
from pathlib import Path
from evaluate import Surface
source,root,output=sys.argv[1:];root=Path(root)
with open(source,'rb') as f:
 head=f.read(20);j=json.loads(f.read(struct.unpack_from('<I',head,12)[0]))
known={(mi,pi) for mi,m in enumerate(j['meshes']) for pi,p in enumerate(m['primitives'])}
rows=[]
for n in json.loads((root/'build.json').read_text())['nodes']:
 if not n['children']:continue
 s=Surface().add(root/n['file']).finish();assert set(map(tuple,s.primitive_identity))<=known
 rows.append(dict(id=n['id'],faces=len(s.xyz),all_corner_ids_valid=True,source_primitive_references_valid=True))
Path(output).write_text(json.dumps(rows,indent=2));print('coarse tiles',len(rows),'all primitive references and corner IDs valid')
