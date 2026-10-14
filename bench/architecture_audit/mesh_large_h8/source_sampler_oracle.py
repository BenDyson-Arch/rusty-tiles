#!/usr/bin/env python3
"""Independent original-source MIP sampler gate on small actual published GLBs.
POT opaque D0 only, page<=128. Alternate filter probes are simulations on actual
decoded pixels; actual published enums are recorded separately. This finite
isotropic footprint test does not establish all camera/rendering equivalence.
"""
import argparse
import hashlib
import json
import math
import pathlib
import sys
import struct
from fractions import Fraction
import leaf_oracle as wire


def linear(v): return v/12.92 if v<=.04045 else ((v+.055)/1.055)**2.4
def srgb(v): return 12.92*v if v<=.0031308 else 1.055*v**(1/2.4)-.055


def original_chain(source,role):
    w,h,pixels=source;pixels=list(pixels)
    if role==2:
        for i in range(0,len(pixels),4):
            pixels[i:i+3]=[wire.quant_byte((v+1)/2) for v in wire.unit_normal(pixels[i:i+4])]
    chain=[(w,h,bytes(pixels))]
    while (w,h)!=(1,1):
        dw,dh=max(1,w//2),max(1,h//2);out=[]
        for y in range(dh):
            for x in range(dw):
                rows=[pixels[(sy*w+sx)*4:(sy*w+sx)*4+4] for sy in range(2*y,min(2*y+2,h)) for sx in range(2*x,min(2*x+2,w))]
                if role==2:
                    unit=[wire.unit_normal(p) for p in rows];v=[sum(p[a] for p in unit) for a in range(3)];length=math.sqrt(sum(c*c for c in v))
                    v=unit[0] if length<=1e-12 else [c/length for c in v]
                    rgb=[wire.quant_byte((c+1)/2) for c in v]
                elif role in (0,4):
                    rgb=[wire.quant_byte(srgb(sum(linear(p[a]/255) for p in rows)/len(rows))) for a in range(3)]
                else: rgb=[wire.quant_byte(sum(p[a]/255 for p in rows)/len(rows)) for a in range(3)]
                out.extend(rgb+[wire.quant_byte(sum(p[3]/255 for p in rows)/len(rows))])
        w,h,pixels=dw,dh,out;chain.append((w,h,bytes(pixels)))
    return chain


def filtered(level,uv,sampler,spatial_linear,role):
    w,h,data=level
    def tap(x,y):
        index=(wire.wrap_tap(y,h,sampler['wrapT'])*w+wire.wrap_tap(x,w,sampler['wrapS']))*4
        return [linear(data[index+a]/255) if role in (0,4) and a<3 else data[index+a]/255 for a in range(4)]
    x,y=Fraction(uv[0])*w,Fraction(uv[1])*h
    if not spatial_linear: return tap(math.floor(x),math.floor(y))
    x-=Fraction(1,2);y-=Fraction(1,2);ix,iy=math.floor(x),math.floor(y);fx,fy=float(x-ix),float(y-iy)
    taps=[tap(ix,iy),tap(ix+1,iy),tap(ix,iy+1),tap(ix+1,iy+1)]
    return [(taps[0][a]*(1-fx)+taps[1][a]*fx)*(1-fy)+(taps[2][a]*(1-fx)+taps[3][a]*fx)*fy for a in range(4)]


def at_lod(chain,uv,lod,sampler,role):
    # Pinned finite probe convention; viewer transition remains an external gate.
    if lod<=0: return filtered(chain[0],uv,sampler,sampler['magFilter']==9729,role)
    enum=sampler['minFilter']
    if enum in (9728,9729): return filtered(chain[0],uv,sampler,enum==9729,role)
    lod=min(max(0,lod),len(chain)-1);spatial=enum in (9985,9987)
    if enum in (9984,9985): return filtered(chain[math.floor(lod+.5)],uv,sampler,spatial,role)
    lo=math.floor(lod);hi=min(lo+1,len(chain)-1);fraction=lod-lo
    a=filtered(chain[lo],uv,sampler,spatial,role);b=filtered(chain[hi],uv,sampler,spatial,role)
    return [x*(1-fraction)+y*fraction for x,y in zip(a,b)]


def expected_pages(original,source_size,binding,dimensions):
    sw,sh=source_size;w,h=dimensions;ox,oy=binding['cropOrigin'];sampler=binding['sourceSampler'];out=[];mip=0
    while True:
        tw,th,data=original[min(mip,len(original)-1)];pixels=bytearray()
        for y in range(h):
            sy=wire.wrap_tap(math.floor(((y+.5)*dimensions[1]/h+oy)*th/sh),th,sampler['wrapT'])
            for x in range(w):
                sx=wire.wrap_tap(math.floor(((x+.5)*dimensions[0]/w+ox)*tw/sw),tw,sampler['wrapS'])
                pixels.extend(data[(sy*tw+sx)*4:(sy*tw+sx)*4+4])
        out.append((w,h,bytes(pixels)))
        if (w,h)==(1,1): return out
        w,h=max(1,w//2),max(1,h//2);mip+=1


def geometry_samples(rows,points,page_uv,source_uv_set,source_size,page_size,crop_origin):
    centroids=[]
    for i in range(4):
        for j in range(4-i):
            for offset in ([1,2] if i+j<3 else [1]):
                u,v=Fraction(3*i+offset,12),Fraction(3*j+offset,12);centroids.append((1-u-v,u,v))
    locations=centroids+[tuple(Fraction(v,12) for v in row) for row in ((12,0,0),(0,12,0),(0,0,12),(6,6,0),(0,6,6),(6,0,6))]
    wire.require(len(locations)==22,'independent sample inventory22')
    result=[]
    dot=lambda a,b:sum(x*y for x,y in zip(a,b))
    for t,row in enumerate(rows):
        p=points[t];a=[p[1][k]-p[0][k] for k in range(3)];b=[p[2][k]-p[0][k] for k in range(3)]
        normal=[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]
        twice_area=math.sqrt(dot(normal,normal));aa,ab,bb=dot(a,a),dot(a,b),dot(b,b)
        if not twice_area: continue
        determinant=twice_area**2 # Cross-product identity avoids subtracting nearly equal Gram terms.
        e1=[v/math.sqrt(aa) for v in a];n=[v/twice_area for v in normal]
        e2=[n[1]*e1[2]-n[2]*e1[1],n[2]*e1[0]-n[0]*e1[2],n[0]*e1[1]-n[1]*e1[0]]
        source_uv=row['uvs'][source_uv_set];output_uv=page_uv[3*t:3*t+3]
        def rho(uv,size):
            gradients=[]
            for axis in range(2):
                d1,d2=uv[1][axis]-uv[0][axis],uv[2][axis]-uv[0][axis]
                gradients.append([(a[k]*(bb*d1-ab*d2)+b[k]*(aa*d2-ab*d1))/determinant for k in range(3)])
            return max(math.sqrt(sum((dot(g,e)*size[axis])**2 for axis,g in enumerate(gradients))) for e in (e1,e2))
        source_rho,page_rho=rho(source_uv,source_size),rho(output_uv,page_size)
        wire.require((source_rho==0)==(page_rho==0),'actualzero UV Jacobian invariant')
        exact_affine=all(Fraction(output_uv[c][axis])*page_size[axis] == Fraction(source_uv[c][axis])*source_size[axis]-crop_origin[axis]
                         for c in range(3) for axis in range(2))
        # Exact captured corner affine implies exact texel Jacobian identity in
        # every decoded triangle frame. No epsilon or numerical ratio clipping.
        bias=0 if source_rho==0 or exact_affine else math.log2(source_rho/page_rho)
        wire.require(math.isfinite(bias),'actual independent finiteLOD bias')
        for bary in locations:
            result.append(([sum(Fraction(source_uv[c][axis])*bary[c] for c in range(3)) for axis in range(2)],
                           [sum(Fraction(output_uv[c][axis])*bary[c] for c in range(3)) for axis in range(2)],twice_area/44,bias))
    return result


def sampler_metrics(pairs,role,profile,mask):
    total=sum(weight for _,_,weight in pairs);wire.require(total>0,'sampler positive geometric weights')
    channels=(0,3) if role==3 else (1,2,3) if role==1 else (0,1,2,3)
    if mask: channels=tuple(c for c in channels if c!=3)
    rows=[]
    for channel in channels:
        def byte(p): return srgb(p[channel])*255 if role in (0,4) and channel<3 else p[channel]*255
        errors=[(abs(byte(expected)-byte(actual)),weight) for expected,actual,weight in pairs]
        mse=sum(e*e*w for e,w in errors)/total;psnr=math.inf if not mse else 10*math.log10(65025/mse);maximum=max(e for e,_ in errors)
        if channel==3: floor,cap=(34,32) if profile=='quality' else (30,80 if profile=='fast' else 64)
        elif role in (0,4): floor,cap=(34,24) if profile=='quality' else (22,56 if profile=='fast' else 32)
        elif role==2: floor,cap=30,112
        else: floor,cap=30,(8 if profile=='quality' else 80) if role==3 else 32
        wire.require(psnr>=floor and maximum<=cap,f'original sampler role{role} channel{channel} PSNR{psnr} max{maximum} floor{floor} cap{cap}')
        rows.append({'channel':channel,'psnr':None if math.isinf(psnr) else psnr,'max':maximum,'floor':floor,'cap':cap})
    angular=None
    if role==2:
        angles=[]
        for expected,actual,weight in pairs:
            q=[v*2-1 for v in expected[:3]];v=[x*2-1 for x in actual[:3]];ql=math.sqrt(sum(x*x for x in q));vl=math.sqrt(sum(x*x for x in v))
            wire.require(ql>1e-12 and vl>1e-12,'filtered normal nonzero')
            angles.append((math.degrees(math.acos(max(-1,min(1,sum(a*b for a,b in zip(v,q))/(vl*ql))))),weight))
        mean=sum(e*w for e,w in angles)/total;accumulated=0;p99=0
        for angle,weight in sorted(angles):
            accumulated+=weight;p99=angle
            if accumulated>=.99*total: break
        wire.require(mean<=3 and p99<=12,'filtered normal ANGLE floors (length belongs to storedtexels)')
        angular={'mean':mean,'p99':p99}
    return {'channels':rows,'filteredAngle':angular}


def controls(profile):
    sampler={'wrapS':10497,'wrapT':33071,'minFilter':9984,'magFilter':9729}
    source=(4,1,bytes([0,0,0,255]*3+[255,255,255,255]));chain=original_chain(source,3)
    good={'cropOrigin':[-4,-1],'sourceSampler':sampler};bad={**good,'cropOrigin':[-3,-1]}
    atlas=expected_pages(chain,(4,1),bad,(16,4));clamp={**sampler,'wrapS':33071,'wrapT':33071};found=[]
    def reject(name,expected,actual):
        try: sampler_metrics([(expected,actual,1)],3,profile,False)
        except AssertionError: found.append(name);return
        raise AssertionError('source sampler insensitive control '+name)
    expected=at_lod(chain,(.375,.5),1,sampler,3)
    reject('wrong-phase',expected,at_lod(atlas,((.375*4+3)/16,(.5+1)/4),1,clamp,3))
    terminal=at_lod(chain,(.375,.5),4,sampler,3)
    reject('coarse-page-mixture',terminal,[.5,.5,.5,1])
    wrong_clamp=chain+[(1,1,bytes([128,128,128,255]))]*3
    reject('ignored-original-terminal-clamp',terminal,at_lod(wrong_clamp,(.375,.5),4,sampler,3))
    reject('wrong-spatial-mip-enum',expected,at_lod(chain,(.375,.5),1,{**sampler,'minFilter':9985},3))
    reject('wrong-derivative-sourceLOD',expected,at_lod(chain,(.375,.5),2,sampler,3))
    try: sampler_metrics([([.5,.5,.5,1],[linear(.5)]*3+[1],1)],0,profile,False)
    except AssertionError: found.append('wrong-sRGB-filter-transfer')
    else: raise AssertionError('gamma control insensitive')
    boundary=wire.f32(.75);word=int.from_bytes(struct.pack('<f',boundary),'little')
    sides=[struct.unpack('<f',(word+delta).to_bytes(4,'little'))[0] for delta in (-1,0,1)]
    for u,selected in zip(sides,(2,3,3)):
        exact=Fraction(u);page_u=(exact*4+4)/16
        wire.require(math.floor(exact*4)==selected,'adjacent represented sourceUV boundary side')
        wire.require(math.floor(page_u*16)-4==selected,'same separate perturbation page boundary side')
        valid=expected_pages(chain,(4,1),good,(16,4))
        # These three probes stay in the corpus: neither side is excluded.
        sampler_metrics([(at_lod(chain,(exact,Fraction(1,2)),.25,sampler,3),
                          at_lod(valid,(page_u,Fraction(3,8)),.25,clamp,3),1)],3,profile,False)
    found.append('exact-boundary-and-both-adjacent-f32UV-sides')
    wire.require(len(found)==7,'seven independent source sampler controls');return found


def lod_trace(chain,uv,lod,sampler):
    enum=sampler['minFilter'];clamped=min(max(0,lod),len(chain)-1)
    if lod<=0: selected=[(0,1)];spatial=sampler['magFilter']==9729
    elif enum in (9728,9729): selected=[(0,1)];spatial=enum==9729
    elif enum in (9984,9985): selected=[(math.floor(clamped+.5),1)];spatial=enum==9985
    else:
        lo=math.floor(clamped);fraction=clamped-lo;selected=[(lo,1-fraction),(min(lo+1,len(chain)-1),fraction)];spatial=enum==9987
    trace=[]
    for level,blend in selected:
        w,h,data=chain[level];x,y=Fraction(uv[0])*w,Fraction(uv[1])*h
        if spatial: x-=Fraction(1,2);y-=Fraction(1,2)
        ix,iy=math.floor(x),math.floor(y)
        taps=[(ix,iy)] if not spatial else [(ix,iy),(ix+1,iy),(ix,iy+1),(ix+1,iy+1)]
        pixels=[]
        for tx,ty in taps:
            sx,sy=wire.wrap_tap(tx,w,sampler['wrapS']),wire.wrap_tap(ty,h,sampler['wrapT'])
            pixels.append({'unwrapped':[tx,ty],'selectedPixel':[sx,sy],'RGBA':list(data[(sy*w+sx)*4:(sy*w+sx)*4+4])})
        trace.append({'level':level,'dimensions':[w,h],'blend':blend,'linear':spatial,'texelCoordinates':[float(x),float(y)],
                      'exactTexelCoordinates':[str(x),str(y)],'texelCoordinateHex':[float(x).hex(),float(y).hex()],'fraction':[float(x-ix),float(y-iy)],'pixels':pixels})
    return {'requestedLOD':lod,'clampedLOD':clamped,'chainLength':len(chain),'levels':trace}


def failure_witness(error,pairs,samples,rows,source_uv,output_uv,original,actual,sampler,role,profile,enum,lod,receipt,binding,pi):
    ranked=sorted(range(len(pairs)),key=lambda i:max(abs((srgb(pairs[i][0][a])-srgb(pairs[i][1][a])) if role in (0,4) and a<3 else pairs[i][0][a]-pairs[i][1][a])*255 for a in range(4)),reverse=True)
    witnesses=[]
    for i in ranked[:8]:
        uv,out,weight,bias=samples[i];expected,received,_=pairs[i]
        witnesses.append({'sampleIndex':i,'triangleIndexInNondegenerateSampleBlocks':i//22,'sourceTriangle':rows[i//22]['source'] if i//22<len(rows) else None,
                          'sourceUV':[float(v) for v in uv],'sourceUVExact':[str(v) for v in uv],'sourceUVHex':[float(v).hex() for v in uv],
                          'pageUV':[float(v) for v in out],'pageUVExact':[str(v) for v in out],'pageUVHex':[float(v).hex() for v in out],'weight':weight,'sourceLODBias':bias,
                          'expectedLinearRGBA':expected,'actualLinearRGBA':received,'expectedComparisonBytes':[srgb(v)*255 if role in (0,4) and a<3 else v*255 for a,v in enumerate(expected)],
                          'actualComparisonBytes':[srgb(v)*255 if role in (0,4) and a<3 else v*255 for a,v in enumerate(received)],
                          'sourceSelection':lod_trace(original,uv,lod+bias,{**sampler,'minFilter':enum}),
                          'pageSelection':lod_trace(actual,out,lod,{**sampler,'minFilter':enum,'wrapS':33071,'wrapT':33071})})
    return {'verdict':'FAIL','error':str(error),'profile':profile,'uri':receipt['uri'],'primitive':pi,'page':binding['page'],'role':role,
            'probeMinFilter':enum,'actualPublishedMinFilter':sampler['minFilter'],'isActualPublishedFilter':enum==sampler['minFilter'],'LOD':lod,
            'cropOrigin':binding['cropOrigin'],'cropSize':binding['cropSize'],'sourceCornerUVs':source_uv,'pageCornerUVs':output_uv,'sampleCount':len(samples),
            'samples':witnesses,'oracleSha256':wire.sha(__file__),'diagnosticOnly':True,'noChangedFloorsOrBoundaryExclusions':True}


def verify(truth_path,output,decoded,run_controls,failure_path=None):
    truth=json.loads(truth_path.read_text());wire.require(truth['mode']=='small','source sampler oracle smallfixture domain')
    wire.require(wire.sha(truth_path.parent/'source.gltf')==truth['inputSha256'],'source document identity')
    for name,digest in truth['assets'].items(): wire.require(wire.sha(truth_path.parent/name)==digest,'source asset identity')
    source_doc=json.loads((truth_path.parent/'source.gltf').read_text())
    for name in ('materials','textures','samplers'): wire.require(source_doc[name]==truth[name],'originalsource authored binding truth')
    sources=[]
    for i,image in enumerate(truth['images']):
        source=wire.decode_png((truth_path.parent/image['uri']).read_bytes());w,h,pixels=source
        wire.require(source==(image['width'],image['height'],bytes(image['rgba'])) and source_doc['images'][i]['uri']==image['uri'],'independent originalPNG sourcepixels')
        wire.require(1<=w<=32 and 1<=h<=32 and w&(w-1)==0 and h&(h-1)==0,'oraclePOTsource<=32 domain')
        wire.require(all(alpha==255 for alpha in pixels[3::4]),'oracleopaquealpha g0 domain (generalMASKnotclaimed)');sources.append(source)
    original={(i,role):original_chain(source,role) for i,source in enumerate(sources) for role in range(5)}
    index_doc=json.loads((decoded/'decoded.json').read_text());wire.require(index_doc['driverSha256']==wire.sha(pathlib.Path(__file__).with_name('decode_ktx.rs')),'actual decoder driverhash')
    index={};used=set();metrics=[];filters=set();leaf_count=page_count=0
    for row in index_doc['pages']:
        key=(row['uri'],row['page']);wire.require(key not in index,'unique actualdecodedpage');index[key]=row
    manifest=json.loads((output/'manifest.json').read_text());profile=manifest['report']['profile']
    expected_tri={tuple(row['source']):row for row in truth['triangles']};seen=set()
    for line in (output/'leaves.jsonl').open():
        receipt=json.loads(line);path=(output/receipt['uri']).resolve();wire.require(path.is_relative_to(output.resolve()) and wire.sha(path)==receipt['sha256'],'physicalactualGLB identity')
        doc,views,packet=wire.decoded(path);wire.require(packet['wireSha256']==receipt['sha256'],'actual meshoptwire hash')
        tables=wire.metadata(doc,views);bindings={(b['primitive'],b['slot']):b for b in receipt['appearance'] if 'slot'in b};pages={b['page']:b for b in receipt['appearance'] if 'quality'in b};actual={}
        for page_id,image in enumerate(doc['images']):
            wire.require(image['mimeType']=='image/ktx2','source sampler actualKTX2');page=pages[page_id];dimensions=(page['width'],page['height']);wire.require(max(dimensions)<=128,'oracle smallpage<=128 domain')
            wire.require('extensions' not in doc['bufferViews'][image['bufferView']] and doc['bufferViews'][image['bufferView']]['buffer']==0,'actualphysical encoded image view')
            rows=[b for b in bindings.values() if b['page']==page_id];wire.require(len(rows)==1,'actual dedicatedbinding page');binding=rows[0];role=binding['slot'];data=views[image['bufferView']]
            wire.check_ktx(data,{role},profile,page);wire.require(binding['densityDrop']==0 and binding.get('mipConstruction')=='dedicated original-source class levels','actualsource-aware D0construction')
            wire.require(binding['pageOffset']==[0,0] and binding['cropSize']==list(dimensions),'actualdedicated wholepage placement')
            sw,sh=sources[binding['sourceImage']][:2];wire.require(binding['cropOrigin'][0]%sw==0 and binding['cropOrigin'][1]%sh==0,'actualoriginalsource POTphase')
            actual[page_id]=wire.read_decoded_chain(decoded,index,receipt['uri'],page_id,receipt['sha256'],data,dimensions);used.add((receipt['uri'],page_id))
            reference=expected_pages(original[(binding['sourceImage'],role)],(sw,sh),binding,dimensions)
            for mip,(wanted,received) in enumerate(zip(reference,actual[page_id])):
                wire.require(wanted[:2]==received[:2],'actual sourceaware mipshape')
                quality=wire.decoded_metrics(wanted[2],received[2],[1]*(wanted[0]*wanted[1]),role,profile)
                metrics.append({'uri':receipt['uri'],'page':page_id,'mip':mip,'storedTexelQuality':quality})
            page_count+=1
        for pi,node in enumerate(doc['nodes']):
            primitive=doc['meshes'][node['mesh']]['primitives'][0];values={name:wire.accessor(doc,views,i) for name,i in primitive['attributes'].items()};wire.check_features(primitive['extensions']['EXT_mesh_features']['featureIds'],tables)
            rows=[];points=[]
            for t in range(len(values['POSITION'])//3):
                triangle_ids=[v[0] for v in values['_FEATURE_ID_1'][3*t:3*t+3]];tid=int(triangle_ids[0])
                wire.require(triangle_ids==[tid]*3 and 0<=tid<tables[1][0],'actualinteger triangle feature domain/allcorners')
                cols=tables[1][1];identity=tuple(cols[n][tid] for n in ('node_index','mesh_index','primitive_index','triangle_index'))
                wire.require(identity in expected_tri and identity not in seen,'actualsource triangleexactcoverage');seen.add(identity);row=expected_tri[identity];rows.append(row)
                wire.require(cols['material_index'][tid]==row['material'] and [cols[f'vertex_{c}'][tid] for c in range(3)]==row['vertices'],'actualoriginal source material/corner provenance')
                for name,key in [('NORMAL','normal'),('TANGENT','tangent'),('COLOR_0','color')]: wire.require(values[name][3*t:3*t+3]==row[key],'literal source attribute/frame '+name)
                translation=node.get('translation',[0,0,0]);scale=node.get('scale',[1,1,1]);points.append([[wire.f32(wire.f32(v[a]*scale[a])+translation[a]) for a in range(3)] for v in values['POSITION'][3*t:3*t+3]])
                for c,position in enumerate(points[-1]):
                    wire.require(math.dist(position,row['positions'][c])<=receipt['geometryError'] and all(receipt['bounds']['min'][a]<=position[a]<=receipt['bounds']['max'][a] for a in range(3)),'actualdecoded geometric correspondence/bounds for footprint')
            wire.require(rows and all(row['material']==rows[0]['material'] for row in rows),'source material primitive closure')
            material=truth['materials'][rows[0]['material']];wire.check_material(doc['materials'][primitive['material']],material)
            for role in range(5):
                binding=bindings[(pi,role)];source_binding=wire.slot_binding(material,role);texture=truth['textures'][source_binding['index']];sampler=truth['samplers'][texture['sampler']];page=pages[binding['page']]
                wire.require(binding['sourceImage']==texture['source'] and binding['sourceUV']==source_binding.get('texCoord',0) and binding['outputUV']==role and binding['sourceSampler']==sampler,'actualsource role/UV/sampler closure')
                output_binding=wire.slot_binding(doc['materials'][primitive['material']],role);output_texture=doc['textures'][output_binding['index']];wire.require(wire.image_index(output_texture)==binding['page'],'actual slot imagepage');wire.check_sampler(doc['samplers'][output_texture['sampler']],sampler)
                source_uv=[uv for row in rows for uv in row['uvs'][binding['sourceUV']]];output_uv=values[f'TEXCOORD_{role}'];sw,sh=sources[binding['sourceImage']][:2];pw,ph=page['width'],page['height'];ox,oy=binding['cropOrigin']
                for uv,out in zip(source_uv,output_uv): wire.require(out==[wire.f32((uv[0]*sw-ox)/pw),wire.f32((uv[1]*sh-oy)/ph)],'actualfixed positiveUV mapping')
                wire.require(min(uv[0]*sw for uv in source_uv)-ox>=sw and pw+ox-max(uv[0]*sw for uv in source_uv)>=sw and min(uv[1]*sh for uv in source_uv)-oy>=sh and ph+oy-max(uv[1]*sh for uv in source_uv)>=sh,'both actualsource periodtap margins')
                samples=geometry_samples(rows,points,output_uv,binding['sourceUV'],(sw,sh),(pw,ph),binding['cropOrigin']);mask=role==0 and material.get('alphaMode')=='MASK';filters.add(sampler['minFilter'])
                for enum in (9984,9985,9986,9987):
                    source_sampler={**sampler,'minFilter':enum};page_sampler={**source_sampler,'wrapS':33071,'wrapT':33071}
                    for lod in [-1]+[i/4 for i in range(4*(len(actual[binding['page']])-1)+1)]:
                        pairs=[(at_lod(original[(binding['sourceImage'],role)],uv,lod+bias,source_sampler,role),at_lod(actual[binding['page']],out,lod,page_sampler,role),weight) for uv,out,weight,bias in samples]
                        try: quality=sampler_metrics(pairs,role,profile,mask)
                        except AssertionError as error:
                            witness=failure_witness(error,pairs,samples,rows,source_uv,output_uv,original[(binding['sourceImage'],role)],actual[binding['page']],sampler,role,profile,enum,lod,receipt,binding,pi)
                            # Diagnostic only: a first alternate-filter failure
                            # must not hide whether this binding's published enum
                            # also fails. Preserve the original FAIL in all cases.
                            published_failures=[]
                            for published_lod in [-1]+[i/4 for i in range(4*(len(actual[binding['page']])-1)+1)]:
                                published_pairs=[(at_lod(original[(binding['sourceImage'],role)],uv,published_lod+bias,sampler,role),
                                                  at_lod(actual[binding['page']],out,published_lod,{**sampler,'wrapS':33071,'wrapT':33071},role),weight)
                                                 for uv,out,weight,bias in samples]
                                try: sampler_metrics(published_pairs,role,profile,mask)
                                except AssertionError as published_error: published_failures.append({'LOD':published_lod,'error':str(published_error)})
                            witness.update(publishedFilterDiagnosticScope='this failing binding only; no artifact approval',publishedFilterQuarterLODProbeFailures=published_failures)
                            witness.update(sourceTruthSha256=wire.sha(truth_path),decodedIndexSha256=wire.sha(decoded/'decoded.json'),leafManifestSha256=wire.sha(output/'leaves.jsonl'))
                            if failure_path: wire.dump(failure_path,witness)
                            print(json.dumps(witness,sort_keys=True),file=sys.stderr)
                            raise
                        metrics.append({'uri':receipt['uri'],'page':binding['page'],'primitive':pi,'role':role,'probeMinFilter':enum,'actualPublishedMinFilter':sampler['minFilter'],'lod':lod,'sampleCount':len(samples),'biasRange':[min(s[3] for s in samples),max(s[3] for s in samples)],'originalSamplerQuality':quality})
                if mask:
                    coverage=wire.opaque_mask_coverage(rows,points,output_uv,actual[binding['page']],material,sampler);floor={'fast':.02,'balanced':.01,'quality':.005}[profile]
                    for row in coverage: wire.require(abs(row['coverage']-row['sourceTarget'])<=floor,'actualopaque sourceBASE finiteMASK g0 residual')
                    metrics.append({'uri':receipt['uri'],'page':binding['page'],'maskOriginalBASECoverage':coverage,'independentSemanticGap':0,'floor':floor})
        leaf_count+=1
    wire.require(seen==set(expected_tri) and leaf_count==manifest['report']['leaves'] and used==set(index),'allactualsource/leaf/decodedimage coverage')
    result={'verdict':'PASS','profile':profile,'pages':page_count,'leaves':leaf_count,'actualPublishedMinFilters':sorted(filters),'simulatedDecodedPixelProbeMinFilters':[9984,9985,9986,9987],'metrics':metrics,'controls':controls(profile) if run_controls else [],'oracleSha256':wire.sha(__file__),'sharedIndependentReaderSha256':wire.sha(wire.__file__),'sourceTruthSha256':wire.sha(truth_path),'decodedIndexSha256':wire.sha(decoded/'decoded.json'),'leafManifestSha256':wire.sha(output/'leaves.jsonl'),'scope':'smallPOTopaqueD0 originalsource classlevels/finite22sampleisotropic quarterLOD; actualpublishedenums distinct from alternatefilter simulations','pointModel':'exact geometric rational barycentric locations on captured f32 UV dyadics; exact texel tap selection; no all GPU interpolation precision guarantee','rendererAcceptance':False,'sourceEffectiveNormalShaderAcceptance':False}
    result['passHash']=hashlib.sha256(json.dumps(result,sort_keys=True,separators=(',',':')).encode()).hexdigest();return result


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('truth',type=pathlib.Path);parser.add_argument('output',type=pathlib.Path);parser.add_argument('decoded',type=pathlib.Path);parser.add_argument('--controls',action='store_true');parser.add_argument('--report',type=pathlib.Path);parser.add_argument('--failure-witness',type=pathlib.Path);args=parser.parse_args()
    failure_path=args.failure_witness or (args.report.with_suffix('.failure.json') if args.report else None)
    if failure_path: wire.require(not failure_path.resolve().is_relative_to(wire.ROOT),'failurewitness cacheonly')
    wire.require(not args.decoded.resolve().is_relative_to(wire.ROOT),'decodedbulk cacheonly');result=verify(args.truth,args.output,args.decoded,args.controls,failure_path)
    if args.report: wire.require(not args.report.resolve().is_relative_to(wire.ROOT),'reportcacheonly');wire.dump(args.report,result)
    print(json.dumps(result,sort_keys=True))


if __name__=='__main__': main()
