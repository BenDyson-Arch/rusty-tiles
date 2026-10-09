"""Independent F1b1 review probes, authored separately from production and acceptance oracle.

PNG/GLB bytes use struct, CRC32 and zlib; Pillow owns only JPEG encode/decode provenance.
No production or test-oracle imports. Generate and check use explicit fresh work paths.
"""
import argparse, hashlib, io, json, pathlib, struct, subprocess, time, zipfile, zlib
from pathlib import Path
import PIL
from PIL import Image

def chunk(kind,data):return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
def png(depth=8):
 raw=b'\0'+(b'\xff\x00\x00\xff' if depth==8 else b'\xff\xff\0\0\0\0\xff\xff')
 return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,depth,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')
def glb(doc,bin):
 text=json.dumps(doc,separators=(',',':')).encode();text+=b' '*(-len(text)%4);bin+=bytes(-len(bin)%4)
 return struct.pack('<4sII',b'glTF',2,28+len(text)+len(bin))+struct.pack('<I4s',len(text),b'JSON')+text+struct.pack('<I4s',len(bin),b'BIN\0')+bin

def generate_basic(work):
 out=Path(work)
 for depth in [8,16]:
  pos=struct.pack('<9f',0,0,0,1,0,0,0,1,0);uv=struct.pack('<6f',0,0,1,0,0,1);im=png(depth);bin=pos+uv+im
  doc={'asset':{'version':'2.0'},'buffers':[{'byteLength':len(bin)}],'bufferViews':[{'buffer':0,'byteLength':36},{'buffer':0,'byteOffset':36,'byteLength':24},{'buffer':0,'byteOffset':60,'byteLength':len(im)}],'accessors':[{'bufferView':0,'componentType':5126,'type':'VEC3','count':3,'min':[0,0,0],'max':[1,1,0]},{'bufferView':1,'componentType':5126,'type':'VEC2','count':3}],'images':[{'bufferView':2,'mimeType':'image/png'}],'textures':[{'source':0}],'materials':[{'pbrMetallicRoughness':{'baseColorTexture':{'index':0}}}],'meshes':[{'primitives':[{'attributes':{'POSITION':0,'TEXCOORD_0':1},'material':0}]}],'nodes':[{'mesh':0}],'scenes':[{'nodes':[0]}],'scene':0}
  (out/('rgba'+str(depth)+'.glb')).write_bytes(glb(doc,bin))
 for count in [1,64,512]:
  n=60000;bin=bytes(n*20)+struct.pack('<3H',0,1,2)
  doc={'asset':{'version':'2.0'},'buffers':[{'byteLength':len(bin)}],'bufferViews':[{'buffer':0,'byteLength':n*12},{'buffer':0,'byteOffset':n*12,'byteLength':n*8},{'buffer':0,'byteOffset':n*20,'byteLength':6}],'accessors':[{'bufferView':0,'componentType':5126,'type':'VEC3','count':n,'min':[0,0,0],'max':[0,0,0]},{'bufferView':1,'componentType':5126,'type':'VEC2','count':n},{'bufferView':2,'componentType':5123,'type':'SCALAR','count':3}],'meshes':[{'primitives':[{'attributes':{'POSITION':0,'TEXCOORD_0':1},'indices':2} for _ in range(count)]}],'nodes':[{'mesh':0}],'scenes':[{'nodes':[0]}],'scene':0}
  (out/('uv_reuse_'+str(count)+'.glb')).write_bytes(glb(doc,bin))

def generate_boundaries(work):
 out=Path(work);b=(out/'rgba8.glb').read_bytes();n=int.from_bytes(b[12:16],'little');doc=json.loads(b[20:20+n]);start=20+n;bin=b[start+8:start+8+int.from_bytes(b[start:start+4],'little')]
 def emit(name,image,mime='image/png'):
  d=json.loads(json.dumps(doc));d['bufferViews'][2]['byteLength']=len(image);d['buffers'][0]['byteLength']=60+len(image);d['images'][0]['mimeType']=mime;(out/(name+'.glb')).write_bytes(glb(d,bin[:60]+image))
 p=png();emit('png_truncated_pixels',p[:40]);emit('png_missing_iend',p[:-12]);emit('png_bad_iend_crc',p[:-1]+bytes([p[-1]^1]));validtext=chunk(b'tEXt',b'author\0independent probe');badtext=validtext[:-1]+bytes([validtext[-1]^1]);emit('png_ancillary_crc_control',p[:-12]+validtext+p[-12:]);emit('png_bad_ancillary_crc',p[:-12]+badtext+p[-12:]);emit('png_one_frame_apng',p[:33]+chunk(b'acTL',struct.pack('>II',1,0))+chunk(b'fcTL',struct.pack('>IIIIIHHBB',0,1,1,0,0,1,10,0,0))+p[33:]);emit('png_after_iend',p+chunk(b'tEXt',b'author\0trailing'))
 buf=io.BytesIO();Image.new('RGB',(1,1),(255,0,0)).save(buf,format='JPEG',quality=93);jpeg=buf.getvalue();assert jpeg[-2:]==b'\xff\xd9';emit('jpeg_control',jpeg,'image/jpeg');emit('jpeg_missing_eoi',jpeg[:-2],'image/jpeg');emit('jpeg_truncated_pixels',jpeg[:300],'image/jpeg')
 Image.open(io.BytesIO(p[:-12]+validtext+p[-12:])).load();Image.open(io.BytesIO(jpeg)).load();print('Authored PNG positive pixels and Pillow JPEG independently decoded; Pillow',__import__('PIL').__version__)

