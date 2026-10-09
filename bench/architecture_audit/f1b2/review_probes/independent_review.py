#!/usr/bin/env python3
"""Fresh manual fixtures and archive inspection. No repository oracle imports.

Stable source files during each subprocess preparation are a precondition.
This suite does not claim atomic multi-file snapshots or hostile-race security.
"""
import argparse, copy, hashlib, io, json, os, pathlib, struct, subprocess, tempfile, zipfile, zlib, urllib.parse

SHA = '0992615de230a84ac435a7aa97fea4c0324e547f1f2c6b8fb413e529d16d383b'
POSITIONS = ((11., 21., 31.), (13., 21., 31.), (11., 24., 31.))
POS = struct.pack('<9f', *(x for p in POSITIONS for x in p))
UV = struct.pack('<6f', 0., 0., 1., 0., 0., 1.)
def png():
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 2, 1, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(b'\0\xff\0\0\0\xff\0')) + chunk(b'IEND', b'')

def padded_png(length):
    image=png(); data=b'\0'*(length-len(image)-12); kind=b'ruSt'
    chunk=struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
    return image[:-12]+chunk+image[-12:]

def document():
    return {'asset': {'version': '2.0'}, 'buffers': [{'uri': 'uv.bin', 'byteLength': 24}, {'uri': 'positions.bin', 'byteLength': 36}],
            'bufferViews': [{'buffer': 0, 'byteLength': 24}, {'buffer': 1, 'byteLength': 36}],
            'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3, 'type': 'VEC2'},
                          {'bufferView': 1, 'componentType': 5126, 'count': 3, 'type': 'VEC3', 'min': [11,21,31], 'max': [13,24,31]}],
            'meshes': [{'primitives': [{'attributes': {'POSITION': 1, 'TEXCOORD_0': 0}, 'material': 0}]}],
            'nodes': [{'mesh': 0}], 'scenes': [{'nodes': [0]}], 'scene': 0,
            'materials': [{'pbrMetallicRoughness': {'baseColorTexture': {'index': 0}}}],
            'textures': [{'source': 0}], 'images': [{'uri': 'pixels.png'}]}

def glb(doc, payload=None):
    js = json.dumps(doc, separators=(',', ':')).encode(); js += b' ' * (-len(js) % 4)
    body = struct.pack('<II', len(js), 0x4e4f534a) + js
    if payload is not None:
        payload += b'\0' * (-len(payload) % 4)
        body += struct.pack('<II', len(payload), 0x004e4942) + payload
    return b'glTF' + struct.pack('<II', 2, 12 + len(body)) + body

def inspect_archive(output, expected_image):
    with zipfile.ZipFile(output) as archive:
        names = archive.namelist()
        reports = [json.loads(archive.read(n)) for n in names if n.endswith('conversion.json')]
        assert len(reports) == 1
        report = reports[0]
        image_names = [n for n in names if n.startswith('textures/')]
        assert len(image_names) == 1
        assert archive.read(image_names[0]) == expected_image
        positions = []; texcoords=[]
        for name in names:
            if not name.endswith('.glb'): continue
            data = archive.read(name)
            assert struct.unpack_from('<I', data, 8)[0] == len(data)
            jl = struct.unpack_from('<I', data, 12)[0]
            doc = json.loads(data[20:20+jl]); at = 20 + jl
            bl, kind = struct.unpack_from('<II', data, at)
            assert kind == 0x004e4942
            payload = data[at+8:at+8+bl]
            for img in doc.get('images', []):
                target = str(pathlib.PurePosixPath(name).parent / img['uri'])
                target = os.path.normpath(target)
                assert target in names, (target, names)
            for mesh in doc['meshes']:
                for primitive in mesh['primitives']:
                    accessor = doc['accessors'][primitive['attributes']['POSITION']]
                    view = doc['bufferViews'][accessor['bufferView']]
                    off = view.get('byteOffset',0)+accessor.get('byteOffset',0)
                    stride = view.get('byteStride',12)
                    positions.extend(struct.unpack_from('<3f', payload, off+i*stride) for i in range(accessor['count']))
                    uv_accessor=doc['accessors'][primitive['attributes']['TEXCOORD_0']]
                    uv_view=doc['bufferViews'][uv_accessor['bufferView']]
                    uv_off=uv_view.get('byteOffset',0)+uv_accessor.get('byteOffset',0)
                    uv_stride=uv_view.get('byteStride',8)
                    assert uv_accessor['componentType']==5126
                    texcoords.extend(struct.unpack_from('<2f',payload,uv_off+i*uv_stride) for i in range(uv_accessor['count']))
        assert sorted(positions) == sorted(POSITIONS), positions
        assert sorted(texcoords)==sorted(((0.,0.),(1.,0.),(0.,1.))),texcoords
        return {'report': report, 'members': names, 'positions': positions, 'image_sha256': hashlib.sha256(expected_image).hexdigest()}

