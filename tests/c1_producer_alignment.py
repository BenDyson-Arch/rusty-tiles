"""Optional actual vector producer framing control against a frozen binary."""
import argparse,hashlib,json,pathlib,struct,subprocess,zipfile

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=pathlib.Path,required=True);p.add_argument('--work',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
 source=pathlib.Path(__file__).parent/'fixtures/c1-framing-staircase.geojson';before=hashlib.sha256(source.read_bytes()).hexdigest();a.work.mkdir(parents=True,exist_ok=False)
 ring=json.loads(source.read_text())['features'][0]['geometry']['coordinates'][0]
 assert len(ring)-1==50 and ring[0]==ring[-1]
 area=abs(sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(ring,ring[1:])))/2;assert area==300
 target=a.work/'staircase.3tz';command=[str(a.binary.resolve()),'vector','-i',str(source.resolve()),'-o',str(target.resolve()),'--sourceCrs','local','--explicit','--reproducible','--maxVertices','32','--maxBytes','16384','--json']
 run=subprocess.run(command,capture_output=True,text=True,timeout=120);assert run.returncode==0,(run.stdout,run.stderr)
 report=json.loads(run.stdout);assert report['counts']['features']==1 and report['counts']['fragmentedPolygons']==1 and report['counts']['skippedFeatures']==0,report
 controls=[]
 with zipfile.ZipFile(target) as z:
  for name in z.namelist():
   assert not name.endswith('.b3dm'),name
   if not name.endswith('.glb'):continue
   raw=z.read(name);total=len(raw);gm,gv,gl=struct.unpack_from('<4sII',raw);assert gm==b'glTF' and gv==2 and gl==total and total%8==0,name
   pos=12;document=None;bin_length=None
   while pos<gl:
    n,kind=struct.unpack_from('<I4s',raw,pos);assert n%4==0 and pos+8+n<=gl,name
    data=raw[pos+8:pos+8+n]
    if kind==b'JSON':document=json.loads(data)
    if kind==b'BIN\0':
     bin_length=n;declared=document['buffers'][0]['byteLength'];assert 0<=n-declared<=3 and data[declared:]==bytes(n-declared),name
    pos+=8+n
   assert pos==gl and bin_length is not None,name
   # A fragmented fill is one EXT_mesh_polygon polygon per triangle.
   fill=any(p.get('extensions',{}).get('EXT_mesh_polygon',{}).get('count',0)*3==document['accessors'][p['indices']]['count']
            for p in document['meshes'][0]['primitives'] if p.get('mode',4)==4)
   if fill:controls.append(dict(member=name,glbBytes=gl,binBytes=bin_length,declaredBufferBytes=declared))
 assert controls,'Actual producer emitted no glTF fill controls'
 assert hashlib.sha256(source.read_bytes()).hexdigest()==before
 evidence=dict(binarySha256=hashlib.sha256(a.binary.read_bytes()).hexdigest(),sourceSha256=before,sourceFacts=dict(ringVertices=50,planarAreaSquareMetres=area,sourceCrs='local'),archiveSha256=hashlib.sha256(target.read_bytes()).hexdigest(),command=command,counts=report['counts'],controls=controls,sourceReadOnly=True)
 a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(evidence,indent=2)+'\n');print(json.dumps({'ok':True,'fillControls':len(controls)}))
if __name__=='__main__':main()