def generate_closure(work):
 out=Path(work);b=(out/'rgba8.glb').read_bytes();n=int.from_bytes(b[12:16],'little');d=json.loads(b[20:20+n]);start=20+n;bin=b[start+8:start+8+int.from_bytes(b[start:start+4],'little')];image=bin[60:60+d['bufferViews'][2]['byteLength']]
 d['images']=[{'bufferView':2,'mimeType':'image/png'} for _ in range(3)];d['samplers']=[{'wrapS':10497,'wrapT':33071,'magFilter':9728,'minFilter':9986},{'wrapS':33648}];d['textures']=[{'source':0},{'source':2,'sampler':1},{'source':2}];d['materials']=[{'pbrMetallicRoughness':{'baseColorTexture':{'index':0}}},{'pbrMetallicRoughness':{'baseColorTexture':{'index':1,'texCoord':0}},'alphaMode':'MASK','alphaCutoff':0.25},{'pbrMetallicRoughness':{'baseColorTexture':{'index':2}}}];d['meshes']=[{'primitives':[{'attributes':{'POSITION':0,'TEXCOORD_0':1},'material':m}]} for m in range(3)];d['nodes']=[{'mesh':0},{'mesh':1,'scale':[-1,1,1]},{'mesh':2,'translation':[3,0,0]}];d['scenes']=[{'nodes':[1,2]},{'nodes':[0]}]
 (out/'selected_closure_reflection.glb').write_bytes(glb(d,bin))

def verify_closure(binary, work):
 out=Path(work);suffix="replay";image=png()
 dest=out/('selected_closure_reflection-'+suffix+'.3tz');r=subprocess.run([binary,'--json','mesh-local-to-3tz','--input',str(out/'selected_closure_reflection.glb'),'--output',str(dest),'--leaf-triangles','1','--force'],capture_output=True,text=True,timeout=30);assert r.returncode==0,r.stdout
 with zipfile.ZipFile(dest) as z:
  names=set(z.namelist());assert names=={'tileset.json','conversion.json','textures/2.png','t/0.glb','t/1.glb','@3dtilesIndex1@'},names;assert z.read('textures/2.png')==image
  rep=json.loads(z.read('conversion.json'));assert(rep['images'],rep['image_bytes'],rep['image_pixels'])==(1,len(image),1)
  corners=[];materials=[]
  for name in ['t/0.glb','t/1.glb']:
   v=z.read(name);n=int.from_bytes(v[12:16],'little');doc=json.loads(v[20:20+n]);begin=20+n;data=v[begin+8:begin+8+int.from_bytes(v[begin:begin+4],'little')];p=doc['meshes'][0]['primitives'][0]
   assert len(doc['materials'])==len(doc['textures'])==len(doc['images'])==1;assert doc['images'][0]['uri']=='../textures/2.png';mat=doc['materials'][0];tex=doc['textures'][0];assert mat['pbrMetallicRoughness']['baseColorTexture']['index']==0;assert tex['source']==0
   if mat.get('alphaMode')=='MASK':assert doc['samplers']==[{'wrapS':33648}] and tex['sampler']==0 and mat['pbrMetallicRoughness']['baseColorTexture']['texCoord']==0
   else:assert 'sampler' not in tex and 'samplers' not in doc and 'texCoord' not in mat['pbrMetallicRoughness']['baseColorTexture']
   attrs={}
   for semantic,width in [('POSITION',3),('TEXCOORD_0',2)]:
    a=doc['accessors'][p['attributes'][semantic]];view=doc['bufferViews'][a['bufferView']];offset=view.get('byteOffset',0)+a.get('byteOffset',0);attrs[semantic]=[struct.unpack_from('<'+'f'*width,data,offset+i*width*4) for i in range(a['count'])]
   corners.append(list(zip(attrs['POSITION'],attrs['TEXCOORD_0'])))
 expected=[[((0.,0.,0.),(0.,0.)),((0.,1.,0.),(0.,1.)),((-1.,0.,0.),(1.,0.))],[((3.,0.,0.),(0.,0.)),((4.,0.,0.),(1.,0.)),((3.,1.,0.),(0.,1.))]]
 assert sorted(corners)==sorted(expected),(corners,expected)
 return {'name':'selected_closure_reflection','exit':r.returncode,'exactInventory':sorted(names),'uniqueImageBytes':len(image),'corners':corners,'stdout':r.stdout}

