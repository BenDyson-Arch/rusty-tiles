#!/usr/bin/env python3
"""Bounded published-byte packed/mixed/synthetic geometry seam witness.
Uses only independent GLB/source readers, never production tree/lattice helpers.
Constant five-slot textures exercise plumbing, not high-frequency fidelity.
"""
import argparse
import hashlib
import json
import pathlib
import struct
import math
import leaf_oracle as wire


def generate(directory):
    directory.mkdir(parents=True,exist_ok=True)
    shared=[wire.f32(x) for x in (-.00137,.00273,-.00391)]
    centers=[(1,1,1),(-40,-40,-40),(40,-40,-40),(-40,40,-40),(-40,-40,40)]
    points=[shared]
    templates=[]
    for c in centers:
        ids=[0,len(points),len(points)+1]
        points.extend([[wire.f32(1.5*c[a]-.5*shared[a]+(.125 if a == 0 else 0)) for a in range(3)],
                       [wire.f32(1.5*c[a]-.5*shared[a]-(.125 if a == 0 else 0)) for a in range(3)]])
        templates.append(ids)
    groups=[40000,1000,1000,1000,1000]
    views,accessors=[],[]
    with (directory/'payload.bin').open('wb') as stream:
        def field(rows,width,shape):
            start=stream.tell()
            for row in rows: stream.write(struct.pack('<'+'f'*width,*row))
            views.append({'buffer':0,'byteOffset':start,'byteLength':stream.tell()-start})
            accessors.append({'bufferView':len(views)-1,'componentType':5126,'count':len(rows),'type':shape})
        field(points,3,'VEC3')
        accessors[0].update(min=[min(p[a] for p in points) for a in range(3)],max=[max(p[a] for p in points) for a in range(3)])
        field([[0,0,1]]*len(points),3,'VEC3')
        field([[1,0,0,1]]*len(points),4,'VEC4')
        field([[.5,.5]]*len(points),2,'VEC2')
        field([[.2,.4,.6,1]]*len(points),4,'VEC4')
        start=stream.tell()
        for ids,count in zip(templates,groups):
            # At most480k temporary bytes, never an expanded geometry list.
            stream.write(struct.pack('<III',*ids)*count)
        views.append({'buffer':0,'byteOffset':start,'byteLength':stream.tell()-start})
        accessors.append({'bufferView':5,'componentType':5125,'count':sum(groups)*3,'type':'SCALAR'})
        length=stream.tell()
    images=wire.image_set(directory,constant=True)
    material=wire.material(0,1)
    for slot in range(5): wire.slot_binding(material,slot)['texCoord']=0
    sampler={'wrapS':33071,'wrapT':33071,'magFilter':9728,'minFilter':9728}
    doc={'asset':{'version':'2.0'},'scene':0,'scenes':[{'nodes':[0]}],'nodes':[{'mesh':0}],
         'meshes':[{'primitives':[{'attributes':{'POSITION':0,'NORMAL':1,'TANGENT':2,'TEXCOORD_0':3,'COLOR_0':4},'indices':5,'material':0}]}],
         'buffers':[{'uri':'payload.bin','byteLength':length}],'bufferViews':views,'accessors':accessors,
         'images':[{'uri':i['uri']} for i in images],'textures':[{'source':i,'sampler':0} for i in range(5)],'samplers':[sampler],'materials':[material]}
    wire.dump(directory/'source.gltf',doc)
    truth={'schemaVersion':1,'triangleCount':sum(groups),'groups':groups,'templates':templates,'points':points,
           'inputSha256':wire.sha(directory/'source.gltf'),'assets':{name:wire.sha(directory/name) for name in ['payload.bin',*(i['uri'] for i in images)]},
           'scope':'packed/mixed/synthetic geometry seams; constant five-slot textures, no detailed appearance claim'}
    wire.dump(directory/'seam-truth.json',truth)
    return {'input':str(directory/'source.gltf'),'truth':str(directory/'seam-truth.json'),'triangles':sum(groups),'payloadBytes':length}


def bits(point):
    return struct.pack('<3f',*point)


def check_position(source,actual,depth,minimum,steps,profile):
    wanted=source if profile in ('quality','lossless') else [wire.f32(round(x/steps[min(depth,minimum)])*steps[min(depth,minimum)]) for x in source]
    wire.require(bits(actual) == bits(wanted),'source-derived minimum-incident-depth decoded f32 position bits')


