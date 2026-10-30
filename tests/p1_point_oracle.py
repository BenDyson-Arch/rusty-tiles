#!/usr/bin/env python3
"""Independent P1 LAS/3TZ acceptance oracle. Python standard library only.

LAS bytes are authored from the LAS 1.2/1.4 public record layout; expected
records are enumerated by a separate raw decoder. ZIP, GLB accessor/scene and
implicit availability/metadata decoding never import rusty-tiles code.
"""
import argparse
import collections
import copy
import hashlib
import json
import math
import pathlib
import struct
import subprocess
import tempfile
import zipfile

FORMATS = {0:20, 1:28, 2:26, 3:34, 6:30, 7:36, 8:38}
KINDS = {'UINT8':'B','INT8':'b','UINT16':'H','INT16':'h','UINT32':'I','INT32':'i',
         'UINT64':'Q','INT64':'q','FLOAT32':'f','FLOAT64':'d'}
EXTRA = [('extra_u8',1,'B'),('extra_i8',2,'b'),('extra_u16',3,'H'),
         ('extra_i16',4,'h'),('extra_u32',5,'I'),('extra_i32',6,'i'),
         ('extra_u64',7,'Q'),('extra_i64',8,'q'),('extra_f32',9,'f'),
         ('extra_f64',10,'d'),('scaled_i16',4,'h')]
IDENTITY = [1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.]


def require(value, message):
    if not value:
        raise AssertionError(message)


def unpack(fmt, data, offset=0):
    size=struct.calcsize('<'+fmt)
    require(offset>=0 and offset+size<=len(data), 'truncated scalar')
    return struct.unpack_from('<'+fmt,data,offset)


def f32(value):
    return struct.unpack('<f',struct.pack('<f',value))[0]


