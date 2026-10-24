#!/usr/bin/env python3
"""Actual KTX complete-level image gate for independently constant RGBA8 sources.
One physical GLB and one decoded RGBA mip at a time; no geometry/renderer/MASK
coverage claim. No producer/reference helper is imported. Cached measurements
may be reused only on exact encoded-image hash, role, expected bytes and profile.
"""
import argparse
import collections
import copy
import hashlib
import json
import math
import pathlib
import struct
import zlib

SLOTS=('baseColorTexture','metallicRoughnessTexture','normalTexture','occlusionTexture','emissiveTexture')
ROOT=pathlib.Path(__file__).resolve().parents[3]


def require(condition,message):
    if not condition: raise AssertionError(message)


def sha(path):
    h=hashlib.sha256()
    with pathlib.Path(path).open('rb') as stream:
        for block in iter(lambda:stream.read(1<<20),b''): h.update(block)
    return h.hexdigest()


def png_constant(path):
    """Decode filtered PNG rows independently; never allocate an entire source RGBA image."""
    require(path.stat().st_size <= 32<<20,'encoded source PNG ceiling')
    compressed=[]; dims=None; done=False
    with path.open('rb') as stream:
        require(stream.read(8) == b'\x89PNG\r\n\x1a\n','source PNG signature')
        while not done:
            header=stream.read(8); require(len(header)==8,'PNG chunk header')
            length,kind=struct.unpack('>I4s',header);require(length <= 32<<20,'PNG chunk bound')
            payload=stream.read(length);crc=stream.read(4)
            require(len(payload)==length and len(crc)==4 and zlib.crc32(kind+payload)&0xffffffff == struct.unpack('>I',crc)[0],'PNG chunk CRC/closure')
            if kind == b'IHDR':
                require(dims is None and length == 13,'PNG IHDR inventory')
                w,h,depth,color,compression,filtering,interlace=struct.unpack('>IIBBBBB',payload)
                require(0<w<=4096 and 0<h<=4096 and w*h<=4_194_304,'source image dimensions/admission')
                require((depth,color,compression,filtering,interlace)==(8,6,0,0,0),'constant oracle domain: RGBA8 noninterlaced PNG')
                dims=(w,h)
            elif kind == b'IDAT': compressed.append(payload)
            elif kind == b'IEND': require(length == 0,'PNG IEND');done=True
        require(stream.read(1)==b'','source PNG trailing bytes')
    require(dims is not None and compressed,'source PNG required chunks')
    w,h=dims; row_size=w*4; previous=bytes(row_size); pending=bytearray(); rows=0; pixel=None
    inflater=zlib.decompressobj();decoded_bytes=0
    def accept(block):
        nonlocal previous,rows,pixel,decoded_bytes
        decoded_bytes+=len(block)
        require(decoded_bytes<=h*(row_size+1),'PNG inflated byte ceiling')
        pending.extend(block)
        while len(pending)>=row_size+1:
            filtering=pending[0];raw=bytes(pending[1:row_size+1]);del pending[:row_size+1]
            require(rows<h and filtering<=4,'PNG row/filter bound')
            if filtering == 0: row=raw
            else:
                row=bytearray(row_size)
                for i,value in enumerate(raw):
                    a=row[i-4] if i>=4 else 0;b=previous[i];c=previous[i-4] if i>=4 else 0
                    p=a+b-c;pa,pb,pc=abs(p-a),abs(p-b),abs(p-c)
                    predictor=(0,a,b,(a+b)//2,a if pa<=pb and pa<=pc else b if pb<=pc else c)[filtering]
                    row[i]=(value+predictor)&255
                row=bytes(row)
            if pixel is None: pixel=row[:4]
            require(row == pixel*w,'original source PNG is constant at every texel')
            previous=row;rows+=1
    for chunk in compressed:
        tail=chunk
        while True:
            block=inflater.decompress(tail,65536);tail=inflater.unconsumed_tail;accept(block)
            if not tail and len(block)<65536: break
    require(inflater.eof and not inflater.unused_data and not pending and rows==h,'PNG deflate/row closure')
    return w,h,tuple(pixel)


def glb(path):
    require(path.stat().st_size <= 96<<20,'physical leaf96MiB ceiling')
    data=path.read_bytes();require(data[:4]==b'glTF' and struct.unpack_from('<II',data,4)==(2,len(data)),'actual GLB2 header')
    length,kind=struct.unpack_from('<II',data,12);require(kind==0x4e4f534a and length%4==0,'GLB JSON chunk')
    end=20+length;doc=json.loads(data[20:end]);size,kind=struct.unpack_from('<II',data,end)
    require(kind==0x004e4942 and size%4==0 and end+8+size==len(data),'GLB BIN closure')
    return data,doc,memoryview(data)[end+8:]


def slot(material,role):
    return (material['pbrMetallicRoughness'] if role<2 else material)[SLOTS[role]]


def literal_material(material):
    value=copy.deepcopy(material);value.pop('name',None)
    for role in range(5):
        info=slot(value,role);info.pop('index');info.pop('texCoord',None)
    return value


def source_material_index(output_material,bindings,source_doc):
    matches=[]
    for i,original in enumerate(source_doc['materials']):
        if literal_material(output_material)!=literal_material(original): continue
        good=True
        for binding in bindings:
            role=binding['slot'];info=slot(original,role);texture=source_doc['textures'][info['index']]
            good &= (texture['source']==binding['sourceImage'] and info.get('texCoord',0)==binding['sourceUV']
                     and source_doc['samplers'][texture['sampler']]==binding['sourceSampler'])
        if good: matches.append(i)
    require(matches,'literal original PBR factors/normalScale/material and source binding closure')
    return matches[0] # Authored aliases with identical literal contexts are equivalent for this image gate.


def unit_byte(pixel):
    vector=[v/127.5-1 for v in pixel[:3]];length=math.sqrt(sum(v*v for v in vector))
    require(length>1e-12,'source constant normal nonzero')
    return tuple(min(255,max(0,math.floor((v/length+1)*127.5+.5))) for v in vector)+(pixel[3],)


def metrics(pixel,data,role,profile):
    require(len(data)>0 and len(data)%4==0,'decoded RGBA byte shape')
    channels=(0,3) if role==3 else (1,2,3) if role==1 else (0,1,2,3)
    result=[];count=len(data)//4
    for channel in channels:
        histogram=collections.Counter(data[channel::4])
        mse=sum((v-pixel[channel])**2*n for v,n in histogram.items())/count
        psnr=math.inf if mse==0 else 10*math.log10(65025/mse);maximum=max(abs(v-pixel[channel]) for v in histogram)
        if channel==3: floor,cap=(34,32) if profile=='quality' else (30,80 if profile=='fast' else 64)
        elif role in (0,4): floor,cap=(34,24) if profile=='quality' else (22,56 if profile=='fast' else 32)
        elif role==2: floor,cap=30,112
        else: floor,cap=30,(8 if profile=='quality' else 80) if role==3 else 32
        require(psnr>=floor and maximum<=cap,f'constant actual role{role} channel{channel}: PSNR{psnr} max{maximum}, fixedfloor{floor} cap{cap}')
        result.append({'channel':channel,'psnr':None if math.isinf(psnr) else psnr,'max':maximum,'floor':floor,'cap':cap})
    normal=None
    if role==2:
        histogram=collections.Counter()
        for i in range(0,len(data),4):
            histogram[bytes(data[i:i+3])]+=1
            require(len(histogram)<=65536,'constant normal oracle distinct-value resource limit')
        q=[v/127.5-1 for v in pixel[:3]];ql=math.sqrt(sum(v*v for v in q));lengths=[];angles=[]
        for rgb,n in histogram.items():
            v=[x/127.5-1 for x in rgb];vl=math.sqrt(sum(x*x for x in v));require(vl>1e-12,'decoded normal nonzero')
            lengths.append((abs(vl-1),n));angles.append((math.degrees(math.acos(max(-1,min(1,sum(a*b for a,b in zip(v,q))/(vl*ql))))),n))
        mean=sum(e*n for e,n in lengths)/count;maximum=max(e for e,_ in lengths);angular=sum(e*n for e,n in angles)/count
        accumulated=0;p99=0
        for angle,n in sorted(angles):
            accumulated+=n;p99=angle
            if accumulated>=.99*count: break
        mean_cap,max_cap={'fast':(.08,.20),'balanced':(.05,.15),'quality':(.03,.10)}[profile]
        require(mean<=mean_cap and maximum<=max_cap and angular<=3 and p99<=12,'fixed constant normal length/angular floors')
        normal={'lengthMean':mean,'lengthMax':maximum,'angleMean':angular,'angleP99':p99}
    return {'channels':result,'normal':normal}


def controls(pixel,role,profile):
    rejected=[]
    def probe(name,mutant):
        try: metrics(pixel,bytes(mutant),role,profile)
        except AssertionError: rejected.append(name);return
        raise AssertionError('insensitive constant image control '+name)
    probe('alpha',pixel[:3]+(0,))
    rgb=(0,pixel[1],pixel[2],pixel[3]) if role==3 else (pixel[0],0,0,pixel[3]) if role==1 else (0,0,0,pixel[3])
    probe('nonbase' if role in (1,3) else 'normal' if role==2 else 'RGB',rgb)
    if role in (0,4):
        probe('gamma',tuple(math.floor((v/255)**2.2*255+.5) for v in pixel[:3])+(pixel[3],))
    return rejected


def verify(truth_path,output,decoded,run_controls):
    truth=json.loads(truth_path.read_text());source=truth_path.parent
    require(sha(source/'source.gltf')==truth['inputSha256'],'source glTF identity')
    for name,digest in truth['assets'].items(): require(sha(source/name)==digest,'source asset identity')
    source_doc=json.loads((source/'source.gltf').read_text())
    constants=[]
    for i,image in enumerate(truth['images']):
        require(source_doc['images'][i]['uri']==image['uri'],'source image URI binding')
        value=png_constant(source/image['uri']);require(value[:2]==(image['width'],image['height']),'source image dimensions truth')
        if 'constantRGBA' in image: require(tuple(image['constantRGBA'])==value[2],'constant declaration equals independently decoded source')
        if 'rgba' in image: require(bytes(image['rgba'])==bytes(value[2])*(value[0]*value[1]),'legacy source pixel truth equals source PNG')
        require(value[2][3]==255,'constant image oracle domain opaque alpha; MASK coverage separate')
        constants.append(value)
    for name in ('materials','textures','samplers'): require(truth[name]==source_doc[name],'source authored binding truth')
    manifest=json.loads((output/'manifest.json').read_text());profile=manifest['report']['profile'];require(profile in ('fast','balanced','quality'),'constant KTX gate requires lossy KTX profile')
    index_doc=json.loads((decoded/'decoded.json').read_text());require(index_doc['schemaVersion']==1 and index_doc['driverSha256']==sha(pathlib.Path(__file__).with_name('decode_ktx.rs')),'independent actual decoder source identity')
    index={};used=set();measurements={};control_rows={};pages=leaves=levels_count=0;largest=0
    for entry in index_doc['pages']:
        key=(entry['uri'],entry['page']);require(key not in index,'decoded page unique');index[key]=entry
    for line in (output/'leaves.jsonl').open():
        receipt=json.loads(line);path=(output/receipt['uri']).resolve();require(path.is_relative_to(output.resolve()),'leaf path confinement')
        require(sha(path)==receipt['sha256'] and path.stat().st_size==receipt['bytes'],'published physical leaf identity')
        raw,doc,bin_data=glb(path);bindings=[b for b in receipt['appearance'] if 'slot' in b]
        page_receipts={b['page']:b for b in receipt['appearance'] if 'quality' in b}
        require(len(page_receipts)==len(doc['images']),'actual page/receipt inventory')
        material_ids={}
        for pi,mesh in enumerate(doc['meshes']):
            require(len(mesh['primitives'])==1,'one prepared primitive per mesh')
            primitive=mesh['primitives'][0];selected=[b for b in bindings if b['primitive']==pi]
            require(len(selected)==5 and {b['slot'] for b in selected}==set(range(5)),'complete five-slot perprimitive binding inventory')
            require({n for n in primitive['attributes'] if n.startswith('TEXCOORD_')}=={f'TEXCOORD_{n}' for n in range(5)},'actual dense five UV accessor closure')
            material_ids[pi]=source_material_index(doc['materials'][primitive['material']],selected,source_doc)
        for b in bindings:
            pi=b['primitive'];role=b['slot'];require(role in range(5) and b['outputUV'] == role,'dense slot outputUV')
            primitive=doc['meshes'][pi]['primitives'][0];material=doc['materials'][primitive['material']]
            source_binding=slot(source_doc['materials'][material_ids[pi]],role)
            source_texture=source_doc['textures'][source_binding['index']]
            require(b['sourceImage']==source_texture['source'] and b['sourceUV']==source_binding.get('texCoord',0),'source image/UV exact role binding')
            expected_sampler=source_doc['samplers'][source_texture['sampler']];require(b['sourceSampler']==expected_sampler,'source sampler receipt')
            target=slot(material,role);texture=doc['textures'][target['index']]
            require(target['texCoord']==role and texture['extensions']['KHR_texture_basisu']['source']==b['page'],'actual dense UV/texture page')
            require(doc['samplers'][texture['sampler']]=={**expected_sampler,'wrapS':33071,'wrapT':33071},'actual original min/mag filters and CLAMP page')
        for page_id,image in enumerate(doc['images']):
            require(image['mimeType']=='image/ktx2','actual KTX2 MIME')
            view=doc['bufferViews'][image['bufferView']];require(view['buffer']==0 and 'extensions' not in view,'physical encoded image view')
            start=view.get('byteOffset',0);end=start+view['byteLength'];require(0<=start<end<=len(bin_data),'physical image range')
            encoded=bin_data[start:end];encoded_hash=hashlib.sha256(encoded).hexdigest();key=(receipt['uri'],page_id)
            require(key in index and key not in used,'actual decoded page index');used.add(key);row=index[key]
            require(row['glbSha256']==receipt['sha256'] and row['pageSha256']==encoded_hash,'decoded actual wire/page identity')
            require(encoded[:12]==b'\xabKTX 20\xbb\r\n\x1a\n','actual KTX2 header')
            vk,size,w,h,depth,layers,faces,count,scheme=struct.unpack_from('<9I',encoded,12)
            require(vk==0 and size==1 and 0<w<=4096 and 0<h<=4096 and w&(w-1)==0 and h&(h-1)==0 and (depth,layers,faces)==(0,0,1),'actual physical POT KTX page')
            require(count==max(w,h).bit_length()==len(row['levels']),'actual complete mip count')
            page=page_receipts[page_id];require((page['width'],page['height'],page['levels'],page['bytes'])==(w,h,count,len(encoded)),'physical receipt page shape')
            selected=[b for b in bindings if b['page']==page_id];require(selected,'referenced actual page')
            roles={b['slot'] for b in selected};require(len(roles)==1,'constant page single class role');role=next(iter(roles))
            pixels={constants[b['sourceImage']][2] for b in selected};require(len(pixels)==1,'constant composite page contributors identical source constants')
            pixel=next(iter(pixels));pixel=unit_byte(pixel) if role==2 else pixel
            if role==2:
                require(unit_byte(pixel)==pixel,'constant oracle domain: stable quantized unit normal; arbitrary constant-normal drift needs explicit source recursion')
            ranges=[]
            for mip in range(count):
                offset,length,uncompressed=struct.unpack_from('<QQQ',encoded,80+24*mip)
                require(length>0 and offset>=80+24*count and offset+length<=len(encoded),'actual KTX payload indexed closure')
                ranges.append((offset,offset+length))
                require(uncompressed>0 if scheme!=1 else uncompressed==0,'actual KTX indexed uncompressed scheme')
            require(all(a[1]<=b[0] for a,b in zip(sorted(ranges),sorted(ranges)[1:])),'actual KTX levels nonoverlap')
            dfd,length=struct.unpack_from('<II',encoded,48)
            require(dfd>=80+24*count and dfd+length<=len(encoded) and length>=16 and struct.unpack_from('<I',encoded,dfd)[0]==length
                    and encoded[dfd+12] in (163,166) and encoded[dfd+14]==(2 if role in (0,4) else 1),'actual DFD class transfer')
            for mip,level in enumerate(row['levels']):
                require((level['mip'],level['width'],level['height'])==(mip,w,h),'actual mip dimensions/order')
                file=(decoded/level['file']).resolve();require(file.is_relative_to(decoded.resolve()),'decoded level confinement')
                require(file.stat().st_size==w*h*4 and sha(file)==level['sha256'],'actual decoded RGBA hash/size')
                cache_key=(encoded_hash,mip,role,pixel,profile)
                if cache_key not in measurements: measurements[cache_key]=metrics(pixel,file.read_bytes(),role,profile)
                if run_controls and role not in control_rows: control_rows[role]=controls(pixel,role,profile)
                # Normal class mips renormalize prior quantized unit pixels;
                # dedicated source-chain pages stop at the original terminal.
                if role==2 and not all(b.get('mipConstruction') for b in selected): pixel=unit_byte(pixel)
                w,h=max(1,w//2),max(1,h//2);levels_count+=1
            pages+=1;largest=max(largest,page['width'],page['height'])
        leaves+=1
    require(used==set(index),'whole decoded page inventory closure')
    require(leaves==manifest['report']['leaves'],'whole actual leaf inventory')
    if run_controls: require(set(control_rows)==set(range(5)),'all five semantic control roles')
    result={'verdict':'PASS','scope':'constant opaque RGBA8 source textures, actual all KTX pages/every mip unchanged class floors; geometry/MASK coverage/renderer separate','leaves':leaves,'pages':pages,'levels':levels_count,'largestPageEdge':largest,'uniqueMeasurements':len(measurements),'measurementRows':[{'encodedImageSha256':k[0],'mip':k[1],'role':k[2],'expectedConstantRGBA':k[3],'profile':k[4],'metrics':v} for k,v in measurements.items()],'controls':control_rows,'profile':profile,'sourceTruthSha256':sha(truth_path),'leafManifestSha256':sha(output/'leaves.jsonl'),'decodedIndexSha256':sha(decoded/'decoded.json'),'oracleSha256':sha(__file__),'decoderDriverSha256':index_doc['driverSha256'],'rendererAcceptance':False,'maskGeometryCoverageAcceptance':False}
    result['passHash']=hashlib.sha256(json.dumps(result,sort_keys=True,separators=(',',':')).encode()).hexdigest();return result


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('truth',type=pathlib.Path);parser.add_argument('output',type=pathlib.Path);parser.add_argument('decoded',type=pathlib.Path);parser.add_argument('--controls',action='store_true');parser.add_argument('--report',type=pathlib.Path);args=parser.parse_args()
    require(not args.decoded.resolve().is_relative_to(ROOT),'bulk decoded image evidence must stay cache-only')
    result=verify(args.truth,args.output,args.decoded,args.controls)
    if args.report:
        require(not args.report.resolve().is_relative_to(ROOT),'reports must stay cache-only');args.report.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,sort_keys=True))


if __name__=='__main__': main()
