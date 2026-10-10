import json,zipfile,hashlib,sys
from fractions import Fraction as Q
from pathlib import Path
artifact=Path(sys.argv[1]);out=[]
with zipfile.ZipFile(artifact) as stream:document=json.loads(stream.read('tileset.json'))
root=document['root']['boundingVolume']['box']
for i,tile in enumerate(document['root']['children']):
    box=tile['boundingVolume']['box']
    for axis,index in enumerate([3,7,11]):
        rlo,rhi=Q(root[axis])-Q(root[index]),Q(root[axis])+Q(root[index])
        clo,chi=Q(box[axis])-Q(box[index]),Q(box[axis])+Q(box[index])
        if clo<rlo:out.append({'child':i,'axis':axis,'side':'low','excess_exact':str(rlo-clo),'excess_metres':float(rlo-clo)})
        if chi>rhi:out.append({'child':i,'axis':axis,'side':'high','excess_exact':str(chi-rhi),'excess_metres':float(chi-rhi)})
print(json.dumps({'artifact':str(artifact),'sha256':hashlib.sha256(artifact.read_bytes()).hexdigest(),'children':len(document['root']['children']),'root_child_containment_failures':out},indent=2))