def fixture(path, count=257, fmt=3, variant='spread', extras=True, metadata=False, additional_extra=0):
    """Write literal LAS records; repeated rows are intentionally byte-identical."""
    require(fmt in FORMATS, 'unsupported fixture format')
    extended=fmt>=6
    header_size=375 if extended else 227
    declarations=(EXTRA+[(f'wide_{i}',7,'Q') for i in range(additional_extra)]) if extras else []
    descriptors=bytearray()
    for name, kind, code in declarations:
        d=bytearray(192);d[2]=kind
        d[4:4+len(name)]=name.encode()
        if name=='scaled_i16':
            d[3]=24
            struct.pack_into('<d',d,112,.125);struct.pack_into('<d',d,136,3.)
        if metadata:
            # Exact integer metadata intentionally exceeds the f64 exact domain.
            d[3]|=7
            values=(9007199254740993,9007199254740995,18446744073709551615) if code=='Q' else (
                (-9007199254740993,-9223372036854775808,9007199254740995) if code=='q' else ((f32(.125123456789),-.375,f32(12345.250000001)) if code=='f' else ((.125123456789,-.375,12345.250000001) if code=='d' else ((-32768,-32768,32767) if name=='scaled_i16' else (0,0,1)))))
            slot='d' if code in 'fd' else ('q' if code in 'bhiq' else 'Q')
            for offset,value in zip((40,64,88),values):struct.pack_into('<'+slot,d,offset,value)
        descriptors.extend(d)
    vlr=b''
    if declarations:
        vlr=struct.pack('<H16sHH32s',0,b'LASF_Spec',4,len(descriptors),b'independent P1 scalar ExtraBytes')+descriptors
    length=FORMATS[fmt]+sum(struct.calcsize('<'+code) for _,_,code in declarations)
    records=[]
    for index in range(count):
        # Every fifth pair is fully identical, including all original attributes.
        i=index-1 if index%10==1 else index
        if variant=='identical':i=0
        if variant=='reordered':i=count-1-i
        if variant=='projected_origin':xyz=[0,0,0]
        elif variant=='geographic':
            xyz=[(0,0,0),(90000,0,0),(-90000,0,0),(0,45000,10000)][i%4]
        else:
            xyz=[(i%17)*713,(i//17)*627,((i*17)%29-14)*113]
            if variant=='outlier' and index==count-1:xyz=[600000,-400000,300000]
        if variant=='deep':
            # A millimetre cluster and points at doubling distances: about 20 octree
            # levels with two children each, beyond one document's skeleton.
            k=index-count+20;xyz=[8<<k]*3 if k>=0 else [i%5,(i//5)%5,(i//25)%5]
        raw=bytearray(length);struct.pack_into('<iiiH',raw,0,*xyz,(i*251)%65536)
        if extended:
            raw[14]=(i%14+1)|((i%14+1)<<4)
            raw[15]=(i%16)|((i%4)<<4)|((i%2)<<6)|(((i//2)%2)<<7)
            raw[16]=(i*11)%256;raw[17]=(i*7)%256
            struct.pack_into('<hHd',raw,18,i%65536-32768,(i*2333)%65536,i/7.)
            if fmt>=7:struct.pack_into('<HHH',raw,30,(i*233)%65536,(i*431)%65536,(i*717)%65536)
            if fmt==8:struct.pack_into('<H',raw,36,(i*977)%65536)
        else:
            raw[14]=(i%7+1)|((i%7+1)<<3)|((i%2)<<6)|(((i//2)%2)<<7)
            raw[15]=(i%20)|((i%2)<<5)|(((i//2)%2)<<6)|(((i//3)%2)<<7)
            struct.pack_into('<bBH',raw,16,i%255-128,(i*7)%256,(i*2333)%65536)
            if fmt in (1,3):struct.pack_into('<d',raw,20,i/7.)
            if fmt in (2,3):struct.pack_into('<HHH',raw,20 if fmt==2 else 28,(i*233)%65536,(i*431)%65536,(i*717)%65536)
        at=FORMATS[fmt]
        for name,_,code in declarations:
            values={'B':(0,255,13),'b':(-128,127,-13),'H':(0,65535,13),
                'h':(-32768,32767,-13),'I':(0,4294967295,13),
                'i':(-2147483648,2147483647,-13),
                'Q':(9007199254740993,18446744073709551615,0),
                'q':(-9007199254740993,-9223372036854775808,9223372036854775807),
                'f':(f32(.125123456789) if metadata else .125,-.375,12345.25),'d':(.125,-.375,12345.250000001)}[code]
            struct.pack_into('<'+code,raw,at,values[i%3]);at+=struct.calcsize('<'+code)
        records.append(bytes(raw))
    h=bytearray(header_size);h[:4]=b'LASF';h[24:26]=bytes((1,4 if extended else 2))
    struct.pack_into('<HII',h,94,header_size,header_size+len(vlr),int(bool(vlr)))
    h[104]=fmt;struct.pack_into('<HI',h,105,length,0 if extended else count)
    scale=(.001,.001,.001)
    offsets=(0.,0.,0.) if variant=='geographic' else ((500000.,0.,123.) if variant=='projected_origin' else (500000.,5000000.,80.))
    struct.pack_into('<ddd',h,131,*scale);struct.pack_into('<ddd',h,155,*offsets)
    xyzs=[unpack('iii',r) for r in records]
    for axis in range(3):
        values=[r[axis]*scale[axis]+offsets[axis] for r in xyzs]
        struct.pack_into('<dd',h,179+axis*16,max(values),min(values))
    if extended:struct.pack_into('<Q',h,247,count)
    pathlib.Path(path).write_bytes(h+vlr+b''.join(records))
    return enumerate_las(path)


def enumerate_las(path):
    """Read required source dimensions from actual LAS bytes, never output schema."""
    data=pathlib.Path(path).read_bytes();require(data[:4]==b'LASF','LAS signature')
    header,offset,vlrs=unpack('HII',data,94);fmt=data[104];length=unpack('H',data,105)[0]
    count=unpack('Q',data,247)[0] if fmt>=6 else unpack('I',data,107)[0]
    scales=unpack('ddd',data,131);offsets=unpack('ddd',data,155)
    dimensions=[];at=header
    for _ in range(vlrs):
        _,user,record,size,_=unpack('H16sHH32s',data,at);at+=54
        if user.rstrip(b'\0')==b'LASF_Spec' and record==4:
            require(size%192==0,'ExtraBytes descriptors')
            for d in range(at,at+size,192):
                kind=data[d+2];options=data[d+3];name=data[d+4:d+36].rstrip(b'\0').decode()
                require(1<=kind<=10,'scalar ExtraBytes')
                code='BbHhIiQqfd'[kind-1]
                dimensions.append((name,code,unpack('d',data,d+112)[0] if options&8 else 1.,unpack('d',data,d+136)[0] if options&16 else 0.,options,data[d:d+192]))
        at+=size
    require(offset+count*length==len(data),'LAS exact record closure')
    rows=[]
    for index in range(count):
        raw=data[offset+index*length:offset+(index+1)*length]
        x,y,z,intensity=unpack('iiiH',raw)
        row=dict(X=x,Y=y,Z=z,intensity=intensity,source_index=index)
        row.update(zip(('source_x','source_y','source_z'),[v*s+o for v,s,o in zip((x,y,z),scales,offsets)]))
        if fmt>=6:
            returns,flags,classification,user=raw[14:18]
            row.update(return_number=returns&15,number_of_returns=returns>>4,synthetic=flags&1,key_point=(flags>>1)&1,withheld=(flags>>2)&1,overlap=(flags>>3)&1,scanner_channel=(flags>>4)&3,scan_direction_flag=(flags>>6)&1,edge_of_flight_line=flags>>7,classification=classification,user_data=user)
            row['scan_angle'],row['point_source_id'],row['gps_time']=unpack('hHd',raw,18)
            if fmt>=7:row.update(zip(('red','green','blue'),unpack('HHH',raw,30)))
            if fmt==8:row['nir']=unpack('H',raw,36)[0]
        else:
            returns,classification=raw[14:16]
            row.update(return_number=returns&7,number_of_returns=(returns>>3)&7,scan_direction_flag=(returns>>6)&1,edge_of_flight_line=returns>>7,classification=classification&31,synthetic=(classification>>5)&1,key_point=(classification>>6)&1,withheld=classification>>7)
            row['scan_angle_rank'],row['user_data'],row['point_source_id']=unpack('bBH',raw,16)
            if fmt in (1,3):row['gps_time']=unpack('d',raw,20)[0]
            if fmt in (2,3):row.update(zip(('red','green','blue'),unpack('HHH',raw,20 if fmt==2 else 28)))
        extra_at=FORMATS[fmt]
        for name,code,scale,shift,_,_ in dimensions:
            value=unpack(code,raw,extra_at)[0];extra_at+=struct.calcsize('<'+code)
            row[name]=value if scale==1 and shift==0 else value*scale+shift
        require(extra_at==length,'unaccounted LAS bytes');rows.append(row)
    return {'rows':rows,'format':fmt,'scales':scales,'offsets':offsets,'extra':dimensions}


def multiply(a,b):
    return [sum(a[k*4+r]*b[c*4+k] for k in range(4)) for c in range(4) for r in range(4)]


def transform(m,p):
    return tuple(sum(m[c*4+r]*p[c] for c in range(3))+m[12+r] for r in range(3))


def node_matrix(node):
    if 'matrix' in node:return node['matrix']
    x,y,z,w=node.get('rotation',[0,0,0,1]);sx,sy,sz=node.get('scale',[1,1,1])
    m=[(1-2*(y*y+z*z))*sx,2*(x*y+z*w)*sx,2*(x*z-y*w)*sx,0,
       2*(x*y-z*w)*sy,(1-2*(x*x+z*z))*sy,2*(y*z+x*w)*sy,0,
       2*(x*z+y*w)*sz,2*(y*z-x*w)*sz,(1-2*(x*x+y*y))*sz,0,
       *node.get('translation',[0,0,0]),1]
    return m


def glb(data):
    require(unpack('4sII',data)==(b'glTF',2,len(data)),'GLB header/length')
    at=12;chunks=[]
    while at<len(data):
        n,kind=unpack('I4s',data,at);at+=8
        require(n%4==0 and at+n<=len(data),'GLB chunk range/alignment')
        chunks.append((kind,data[at:at+n]));at+=n
    require([k for k,_ in chunks]==[b'JSON',b'BIN\0'],'GLB admitted JSON/BIN chunks')
    doc=json.loads(chunks[0][1]);binary=chunks[1][1]
    require(set(doc.get('extensionsRequired',[]))<= {'EXT_structural_metadata','EXT_mesh_features','KHR_materials_unlit','KHR_mesh_quantization'},'unsupported required GLB extension')
    require(len(doc['buffers'])==1 and 'uri' not in doc['buffers'][0],'embedded buffer profile')
    declared=doc['buffers'][0]['byteLength']
    require(0<=len(binary)-declared<=3,'actual BIN length')
    require(all(v==0 for v in binary[declared:]),'BIN padding')
    for view in doc.get('bufferViews',[]):
        require('EXT_meshopt_compression' not in view.get('extensions',{}),'unsupported compressed bufferView')
        start=view.get('byteOffset',0);end=start+view['byteLength']
        require(view.get('buffer',0)==0 and 0<=start<=end<=declared,'bufferView actual payload range')
    for accessor in doc.get('accessors',[]):
        accessor_values(doc,binary,accessor)
    return doc,binary


def view_bytes(doc,binary,index):
    require(isinstance(index,int) and 0<=index<len(doc['bufferViews']),'bufferView reference')
    v=doc['bufferViews'][index];at=v.get('byteOffset',0);end=at+v['byteLength']
    require(v.get('buffer',0)==0 and 0<=at<=end<=doc['buffers'][0]['byteLength']<=len(binary),'view range')
    return binary[at:end]


def accessor_values(doc,binary,a):
    require('sparse' not in a and 'bufferView' in a,'unsupported sparse/implicit accessor')
    code={5120:'b',5121:'B',5122:'h',5123:'H',5125:'I',5126:'f'}[a['componentType']]
    width={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']]
    view=doc['bufferViews'][a['bufferView']];data=view_bytes(doc,binary,a['bufferView'])
    scalar=struct.calcsize('<'+code);size=width*scalar;stride=view.get('byteStride',size)
    offset=a.get('byteOffset',0);count=a['count']
    require(isinstance(count,int) and 0<=count<=16777217,'accessor count limit')
    require(offset%scalar==0 and stride>=size and stride%scalar==0,'accessor offset/stride')
    require(offset+(count-1)*stride+size<=len(data) if count else offset<=len(data),'accessor actual range')
    return [unpack(code*width,data,offset+i*stride) for i in range(count)]


def decoded_points(data):
    doc,binary=glb(data);meta=doc['extensions']['EXT_structural_metadata']
    require(len(meta['propertyTables'])==1,'point property table count')
    table=meta['propertyTables'][0];schema=meta['schema']['classes'][table['class']]['properties']
    columns={}
    for name,column in table['properties'].items():
        declaration=schema[name];code=KINDS[declaration['componentType']]
        raw=view_bytes(doc,binary,column['values']);require(len(raw)==table['count']*struct.calcsize('<'+code),'metadata scalar count')
        columns[name]=[v[0] for v in struct.iter_unpack('<'+code,raw)]
    result=[];steps=[]
    scenes=doc.get('scenes',[]);require(scenes,'GLB selected scene')
    active=set()
    def visit(index,parent):
        require(index not in active,'GLB node cycle');active.add(index)
        node=doc['nodes'][index];m=multiply(parent,node_matrix(node))
        if 'mesh' in node:
            for primitive in doc['meshes'][node['mesh']]['primitives']:
                require(primitive.get('mode',4)==0 and 'indices' not in primitive,'nonindexed POINTS profile')
                require('KHR_draco_mesh_compression' not in primitive.get('extensions',{}),'unsupported compressed primitive')
                a=doc['accessors'][primitive['attributes']['POSITION']]
                quantized=(a['componentType'],a.get('normalized'))==(5123,True) and 'KHR_mesh_quantization' in doc['extensionsRequired']
                require(a['type']=='VEC3' and (quantized or (a['componentType']==5126 and not a.get('normalized',False))),'POSITION float32 or quantized UNSIGNED_SHORT')
                points=[tuple(v/65535 for v in p) if quantized else p for p in accessor_values(doc,binary,a)];require(len(points)==table['count'],'POSITION/table cardinality')
                # Half a quantization step per glTF axis, through the node scale, in z-up order.
                h=[.5/65535*math.sqrt(sum(m[4*c+r]**2 for r in range(3))) if quantized else 0. for c in range(3)];step=(h[0],h[2],h[1])
                if '_FEATURE_ID_0' in primitive['attributes']:
                    feature=doc['accessors'][primitive['attributes']['_FEATURE_ID_0']]
                    require(not feature.get('normalized',False),'feature IDs unnormalized')
                    ids=accessor_values(doc,binary,feature)
                    require([v[0] for v in ids]==list(range(table['count'])),'feature IDs address original property rows')
                    definition=primitive['extensions']['EXT_mesh_features']['featureIds'][0]
                    require(definition['attribute']==0 and definition['propertyTable']==0 and definition['featureCount']==table['count'],'feature table mapping')
                if 'red' in columns:
                    color=doc['accessors'][primitive['attributes']['COLOR_0']]
                    require(color['componentType']==5121 and color['type']=='VEC3' and color.get('normalized') is True,'RGB render encoding')
                    colors=accessor_values(doc,binary,color)
                    require(colors==[tuple((v+128)//257 for v in rgb) for rgb in zip(columns['red'],columns['green'],columns['blue'])],'rendered COLOR_0 is source RGB rounded to 8 bits')
                local=[transform(m,p) for p in points]
                # glTF Y up -> 3D Tiles Z up, per 3D Tiles glTF axis convention.
                result.extend((p[0],-p[2],p[1]) for p in local);steps.extend([step]*len(local))
                for field,attribute in [('classification','_CLASSIFICATION'),('intensity','_INTENSITY'),('return_number','_RETURN_NUMBER')]:
                    if attribute in primitive['attributes']:
                        values=accessor_values(doc,binary,doc['accessors'][primitive['attributes'][attribute]])
                        require([v[0] for v in values]==columns[field],'property attribute fidelity')
        for child in node.get('children',[]):visit(child,m)
        active.remove(index)
    for node in scenes[doc.get('scene',0)]['nodes']:visit(node,IDENTITY)
    require(len(result)==table['count'],'rendered point cardinality')
    return result,columns,schema,steps


def morton_xyz(index,level):
    return tuple(sum(((index>>(bit*3+axis))&1)<<bit for bit in range(level)) for axis in range(3))


FOREST_SKELETON_HEIGHT=4


def implicit_tree(root,members,used):
    """Decode one forest implicit root: a single subtree with no child subtrees.

    Octree BFS/Morton availability and available-row metadata ranks are decoded
    from the subtree bytes. Every tile, including the root, takes its box and
    error from its row; the level-0 row must equal the root tile bitwise.
    """
    spec=root['implicitTiling'];require(spec['subdivisionScheme']=='OCTREE','octree profile')
    levels=spec['subtreeLevels']
    require(1<=levels<=FOREST_SKELETON_HEIGHT,'bounded implicit root height')
    require(spec['availableLevels']==levels,'implicit root is exactly one subtree')
    require('children' not in root,'implicit root has explicit children')
    template=spec['subtrees']['uri'];templates=root.get('contents',[root.get('content')]);require(all(templates),'implicit content templates');nodes={}
    require(not any(t['uri'].endswith('.json') for t in templates),'JSON content template on an implicit root')
    def uri(template,c):
        return template.format(level=c[0],x=c[1],y=c[2],z=c[3])
    name=uri(template,(0,0,0,0));require(name not in used,'repeated subtree');used.add(name)
    data=members[name];magic,version,jlen,blen=unpack('4sIQQ',data)
    require(magic==b'subt' and version==1 and 24+jlen+blen==len(data),'subtree framing')
    doc=json.loads(data[24:24+jlen]);binary=data[24+jlen:]
    require(doc['childSubtreeAvailability']=={'constant':0},'forest subtree has no child subtrees')
    def bits(declaration,count):
        if 'constant' in declaration:
            require(declaration['constant'] in (0,1),'availability constant')
            return [bool(declaration['constant'])]*count
        raw=view_bytes(doc,binary,declaration['bitstream']);require(len(raw)==(count+7)//8,'availability bytes')
        values=[bool(raw[i//8]&(1<<(i%8))) for i in range(count)]
        require(sum(values)==declaration.get('availableCount',sum(values)),'availability count')
        require(not any(raw[i//8]&(1<<(i%8)) for i in range(count,len(raw)*8)),'availability padding')
        return values
    count=(8**levels-1)//7;tiles=bits(doc['tileAvailability'],count)
    declarations=doc['contentAvailability'];declarations=declarations if isinstance(declarations,list) else [declarations]
    require(len(declarations)==len(templates),'implicit content slot count')
    contents=[bits(d,count) for d in declarations]
    require(tiles[0],'available subtree root')
    require(any(tiles[(8**(levels-1)-1)//7:]),'availableLevels reaches an available tile')
    table=doc['propertyTables'][doc['tileMetadata']];props=table['properties'];require(table['count']==sum(tiles),'metadata rank count')
    boxes=view_bytes(doc,binary,props['boundingBox']['values']);errors=view_bytes(doc,binary,props['geometricError']['values'])
    require(len(boxes)==sum(tiles)*96 and len(errors)==sum(tiles)*8,'semantic metadata lengths')
    rank=0
    for level in range(levels):
        offset=(8**level-1)//7
        for index in range(8**level):
            bit=offset+index
            require(not any(c[bit] for c in contents) or tiles[bit],'content on unavailable tile')
            if not tiles[bit]:continue
            if level:require(tiles[(8**(level-1)-1)//7+index//8],'availability parent')
            coord=(level,*morton_xyz(index,level))
            box=unpack('d'*12,boxes,rank*96);error=unpack('d',errors,rank*8)[0];rank+=1
            node={'boundingVolume':{'box':box},'geometricError':error,'children':[]}
            available=[uri(t['uri'],coord) for t,c in zip(templates,contents) if c[bit]]
            require(len(available)==1,'one reachable point content per tile')
            node['content']={'uri':available[0]}
            require(coord not in nodes,'duplicate implicit coordinate');nodes[coord]=node
    for coord,node in nodes.items():
        if coord[0]:
            parent=(coord[0]-1,*(v//2 for v in coord[1:]));require(parent in nodes,'implicit parent closure');nodes[parent]['children'].append(node)
    expanded=nodes[(0,0,0,0)]
    # Bitwise: struct round trip of the JSON double against the row double.
    require(struct.pack('<12d',*root['boundingVolume']['box'])==struct.pack('<12d',*expanded['boundingVolume']['box']),'level-0 row box equals the implicit root tile')
    require(struct.pack('<d',root['geometricError'])==struct.pack('<d',expanded['geometricError']),'level-0 row error equals the implicit root tile')
    expanded['transform']=root.get('transform',IDENTITY)
    return expanded


def expand_forest(tile,members,used,schema,stats):
    """Expand the bounded forest into one explicit tree.

    Explicit tiles are the skeleton and its explicit leaves. A link tile is an
    explicit tile whose only content is an explicit external tileset document;
    the link and the document root are one tile, so the link's transform and
    the document root's content and children compose. Implicit roots expand
    through implicit_tree. JSON content templates are not part of the profile.
    """
    require('implicitChildIndex' not in tile.get('extras',{}),'written implicitChildIndex')
    if 'implicitTiling' in tile:
        require(schema,'implicit root in a document without the metadata schema')
        stats['implicit_roots']+=1
        return implicit_tree(tile,members,used)
    name=tile.get('content',{}).get('uri','')
    if name.endswith('.json'):
        require(not tile.get('children') and 'contents' not in tile,'link tile has only its document')
        require(name in members and name not in used,'external tileset closure/cycle');used.add(name)
        document=json.loads(members[name]);linked=document['root']
        require('transform' not in linked,'external document root frame')
        require(list(linked['boundingVolume']['box'])==list(tile['boundingVolume']['box']) and linked['geometricError']==tile['geometricError'],'link and document root semantics')
        stats['documents']+=1
        node=dict(expand_forest(linked,members,used,'schema' in document,stats))
        node['transform']=tile.get('transform',IDENTITY)
        return node
    stats['explicit']+=1
    node=dict(tile);node['children']=[expand_forest(child,members,used,schema,stats) for child in tile.get('children',[])]
    return node


def ecef(row,height=0.):
    lon,lat,h=row['source_x'],row['source_y'],row['source_z']+height
    lon,lat=math.radians(lon),math.radians(lat)
    a=6378137.;e2=6.6943799901413165e-3;n=a/math.sqrt(1-e2*math.sin(lat)**2)
    return ((n+h)*math.cos(lat)*math.cos(lon),(n+h)*math.cos(lat)*math.sin(lon),(n*(1-e2)+h)*math.sin(lat))


def box_contains(box,p,tolerance=2e-5):
    # Current admitted point bounds are axis aligned in each tile frame.
    require(all(abs(box[3+a*3+b])<1e-12 for a in range(3) for b in range(3) if a!=b),'axis aligned point bounds')
    return all(abs(p[a]-box[a])<=abs(box[3+a*3+a])+tolerance for a in range(3))


def voxel(p,anchor,edge):
    return tuple(math.floor((p[a]-anchor[a])/edge) for a in range(3))


def voxels(descendants,positions,anchor,parent_edge,budget):
    """Edge of the finest fitting cubic grid: two halving chains from the parent edge (or
    longest/(side-.5)) and its /sqrt(2), to `budget` voxels along the longest axis."""
    longest=max(max(positions[i][a] for i in descendants)-min(positions[i][a] for i in descendants) for a in range(3))
    side=1
    while (side+1)**3<=budget:side+=1
    top=parent_edge or max(longest,2.2250738585072014e-308)/(side-.5);edges=[]
    for edge in (top,top/math.sqrt(2)):
        while True:
            edges.append(edge)
            if longest==0 or longest/edge>=budget or not edge/2>0:break
            edge/=2
    return next(e for e in sorted(edges) if len({voxel(positions[i],anchor,e) for i in descendants})<=budget)


def check_index(z, raw):
    """3TZ index is MD5 name -> little-endian local ZIP header offset."""
    infos=z.infolist()
    require(infos[0].filename=='tileset.json' and infos[-1].filename=='@3dtilesIndex1@','3TZ member ordering')
    require(all(i.compress_type==zipfile.ZIP_STORED for i in infos),'3TZ stored members')
    raw_index=z.read('@3dtilesIndex1@');require(len(raw_index)==24*(len(infos)-1),'3TZ index length')
    wanted={hashlib.md5(i.filename.encode()).digest():i for i in infos[:-1]};previous=None
    for low,high,offset in struct.iter_unpack('<QQQ',raw_index):
        require(previous is None or previous<(low,high),'3TZ index sorting');previous=(low,high)
        digest=struct.pack('<QQ',low,high);require(digest in wanted,'3TZ index membership')
        info=wanted.pop(digest);require(offset==info.header_offset,'3TZ index offset')
        require(raw[offset:offset+4]==b'PK\x03\x04','3TZ local header')
        name_length,extra_length=unpack('HH',raw,offset+26)
        require(raw[offset+30:offset+30+name_length].decode()==info.filename,'3TZ indexed member name')
        begin=offset+30+name_length+extra_length
        require(raw[begin:begin+info.file_size]==z.read(info.filename),'3TZ indexed bytes')
    require(not wanted,'3TZ complete index')


def write_archive(path,members):
    # Independent writer used only for controls, rebuilding valid offsets/CRCs.
    with zipfile.ZipFile(path,'w') as z:
        order=['tileset.json']+[n for n in members if n not in ('tileset.json','@3dtilesIndex1@')]
        for name in order:z.writestr(name,members[name])
        index=[]
        for info in z.infolist():
            low,high=struct.unpack('<QQ',hashlib.md5(info.filename.encode()).digest())
            index.append((low,high,info.header_offset))
        z.writestr('@3dtilesIndex1@',b''.join(struct.pack('<QQQ',*row) for row in sorted(index)))


def source_extra_schema(expected,schema):
    for name,code,scale,offset,flags,descriptor in expected['extra']:
        declaration=schema[name]
        if code in ('q','Q') and not flags&24:
            require(declaration['componentType']==('INT64' if code=='q' else 'UINT64'),'exact 64-bit component schema')
        slot='d' if code in ('f','d') else ('q' if code in ('b','h','i','q') else 'Q')
        for flag,at,key in [(1,40,'noData'),(2,64,'min'),(4,88,'max')]:
            if flags&flag:
                raw_value=unpack(slot,descriptor,at)[0]
                value=raw_value if not flags&24 else raw_value*scale+offset
                require(key in declaration and declaration[key]==value,f'{name}: required ExtraBytes {key} schema differs: {declaration.get(key)} != {value}')


def audit(source,archive,max_points,geographic=False,height=0.,require_multileaf=True,check_extra_metadata=True,expected_positions_override=None):
    expected=enumerate_las(source);rows=expected['rows'];expected_positions=expected_positions_override or [ecef(row,height) if geographic else tuple(row[n] for n in ('source_x','source_y','source_z')) for row in rows]
    with zipfile.ZipFile(archive) as z:
        require(z.testzip() is None,'ZIP CRC')
        check_index(z,pathlib.Path(archive).read_bytes())
        require(len(z.namelist())==len(set(z.namelist())),'duplicate ZIP member')
        members={name:z.read(name) for name in z.namelist()}
    for name in members:
        require(not name.startswith('/') and '..' not in pathlib.PurePosixPath(name).parts,'archive authority escape')
    manifest=json.loads(members['tileset.json']);report=json.loads(members['conversion.json'])
    used={'tileset.json','conversion.json','@3dtilesIndex1@'};forest={'explicit':0,'implicit_roots':0,'documents':1}
    root=expand_forest(manifest['root'],members,used,'schema' in manifest,forest);implicit=forest['implicit_roots']>0
    # Cesium reads the metadata schema only from tileset.json.
    require(('schema' in manifest)==implicit,'tileset.json schema iff an implicit root exists')
    leaves=[];tiles=0;rounding=0.
    def visit(node,parent):
        nonlocal tiles,rounding
        m=multiply(parent,node.get('transform',IDENTITY));error=node['geometricError'];require(math.isfinite(error) and error>=0,'finite geometric error')
        box=node['boundingVolume']['box'];require(all(math.isfinite(v) for v in box),'finite bounds')
        uri=node['content']['uri'];require(uri in members,'reachable content resource');used.add(uri)
        positions,columns,schema,steps=decoded_points(members[uri]);ids=columns['source_index'];tiles+=1
        if check_extra_metadata:source_extra_schema(expected,schema)
        require(len(ids)<=max_points and len(ids)>0,'point budget')
        required=set(rows[0]);require(required<=set(columns),'missing required source field: '+str(sorted(required-set(columns))))
        world=[transform(m,p) for p in positions]
        for at,index in enumerate(ids):
            require(isinstance(index,int) and 0<=index<len(rows),'source index range')
            for name,value in rows[index].items():require(columns[name][at]==value,f'{uri}: {name} differs at source {index}: {columns[name][at]} != {value}')
            delta=math.dist(world[at],expected_positions[index]);rounding=max(rounding,delta)
            # Half a 16-bit quantization step, float32-scale evaluation and geospatial
            # reference allowances. Leaf error 0 states that nothing is omitted below.
            slack=max(3e-6,math.sqrt(sum((abs(v)*2**-24)**2 for v in positions[at])))+(2e-8 if geographic else 0.)
            require(all(abs(world[at][a]-expected_positions[index][a])<=steps[at][a]+slack for a in range(3)),f'{uri}: rendered POSITION differs by more than half a step on an axis at source {index}')
            require(delta<=report['maxPositionRoundingMetres']+slack,f'{uri}: position error exceeds the reported rounding')
            require(box_contains(box,positions[at]),f'{uri}: rendered position escapes bounds')
        descendants=[];children=node.get('children',[])
        if children:
            for child in children:
                require(error+1e-8>=child['geometricError'],'nonmonotonic geometric error')
                descendants.extend(visit(child,m))
            require(len(descendants)>max_points,'unneeded parent sampling')
            samples[id(node)]=ids,descendants
            distance=max(min(math.dist(expected_positions[i],p) for p in world) for i in descendants)
            require(distance<=error+3e-6,'parent nearest-representative error claim')
        else:
            require(error==0,'leaf geometric error');leaves.extend(ids);descendants=ids
            # Leaves keep 16-bit codes only where a step is within the source precision.
            extent=[max(p[a] for p in positions)-min(p[a] for p in positions) for a in range(3)];precision=report['sourcePrecisionMetres']
            if any(steps[0]):require(all(2*steps[0][a]<=precision*(1+1e-9) for a in range(3) if extent[a]),f'{uri}: 16-bit leaf coarser than the source')
            else:require(any(e/65535>precision*(1-1e-9) for e in extent),f'{uri}: float32 leaf where 16-bit codes suffice')
        # Invert admitted translation-only tile transforms, composed through
        # skeleton tiles, links and each implicit root's frame.
        require(m[:12]==IDENTITY[:12],'point tile translation profile')
        for index in descendants:
            require(box_contains(box,tuple(expected_positions[index][a]-m[12+a] for a in range(3))),'descendant source escapes transformed bounds')
        return descendants
    # One source step in output metres, measured independently at the first record.
    if not expected_positions_override:
        precision=min(expected['scales'])
        if geographic:precision=min(math.dist(ecef({**rows[0],n:rows[0][n]+v},height),expected_positions[0]) for n,v in zip(('source_x','source_y','source_z'),expected['scales']))
        require(math.isclose(report['sourcePrecisionMetres'],precision,rel_tol=1e-6),'reported source precision')
    samples={};visit(root,IDENTITY)
    anchor=[min(p[a] for p in expected_positions) for a in range(3)]
    def sampled(node,parent_edge):
        if id(node) not in samples:return
        ids,descendants=samples[id(node)]
        edge=voxels(descendants,expected_positions,anchor,parent_edge,max_points)
        selected={}
        for i in sorted(descendants):selected.setdefault(voxel(expected_positions[i],anchor,edge),i)
        require(collections.Counter(ids)==collections.Counter(selected.values()),'parent first-point voxel representatives')
        for child in node['children']:sampled(child,edge)
    sampled(root,None)
    require(collections.Counter(leaves)==collections.Counter(range(len(rows))),'leaf source multiplicity/full coverage')
    require(not require_multileaf or tiles>1,'forced multileaf')
    require(report['points']==len(rows) and report['maxPoints']==max_points and report['tiles']==tiles,'required report point metrics')
    require(set(report['properties'])>=set(rows[0]),'required report fields')
    require(set(members)<=used,'unreachable resource inventory: '+str(sorted(set(members)-used)))
    return {'source_points':len(rows),'leaf_records':len(leaves),'tiles':tiles,'implicit':implicit,'forest':forest,'max_world_position_difference_metres':rounding,'required_fields':sorted(rows[0]),'source_multiplicity':'exact','resource_closure':'exact'}


def lean_equivalent(full,lean):
    """Default output is the audited --metadata output without metadata."""
    with zipfile.ZipFile(full) as a,zipfile.ZipFile(lean) as b:
        require(a.namelist()==b.namelist(),'default member inventory')
        for name in a.namelist():
            x,y=a.read(name),b.read(name)
            if name.endswith('.glb'):
                (dx,bx),(dy,by)=glb(x),glb(y);px,py=(d['meshes'][0]['primitives'][0]['attributes'] for d in (dx,dy))
                require('extensions' not in dy and not {'EXT_mesh_features','EXT_structural_metadata'}&set(dy['extensionsUsed']),'default tile has no metadata')
                require(set(py)=={'POSITION','COLOR_0'}&set(px) and dx['nodes']==dy['nodes'],'default attributes and placement')
                for key in py:require(accessor_values(dx,bx,dx['accessors'][px[key]])==accessor_values(dy,by,dy['accessors'][py[key]]),'default '+key)
            elif name not in ('conversion.json','@3dtilesIndex1@'):require(x==y,'default '+name)


def pack_glb(doc,binary):
    j=json.dumps(doc,separators=(',',':')).encode();j+=b' '*((-len(j))%4);binary+=b'\0'*((-len(binary))%4)
    return struct.pack('<4sII',b'glTF',2,28+len(j)+len(binary))+struct.pack('<I4s',len(j),b'JSON')+j+struct.pack('<I4s',len(binary),b'BIN\0')+binary


def controls(source,archive,max_points):
    """Mutations are validated by this reader; neither packaging nor product validator is the oracle."""
    with zipfile.ZipFile(archive) as z:members={n:z.read(n) for n in z.namelist()}
    manifest=json.loads(members['tileset.json'])
    root=expand_forest(manifest['root'],members,set(),'schema' in manifest,{'explicit':0,'implicit_roots':0,'documents':1})
    def leaves(node):
        if node.get('children'):
            for child in node['children']:yield from leaves(child)
        else:yield node['content']['uri']
    leaf=next(leaves(root));out=[]
    for kind in ('wrong_position','missing_field','duplicate_record','truncated_bin','accessor_overrun','orphan_resource','missing_content'):
        modified=dict(members);doc,binary=glb(modified[leaf]);binary=bytearray(binary)
        table=doc['extensions']['EXT_structural_metadata']['propertyTables'][0]
        if kind=='wrong_position':
            a=doc['accessors'][doc['meshes'][0]['primitives'][0]['attributes']['POSITION']];at=doc['bufferViews'][a['bufferView']].get('byteOffset',0)+a.get('byteOffset',0)
            struct.pack_into('<f',binary,at,100000.)
        elif kind=='missing_field':del table['properties']['intensity']
        elif kind=='duplicate_record':
            col=table['properties']['source_index'];v=doc['bufferViews'][col['values']];at=v.get('byteOffset',0)
            # Replace by another globally valid source ID; catches cross-leaf repeats.
            current=unpack('Q',binary,at)[0];struct.pack_into('<Q',binary,at,(current+1)%len(enumerate_las(source)['rows']))
        elif kind=='truncated_bin':binary=bytearray()
        elif kind=='accessor_overrun':doc['accessors'][0]['count']=16777217
        elif kind=='orphan_resource':modified['unreachable.bin']=b'orphan'
        elif kind=='missing_content':del modified[leaf]
        if kind not in ('orphan_resource','missing_content'):modified[leaf]=pack_glb(doc,bytes(binary))
        with tempfile.TemporaryDirectory() as tmp:
            target=pathlib.Path(tmp)/'control.3tz'
            write_archive(target,modified)
            try:audit(source,target,max_points)
            except (AssertionError,KeyError,ValueError,IndexError) as error:out.append({'control':kind,'rejected':True,'reason':str(error)})
            else:raise AssertionError('oracle accepted '+kind)
    return out


def forest_controls(source,archive,max_points):
    """Forest mutations, each of which this reader must reject."""
    with zipfile.ZipFile(archive) as z:members={n:z.read(n) for n in z.namelist()}
    documents=['tileset.json']+sorted(n for n in members if n.endswith('.json') and n not in ('tileset.json','conversion.json'))
    require(len(documents)>1,'forest control archive has external documents')
    def implicit_root(node):
        if 'implicitTiling' in node:return node
        for child in node.get('children',[]):
            found=implicit_root(child)
            if found:return found
    holder=next(n for n in documents if implicit_root(json.loads(members[n])['root']))
    def edit_document(modified,name,patch):
        doc=json.loads(modified[name]);patch(doc);modified[name]=json.dumps(doc).encode()
    def edit_subtree(modified,patch):
        root=implicit_root(json.loads(modified[holder])['root'])
        name=root['implicitTiling']['subtrees']['uri'].format(level=0,x=0,y=0,z=0)
        data=modified[name];_,version,jlen,_=unpack('4sIQQ',data)
        doc=json.loads(data[24:24+jlen]);binary=bytearray(data[24+jlen:]);patch(doc,binary)
        j=json.dumps(doc,separators=(',',':')).encode();j+=b' '*((-len(j))%8)
        modified[name]=struct.pack('<4sIQQ',b'subt',version,len(j),len(binary))+j+bytes(binary)
    def json_template(doc):
        root=implicit_root(doc['root']);content=root['contents'][0] if 'contents' in root else root['content']
        content['uri']=content['uri'].rsplit('.',1)[0]+'.json'
    def levels(doc):
        tiling=implicit_root(doc['root'])['implicitTiling'];tiling['availableLevels']=tiling['subtreeLevels']+1
    def child_subtree(doc,binary):doc['childSubtreeAvailability']={'constant':1}
    def row_ulp(doc,binary):
        table=doc['propertyTables'][doc['tileMetadata']];view=doc['bufferViews'][table['properties']['boundingBox']['values']]
        at=view.get('byteOffset',0);struct.pack_into('<d',binary,at,math.nextafter(unpack('d',binary,at)[0],math.inf))
    def link_box(doc):doc['root']['boundingVolume']['box'][0]+=1.
    out=[]
    for kind in ('json_template','levels','child_subtree','row_ulp','link_box','missing_root_schema','unreferenced_member'):
        modified=dict(members)
        if kind=='json_template':edit_document(modified,holder,json_template)
        elif kind=='levels':edit_document(modified,holder,levels)
        elif kind=='child_subtree':edit_subtree(modified,child_subtree)
        elif kind=='row_ulp':edit_subtree(modified,row_ulp)
        elif kind=='link_box':edit_document(modified,documents[1],link_box)
        elif kind=='missing_root_schema':edit_document(modified,'tileset.json',lambda doc:doc.pop('schema'))
        elif kind=='unreferenced_member':modified['implicit-content/unreferenced.glb']=next(v for n,v in members.items() if n.startswith('implicit-content/'))
        with tempfile.TemporaryDirectory() as tmp:
            target=pathlib.Path(tmp)/'control.3tz'
            write_archive(target,modified)
            try:audit(source,target,max_points)
            except (AssertionError,KeyError,ValueError,IndexError) as error:out.append({'control':kind,'rejected':True,'reason':str(error)})
            else:raise AssertionError('oracle accepted forest control '+kind)
    return out


def run(binary,output_dir,quick=False,baseline=False):
    binary=pathlib.Path(binary).resolve();directory=pathlib.Path(output_dir);directory.mkdir(parents=True,exist_ok=True)
    results=[];controls_result=[]
    # Explicit point output retired in 0.4.0 (H6); every case is the implicit forest.
    cases=[(3,'spread',11),(3,'identical',3),(3,'outlier',1),(3,'reordered',17)]
    cases.append((3,'spread',11,1))
    # A chain deeper than one document's skeleton: links and external documents.
    cases.append((3,'deep',11))
    if not quick:cases.extend((fmt,'spread',3) for fmt in FORMATS if fmt!=3)
    for number,case in enumerate(cases):
        fmt,variant,chunk=case[:3];budget=case[3] if len(case)>3 else 16
        source=directory/f'{number}-{fmt}-{variant}.las';archive=source.with_suffix('.3tz');fixture(source,fmt=fmt,variant=variant)
        command=[str(binary),'--json','point-cloud','-i',str(source),'-o',str(archive),'--sourceCrs','local','--maxPoints',str(budget),'--chunkPoints',str(chunk),'--metadata']
        completed=subprocess.run(command,capture_output=True,text=True)
        require(completed.returncode==0,'conversion failed: '+completed.stdout+completed.stderr)
        result=audit(source,archive,budget);result.update(format=fmt,variant=variant,chunk_points=chunk,max_points=budget);results.append(result)
        if number==0:controls_result=controls(source,archive,16)
        lean=source.with_suffix('.lean.3tz')
        require(subprocess.run(command[:6]+[str(lean)]+command[7:-1],capture_output=True).returncode==0,'default conversion')
        lean_equivalent(archive,lean)
        if variant=='deep':controls_result=controls_result+forest_controls(source,archive,budget)
    forests=[r['forest'] for r in results if r['implicit']]
    require(any(f['explicit'] and f['implicit_roots']>1 for f in forests),'implicit roots below an explicit skeleton were exercised')
    require(any(f['documents']>1 for f in forests),'explicit external documents were exercised')
    profile=profile_controls(binary,directory,baseline)
    repeated=concurrent_controls(binary,directory)
    return {'binary':str(binary),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'cases':results,'negative_controls':controls_result,'default_output':'lean_equivalent','profile_controls':profile,'concurrent_repeated_controls':repeated,'oracle_sha256':hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()}



def invoke(binary,source,archive,options=()):
    result=subprocess.run([str(binary),'--json','point-cloud','--metadata','-i',str(source),'-o',str(archive),*options],capture_output=True,text=True)
    try:response=json.loads(result.stdout)
    except ValueError:raise AssertionError('non-JSON CLI response: '+result.stdout+result.stderr)
    return result,response


def profile_controls(binary,directory,baseline=False):
    results=[]
    source=directory/'schema.las';archive=source.with_suffix('.3tz');fixture(source,metadata=True)
    completed,response=invoke(binary,source,archive,['--sourceCrs','local','--maxPoints','16','--metadata-attributes'])
    require(completed.returncode==0,'schema control conversion: '+completed.stdout)
    try:observed=audit(source,archive,16)
    except AssertionError as error:
        if not baseline:raise
        require('required ExtraBytes' in str(error),'unexpected baseline fidelity failure')
        observed=audit(source,archive,16,check_extra_metadata=False)
        results.append({'case':'ExtraBytes schema metadata','baseline_gap':str(error),'raw_64_bit_payload':'exact'})
    else:results.append({'case':'ExtraBytes schema metadata','result':'passed','raw_64_bit_payload':'exact'})
    for flags in (8,16):
        source=directory/f'single-scale-flag-{flags}.las';archive=source.with_suffix('.3tz');fixture(source,count=23)
        content=bytearray(source.read_bytes());content[227+54+10*192+3]=flags;source.write_bytes(content)
        completed,_=invoke(binary,source,archive,['--sourceCrs','local','--maxPoints','16'])
        require(completed.returncode==0,'independent scale/offset flag conversion')
        results.append({'case':'independent ExtraBytes scale/offset flags','flags':flags,'result':audit(source,archive,16)})
    # Analytic WGS84 geographic-to-ECEF; literals at equator/central meridian,
    # independent ellipsoid formula at 45 degrees. No production transform call.
    source=directory/'geographic.las';archive=source.with_suffix('.3tz');fixture(source,count=23,variant='geographic')
    options=['--sourceCrs','EPSG:4326','--heightOffset','7','--maxPoints','16','--chunkPoints','3']
    completed,_=invoke(binary,source,archive,options);require(completed.returncode==0,'analytic geographic conversion: '+completed.stdout)
    observed=audit(source,archive,16,geographic=True,height=7.);results.append({'case':'analytic WGS84 ECEF','result':observed})
    source=directory/'utm-origin.las';archive=source.with_suffix('.3tz');fixture(source,count=23,variant='projected_origin')
    options=['--sourceCrs','EPSG:32632','--heightOffset','7','--maxPoints','16','--chunkPoints','3']
    completed,_=invoke(binary,source,archive,options);require(completed.returncode==0,'analytic UTM origin conversion: '+completed.stdout)
    position=ecef({'source_x':9.,'source_y':0.,'source_z':123.},7.)
    observed=audit(source,archive,16,expected_positions_override=[position]*23)
    results.append({'case':'analytic UTM32 false easting/equator ECEF','reference':'EPSG32632 false easting500000/northing0 is longitude9deg/latitude0; independent WGS84 ellipsoid formula at h130m','result':observed})
    for kind in ('scaled_u64','scaled_i64','vector_extra','nonfinite_extra','truncated_source','local_height','missing_height','source_alias','hardlink_alias','zero_scale','negative_scale','scaled_overflow','header_size_zero','point_offset_zero','point_offset_u32max','point_offset_beyond_file','evlr_offset_u64max','evlr_offset_beyond_file','f32_unrepresentable_nodata','duplicate_projected_geokey','duplicate_geographic_geokey','untyped_extra','waveform_format'):
        source=directory/f'refused-{kind}.las';target=directory/f'refused-{kind}.3tz'
        if kind=='source_alias':source=target
        fixture(source,count=23,fmt=7 if kind.startswith('evlr_') else 3,variant='geographic' if kind=='duplicate_geographic_geokey' else 'spread',extras=not kind.startswith(('header_size','point_offset')))
        data=bytearray(source.read_bytes());options=['--sourceCrs','local','--force']
        if kind in ('scaled_u64','scaled_i64'):
            dimension=6 if kind=='scaled_u64' else 7;d=227+54+dimension*192;data[d+3]|=8;struct.pack_into('<d',data,d+112,.125)
        elif kind in ('zero_scale','negative_scale','scaled_overflow'):
            d=227+54+10*192;struct.pack_into('<d',data,d+112,{'zero_scale':0.,'negative_scale':-.125,'scaled_overflow':1e308}[kind])
        elif kind=='header_size_zero':struct.pack_into('<H',data,94,0)
        elif kind.startswith('point_offset_'):struct.pack_into('<I',data,96,{'point_offset_zero':0,'point_offset_u32max':4294967295,'point_offset_beyond_file':len(data)+1}[kind])
        elif kind.startswith('evlr_offset_'):
            struct.pack_into('<QI',data,235,18446744073709551615 if kind=='evlr_offset_u64max' else len(data)+1,1)
        elif kind=='f32_unrepresentable_nodata':
            d=227+54+8*192;data[d+3]=1;struct.pack_into('<d',data,d+40,.125123456789)
        elif kind in ('duplicate_projected_geokey','duplicate_geographic_geokey'):
            key=3072 if kind=='duplicate_projected_geokey' else 2048
            values=(32632,32633) if key==3072 else (4326,4326)
            payload=struct.pack('<'+'H'*12,1,1,0,2,key,0,1,values[0],key,0,1,values[1])
            vlr=struct.pack('<H16sHH32s',0,b'LASF_Projection',34735,len(payload),b'duplicate GeoKey independent control')+payload
            old_offset=unpack('I',data,96)[0];data=data[:old_offset]+vlr+data[old_offset:]
            struct.pack_into('<I',data,96,old_offset+len(vlr));struct.pack_into('<I',data,100,unpack('I',data,100)[0]+1)
            options=['--sourceCrs','header','--heightOffset','0','--force']
        elif kind=='untyped_extra':data[227+54+2]=0;data[227+54+3]=1
        elif kind=='waveform_format':
            fixture(source,count=23,fmt=1,extras=False);original_data=source.read_bytes();header=bytearray(original_data[:227]+b'\0'*8);header[25]=3;header[104]=4
            struct.pack_into('<H',header,94,235);struct.pack_into('<I',header,96,235);struct.pack_into('<H',header,105,57)
            data=header+b''.join(original_data[227+i*28:227+(i+1)*28]+b'\0'*29 for i in range(23))
        elif kind=='vector_extra':data[227+54+2]=11
        elif kind=='nonfinite_extra':
            point_offset=unpack('I',data,96)[0];extra_at=FORMATS[3]+sum(struct.calcsize('<'+code) for _,_,code in EXTRA[:9]);struct.pack_into('<d',data,point_offset+extra_at,float('nan'))
        elif kind=='truncated_source':data=data[:-1]
        elif kind=='local_height':options+=['--heightOffset','0']
        elif kind=='missing_height':options=['--sourceCrs','EPSG:4326','--force']
        source.write_bytes(data)
        if kind=='source_alias':target=source
        elif kind=='hardlink_alias':__import__('os').link(source,target)
        else:target.write_bytes(b'existing output sentinel')
        original=target.read_bytes();completed,response=invoke(binary,source,target,options)
        if completed.returncode==0 and baseline and kind in ('scaled_u64','scaled_i64','source_alias','hardlink_alias','zero_scale','negative_scale','header_size_zero','f32_unrepresentable_nodata','duplicate_projected_geokey','duplicate_geographic_geokey'):
            results.append({'case':kind,'baseline_gap':'accepted unsupported/unsafe profile','output_replaced':target.read_bytes()!=original,'original_source_sha256':hashlib.sha256(data).hexdigest(),'output_after_sha256':hashlib.sha256(target.read_bytes()).hexdigest(),'response':response});continue
        require(completed.returncode!=0 and response.get('ok') is False,'negative profile accepted: '+kind)
        require(target.read_bytes()==original,'refusal replaced old output: '+kind)
        if kind in ('f32_unrepresentable_nodata','untyped_extra','waveform_format') and not baseline:require(response['error']['code']=='unsupported',kind+': expected unsupported source profile')
        require(not any(p.name.startswith(('.tiles-work-','.rusty-tiles-')) for p in directory.iterdir()),'refusal workspace leak')
        if kind.startswith(('header_size','point_offset','evlr_offset')) and not baseline:require(response['error']['code']=='invalid_input','malformed header error kind')
        results.append({'case':kind,'result':'rejected','error':response.get('error')})
    return results




def concurrent_controls(binary,directory):
    import concurrent.futures
    source=directory/'concurrent-source.las';fixture(source)
    outputs=[directory/'concurrent-0.3tz',directory/'concurrent-1.3tz']
    def convert(output):
        completed,response=invoke(binary,source,output,['--sourceCrs','local','--maxPoints','16','--chunkPoints','3','--force'])
        require(completed.returncode==0,'concurrent/repeated conversion: '+completed.stdout)
        observed=audit(source,output,16)
        with zipfile.ZipFile(output) as z:report=json.loads(z.read('conversion.json'))
        require(response['counts']['points']==report['points'] and response['counts']['tiles']==report['tiles'],'CLI/embedded report counts')
        for key in ('maxPoints','chunkPoints','maxPositionRoundingMetres'):
            require(response['settings'][key]==report[key],'CLI/embedded report settings')
        return observed
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        results=list(pool.map(convert,outputs))
    first=[hashlib.sha256(output.read_bytes()).hexdigest() for output in outputs]
    repeated=[convert(output) for output in outputs]
    # Decoded source semantics are the authority. Byte repeatability is supplemental.
    return {'concurrent_jobs':2,'result':'passed','decoded_tiles':[r['tiles'] for r in results],'repeat_decoded_tiles':[r['tiles'] for r in repeated],'byte_repeatable':[before==hashlib.sha256(output.read_bytes()).hexdigest() for before,output in zip(first,outputs)]}


def self_test():
    with tempfile.TemporaryDirectory() as tmp:
        source=pathlib.Path(tmp)/'literal.las';fixture(source,count=3,metadata=True)
        records=enumerate_las(source)['rows']
        require(records[0]['source_x']==500000. and records[0]['source_y']==5000000. and records[0]['source_z']==78.418,'literal LAS scale/offset')
        require(records[0]['extra_u64']==9007199254740993 and records[0]['extra_i64']==-9007199254740993,'literal LAS integer extremes')
        source_extra=enumerate_las(source)['extra'];declared=unpack('d',source_extra[8][5],40)[0]
        require(records[0]['extra_f32']==declared,'source FLOAT32 noData sentinel exact widened declaration')
        require(records[0]['scaled_i16']==-4093.,'literal LAS scalar scale/offset')
        require({k:v for k,v in records[0].items() if k!='source_index'}=={k:v for k,v in records[1].items() if k!='source_index'},'byte-identical duplicate source records')
    binary=b'padding!'+struct.pack('<fffff',99.,1.,2.,3.,99.)+b'\0'*4+struct.pack('<Q',9007199254740993)
    doc={'asset':{'version':'2.0'},'buffers':[{'byteLength':len(binary)}],
        'bufferViews':[{'buffer':0,'byteOffset':8,'byteLength':20,'byteStride':16},{'buffer':0,'byteOffset':32,'byteLength':8}],
        'accessors':[{'bufferView':0,'byteOffset':4,'componentType':5126,'count':1,'type':'VEC3'}],
        'meshes':[{'primitives':[{'attributes':{'POSITION':0},'mode':0}]}],
        'nodes':[{'translation':[5,7,11],'scale':[2,3,4],'children':[1]},{'rotation':[0,0,math.sqrt(.5),math.sqrt(.5)],'mesh':0}],
        'scenes':[{'nodes':[0]}],'scene':0,
        'extensions':{'EXT_structural_metadata':{'schema':{'classes':{'point':{'properties':{'source_index':{'type':'SCALAR','componentType':'UINT64'}}}}},'propertyTables':[{'class':'point','count':1,'properties':{'source_index':{'values':1}}}]}}}
    positions,columns,_,_=decoded_points(pack_glb(doc,binary))
    require(math.dist(positions[0],(1.,-23.,10.))<1e-12,'literal nested scene transform and axis conversion')
    require(columns['source_index']==[9007199254740993],'literal decoded UINT64 metadata')
    controls=[]
    for name in ('short_bin','view_outside','accessor_offset','accessor_count','bad_stride','sparse','node_cycle','unsupported_extension','compressed_view'):
        changed=copy.deepcopy(doc);data=binary
        if name=='short_bin':data=b''
        elif name=='view_outside':changed['bufferViews'][0]['byteLength']=200
        elif name=='accessor_offset':changed['accessors'][0]['byteOffset']=12
        elif name=='accessor_count':changed['accessors'][0]['count']=2
        elif name=='bad_stride':changed['bufferViews'][0]['byteStride']=10
        elif name=='sparse':changed['accessors'][0]['sparse']={}
        elif name=='node_cycle':changed['nodes'][1]['children']=[0]
        elif name=='unsupported_extension':changed['extensionsRequired']=['unsupported_test']
        elif name=='compressed_view':changed['bufferViews'][0]['extensions']={'EXT_meshopt_compression':{}}
        try:decoded_points(pack_glb(changed,data))
        except (AssertionError,KeyError,ValueError,IndexError):controls.append(name)
        else:raise AssertionError('self-test accepted '+name)
    require(math.dist(ecef({'source_x':0.,'source_y':0.,'source_z':0.}),(6378137.,0.,0.))<1e-9,'literal analytic equatorial ECEF')
    require(math.dist(ecef({'source_x':90.,'source_y':0.,'source_z':0.}),(0.,6378137.,0.))<1e-8,'literal analytic ninety-degree ECEF')
    return {'literal_LAS_and_scene_controls':'passed','payload_negative_controls':controls}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary');parser.add_argument('--self-test',action='store_true');parser.add_argument('--laz-python',help='Python with laspy/lazrs, required when claiming LAZ acceptance');parser.add_argument('--output-dir');parser.add_argument('--json-output');parser.add_argument('--quick',action='store_true');parser.add_argument('--baseline',action='store_true');parser.add_argument('--fixture');parser.add_argument('--format',type=int,default=3);parser.add_argument('--variant',default='spread',choices=('spread','identical','reordered','outlier','deep'))
    args=parser.parse_args()
    if args.self_test:print(json.dumps(self_test(),indent=2));return
    self_test()
    if args.fixture:
        records=fixture(args.fixture,fmt=args.format,variant=args.variant);print(json.dumps({'points':len(records['rows']),'format':records['format']}));return
    require(args.binary,'--binary required; absent binary never skips acceptance')
    if args.output_dir:result=run(args.binary,args.output_dir,args.quick,args.baseline)
    else:
        with tempfile.TemporaryDirectory(prefix='p1-oracle-') as tmp:result=run(args.binary,tmp,args.quick,args.baseline)
    if args.laz_python:
        with tempfile.TemporaryDirectory(prefix='p1-laz-result-') as tmp:
            output=pathlib.Path(tmp)/'result.json'
            completed=subprocess.run([args.laz_python,str(pathlib.Path(__file__).with_name('p1_laz_adapter.py')),'--binary',args.binary,'--json-output',str(output)],capture_output=True,text=True)
            require(completed.returncode==0,'LAZ adapter acceptance failed: '+completed.stdout+completed.stderr)
            result['laz_acceptance']=json.loads(output.read_text())
    encoded=json.dumps(result,indent=2)+'\n'
    if args.json_output:pathlib.Path(args.json_output).write_text(encoded)
    print(encoded)

if __name__=='__main__':main()
