#!/usr/bin/env python3
"""Independent H8 source/leaf driver. Generate fixtures; verify published bytes.
No producer helper is imported. KTX2 container checks are NOT decoded appearance
acceptance. Scale generator is bounded by one grid row / fixed repeated blocks.
"""
import argparse
import array
import base64
import copy
import hashlib
import json
import math
import pathlib
import struct
import subprocess
import sys
import zlib

ROOT = pathlib.Path(__file__).resolve().parents[3]
DECODER = pathlib.Path(__file__).with_name('leaf_decode.cjs')
SLOTS = ('baseColorTexture', 'metallicRoughnessTexture', 'normalTexture', 'occlusionTexture', 'emissiveTexture')


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def sha(path):
    h = hashlib.sha256()
    with pathlib.Path(path).open('rb') as f:
        while block := f.read(1 << 20):
            h.update(block)
    return h.hexdigest()


def dump(path, value):
    pathlib.Path(path).write_text(json.dumps(value, sort_keys=True, indent=2) + '\n')


def png(w, h, rgba):
    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))
    rows = b''.join(b'\0' + bytes(rgba[y*w*4:(y+1)*w*4]) for y in range(h))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')


def decode_png(data):
    require(data[:8] == b'\x89PNG\r\n\x1a\n', 'PNG signature')
    offset, compressed, info, ended = 8, bytearray(), None, False
    while offset < len(data):
        length = struct.unpack_from('>I', data, offset)[0]
        kind, body = data[offset+4:offset+8], data[offset+8:offset+8+length]
        require(offset+length+12 <= len(data), 'PNG framing')
        require(zlib.crc32(kind+body) == struct.unpack_from('>I', data, offset+8+length)[0], 'PNG CRC')
        if kind == b'IHDR':
            info = struct.unpack('>IIBBBBB', body)
        elif kind == b'IDAT':
            compressed.extend(body)
        elif kind == b'IEND':
            ended = True
            require(offset+12 == len(data), 'PNG trailing bytes')
        offset += length+12
    require(ended and info and info[2:] == (8, 6, 0, 0, 0), 'RGBA8 PNG domain')
    w, h = info[:2]
    raw = zlib.decompress(compressed)
    require(len(raw) == h*(w*4+1), 'PNG decoded row length')
    pixels, previous, offset = bytearray(), bytearray(w*4), 0
    for _ in range(h):
        filtering, encoded = raw[offset], raw[offset+1:offset+1+w*4]
        require(filtering <= 4, 'PNG row filter')
        row = bytearray(w*4)
        for i, value in enumerate(encoded):
            a, b, c = (row[i-4] if i >= 4 else 0), previous[i], (previous[i-4] if i >= 4 else 0)
            p = a+b-c
            pa, pb, pc = abs(p-a), abs(p-b), abs(p-c)
            predictor = (0, a, b, (a+b)//2, a if pa <= pb and pa <= pc else b if pb <= pc else c)[filtering]
            row[i] = (value+predictor) & 255
        pixels.extend(row)
        previous, offset = row, offset+w*4+1
    return w, h, bytes(pixels)


def image_set(directory, constant=False, unit_normals=False, codec_positive=False, codec_low_variance=False):
    dims = [(4,4),(3,5),(4,4),(5,3),(2,4)] if not constant else [(8,8)]*5
    colors = [(100,150,200,255),(128,190,70,255),(128,128,255,255),(220,0,0,255),(20,40,80,255)]
    images = []
    for role, (w,h) in enumerate(dims):
        pixels = []
        for y in range(h):
            for x in range(w):
                if constant:
                    p = colors[role]
                elif codec_low_variance:
                    # Limited-variation corpus, never a presumed codec pass or a floor waiver.
                    variants = (((120,100,180,255),(136,116,196,255)),
                                ((128,100,120,255),(128,116,136,255)),
                                ((132,128,255,255),(124,128,255,255)),
                                ((100,0,0,255),(116,0,0,255)),
                                ((120,100,180,255),(136,116,196,255)))
                    p = variants[role][(x+y) % 2]
                elif codec_positive and role != 2:
                    # Explicit bounded positive candidate; original transparent/nonunit corpus remains default.
                    bit = (x+y) % 2
                    p = (((96,128,160,255) if bit else (160,128,96,255)) if role in (0,4) else
                         ((128,192,64,255) if bit else (128,64,192,255)) if role == 1 else
                         ((192,0,0,255) if bit else (64,0,0,255)))
                elif role == 2:
                    p = (((218,128,218,255) if (x+y)%2 else (37,128,218,255)) if unit_normals else
                         ((220,128,190,255) if (x+y)%2 else (150,120,180,255)))  # equal-magnitude byte-unit approximation OR deliberate nonunit
                else:
                    p = ((17+x*31+role*29)%256, (23+y*47+role*17)%256, (91+x*13+y*11)%256,
                         (0 if (x+y)%3 == 0 else 204 if (x+y)%3 == 1 else 255) if role == 0 else 255)
                pixels.extend(p)
        name = f'image-{role}.png'
        (directory/name).write_bytes(png(w,h,pixels))
        images.append({'uri': name, 'width': w, 'height': h, 'rgba': pixels})
    return images


def material(texture_start, scale, mask=False):
    bindings = [{'index': texture_start+i, 'texCoord': (1 if i in (1,3,4) else 0)} for i in range(5)]
    return {'pbrMetallicRoughness': {'baseColorTexture': bindings[0], 'metallicRoughnessTexture': bindings[1],
            'baseColorFactor': [0.6,0.7,0.8,0.8 if mask else 1.0], 'metallicFactor': 0.25, 'roughnessFactor': 0.75},
            'normalTexture': {**bindings[2], 'scale': scale}, 'occlusionTexture': {**bindings[3], 'strength': 0.5},
            'emissiveTexture': bindings[4], 'emissiveFactor': [0.1,0.2,0.3], 'doubleSided': True,
            'alphaMode': 'MASK' if mask else 'OPAQUE', **({'alphaCutoff': 0.64} if mask else {})}


def generate(directory, unit_normals=False, codec_positive=False, codec_low_variance=False):
    directory.mkdir(parents=True, exist_ok=True)
    images = image_set(directory,unit_normals=unit_normals or codec_positive or codec_low_variance,codec_positive=codec_positive,codec_low_variance=codec_low_variance)
    payload, views, accessors, meshes, truth = bytearray(), [], [], [], []
    def accessor(values, shape, width, position=False):
        index = len(accessors)
        start = len(payload)
        for row in values:
            payload.extend(struct.pack('<'+'f'*width, *row))
        views.append({'buffer':0, 'byteOffset':start, 'byteLength':len(payload)-start})
        a = {'bufferView':index,'componentType':5126,'count':len(values),'type':shape}
        if position:
            a.update(min=[min(p[i] for p in values) for i in range(3)],max=[max(p[i] for p in values) for i in range(3)])
        accessors.append(a)
        return index
    uv0 = [[-0.25,0.125],[1.125,0.125],[0.125,1.125],[0.25,0.25],[0.75,0.25],[0.25,0.75]]
    uv1 = [[0.125,-0.125],[0.875,0.125],[0.125,0.875],[0.,0.],[0.5,0.],[0.,0.5]]
    colors = [[0.2,0.4,0.6,a] for a in (1.,0.5,0.25,1.,0.5,1.)]
    for node in range(2):
        pos = [[0.000137,0.000173,0.],[1.25,0.,0.125],[0.,1.125,0.], [2.,0.,0.],[2.5,0.,0.],[2.,0.5,0.]]
        attrs = {'POSITION': accessor(pos,'VEC3',3,True), 'NORMAL':accessor([[0,0,1]]*6,'VEC3',3),
                 'TANGENT':accessor([[1,0,0,1]]*6,'VEC4',4),'TEXCOORD_0':accessor(uv0,'VEC2',2),
                 'TEXCOORD_1':accessor(uv1,'VEC2',2),'COLOR_0':accessor(colors,'VEC4',4)}
        meshes.append({'primitives':[{'attributes':attrs,'material':node}]})
        translation = [0.125,-0.25,0.5] if node == 0 else [20.375,3.125,-1.25]
        reflect = 1 if node == 0 else -1
        for triangle in range(2):
            ids = [triangle*3, triangle*3+1, triangle*3+2]
            if reflect < 0:
                ids[1],ids[2] = ids[2],ids[1]
            truth.append({'source':[node,node,0,triangle], 'expanded':node, 'material':node, 'vertices':ids,
                          'positions':[[f32(reflect*f32(pos[v][0])+translation[0]),f32(f32(pos[v][1])+translation[1]),f32(f32(pos[v][2])+translation[2])] for v in ids],
                          'normal':[[0.,0.,1.]]*3,'tangent':[[float(reflect),0.,0.,float(reflect)]]*3,
                          'color':[[f32(c) for c in colors[v]] for v in ids],
                          'uvs':[[[f32(c) for c in uv[v]] for v in ids] for uv in (uv0,uv1)]})
    samplers = [{'wrapS':10497,'wrapT':33648,'magFilter':9728,'minFilter':9728},
                {'wrapS':33071,'wrapT':33071,'magFilter':9729,'minFilter':9729}]
    textures = [{'source':i%5,'sampler':i//5} for i in range(10)]
    materials = [material(0,0.0),material(5,-2.0,True)]
    doc = {'asset':{'version':'2.0'},'scene':0,'scenes':[{'nodes':[0,1]}],
           'nodes':[{'mesh':0,'translation':[0.125,-0.25,0.5]}, {'mesh':1,'translation':[20.375,3.125,-1.25],'scale':[-1,1,1]}],
           'meshes':meshes,'buffers':[{'uri':'payload.bin','byteLength':len(payload)}], 'bufferViews':views,'accessors':accessors,
           'images':[{'uri':i['uri']} for i in images],'textures':textures,'samplers':samplers,'materials':materials}
    (directory/'payload.bin').write_bytes(payload)
    dump(directory/'source.gltf',doc)
    expected = {'schemaVersion':1,'mode':'small','codecPositiveCandidate':codec_positive,'codecLowVarianceCandidate':codec_low_variance,'signalScope':'opaque low-variance checker only; no general codec claim' if codec_low_variance else 'original corpus or explicit codec-positive variant','normalSignal':'equal-magnitude-byte-unit-approximation' if unit_normals or codec_positive or codec_low_variance else 'varying-nonunit-source', 'triangles':truth,'images':images,'materials':materials,'textures':textures,'samplers':samplers}
    expected['inputSha256'] = sha(directory/'source.gltf')
    expected['assets'] = {name:sha(directory/name) for name in ['payload.bin',*(i['uri'] for i in images)]}
    dump(directory/'truth.json',expected)
    return {'input':str(directory/'source.gltf'), 'truth':str(directory/'truth.json'), 'triangles':len(truth)}


def generate_scale(directory, triangles):
    """True growing indexed grid; no full vertex/triangle arrays are retained."""
    require(triangles > 0, 'positive scale count')
    directory.mkdir(parents=True,exist_ok=True)
    columns = max(1,math.isqrt((triangles+1)//2))
    rows = (triangles+2*columns-1)//(2*columns)
    vertices = (columns+1)*(rows+1)
    views, accessors = [], []
    with (directory/'payload.bin').open('wb') as output:
        def field(width, shape, writer, extrema=None):
            start = output.tell()
            writer(output)
            views.append({'buffer':0,'byteOffset':start,'byteLength':output.tell()-start})
            a = {'bufferView':len(views)-1,'componentType':5126,'count':vertices,'type':shape}
            if extrema:
                a.update(min=extrema[0],max=extrema[1])
            accessors.append(a)
        def positions(f):
            for y in range(rows+1):
                block = array.array('f')
                for x in range(columns+1):
                    block.extend(grid_position(x,y))
                if sys.byteorder != 'little': block.byteswap()
                f.write(block.tobytes())
        def uv(f):
            for y in range(rows+1):
                block = array.array('f')
                for x in range(columns+1):block.extend((f32(x/columns),f32(y/rows)))
                if sys.byteorder != 'little':block.byteswap()
                f.write(block.tobytes())
        def repeated(value):
            packed = struct.pack('<'+'f'*len(value),*value)
            block = packed*8192
            def write(f):
                whole,tail = divmod(vertices,8192)
                for _ in range(whole):f.write(block)
                f.write(packed*tail)
            return write
        field(3,'VEC3',positions,([f32(0.000137),f32(0.000173),0.],[f32(columns/10+0.000137),f32(rows/10+0.000173),f32(min(columns,16)*0.0001)]))
        field(3,'VEC3',repeated((0,0,1)))
        field(4,'VEC4',repeated((1,0,0,1)))
        field(2,'VEC2',uv)
        field(4,'VEC4',repeated((0.2,0.4,0.6,1)))
        start = output.tell()
        remaining = triangles
        for y in range(rows):
            block = array.array('I')
            for x in range(columns):
                a = y*(columns+1)+x
                for values in ((a,a+1,a+columns+1),(a+1,a+columns+2,a+columns+1)):
                    if remaining == 0:break
                    block.extend(values);remaining -= 1
                if remaining == 0:break
            if sys.byteorder != 'little':block.byteswap()
            output.write(block.tobytes())
            if remaining == 0:break
        require(remaining == 0,'scale index count')
        views.append({'buffer':0,'byteOffset':start,'byteLength':output.tell()-start})
        accessors.append({'bufferView':len(views)-1,'componentType':5125,'count':triangles*3,'type':'SCALAR'})
        length = output.tell()
    images = image_set(directory,True)
    mat = material(0,1.0)
    for i,name in enumerate(SLOTS):
        target = mat['pbrMetallicRoughness'] if i<2 else mat
        target[name]['texCoord'] = 0
    sampler = {'wrapS':33071,'wrapT':33071,'magFilter':9728,'minFilter':9728}
    doc = {'asset':{'version':'2.0'},'scene':0,'scenes':[{'nodes':[0]}],'nodes':[{'mesh':0}],
           'meshes':[{'primitives':[{'attributes':{'POSITION':0,'NORMAL':1,'TANGENT':2,'TEXCOORD_0':3,'COLOR_0':4},'indices':5,'material':0}]}],
           'buffers':[{'uri':'payload.bin','byteLength':length}],'bufferViews':views,'accessors':accessors,
           'images':[{'uri':i['uri']} for i in images],'textures':[{'source':i,'sampler':0} for i in range(5)],'samplers':[sampler],'materials':[mat]}
    dump(directory/'source.gltf',doc)
    truth = {'schemaVersion':1,'mode':'scale','triangleCount':triangles,'columns':columns,'rows':rows,'vertices':vertices,
             'materials':[mat],'images':images,'textures':doc['textures'],'samplers':[sampler],'inputSha256':sha(directory/'source.gltf'),
             'signalScope':'five real constant8x8 slots; no high-frequency normal/mip or mixed-sampler acceptance claim',
             'assets':{name:sha(directory/name) for name in ['payload.bin',*(i['uri'] for i in images)]}}
    dump(directory/'truth.json',truth)
    return {'input':str(directory/'source.gltf'),'truth':str(directory/'truth.json'),'triangles':triangles,'vertices':vertices,'payloadBytes':length}


def grid_position(x,y):
    return (f32(x/10+0.000137),f32(y/10+0.000173),f32((x%17)*0.0001))


def expected_scale(truth, ordinal):
    c = truth['columns']
    cell, parity = divmod(ordinal,2)
    y,x = divmod(cell,c)
    a = y*(c+1)+x
    ids = [a,a+1,a+c+1] if parity == 0 else [a+1,a+c+2,a+c+1]
    return {'source':[0,0,0,ordinal],'expanded':0,'material':0,'vertices':ids,
            'positions':[list(grid_position(v%(c+1),v//(c+1))) for v in ids],
            'normal':[[0.,0.,1.]]*3,'tangent':[[1.,0.,0.,1.]]*3,'color':[[f32(.2),f32(.4),f32(.6),1.]]*3,
            'uvs':[[[f32((v%(c+1))/c),f32((v//(c+1))/truth['rows'])] for v in ids]]}


def decoded(path, cache=None):
    result = subprocess.run(['node',str(DECODER),str(path)],check=True,capture_output=True,text=True)
    packet = json.loads(result.stdout)
    if cache:
        cache.mkdir(parents=True,exist_ok=True)
        dump(cache/(path.stem+'.decoded.json'),packet)
    return packet['document'], [base64.b64decode(v,validate=True) for v in packet['views']],packet


def accessor(doc, views, identifier):
    a = doc['accessors'][identifier]
    require(not a.get('normalized',False),'unexpected normalized accessor')
    component = a['componentType']
    require(component in (5123,5126),'writer component domain')
    fmt,size = ('H',2) if component == 5123 else ('f',4)
    width = {'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']]
    view = doc['bufferViews'][a['bufferView']]
    stride = view.get('byteStride',size*width)
    data = views[a['bufferView']]
    start = a.get('byteOffset',0)
    require(start+(a['count']-1)*stride+size*width <= len(data),'accessor range')
    return [list(struct.unpack_from('<'+fmt*width,data,start+i*stride)) for i in range(a['count'])]


def metadata(doc, views):
    extension = doc['extensions']['EXT_structural_metadata']
    require(extension['schema']['id'] == 'rusty_tiles_large_source_v1','numeric provenance schema')
    tables = []
    for table in extension['propertyTables']:
        spec = extension['schema']['classes'][table['class']]['properties']
        columns = {}
        for name, prop in table['properties'].items():
            component = spec[name]['componentType']
            fmt,width = {'UINT64':('Q',8),'UINT32':('I',4),'UINT8':('B',1)}[component]
            data = views[prop['values']]
            require(len(data) == table['count']*width,'metadata exact column length')
            columns[name] = [v[0] for v in struct.iter_unpack('<'+fmt,data)]
        tables.append((table['count'],columns))
    require(len(tables) == 2,'two picking tables')
    return tables


def slot_binding(material_value, slot):
    return (material_value['pbrMetallicRoughness'] if slot < 2 else material_value)[SLOTS[slot]]


def image_index(texture):
    return texture.get('extensions',{}).get('KHR_texture_basisu',{}).get('source',texture.get('source'))


def wrap_tap(value,size,mode):
    if mode == 33071:
        return min(max(value,0),size-1)
    if mode == 10497:
        return value%size
    if mode == 33648:
        phase = value%(2*size)
        return phase if phase < size else 2*size-1-phase
    raise AssertionError('source sampler wrap enum')


def check_ktx(data, roles, profile, receipt):
    require(data[:12] == b'\xabKTX 20\xbb\r\n\x1a\n','KTX2 signature')
    require(len(data) >= 80,'KTX2 header length')
    vk,size,w,h,depth,layers,faces,count,scheme = struct.unpack_from('<9I',data,12)
    require(vk == 0 and size == 1 and w > 0 and h > 0 and depth == 0 and layers == 0 and faces == 1,'KTX2 Basis 2D shape')
    require(count == max(w,h).bit_length(),'complete mip chain to1x1')
    require((w,h,count) == (receipt['width'],receipt['height'],receipt['levels']),'KTX2/receipt shape')
    require(len(data) >= 80+24*count,'KTX2 mip index')
    ranges = []
    for level in range(count):
        offset,length,raw = struct.unpack_from('<QQQ',data,80+24*level)
        require(length > 0 and offset >= 80+24*count and offset+length <= len(data),'KTX2 indexed payload closure')
        ranges.append((offset,offset+length))
        require(raw > 0 if scheme != 1 else raw == 0,'KTX2 scheme length')
    require(all(a[1] <= b[0] for a,b in zip(sorted(ranges),sorted(ranges)[1:])),'KTX2 overlapping mips')
    dfd,length = struct.unpack_from('<II',data,48)
    require(dfd >= 80+24*count and length >= 16 and dfd+length <= len(data),'KTX2 DFD range')
    require(struct.unpack_from('<I',data,dfd)[0] == length,'DFD total size')
    transfer = data[dfd+14]
    expected_transfer = 2 if next(iter(roles)) in (0,4) else 1
    require(all((r in (0,4)) == (next(iter(roles)) in (0,4)) for r in roles),'semantic page mixed classes')
    require(transfer == expected_transfer,'DFD semantic transfer class')
    require(data[dfd+12] in (163,166),'Basis color model')
    # Receipts are declarations only: assert frozen floor policy, NOT measurements.
    quality_rows = [q for q in receipt['quality'] if 'channels' in q]
    require({q['mip'] for q in quality_rows} == set(range(count)),'quality receipt every level')
    role = next(iter(roles))
    channels = {0,3} if role == 3 else {1,2,3} if role == 1 else {0,1,2,3}
    for q in quality_rows:
        require({c['channel'] for c in q['channels']} == channels,'meaningful-channel receipt inventory')
        for c in q['channels']:
            ch = c['channel']
            if ch == 3:
                floor,cap = (34,32) if profile == 'quality' else (30,80 if profile == 'fast' else 64)
            elif role in (0,4):
                floor,cap = (34,24) if profile == 'quality' else (22,56 if profile == 'fast' else 32)
            elif role == 2:
                floor,cap = 30,112
            else:
                floor,cap = 30,(8 if profile == 'quality' else 80) if role == 3 else 32
            require((c['floor'],c['cap']) == (floor,cap),'unchanged declared quality floors')


def check_material(output, source):
    expected = copy.deepcopy(source)
    for slot in range(5):
        a,b = slot_binding(output,slot),slot_binding(expected,slot)
        require('index' in a and a.get('texCoord',0) == slot,'dense slot UV assignment')
        b['index'],b['texCoord'] = a['index'],a.get('texCoord',0)
    require(output == expected,'literal material factors/normalScale preserved')


def check_features(features, tables):
    require(len(features) == 2,'complete primitive/triangle feature inventory')
    observed=set()
    for feature in features:
        require(set(feature) == {'featureCount','attribute','propertyTable','label'},'pinned numeric picking feature fields')
        table=feature['propertyTable']
        require(type(table) is int and table in (0,1) and table not in observed,'unique numeric picking property table')
        observed.add(table)
        require(feature['attribute'] == table,'picking feature selects matching _FEATURE_ID accessor')
        require(feature['featureCount'] == tables[table][0],'global feature/property-table domain')
        require(feature['label'] == ('source_primitive' if table == 0 else 'source_triangle'),'picking labels')
    require(observed == {0,1},'both numeric picking levels present')


def check_sampler(output, source):
    require(source['wrapS'] in (33071,10497,33648) and source['wrapT'] in (33071,10497,33648),'source wrap inventory')
    require(source['magFilter'] in (9728,9729) and source['minFilter'] in (9728,9729,9984,9985,9986,9987),'source filter inventory')
    require(output == {**source,'wrapS':33071,'wrapT':33071},'atlas clamp with exact source min/mag filters')


def check_pixels(page, source, binding):
    w,h,pixels = page
    sw,sh = source['width'],source['height']
    ox,oy = binding['cropOrigin']
    px,py = binding['pageOffset']
    cw,ch = binding['cropSize']
    require(px >= 0 and py >= 0 and px+cw <= w and py+ch <= h,'crop/page rectangle')
    # Every copied crop cell INCLUDING source-signal halo and two outer gutters;
    # lossless producer does not class-normalize/recolor/filter these values.
    sampler = binding['sourceSampler']
    for y in range(ch):
        sy = wrap_tap(oy+y-2,sh,sampler['wrapT'])
        for x in range(cw):
            sx = wrap_tap(ox+x-2,sw,sampler['wrapS'])
            target = ((py+y)*w+px+x)*4
            original = (sy*sw+sx)*4
            require(pixels[target:target+4] == bytes(source['rgba'][original:original+4]),'PNG literal crop/source RGBA including transparent RGB/nonunit normal')


def check_page_mapping(uvs, source_uvs, source, binding, page_size):
    require(binding['densityDrop'] == 0,'this acceptance fixture requires no density reduction')
    dimensions = (source['width'],source['height'])
    origins = [math.floor(min(uv[a] for uv in source_uvs)*dimensions[a])-1 for a in range(2)]
    ends = [math.ceil(max(uv[a] for uv in source_uvs)*dimensions[a])+1 for a in range(2)]
    require(binding['cropOrigin'] == origins,'source-authored crop origin/halo')
    require(binding['cropSize'] == [ends[a]-origins[a]+4 for a in range(2)],'source-authored crop size/halo')
    for uv,source_uv in zip(uvs,source_uvs):
        wanted = [f32((source_uv[a]*dimensions[a]-origins[a]+2+binding['pageOffset'][a])/page_size[a]) for a in range(2)]
        require(uv == wanted,'positive crop UV map/half-texel origin')


def quant_byte(value):
    return min(255,max(0,math.floor(value*255+0.5)))


def unit_normal(pixel):
    vector = [x/127.5-1 for x in pixel[:3]]
    length = math.sqrt(sum(x*x for x in vector))
    require(length > 1e-12,'independent normal donor zero')
    return [x/length for x in vector]


def source_page_chain(truth, bindings, page_id, dimensions):
    """Independent source reference; brute-force gutters, no producer mip data."""
    w,h = dimensions
    require(4 <= w <= 32 and 4 <= h <= 32 and w&(w-1) == 0 and h&(h-1) == 0,
            'decoded oracle domain: small POT page <=32')
    selected = [b for b in bindings.values() if b['page'] == page_id]
    roles = {b['slot'] for b in selected}
    require(len(roles) == 1,'decoded oracle domain: one semantic role per page')
    role = next(iter(roles))
    pixels,support = [0]*(w*h*4),[0]*(w*h)
    for binding in selected:
        require(binding['densityDrop'] == 0,'decoded oracle domain: D0 only')
        source = truth['images'][binding['sourceImage']]
        require(all(a == 255 for a in source['rgba'][3::4]),'decoded oracle domain: opaque source alpha only; variable MASK unsupported')
        ox,oy = binding['cropOrigin']; px,py = binding['pageOffset']; cw,ch = binding['cropSize']
        sampler = binding['sourceSampler']
        # The two outer cells are gutters; cropOrigin already includes the REAL one-texel source halo.
        for y in range(2,ch-2):
            for x in range(2,cw-2):
                tx,ty = px+x,py+y
                require(0 <= tx < w and 0 <= ty < h,'source signal crop/page closure')
                i = ty*w+tx
                require(support[i] == 0,'independent crop interiors must not overlap')
                sx = wrap_tap(ox+x-2,source['width'],sampler['wrapS'])
                sy = wrap_tap(oy+y-2,source['height'],sampler['wrapT'])
                p = source['rgba'][(sy*source['width']+sx)*4:(sy*source['width']+sx)*4+4]
                if role == 2:
                    p = [quant_byte((v+1)/2) for v in unit_normal(p)]+[p[3]]
                pixels[i*4:i*4+4] = p; support[i] = 1
    require(sum(support) > 0,'independent page real source signal')
    def gutters(width,height,rgba,weights):
        donors = [i for i,v in enumerate(weights) if v]
        for i,weight in enumerate(weights):
            if not weight:
                owner = min(donors,key=lambda j:(abs(i%width-j%width)+abs(i//width-j//width),j))
                rgba[i*4:i*4+4] = rgba[owner*4:owner*4+4]
    gutters(w,h,pixels,support)
    chain = [(w,h,bytes(pixels),support)]
    def linear(x):
        v=x/255
        return v/12.92 if v <= .04045 else ((v+.055)/1.055)**2.4
    def srgb(v):
        return quant_byte(12.92*v if v <= .0031308 else 1.055*v**(1/2.4)-.055)
    while (w,h) != (1,1):
        dw,dh = max(1,w//2),max(1,h//2)
        target,weights = [0]*(dw*dh*4),[0]*(dw*dh)
        for y in range(dh):
            for x in range(dw):
                cells = [sy*w+sx for sy in range(2*y,min(2*y+2,h)) for sx in range(2*x,min(2*x+2,w)) if support[sy*w+sx]]
                if not cells:
                    continue
                vectors = [unit_normal(pixels[i*4:i*4+4]) if role == 2 else
                           [linear(c) for c in pixels[i*4:i*4+3]] if role in (0,4) else
                           [c/255 for c in pixels[i*4:i*4+3]] for i in cells]
                total = sum(support[i] for i in cells)
                rgb = [sum(v[a]*support[i] for i,v in zip(cells,vectors)) for a in range(3)]
                if role == 2:
                    length = math.sqrt(sum(v*v for v in rgb))
                    if length <= 1e-12:
                        donor = max(range(len(cells)),key=lambda k:(support[cells[k]],-cells[k]))
                        rgb = vectors[donor]
                    else:
                        rgb = [v/length for v in rgb]
                    encoded = [quant_byte((v+1)/2) for v in rgb]
                else:
                    encoded = [(srgb(v/total) if role in (0,4) else quant_byte(v/total)) for v in rgb]
                alpha = quant_byte(sum(pixels[i*4+3]/255*support[i] for i in cells)/total)
                j = y*dw+x; target[j*4:j*4+4] = encoded+[alpha]; weights[j] = total
        gutters(dw,dh,target,weights)
        w,h,pixels,support = dw,dh,target,weights
        chain.append((w,h,bytes(pixels),support))
    return role,chain


def decoded_metrics(expected, actual, weights, role, profile):
    require(len(expected) == len(actual) == len(weights)*4,'decoded artifact RGBA shape')
    total = sum(weights)
    channels = (0,3) if role == 3 else (1,2,3) if role == 1 else (0,1,2,3)
    metrics = []
    for channel in channels:
        errors = [(abs(expected[i*4+channel]-actual[i*4+channel]),weight) for i,weight in enumerate(weights) if weight]
        mse = sum(e*e*weight for e,weight in errors)/total
        psnr = math.inf if mse == 0 else 10*math.log10(255*255/mse)
        maximum = max(e for e,_ in errors)
        if channel == 3:
            floor,cap = (34,32) if profile == 'quality' else (30,80 if profile == 'fast' else 64)
        elif role in (0,4):
            floor,cap = (34,24) if profile == 'quality' else (22,56 if profile == 'fast' else 32)
        elif role == 2:
            floor,cap = 30,112
        else:
            floor,cap = 30,((8 if profile == 'quality' else 80) if role == 3 else 32)
        require(psnr >= floor and maximum <= cap,f'independent decoded role{role} channel{channel} PSNR{psnr} max{maximum} floor{floor} cap{cap}')
        metrics.append({'channel':channel,'psnr':None if math.isinf(psnr) else psnr,'max':maximum})
    normal = None
    if role == 2:
        lengths,angles = [],[]
        for i,weight in enumerate(weights):
            if not weight: continue
            v = [x/127.5-1 for x in actual[i*4:i*4+3]]
            q = [x/127.5-1 for x in expected[i*4:i*4+3]]
            vl,ql = math.sqrt(sum(x*x for x in v)),math.sqrt(sum(x*x for x in q))
            require(vl > 1e-12 and ql > 1e-12,'independent decoded normal nonzero')
            lengths.append((abs(vl-1),weight))
            angles.append((math.degrees(math.acos(max(-1,min(1,sum(a*b for a,b in zip(v,q))/(vl*ql))))),weight))
        mean = sum(e*w for e,w in lengths)/total; maximum=max(e for e,_ in lengths)
        angular = sum(e*w for e,w in angles)/total
        cumulative,p99 = 0,0
        for angle,weight in sorted(angles):
            cumulative += weight; p99=angle
            if cumulative >= .99*total: break
        mean_cap,max_cap = {'fast':(.08,.20),'balanced':(.05,.15),'quality':(.03,.10)}[profile]
        require(mean <= mean_cap and maximum <= max_cap and angular <= 3 and p99 <= 12,'independent normal length/angular floors')
        normal={'lengthMean':mean,'lengthMax':maximum,'angleMean':angular,'angleP99':p99}
    return {'channels':metrics,'normal':normal}


def read_decoded_chain(directory, index, uri, page_id, glb_hash, encoded, dimensions):
    key=(uri,page_id)
    require(key in index,'missing actual decoded page')
    row=index[key]
    require(row['glbSha256'] == glb_hash and row['pageSha256'] == hashlib.sha256(encoded).hexdigest(),'decoded output wire/page identity')
    w,h=dimensions; levels=[]
    for mip,level in enumerate(row['levels']):
        require((level['mip'],level['width'],level['height']) == (mip,w,h),'actual decoded mip inventory/order')
        path=(directory/level['file']).resolve()
        require(path.is_relative_to(directory.resolve()),'decoded path confinement')
        data=path.read_bytes()
        require(sha(path) == level['sha256'] and len(data) == w*h*4,'decoded level hash/shape')
        levels.append((w,h,data)); w,h=max(1,w//2),max(1,h//2)
    require(len(levels) == max(dimensions).bit_length() and levels[-1][:2] == (1,1),'decoded complete authored levels')
    return levels


def artifact_sensitivity(expected, actual, weights, role, profile):
    """Negative decoded-byte probes; no altered allowance or reference receipts."""
    labels=[]
    def rejected(label, channels, value):
        mutant=bytearray(actual)
        for i,weight in enumerate(weights):
            if weight:
                for channel in channels:
                    mutant[i*4+channel]=value(channel,expected[i*4+channel])
        try:
            decoded_metrics(expected,mutant,weights,role,profile)
        except AssertionError:
            labels.append(label); return
        raise AssertionError('decoded artifact control failed to reject '+label)
    rejected('alpha-source-reference',(3,),lambda c,x:0)
    channels=(0,) if role == 3 else (1,2) if role == 1 else (0,1,2)
    rejected('nonbase-data' if role in (1,3) else 'normal-vector' if role == 2 else 'colour-RGB',channels,lambda c,x:0)
    if role in (0,4):
        rejected('gamma-transfer',(0,1,2),lambda c,x:quant_byte((x/255)**2.2))
    return labels


def opaque_mask_coverage(rows, delivered_points, output_uvs, levels, material, sampler):
    """Original opaque-source labels at 16 equal-area centroids, weighted by decoded area."""
    if material.get('alphaMode') != 'MASK': return []
    factor=material['pbrMetallicRoughness']['baseColorFactor'][3]
    cutoff=material.get('alphaCutoff',.5)
    centroids=[]
    for i in range(4):
        for j in range(4-i):
            u,v=(3*i+1)/12,(3*j+1)/12
            centroids.append((1-u-v,u,v))
            if i+j < 3:
                u,v=(3*i+2)/12,(3*j+2)/12
                centroids.append((1-u-v,u,v))
    require(len(centroids) == 16,'independent finite geometry sample inventory')
    samples=[]
    for t,row in enumerate(rows):
        p=delivered_points[t]
        a=[p[1][k]-p[0][k] for k in range(3)]; b=[p[2][k]-p[0][k] for k in range(3)]
        cross=(a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0])
        area=math.sqrt(sum(v*v for v in cross))/2
        if not area: continue
        for bary in centroids:
            alpha=sum(row['color'][c][3]*bary[c] for c in range(3))
            uv=[sum(output_uvs[t*3+c][axis]*bary[c] for c in range(3)) for axis in range(2)]
            samples.append((uv,alpha,area/16,alpha*factor >= cutoff))
    total=sum(s[2] for s in samples)
    target=0 if total == 0 else sum(s[2] for s in samples if s[3])/total
    def filtered(w,h,data,uv,linear):
        def tap(x,y): return data[(min(max(y,0),h-1)*w+min(max(x,0),w-1))*4+3]/255
        x,y=uv[0]*w,uv[1]*h
        if not linear: return tap(math.floor(x),math.floor(y))
        x-=.5; y-=.5; ix,iy=math.floor(x),math.floor(y); fx,fy=x-ix,y-iy
        return ((1-fx)*tap(ix,iy)+fx*tap(ix+1,iy))*(1-fy)+((1-fx)*tap(ix,iy+1)+fx*tap(ix+1,iy+1))*fy
    out=[]
    for mip,(w,h,data) in enumerate(levels):
        linear=sampler['magFilter'] == 9729 if mip == 0 else sampler['minFilter'] in (9729,9985,9987)
        coverage=0 if total == 0 else sum(weight for uv,alpha,weight,_ in samples if filtered(w,h,data,uv,linear)*alpha*factor >= cutoff)/total
        out.append({'mip':mip,'sourceTarget':target,'coverage':coverage,'sampleCount':len(samples)})
    return out


def verify(truth_path, output, decode_cache=None, controls=False, ktx_decoded=None):
    truth = json.loads(truth_path.read_text())
    require(sha(truth_path.parent/'source.gltf') == truth['inputSha256'],'source document identity')
    for name,digest in truth.get('assets',{}).items():
        require(sha(truth_path.parent/name) == digest,'source payload/image identity')
    small = truth['mode'] == 'small'
    decoded_index,decoded_wire_seen,artifact_metrics = {},set(),[]
    if ktx_decoded:
        require(small,'decoded artifact oracle supports small fixtures only')
        for source_image in truth['images']:
            original=decode_png((truth_path.parent/source_image['uri']).read_bytes())
            require(original == (source_image['width'],source_image['height'],bytes(source_image['rgba'])),
                    'source PNG decoded dimensions/pixels equal independent fixture truth')
        decoded_document=json.loads((ktx_decoded/'decoded.json').read_text())
        require(decoded_document['schemaVersion'] == 1 and decoded_document['driverSha256'] == sha(pathlib.Path(__file__).with_name('decode_ktx.rs')),'pinned external KTX decoder driver')
        for row in decoded_document['pages']:
            key=(row['uri'],row['page']); require(key not in decoded_index,'duplicate decoded page'); decoded_index[key]=row
    expected = {tuple(r['source']):r for r in truth.get('triangles',[])}
    total = len(expected) if small else truth['triangleCount']
    seen = set() if small else bytearray((total+7)//8)
    manifest = json.loads((output/'manifest.json').read_text())
    profile = manifest['report']['profile']
    require(manifest['provenanceSchema'] == 'rusty_tiles_large_source_v1','published provenance declaration')
    require(manifest['report']['triangles'] == total,'published triangle report')
    require(manifest['lattice']['common_origin'] == [0.,0.,0.],'zero lattice origin')
    leaves, observed, png_pages, ktx_pages, checks, decoder_hash = 0,0,0,0,0,None
    seams = {}
    first_control = None
    for line in (output/'leaves.jsonl').open():
        receipt = json.loads(line)
        member = (output/receipt['uri']).resolve()
        require(member.is_relative_to(output.resolve()),'member URI confinement')
        require(sha(member) == receipt['sha256'] and member.stat().st_size == receipt['bytes'],'published member identity')
        doc,views,packet = decoded(member,decode_cache)
        require(packet['wireSha256'] == receipt['sha256'],'independent wire identity')
        if decoder_hash:
            require(decoder_hash == packet['decoderSha256'],'one pinned meshopt decoder')
        decoder_hash = packet['decoderSha256']
        tables = metadata(doc,views)
        appearance = receipt['appearance']
        bindings = {(b['primitive'],b['slot']):b for b in appearance if 'slot' in b}
        page_receipts = {b['page']:b for b in appearance if 'quality' in b}
        pages,actual_chains,reference_chains = {},{},{}
        for image_id,image in enumerate(doc.get('images',[])):
            data = views[image['bufferView']]
            page = page_receipts[image_id]
            require(page['bytes'] == len(data) and page['mime'] == image['mimeType'],'page bytes/MIME receipt')
            roles = {b['slot'] for b in bindings.values() if b['page'] == image_id}
            require(roles,'unreferenced page receipt')
            if image['mimeType'] == 'image/png':
                require(profile == 'lossless','PNG is lossless exception only')
                pages[image_id] = decode_png(data)
                require(pages[image_id][:2] == (page['width'],page['height']) and page['levels'] == 1,'PNG page shape')
                png_pages += 1
            else:
                require(image['mimeType'] == 'image/ktx2' and profile != 'lossless','KTX2 output role')
                check_ktx(data,roles,profile,page)
                pages[image_id] = (page['width'],page['height'],None)
                ktx_pages += 1
                if ktx_decoded:
                    dimensions=(page['width'],page['height'])
                    actual_chains[image_id]=read_decoded_chain(ktx_decoded,decoded_index,receipt['uri'],image_id,receipt['sha256'],data,dimensions)
                    decoded_wire_seen.add((receipt['uri'],image_id))
                    role,reference=source_page_chain(truth,bindings,image_id,dimensions)
                    reference_chains[image_id]=reference
                    require(len(reference) == len(actual_chains[image_id]),'source/actual complete level count')
                    for mip,((w,h,expected_pixels,weights),actual) in enumerate(zip(reference,actual_chains[image_id])):
                        require((w,h) == actual[:2],'source/actual mip shape')
                        metrics=decoded_metrics(expected_pixels,actual[2],weights,role,profile)
                        probes=artifact_sensitivity(expected_pixels,actual[2],weights,role,profile) if controls and mip == 0 else []
                        artifact_metrics.append({'uri':receipt['uri'],'page':image_id,'mip':mip,'role':role,'metrics':metrics,'controls':probes})
        require(len(doc['nodes']) == len(doc['meshes']),'one geometry node per prepared primitive')
        counted = 0
        for pi,node in enumerate(doc['nodes']):
            require(node['mesh'] == pi and set(node) <= {'mesh','translation','scale'},'local diagonal node closure')
            primitive = doc['meshes'][pi]['primitives'][0]
            require(primitive['mode'] == 4 and 'indices' not in primitive,'nonindexed TRIANGLES')
            attrs = primitive['attributes']
            values = {name:accessor(doc,views,identifier) for name,identifier in attrs.items()}
            count = len(values['POSITION'])
            require(count % 3 == 0 and all(len(v) == count for v in values.values()),'every corner retained')
            require(set(attrs) == {'POSITION','NORMAL','TANGENT','COLOR_0','_FEATURE_ID_0','_FEATURE_ID_1',*(f'TEXCOORD_{i}' for i in range(5))},'dense five-slot attribute closure')
            check_features(primitive['extensions']['EXT_mesh_features']['featureIds'],tables)
            original_rows,delivered_rows = [],[]
            tcolumns = tables[1][1]
            for ti in range(count//3):
                ids = [v[0] for v in values['_FEATURE_ID_1'][ti*3:ti*3+3]]
                primitive_ids = [v[0] for v in values['_FEATURE_ID_0'][ti*3:ti*3+3]]
                require(ids == [ids[0]]*3 and ids[0] == int(ids[0]) and 0 <= ids[0] < tables[1][0],'triangle feature ID bounds/corners')
                require(primitive_ids == [primitive_ids[0]]*3 and primitive_ids[0] == int(primitive_ids[0]) and 0 <= primitive_ids[0] < tables[0][0],'primitive feature ID bounds/corners')
                tid,pid = int(ids[0]),int(primitive_ids[0])
                identity = tuple(tcolumns[n][tid] for n in ('node_index','mesh_index','primitive_index','triangle_index'))
                pcols = tables[0][1]
                require(identity[:3] == tuple(pcols[n][pid] for n in ('node_index','mesh_index','primitive_index')),'primitive/triangle identity relation')
                if small:
                    require(identity in expected and identity not in seen,'source triangle exact-once provenance')
                    seen.add(identity);source = expected[identity]
                else:
                    ordinal = identity[3]
                    require(identity[:3] == (0,0,0) and 0 <= ordinal < total,'scale source provenance')
                    require(not (seen[ordinal//8] & (1 << (ordinal%8))),'scale duplicate source triangle')
                    seen[ordinal//8] |= 1 << (ordinal%8)
                    source = expected_scale(truth,ordinal)
                original_rows.append(source)
                require(tcolumns['expanded_ordinal'][tid] == source['expanded'] and tcolumns['material_index'][tid] == source['material'],'expanded/material identity')
                require([tcolumns[f'vertex_{i}'][tid] for i in range(3)] == source['vertices'],'authored corner identity/reflected winding')
                require(tcolumns['attribute_flags'][tid] == (31 if small else 23),'captured attribute presence')
                require(tcolumns['degenerate'][tid] == 0,'fixture source degeneracy flag')
                for name,truth_name in [('NORMAL','normal'),('TANGENT','tangent'),('COLOR_0','color')]:
                    require(values[name][ti*3:ti*3+3] == source[truth_name],'source '+name+' preservation/TBN')
                translation,scale = node.get('translation',[0.,0.,0.]),node.get('scale',[1.,1.,1.])
                decoded_position = []
                for corner in range(3):
                    p = values['POSITION'][ti*3+corner]
                    delivered = [f32(f32(p[a]*scale[a])+translation[a]) for a in range(3)]
                    require(delivered == [p[a]*scale[a]+translation[a] for a in range(3)],'real TRS equals source-snapped f32')
                    require(all(receipt['bounds']['min'][a] <= delivered[a] <= receipt['bounds']['max'][a] for a in range(3)),'published decoded bound')
                    distance = math.dist(delivered,source['positions'][corner])
                    require(distance <= receipt['geometryError'],'bidirectional triangle correspondence error')
                    decoded_position.append(delivered)
                    if small:
                        position_key = tuple(source['positions'][corner])
                        seams.setdefault(position_key,[]).append((receipt['depth'],delivered))
                delivered_rows.append(decoded_position)
                if first_control is None:
                    first_control = (values,source,receipt,doc,views,bindings,page_receipts,pi)
                counted += 1;observed += 1;checks += 1
            source_material = truth['materials'][original_rows[0]['material']]
            require(all(r['material'] == original_rows[0]['material'] for r in original_rows),'prepared primitive source material grouping')
            output_material = doc['materials'][primitive['material']]
            check_material(output_material,source_material)
            for slot in range(5):
                binding = bindings[(pi,slot)]
                source_binding = slot_binding(source_material,slot)
                source_texture = truth['textures'][source_binding['index']]
                require(binding['sourceImage'] == source_texture['source'] and binding['sourceUV'] == source_binding.get('texCoord',0) and binding['outputUV'] == slot,'receipt source/slot identity')
                expected_sampler = truth['samplers'][source_texture['sampler']]
                require(binding['sourceSampler'] == expected_sampler,'source sampler/wrap/filter receipt')
                output_binding = slot_binding(output_material,slot)
                texture = doc['textures'][output_binding['index']]
                page_id = image_index(texture)
                require(page_id == binding['page'],'slot texture/page linkage')
                output_sampler = doc['samplers'][texture['sampler']]
                check_sampler(output_sampler,expected_sampler)
                source_uvs = [uv for r in original_rows for uv in r['uvs'][binding['sourceUV']]]
                source_image = truth['images'][binding['sourceImage']]
                check_page_mapping(values[f'TEXCOORD_{slot}'],source_uvs,source_image,binding,pages[page_id][:2])
                if pages[page_id][2] is not None:
                    check_pixels(pages[page_id],source_image,binding)
                if ktx_decoded and slot == 0 and source_material.get('alphaMode') == 'MASK':
                    # Opaque source makes independently attainable optimum g=0 at literal scale1.
                    reference_levels=[level[:3] for level in reference_chains[page_id]]
                    reference_coverage=opaque_mask_coverage(original_rows,delivered_rows,values['TEXCOORD_0'],reference_levels,source_material,expected_sampler)
                    actual_coverage=opaque_mask_coverage(original_rows,delivered_rows,values['TEXCOORD_0'],actual_chains[page_id],source_material,expected_sampler)
                    floor={'fast':.02,'balanced':.01,'quality':.005}[profile]
                    for baseline,actual in zip(reference_coverage,actual_coverage):
                        require(baseline['coverage'] == baseline['sourceTarget'],'decoded oracle domain: literal opaque MASK optimum must be zero')
                        require(abs(actual['coverage']-baseline['sourceTarget']) <= floor,'independent original-source finite MASK residual')
                        artifact_metrics.append({'uri':receipt['uri'],'page':page_id,'primitive':pi,'mask':actual,'independentSemanticGap':0,'floor':floor})
        require(counted == receipt['end']-receipt['first'],'leaf rank-span count')
        require(counted <= {'fast':8192,'balanced':8192,'quality':8192,'lossless':8192}[profile],'profile leaf item bound (CesiumJS property-table width, H16)')
        leaves += 1
    require(observed == total and leaves == manifest['report']['leaves'],'whole output source coverage/count')
    if small:
        require(seen == set(expected),'missing source triangle')
        steps = [f32(step) for step in manifest['lattice']['steps']]
        for source,incidence in seams.items():
            minimum = min(depth for depth,_ in incidence)
            for depth,p in incidence:
                step = steps[min(depth,minimum)]
                snapped = [f32(round(x/step)*step) if profile not in ('quality','lossless') else x for x in source]
                require(p == snapped,'independent all-incident shallowest lattice snap')
    if controls and first_control:
        sensitivity_controls(first_control,truth)
    if ktx_decoded:
        require(decoded_wire_seen == set(decoded_index),'decoded artifact page inventory exact closure')
    result = {'verdict':'PASS','schemaVersion':1,'triangles':observed,'leaves':leaves,'pngPages':png_pages,'ktxPages':ktx_pages,
              'checks':checks,'controlsExecuted':bool(controls),'decodedTextureAcceptance':'source-derived small opaque D0 complete KTX chains and finite MASK metric' if ktx_decoded and ktx_pages else False if ktx_pages else 'lossless level0 copied crop only',
              'rendererAcceptance':False,'normalSignal':truth.get('normalSignal',truth.get('signalScope')),'sourceEffectiveNormalAcceptance':False,'scaleSeamScope':'small all-incident check' if small else 'per-leaf decoded bounds/provenance; no scale-global seam certification',
              'truthSha256':sha(truth_path),'manifestSha256':sha(output/'manifest.json'),'leafManifestSha256':sha(output/'leaves.jsonl'),
              'oracleSha256':sha(__file__),'driverSha256':sha(DECODER),'meshoptDecoderSha256':decoder_hash}
    if ktx_decoded:
        result.update(decodedArtifactIndexSha256=sha(ktx_decoded/'decoded.json'),ktxDecoderDriverSha256=decoded_document['driverSha256'],artifactMetrics=artifact_metrics,decodedOracleScope='small POT<=32 opaque D0; raw-unit semantic normal reference; no source-effective filtered normal/rendering equality')
    result['passHash'] = hashlib.sha256(json.dumps(result,sort_keys=True,separators=(',',':')).encode()).hexdigest()
    return result


def sensitivity_controls(snapshot, truth):
    """Mutate decoded semantic models, not producer source or merely wire hashes."""
    values,source,receipt,doc,views,bindings,page_receipts,pi = snapshot
    failures = []
    def reject(name, check):
        try:
            check()
        except AssertionError:
            failures.append(name)
        else:
            raise AssertionError('insensitive oracle control: '+name)
    mutant_tbn = copy.deepcopy(values['TANGENT'][:3])
    mutant_tbn[0][3] *= -1
    reject('tangent-handedness',lambda:require(mutant_tbn == source['tangent'],'TBN control'))
    primitive = doc['meshes'][pi]['primitives'][0]
    output_material = copy.deepcopy(doc['materials'][primitive['material']])
    output_material['normalTexture']['scale'] += 1
    reject('literal-normal-scale',lambda:check_material(output_material,truth['materials'][source['material']]))
    feature = primitive['extensions']['EXT_mesh_features']['featureIds'][1]
    mutant_id = feature['featureCount']
    reject('feature-id-upper-bound',lambda:require(mutant_id < feature['featureCount'],'feature bound control'))
    moved = source['positions'][0][:]
    moved[0] += max(1.,receipt['geometryError']*4)
    reject('decoded-position-correspondence',lambda:require(math.dist(moved,source['positions'][0]) <= receipt['geometryError'],'position control'))
    reject('authored-corner-provenance',lambda:require([v+1 for v in source['vertices']] == source['vertices'],'source identity control'))
    tables = metadata(doc,views)
    mutant_features=copy.deepcopy(primitive['extensions']['EXT_mesh_features']['featureIds'])
    mutant_features[1]['attribute']=0
    reject('feature-accessor-crosslink',lambda:check_features(mutant_features,tables))
    reject('feature-inventory-missing-triangle',lambda:check_features(mutant_features[:1],tables))
    source_rows = []
    expected = {tuple(r['source']):r for r in truth.get('triangles',[])}
    for i in range(len(values['POSITION'])//3):
        identifier = int(values['_FEATURE_ID_1'][3*i][0])
        columns = tables[1][1]
        identity = tuple(columns[n][identifier] for n in ('node_index','mesh_index','primitive_index','triangle_index'))
        source_rows.append(expected[identity] if truth['mode'] == 'small' else expected_scale(truth,identity[3]))
    binding = bindings[(pi,0)]
    source_sampler=binding['sourceSampler']
    mutant_sampler={**source_sampler,'wrapS':33071,'wrapT':33071,
                    'magFilter':9729 if source_sampler['magFilter'] == 9728 else 9728}
    reject('sampler-mag-filter-crosslink',lambda:check_sampler(mutant_sampler,source_sampler))
    page_size = (page_receipts[binding['page']]['width'],page_receipts[binding['page']]['height'])
    original_uv = [uv for row in source_rows for uv in row['uvs'][binding['sourceUV']]]
    mutant_uv = copy.deepcopy(values['TEXCOORD_0'])
    mutant_uv[0][0] = f32(mutant_uv[0][0]+0.5/page_size[0])
    reject('half-source-texel-UV-origin',lambda:check_page_mapping(mutant_uv,original_uv,truth['images'][binding['sourceImage']],binding,page_size))
    require(len(failures) == 9,'nine sensitive decoded controls')
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command',required=True)
    generator = sub.add_parser('generate')
    generator.add_argument('directory',type=pathlib.Path)
    generator.add_argument('--unit-normals',action='store_true',help='Explicit positive fixture variant; does not accept arbitrary nonunit normal sampling')
    generator.add_argument('--codec-positive',action='store_true',help='Explicit opaque checker/data/equal-magnitude normal candidate; codec success requires actual verification; original negative corpus unchanged')
    generator.add_argument('--codec-low-variance',action='store_true',help='Limited-variation opaque checker and small unit-normal approximation; actual codec acceptance required, all floors unchanged')
    scale = sub.add_parser('generate-scale')
    scale.add_argument('directory',type=pathlib.Path)
    scale.add_argument('--triangles',type=int,default=10_000_000)
    checker = sub.add_parser('verify')
    checker.add_argument('truth',type=pathlib.Path)
    checker.add_argument('output',type=pathlib.Path)
    checker.add_argument('--decode-cache',type=pathlib.Path)
    checker.add_argument('--ktx-decoded',type=pathlib.Path,help='Cache index/RGBA levels from dev-only decode_ktx.rs; small opaque D0 independent source-chain gate')
    checker.add_argument('--controls',action='store_true')
    checker.add_argument('--report',type=pathlib.Path)
    args = parser.parse_args()
    if args.command == 'generate':
        result = generate(args.directory,args.unit_normals,args.codec_positive,args.codec_low_variance)
    elif args.command == 'generate-scale':
        result = generate_scale(args.directory,args.triangles)
    else:
        if args.decode_cache:
            require(not args.decode_cache.resolve().is_relative_to(ROOT),'bulk decoded evidence must stay outside repository')
        if args.ktx_decoded:
            require(not args.ktx_decoded.resolve().is_relative_to(ROOT),'decoded KTX evidence must stay outside repository')
        result = verify(args.truth,args.output,args.decode_cache,args.controls,args.ktx_decoded)
        if args.report:
            require(not args.report.resolve().is_relative_to(ROOT),'execution report must stay outside repository')
            dump(args.report,result)
    print(json.dumps(result,sort_keys=True))


if __name__ == '__main__':
    main()