def verify(truth_path,output,controls):
    truth=json.loads(truth_path.read_text())
    wire.require(wire.sha(truth_path.parent/'source.gltf') == truth['inputSha256'],'source document hash')
    for name,digest in truth['assets'].items(): wire.require(wire.sha(truth_path.parent/name) == digest,'source asset hash')
    manifest=json.loads((output/'manifest.json').read_text())
    profile=manifest['report']['profile']; steps=manifest['lattice']['steps']
    wire.require(manifest['lattice']['common_origin'] == [0,0,0],'shared zero origin')
    total=truth['triangleCount']; seen=bytearray((total+7)//8)
    incident=[{} for _ in truth['points']]
    packed=synthetic=leaves=observed=0; decoder_hash=None
    starts=[]; accumulated=0
    for count in truth['groups']: starts.append(accumulated); accumulated+=count
    for line in (output/'leaves.jsonl').open():
        receipt=json.loads(line); path=(output/receipt['uri']).resolve()
        wire.require(path.is_relative_to(output.resolve()),'member path confinement')
        wire.require(wire.sha(path) == receipt['sha256'] and path.stat().st_size == receipt['bytes'],'published wire hash/size')
        doc,views,packet=wire.decoded(path)
        if decoder_hash: wire.require(packet['decoderSha256'] == decoder_hash,'one exact meshopt decoder')
        decoder_hash=packet['decoderSha256']
        packed+=int(receipt['packed']); synthetic+=int(receipt['depth'] > 21); leaves+=1
        tables=wire.metadata(doc,views); count=0
        for node in doc['nodes']:
            primitive=doc['meshes'][node['mesh']]['primitives'][0]
            wire.check_features(primitive['extensions']['EXT_mesh_features']['featureIds'],tables)
            attrs=primitive['attributes']; positions=wire.accessor(doc,views,attrs['POSITION']); ids=wire.accessor(doc,views,attrs['_FEATURE_ID_1'])
            wire.require(doc['accessors'][attrs['POSITION']]['componentType'] == (5123 if profile in ('fast','balanced') else 5126),'profile actual position storage')
            wire.require(len(positions)%3 == 0 and len(ids) == len(positions),'whole source corners')
            translation=node.get('translation',[0,0,0]); scale=node.get('scale',[1,1,1])
            for t in range(len(positions)//3):
                tid=int(ids[t*3][0]); wire.require([row[0] for row in ids[t*3:t*3+3]] == [tid]*3 and 0 <= tid < tables[1][0],'actual triangle feature bounds/allcorners')
                columns=tables[1][1]; ordinal=columns['triangle_index'][tid]
                wire.require([columns[n][tid] for n in ('node_index','mesh_index','primitive_index')] == [0,0,0] and 0 <= ordinal < total,'source numeric identity')
                wire.require(not seen[ordinal//8]&(1<<(ordinal%8)),'exact-once triangle coverage');seen[ordinal//8]|=1<<(ordinal%8)
                group=max(i for i,start in enumerate(starts) if start <= ordinal); original_ids=truth['templates'][group]
                wire.require([columns[f'vertex_{c}'][tid] for c in range(3)] == original_ids,'authored source corner identity')
                for corner,source_id in enumerate(original_ids):
                    local=positions[t*3+corner]
                    actual=[wire.f32(wire.f32(local[a]*scale[a])+translation[a]) for a in range(3)]
                    wire.require(math.dist(actual,truth['points'][source_id]) <= receipt['geometryError'],'actual source correspondence bound')
                    wire.require(all(receipt['bounds']['min'][a] <= actual[a] <= receipt['bounds']['max'][a] for a in range(3)),'decoded actual leaf bounds')
                    # Only11source points; retain one exact decoded value per incidentdepth.
                    depth=receipt['depth']; prior=incident[source_id].get(depth)
                    wire.require(prior is None or bits(prior) == bits(actual),'within-depth cross-leaf exact decode identity')
                    incident[source_id][depth]=actual
                observed+=1;count+=1
        wire.require(count == receipt['end']-receipt['first'],'actual leaf source rank span')
    wire.require(observed == total == manifest['report']['triangles'],'complete actual source coverage')
    wire.require(leaves == manifest['report']['leaves'] and packed > 0 and synthetic > 0,'required packed and synthetic>21 actual leaf witnesses')
    mixed=unequal=0
    for source_id,depths in enumerate(incident):
        wire.require(depths,'every source position observed')
        minimum=min(depths)
        mixed+=int(len(depths)>1)
        unequal+=int(len({steps[d] for d in depths})>1)
        reference=None
        for depth,actual in depths.items():
            check_position(truth['points'][source_id],actual,depth,minimum,steps,profile)
            if reference is not None: wire.require(bits(actual) == bits(reference),'mixed-depth actual decoded seam bits')
            reference=actual
    wire.require(mixed > 0,'required mixed-depth shared source position')
    if profile in ('fast','balanced'): wire.require(unequal > 0,'required genuinely unequal lattice-step incidence')
    if controls:
        minimum=min(incident[0]); depth=max(incident[0]); mutant=incident[0][depth][:]
        mutant[0]=wire.f32(mutant[0]+max(steps[minimum],.001))
        try: check_position(truth['points'][0],mutant,depth,minimum,steps,profile)
        except AssertionError: pass
        else: raise AssertionError('mutated onecorner seam control insensitive')
    result={'verdict':'PASS','profile':profile,'triangles':observed,'leaves':leaves,'packedLeaves':packed,'syntheticLeaves':synthetic,'mixedDepthSourcePositions':mixed,'unequalStepSourcePositions':unequal,'controlsExecuted':controls,'oracleSha256':wire.sha(__file__),'sharedReaderSha256':wire.sha(wire.__file__),'decoderSha256':decoder_hash,'truthSha256':wire.sha(truth_path),'leafManifestSha256':wire.sha(output/'leaves.jsonl'),'rendererAcceptance':False,'appearanceAcceptance':False}
    result['passHash']=hashlib.sha256(json.dumps(result,sort_keys=True,separators=(',',':')).encode()).hexdigest()
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__); commands=parser.add_subparsers(dest='command',required=True)
    gen=commands.add_parser('generate');gen.add_argument('directory',type=pathlib.Path)
    check=commands.add_parser('verify');check.add_argument('truth',type=pathlib.Path);check.add_argument('output',type=pathlib.Path);check.add_argument('--controls',action='store_true');check.add_argument('--report',type=pathlib.Path)
    args=parser.parse_args()
    if args.command == 'generate': result=generate(args.directory)
    else:
        result=verify(args.truth,args.output,args.controls)
        if args.report: wire.dump(args.report,result)
    print(json.dumps(result,sort_keys=True))


if __name__ == '__main__': main()
