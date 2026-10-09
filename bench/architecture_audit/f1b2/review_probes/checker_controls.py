#!/usr/bin/env python3
"""Probe sensitivity with valid alternative source plans yielding plausible output.

These mutate source plans, not the production implementation; they establish
the independent output checker detects wrong selection and URI decoding.
"""
import argparse, hashlib, io, json, pathlib, struct, subprocess, tempfile
from PIL import Image
from independent_review import document, png, POS, UV, inspect_archive

SHA='d2f84710f4196511c54fcdb25f71b56f245c38247ae81569370aa2f097301ac3'
def main():
    parser=argparse.ArgumentParser();parser.add_argument('binary');parser.add_argument('--sha256',default=SHA);parser.add_argument('--receipt',required=True);args=parser.parse_args()
    binary=pathlib.Path(args.binary).resolve();actual_sha=hashlib.sha256(binary.read_bytes()).hexdigest();assert actual_sha==args.sha256
    cases=[]
    with tempfile.TemporaryDirectory(prefix='independent-checker-controls-') as temp:
        for name in ['wrong_buffer_selection','double_decoded_uri_selection']:
            work=pathlib.Path(temp)/name;work.mkdir();doc=document()
            (work/'uv.bin').write_bytes(UV);(work/'positions.bin').write_bytes(POS);(work/'pixels.png').write_bytes(png())
            if name=='wrong_buffer_selection':
                # Simulate selecting a valid alternative geometry resource.
                (work/'alternative.bin').write_bytes(struct.pack('<9f',12,21,31,14,21,31,12,24,31))
                doc['buffers'][1]['uri']='alternative.bin'
                doc['accessors'][1]['min']=[12,21,31];doc['accessors'][1]['max']=[14,24,31]
            else:
                # Correct URI image%2520.png selects image%20.png after one
                # decoding pass; corrupting the plan to image%20.png simulates
                # another pass, selecting a different, equally valid PNG.
                (work/'image%20.png').write_bytes(png())
                stream=io.BytesIO();Image.new('RGB',(2,1),(90,80,70)).save(stream,format='PNG')
                (work/'image .png').write_bytes(stream.getvalue())
                doc['images'][0]['uri']='image%20.png'
            root=work/'source.gltf';root.write_text(json.dumps(doc));out=work/'out.3tz'
            result=subprocess.run([str(binary),'mesh-local-to-3tz','--input',str(root),'--output',str(out),'--leaf-triangles','1','--json'],capture_output=True,text=True)
            assert result.returncode==0,result.stdout+result.stderr
            detected=False;reason=None
            try:inspect_archive(out,png())
            except AssertionError as error:detected=True;reason=str(error)
            assert detected,name
            cases.append({'control':name,'conversion_exit':result.returncode,'plausible_output':json.loads(result.stdout),'checker_detected':detected,'assertion_detail':reason})
    pathlib.Path(args.receipt).write_text(json.dumps({'binary':str(binary),'sha256':actual_sha,'method':'valid alternative source plans, not implementation mutation','cases':cases},indent=2)+'\n')
    print(json.dumps({'controls':len(cases),'detected':sum(row['checker_detected'] for row in cases)}))
if __name__=='__main__':main()