CASES = [('rgba8', 0), ('rgba16', 2), ('png_missing_iend', 3),
         ('png_bad_iend_crc', 3), ('png_ancillary_crc_control', 0),
         ('png_bad_ancillary_crc', 3), ('png_one_frame_apng', 2),
         ('png_after_iend', 3), ('png_truncated_pixels', 3),
         ('jpeg_control', 0), ('jpeg_missing_eoi', 3),
         ('jpeg_truncated_pixels', 3), ('uv_reuse_1', 0),
         ('uv_reuse_64', 0), ('uv_reuse_512', 0)]

def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def generate(work):
    work = Path(work)
    work.mkdir(parents=True, exist_ok=False)
    generate_basic(work)
    generate_boundaries(work)
    generate_closure(work)
    manifest = {
        'generatorSha256': digest(__file__),
        'inputs': {p.name: digest(p) for p in sorted(work.glob('*.glb'))},
        'jpegProvenance': {'encoderAndIndependentDecoder': 'Pillow',
                           'version': PIL.__version__, 'dimensions': [1, 1],
                           'sourcePixels': [[255, 0, 0]], 'quality': 93},
    }
    (work / 'inputs.json').write_text(json.dumps(manifest, indent=2) + '\n')
    return manifest

def exact_forwarding(work, case, extension, cli_report):
    source = (work / (case + '.glb')).read_bytes()
    length = int.from_bytes(source[12:16], 'little')
    doc = json.loads(source[20:20 + length])
    start = 20 + length
    binary = source[start + 8:start + 8 + int.from_bytes(source[start:start + 4], 'little')]
    view = doc['bufferViews'][doc['images'][0]['bufferView']]
    offset = view.get('byteOffset', 0)
    encoded = binary[offset:offset + view['byteLength']]
    with zipfile.ZipFile(work / (case + '-replay.3tz')) as archive:
        assert archive.read('textures/0.' + extension) == encoded
        assert json.loads(archive.read('conversion.json')) == cli_report
    return {'case': case, 'imageBytes': len(encoded),
            'imageSha256': hashlib.sha256(encoded).hexdigest(),
            'conversionReportEqualsCli': True}

def check(binary, work, output, expected_sha=None):
    binary, work = Path(binary).resolve(), Path(work).resolve()
    binary_sha = digest(binary)
    if expected_sha is not None:
        assert binary_sha == expected_sha, (binary_sha, expected_sha)
    manifest = json.loads((work / 'inputs.json').read_text())
    assert manifest['generatorSha256'] == digest(__file__), 'generator changed after generation'
    for name, expected in manifest['inputs'].items():
        assert digest(work / name) == expected, ('input changed', name)
    rows = []
    forwarding = []
    for name, expected in CASES:
        destination = work / (name + '-replay.3tz')
        started = time.monotonic()
        result = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz',
                                 '--input', str(work / (name + '.glb')),
                                 '--output', str(destination), '--leaf-triangles',
                                 '100000', '--force'], capture_output=True,
                                text=True, timeout=30)
        elapsed = time.monotonic() - started
        report = json.loads(result.stdout)
        assert result.returncode == expected, (name, result.returncode, result.stdout)
        if expected:
            assert report['error']['code'] == ('unsupported' if expected == 2 else 'invalid_input')
        rows.append({'name': name, 'inputSha256': digest(work / (name + '.glb')),
                     'expectedExit': expected, 'exit': result.returncode,
                     'seconds': elapsed, 'report': report, 'stderr': result.stderr})
        if name in {'rgba8', 'png_ancillary_crc_control', 'jpeg_control'}:
            forwarding.append(exact_forwarding(work, name, 'jpg' if name == 'jpeg_control' else 'png',
                                                report['meshReport']))
        print(name, result.returncode, round(elapsed, 4), flush=True)
    rows.append(verify_closure(str(binary), work))
    receipt = {'binary': str(binary), 'binarySha256': binary_sha,
               'generatorSha256': digest(__file__), 'inputManifest': manifest,
               'rows': rows, 'exactImageForwarding': forwarding}
    Path(output).write_text(json.dumps(receipt, indent=2) + '\n')
    return receipt

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    gen = commands.add_parser('generate')
    gen.add_argument('--work', required=True, type=Path)
    run = commands.add_parser('check')
    run.add_argument('--work', required=True, type=Path)
    run.add_argument('--binary', required=True, type=Path)
    run.add_argument('--output', required=True, type=Path)
    run.add_argument('--expected-sha256')
    args = parser.parse_args()
    if args.command == 'generate':
        generate(args.work)
    else:
        check(args.binary, args.work, args.output, args.expected_sha256)