def main():
    parser=argparse.ArgumentParser(); parser.add_argument('binary'); parser.add_argument('--sha256',default=SHA); parser.add_argument('--source-commit',default='42b895f0513f8a3aaa3f58879bcef2118adcfd77'); parser.add_argument('--receipt', required=True); args=parser.parse_args()
    binary=pathlib.Path(args.binary).resolve(); actual=hashlib.sha256(binary.read_bytes()).hexdigest()
    assert actual == args.sha256, (actual, args.sha256)
    rows=[]
    with tempfile.TemporaryDirectory(prefix='fresh-f1b2-review-') as temp:
        base=pathlib.Path(temp)
        def run(name, edit=None, setup=None, expected='ok', output=None, payload=None, as_glb=False, json_size=None):
            work=base/name; work.mkdir(); doc=document(); image=png()
            (work/'uv.bin').write_bytes(UV); (work/'positions.bin').write_bytes(POS); (work/'pixels.png').write_bytes(image)
            if edit: edit(doc)
            if setup: setup(work, doc)
            root=work/('source.glb' if as_glb else 'source.gltf')
            root_bytes=glb(doc,payload) if as_glb else json.dumps(doc).encode()+b'\n\t\r '
            if json_size is not None: root_bytes += b' ' * (json_size-len(root_bytes))
            root.write_bytes(root_bytes)
            out=output(work,root) if output else work/'out.3tz'
            before={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in work.rglob('*') if p.is_file()}
            before_entries={str(p.relative_to(work)) for p in work.rglob('*')}
            result=subprocess.run([str(binary),'mesh-local-to-3tz','--input',str(root),'--output',str(out),'--leaf-triangles','1','--json','--progress','json','--force'],cwd='/',capture_output=True,text=True)
            record={'case':name,'expected':expected,'exit':result.returncode,'stdout':result.stdout,'stderr':result.stderr}
            if result.returncode == 0:
                record['archive']=inspect_archive(out, (work/os.path.normpath(urllib.parse.unquote(doc['images'][0]['uri']))).read_bytes() if 'uri' in doc['images'][0] else image)
                declared_files={}
                for resource in doc['buffers']+doc['images']:
                    if 'uri' not in resource: continue
                    path=work/os.path.normpath(urllib.parse.unquote(resource['uri'])); stat=path.stat()
                    declared_files[(stat.st_dev,stat.st_ino)]=stat.st_size
                report=record['archive']['report']
                assert report['schema_version']==3 and report['profile']=='f1b-local-gltf-v1'
                assert report['source_bytes']==len(root_bytes)
                assert report['external_files']==len(declared_files)
                assert report['external_bytes']==sum(declared_files.values())
                assert report['images']==1 and report['image_pixels']==2
            else:
                record['source_preserved']=all(pathlib.Path(p).exists() and hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()==h for p,h in before.items())
                record['entries_after']=[str(p.relative_to(work)) for p in work.rglob('*')]
                assert record['source_preserved']
                assert set(record['entries_after'])==before_entries
                assert '"phase":"mesh_leaves"' not in result.stderr
            record['actual_kind']='ok' if result.returncode==0 else json.loads(result.stdout)['error']['kind']
            record['expectation_met']=record['actual_kind']==expected
            rows.append(record)
        run('multibuffer_external_png')
        run('glb_without_bin_external_buffers',as_glb=True)
        run('external_view_image_same_offset_other_buffer',edit=lambda d:(d['buffers'].append({'uri':'pixels.png','byteLength':len(png())}),d['bufferViews'].append({'buffer':2,'byteLength':len(png())}),d['images'].__setitem__(0,{'bufferView':2,'mimeType':'image/png'})))
        run('embedded_uv_external_positions',edit=lambda d:d['buffers'][0].pop('uri'),as_glb=True,payload=UV)
        run('buffer_trailing_bytes',setup=lambda w,d:(w/'positions.bin').write_bytes(POS+b'external trailing data'))
        run('hardlink_unused_dedup',edit=lambda d:d['buffers'].append({'uri':'alias.bin','byteLength':36}),setup=lambda w,d:os.link(w/'positions.bin',w/'alias.bin'))
        run('literal_percent_once',edit=lambda d:d['buffers'][1].__setitem__('uri','percent%2520.bin'),setup=lambda w,d:(w/'positions.bin').rename(w/'percent%20.bin'))
        def once_image_setup(w,d):
            (w/'pixels.png').rename(w/'image%20.png')
            from PIL import Image
            stream=io.BytesIO(); Image.new('RGB',(2,1),(90,80,70)).save(stream,format='PNG')
            (w/'image .png').write_bytes(stream.getvalue())
        run('image_percent_once_with_plausible_decoy',edit=lambda d:d['images'][0].__setitem__('uri','image%2520.png'),setup=once_image_setup)
        run('encoded_hash',edit=lambda d:d['buffers'][1].__setitem__('uri','position%23.bin'),setup=lambda w,d:(w/'positions.bin').rename(w/'position#.bin'))
        run('unicode_raw',edit=lambda d:d['buffers'][1].__setitem__('uri','日本.bin'),setup=lambda w,d:(w/'positions.bin').rename(w/'日本.bin'))
        run('contained_parent',edit=lambda d:d['buffers'][1].__setitem__('uri','unused/../positions.bin'))
        run('unused_bad_image',edit=lambda d:d['images'].append({'uri':'bad.png'}),setup=lambda w,d:(w/'bad.png').write_bytes(b'bad'),expected='invalid_input')
        run('unused_missing_buffer',edit=lambda d:d['buffers'].append({'uri':'missing.bin','byteLength':1}),expected='io')
        run('forbidden_uri_before_missing',edit=lambda d:(d['buffers'][0].__setitem__('uri','missing.bin'),d['images'][0].__setitem__('uri','data:image/png;base64,a')),expected='unsupported')
        run('extension_before_missing',edit=lambda d:(d['buffers'][0].__setitem__('uri','missing.bin'),d['nodes'].append({'extensions':{}})),expected='unsupported')
        run('mime_mismatch',edit=lambda d:d['images'][0].__setitem__('mimeType','image/jpeg'),expected='invalid_input')
        run('encoded_separator',edit=lambda d:d['buffers'][1].__setitem__('uri','sub%2fpositions.bin'),expected='unsupported')
        run('raw_space',edit=lambda d:d['buffers'][1].__setitem__('uri','a b.bin'),expected='invalid_input')
        run('raw_backslash',edit=lambda d:d['buffers'][1].__setitem__('uri','a\\b.bin'),expected='invalid_input')
        run('invalid_escape',edit=lambda d:d['buffers'][1].__setitem__('uri','bad%GG.bin'),expected='invalid_input')
        run('decoded_bad_utf8',edit=lambda d:d['buffers'][1].__setitem__('uri','bad%FF.bin'),expected='invalid_input')
        run('raw_portable_forbidden_angle',edit=lambda d:d['buffers'][1].__setitem__('uri','bad<.bin'),expected='invalid_input')
        run('encoded_portable_forbidden_angle',edit=lambda d:d['buffers'][1].__setitem__('uri','bad%3C.bin'),expected='unsupported')
        run('raw_reserved_plus',edit=lambda d:d['buffers'][1].__setitem__('uri','plus+.bin'),setup=lambda w,d:(w/'positions.bin').rename(w/'plus+.bin'),expected='invalid_input')
        run('encoded_reserved_plus',edit=lambda d:d['buffers'][1].__setitem__('uri','plus%2B.bin'),setup=lambda w,d:(w/'positions.bin').rename(w/'plus+.bin'))
        run('terminal_literal_dot',edit=lambda d:d['buffers'][1].__setitem__('uri','positions.bin/.'),expected='invalid_input')
        run('terminal_encoded_dot',edit=lambda d:d['buffers'][1].__setitem__('uri','positions.bin/%2e'),expected='invalid_input')
        run('terminal_decoded_parent',edit=lambda d:d['buffers'][1].__setitem__('uri','positions.bin/fake/%2e%2e'),expected='invalid_input')
        run('parent_escape',edit=lambda d:d['buffers'][1].__setitem__('uri','../positions.bin'),expected='unsupported')
        run('symlink_leaf',edit=lambda d:d['buffers'][1].__setitem__('uri','alias.bin'),setup=lambda w,d:os.symlink(w/'positions.bin',w/'alias.bin'),expected='unsupported')
        run('symlink_directory',edit=lambda d:d['buffers'][1].__setitem__('uri','alias/positions.bin'),setup=lambda w,d:os.symlink(w,w/'alias'),expected='unsupported')
        run('fifo_leaf',edit=lambda d:d['buffers'][1].__setitem__('uri','fifo.bin'),setup=lambda w,d:os.mkfifo(w/'fifo.bin'),expected='invalid_input')
        run('root_resource_alias',edit=lambda d:d['buffers'].append({'uri':'source.gltf','byteLength':1}),expected='unsupported')
        run('output_dependency_hardlink',output=lambda w,r:(os.link(w/'positions.bin',w/'alias.3tz') or w/'alias.3tz'),expected='invalid_request')
        run('output_root_hardlink',output=lambda w,r:(os.link(r,w/'alias.3tz') or w/'alias.3tz'),expected='invalid_request')
        run('output_descendant_of_root',output=lambda w,r:r/'out.3tz',expected='invalid_request')
        run('output_descendant_of_dependency',output=lambda w,r:w/'positions.bin'/'out.3tz',expected='invalid_request')
        run('min_exceeds_max_before_missing',edit=lambda d:(d['accessors'][1].__setitem__('min',[14,21,31]),d['buffers'][0].__setitem__('uri','missing.bin')),expected='invalid_input')
        run('unused_min_exceeds_max_before_missing',edit=lambda d:(d['accessors'].append({'bufferView':0,'componentType':5126,'count':3,'type':'VEC2','min':[2,2],'max':[1,1]}),d['buffers'][0].__setitem__('uri','missing.bin')),expected='invalid_input')
        run('inexact_accessor_extrema',edit=lambda d:(d['accessors'][1].__setitem__('min',[0,0,0]),d['accessors'][1].__setitem__('max',[100,100,100])),expected='invalid_input')
        run('float_extrema_component_rounding',edit=lambda d:(d['accessors'][1].__setitem__('min',[11.0000001,21.0000001,31.0000001]),d['accessors'][1].__setitem__('max',[13.0000001,24.0000001,31.0000001])))
        def normalized_uv(w,d,component,bits):
            maximum=(1<<bits)-1
            payload=bytes([0,0,0,0,255,0,0,0,0,255,0,0]) if bits==8 else struct.pack('<6H',0,0,maximum,0,0,maximum)
            (w/'uv.bin').write_bytes(payload)
            d['buffers'][0]['byteLength']=12;d['bufferViews'][0]['byteLength']=12;d['bufferViews'][0]['byteStride']=4
            d['accessors'][0].update({'componentType':component,'normalized':True,'min':[0,0],'max':[maximum,maximum]})
        run('normalized_u8_raw_extrema',setup=lambda w,d:normalized_uv(w,d,5121,8))
        run('normalized_u16_raw_extrema',setup=lambda w,d:normalized_uv(w,d,5123,16))
        def incorrect_normalized_extrema(w,d):
            normalized_uv(w,d,5121,8);d['accessors'][0]['max']=[1,1]
        run('normalized_u8_unit_extrema_invalid',setup=incorrect_normalized_extrema,expected='invalid_input')
        run('json_ceiling_exact',json_size=1024*1024)
        run('json_ceiling_plus_one',json_size=1024*1024+1,expected='unsupported')
        run('dependency_ceiling_exact',setup=lambda w,d:(w/'positions.bin').write_bytes(POS+b'\0'*(32*1024*1024-len(POS))))
        run('dependency_ceiling_plus_one',setup=lambda w,d:(w/'positions.bin').write_bytes(POS+b'\0'*(32*1024*1024+1-len(POS))),expected='unsupported')
        run('aggregate_unique_ceiling_exceeded',edit=lambda d:d['buffers'].append({'uri':'unused.bin','byteLength':1}),setup=lambda w,d:((w/'positions.bin').write_bytes(POS+b'\0'*(32*1024*1024-len(POS))),(w/'unused.bin').write_bytes(b'\0'*(32*1024*1024))),expected='unsupported')
        run('declared_buffer_ceiling_before_missing',edit=lambda d:(d['buffers'][1].__setitem__('byteLength',32*1024*1024),d['buffers'][0].__setitem__('uri','missing.bin')),expected='unsupported')
        run('image_count_32_aliases',edit=lambda d:d['images'].extend([{'uri':'pixels.png'}]*31))
        run('image_count_33_before_missing',edit=lambda d:(d['images'].extend([{'uri':'pixels.png'}]*32),d['buffers'][0].__setitem__('uri','missing.bin')),expected='unsupported')
        run('copied_image_bytes_exact',edit=lambda d:d['images'].extend([{'uri':'pixels.png'}]*31),setup=lambda w,d:(w/'pixels.png').write_bytes(padded_png(1024*1024)))
        run('copied_image_bytes_exceeded',edit=lambda d:d['images'].extend([{'uri':'pixels.png'}]*31),setup=lambda w,d:(w/'pixels.png').write_bytes(padded_png(1024*1024+1)),expected='unsupported')
        run('uri_components_exact_128',edit=lambda d:d['buffers'][1].__setitem__('uri','./'*127+'positions.bin'))
        run('uri_components_129',edit=lambda d:d['buffers'][1].__setitem__('uri','./'*128+'positions.bin'),expected='unsupported')
        from PIL import Image
        def jpeg_setup(w,d):
            stream=io.BytesIO(); Image.new('RGB',(2,1),(200,30,40)).save(stream,format='JPEG'); (w/'pixels.jpg').write_bytes(stream.getvalue()); d['images'][0]['uri']='pixels.jpg'
        run('jpeg_exact_forward',setup=jpeg_setup)
    receipt={'binary':str(binary),'sha256':actual,'source_commit':args.source_commit,'fixtures':'manual stdlib glTF/buffers/PNG; Pillow generated JPEG; no oracle imports','stable_source_precondition':True,'cases':rows}
    pathlib.Path(args.receipt).write_text(json.dumps(receipt,indent=2)+'\n')
    print(json.dumps({'cases':len(rows),'mismatches':[{k:r[k] for k in ('case','expected','actual_kind')} for r in rows if not r['expectation_met']]},indent=2))

if __name__=='__main__':main()
