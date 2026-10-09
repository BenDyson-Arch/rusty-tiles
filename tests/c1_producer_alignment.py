"""Optional actual vector producer framing control against a frozen binary."""
import argparse,hashlib,json,pathlib,struct,subprocess,zipfile

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=pathlib.Path,required=True);p.add_argument('--work',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
 source=pathlib.Path(__file__).parent/'fixtures/countries-source.geojson';before=hashlib.sha256(source.read_bytes()).hexdigest();a.work.mkdir(parents=True,exist_ok=False)
 target=a.work/'countries.3tz';command=[str(a.binary.resolve()),'vector','-i',str(source.resolve()),'-o',str(target.resolve()),'--explicit','--reproducible','--maxVertices','32','--maxBytes','16384','--json']
 run=subprocess.run(command,capture_output=True,text=True,timeout=120);assert run.returncode==0,(run.stdout,run.stderr)
 controls=[]
 with zipfile.ZipFile(target) as z:
  for name in z.namelist():
   if not name.endswith('.b3dm'):continue
   raw=z.read(name);magic,version,total,*lengths=struct.unpack_from('<4s6I',raw);assert magic==b'b3dm' and version==1 and total==len(raw) and total%8==0,name
   start=28+sum(lengths);assert start%8==0,name
   gm,gv,gl=struct.unpack_from('<4sII',raw,start);assert gm==b'glTF' and gv==2 and gl%4==0 and start+gl<=total,name
   assert raw[start+gl:]==bytes(total-start-gl),name
   pos=start+12;document=None;bin_length=None
   while pos<start+gl:
    n,kind=struct.unpack_from('<I4s',raw,pos);assert n%4==0 and pos+8+n<=start+gl,name
    data=raw[pos+8:pos+8+n]
    if kind==b'JSON':document=json.loads(data)
    if kind==b'BIN\0':
     bin_length=n;declared=document['buffers'][0]['byteLength'];assert 0<=n-declared<=3 and data[declared:]==bytes(n-declared),name
    pos+=8+n
   assert pos==start+gl and bin_length is not None,name
   controls.append(dict(member=name,b3dmBytes=total,glbBytes=gl,binBytes=bin_length,declaredBufferBytes=declared))
 assert controls,'Actual producer emitted no b3dm controls'
 assert hashlib.sha256(source.read_bytes()).hexdigest()==before
 evidence=dict(binarySha256=hashlib.sha256(a.binary.read_bytes()).hexdigest(),sourceSha256=before,archiveSha256=hashlib.sha256(target.read_bytes()).hexdigest(),command=command,controls=controls,sourceReadOnly=True)
 a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(evidence,indent=2)+'\n');print(json.dumps({'ok':True,'b3dmControls':len(controls)}))
if __name__=='__main__':main()
