"""Availability-driven expansion of the approved baseline's occupied octree."""
import json,struct,os
from pathlib import Path


def tree(root):
    root=Path(root);out={}
    def add(file,parent,depth,ge,box):
        assert file not in out and (root/file).is_file()
        out[file]=dict(parent=parent,depth=depth,ge=ge,box=box)
    def walk(n,parent,depth,base=''):
        uri=os.path.normpath(os.path.join(base,n['content']['uri']))
        if uri.endswith('.json'):
            document=json.loads((root/uri).read_text())
            walk(document['root'],parent,depth,str(Path(uri).parent));return
        if 'implicitTiling' not in n:
            assert '{' not in uri and uri.endswith('.glb')
            add(uri,parent,depth,n['geometricError'],n['boundingVolume']['box'])
            for c in n.get('children',[]):walk(c,uri,depth+1,base)
            return
        it=n['implicitTiling'];assert it['subdivisionScheme']=='OCTREE' and not n.get('children')
        sub=os.path.normpath(os.path.join(base,it['subtrees']['uri']))
        for k in ['level','x','y','z']:sub=sub.replace('{'+k+'}','0')
        b=(root/sub).read_bytes();assert b[:4]==b'subt';jl,bl=struct.unpack_from('<QQ',b,8);j=json.loads(b[24:24+jl]);binary=b[24+jl:24+jl+bl]
        assert j['childSubtreeAvailability'].get('constant')==0,'this profile admits one subtree per implicit root'
        def view(i):
            v=j['bufferViews'][i];return binary[v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']]
        def bit(spec,i):
            if 'constant' in spec:return bool(spec['constant'])
            data=view(spec['bitstream']);return bool((data[i//8]>>(i%8))&1)
        ca=j['contentAvailability'];assert len(ca)==1
        pt=j['propertyTables'][j['tileMetadata']];ges=view(pt['properties']['geometricError']['values']);boxes=view(pt['properties']['boundingBox']['values'])
        row=0;offset=0;files={}
        for level in range(it['subtreeLevels']):
            for morton in range(8**level):
                exists=bit(j['tileAvailability'],offset+morton);assert bit(ca[0],offset+morton)==exists,'contentless cells not supported'
                if not exists:continue
                assert level<it['availableLevels']
                coords=[sum(((morton>>(3*i+a))&1)<<i for i in range(level)) for a in range(3)]
                file=uri.replace('{level}',str(level))
                for k,v in zip(['x','y','z'],coords):file=file.replace('{'+k+'}',str(v))
                par=parent if level==0 else files[(level-1,*(c>>1 for c in coords))]
                add(file,par,depth+level,struct.unpack_from('<d',ges,8*row)[0],list(struct.unpack_from('<12d',boxes,96*row)))
                files[(level,*coords)]=file;row+=1
            offset+=8**level
        assert row==pt['count']
    walk(json.loads((root/'tileset.json').read_text())['root'],None,0)
    physical={str(f.relative_to(root)) for folder in ['content','implicit-content'] for f in (root/folder).glob('*.glb')}
    assert set(out)==physical,'unreferenced or unavailable baseline contents'
    return out
