"""Independent path distance, parent-content, identity and topology regressions."""
import importlib.util
import json
import pathlib
import struct
import tempfile
import types
import unittest

import numpy as np
from osgeo import ogr

ROOT = pathlib.Path(__file__).resolve().parents[1]
from vector_test_support import vector


def distance(points, path):
    points, path = np.asarray(points), np.asarray(path)
    a,b = path[:-1],path[1:]
    ab = b-a
    t = np.clip(np.einsum('pki,ki->pk',points[:,None,:]-a,ab)/np.maximum(np.sum(ab*ab,axis=1),1e-30),0,1)
    closest = a+t[:,:,None]*ab
    return np.linalg.norm(points[:,None,:]-closest,axis=2).min(axis=1)


def read(path):
    data = path.read_bytes()
    if data[:4]==b'b3dm':
        header=struct.unpack_from('<4s6I',data)
        data=data[28+sum(header[3:]):]
    n = struct.unpack_from('<I',data,12)[0]
    doc = json.loads(data[20:20+n])
    binary = data[28+n:]
    def view(index,dtype,count=-1):
        v=doc['bufferViews'][index]
        return np.frombuffer(binary[v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']],dtype,count)
    meta=doc['extensions']['EXT_structural_metadata']
    table=meta['propertyTables'][0]
    idcol=table['properties']['_source_id']
    strings=view(idcol['values'],'u1').tobytes()
    offsets=view(idcol['stringOffsets'],'<u4')
    ids=[strings[a:b].decode() for a,b in zip(offsets[:-1],offsets[1:])]
    positions=[]
    for primitive in doc['meshes'][0]['primitives']:
        ac=doc['accessors'][primitive['attributes']['POSITION']]
        positions.append(view(ac['bufferView'],'<f4',ac['count']*3).reshape(-1,3))
    return doc,positions,ids


def accessors(path):
    """Decode uncompressed accessors independently, respecting component stride."""
    data=path.read_bytes()
    if data[:4]==b'b3dm':data=data[28+sum(struct.unpack_from('<4s6I',data)[3:]):]
    n=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+n]);binary=data[28+n:]
    def decode(index):
        ac=doc['accessors'][index];v=doc['bufferViews'][ac['bufferView']]
        dtype=np.dtype({5123:'<u2',5125:'<u4',5126:'<f4'}[ac['componentType']])
        width={'SCALAR':1,'VEC3':3}[ac['type']]
        values=np.ndarray((ac['count'],width),dtype,buffer=binary,
            offset=v.get('byteOffset',0)+ac.get('byteOffset',0),
            strides=(v.get('byteStride',dtype.itemsize*width),dtype.itemsize)).copy()
        return values.reshape(-1) if width==1 else values
    return doc,decode


def parts(path):
    """Decode semantic parts, rather than treating shared vertex arrays as paths."""
    doc,decode=accessors(path);out=[]
    for prim in doc['meshes'][0]['primitives']:
        pos=decode(prim['attributes']['POSITION']);fids=decode(prim['attributes']['_FEATURE_ID_0'])
        indices=decode(prim['indices']);mode=prim['mode'];polygon=prim.get('extensions',{}).get('EXT_mesh_polygon')
        if polygon:
            loops=decode(polygon['loopIndices']);starts=decode(polygon['loopIndicesOffsets'])
            # Loop indices may be 8-, 16- or 32-bit; each has its own restart value.
            restart={5121:0xff,5123:0xffff,5125:0xffffffff}[doc['accessors'][polygon['loopIndices']]['componentType']]
            groups=[np.unique(loop[loop!=restart]) for loop in np.split(loops,starts[1:])]
        elif mode==3:
            groups=[group[group!=0xffffffff] for group in np.split(indices,np.flatnonzero(indices==0xffffffff))]
        elif mode==4:groups=indices.reshape(-1,3)
        else:groups=[indices[fids[indices]==fid] for fid in np.unique(fids[indices])]
        for group in groups:
            if len(group):
                fid=int(fids[group[0]])
                if not np.all(fids[group]==fid):raise AssertionError('semantic part crosses feature identities')
                out.append((mode,fid,pos[group]))
    return out


def rings(path, offset):
    """Ordered polygon rings per source ID in the tile frame (Z-up) plus `offset`."""
    doc,decode=accessors(path);_,_,ids=read(path);out={}
    for prim in doc['meshes'][0]['primitives']:
        pos=decode(prim['attributes']['POSITION'])[:,[0,2,1]]*[1,-1,1]+offset
        fid=decode(prim['attributes']['_FEATURE_ID_0']);poly=prim['extensions']['EXT_mesh_polygon']
        loops=decode(poly['loopIndices']);starts=decode(poly['loopIndicesOffsets'])
        restart={5121:0xff,5123:0xffff,5125:0xffffffff}[doc['accessors'][poly['loopIndices']]['componentType']]
        for loop in np.split(loops,starts[1:]):
            for ring in np.split(loop,np.flatnonzero(loop==restart)):
                ring=ring[ring!=restart]
                if len(ring):out.setdefault(ids[int(fid[ring[0]])],[]).append(pos[ring])
    return out


class VectorLodTests(unittest.TestCase):
    def test_nonplanar_layers_have_real_parent_content_and_exact_leaf_vertices(self):
        from test_vector_gpkg import gpkg,nodes
        from test_vector_reuse import details
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=root/'wavy.gpkg';features=[]
            a=np.linspace(0,2*np.pi,200,endpoint=False)
            for i in range(32):
                ring=np.column_stack([i*2+.15*np.cos(a),.15*np.sin(a),.005*np.sin(5*a)]).tolist()
                features.append((i+1,dict(type='Polygon',coordinates=[ring+[ring[0]]]),1))
            gpkg(p,[('wavy',3857,features)])
            out=root/'out';vector.run(types.SimpleNamespace(input=str(p),output=str(out),source_crs='local',max_features=4,lod_tolerance=.03,lod_levels=3))
            manifest=json.loads((out/'tileset.json').read_text())
            parents=[n for n in nodes(manifest['root']) if n.get('children') and 'content' in n]
            self.assertGreater(len(parents),0)
            for node in parents:
                self.assertLessEqual(node['extras']['vertices'],node['extras']['featureFragments']*200)
                self.assertLessEqual(node['extras']['geometryErrorMetres'],node['extras']['toleranceMetres']+1e-12)
            self.assertTrue(any(n['extras']['vertices']<n['extras']['featureFragments']*200 for n in parents))
            decoded=details(out);self.assertEqual(len(decoded),32)
            for fid,g,_ in features:
                xyz=decoded[('wavy',str(fid),4)][0][1]
                self.assertEqual(len(xyz),200)
                np.testing.assert_allclose(xyz,g['coordinates'][0][:-1],atol=2e-5,rtol=0)

    def test_parent_content_reduces_vertices_preserves_identity_and_source_leaf(self):
        with tempfile.TemporaryDirectory() as tmp:
            source=pathlib.Path(tmp)/'source.geojson'
            coords=[[9+i*.000001,45+math_y,90+(.05*np.cos(i))] for i,math_y in
                    enumerate(.00000005*np.sin(np.arange(201)))]
            feature=dict(type='Feature',id='ordinary-road',properties=dict(name='road',count=7),
                         geometry=dict(type='LineString',coordinates=coords))
            source.write_text(json.dumps(dict(type='FeatureCollection',features=[feature])))
            out=pathlib.Path(tmp)/'tiles'
            vector.run(types.SimpleNamespace(input=str(source),output=str(out),max_features=64,lod_tolerance=.2,lod_levels=3))
            manifest=json.loads((out/'tileset.json').read_text())
            node=manifest['root']
            _,coarse,ids=read(out/node['content']['uri'])
            self.assertEqual(ids,['"ordinary-road"'])
            self.assertLess(len(coarse[0]),len(coords)//4)
            self.assertGreater(node['geometricError'],0)
            while 'children' in node:
                child=node['children'][0]
                self.assertGreaterEqual(node['geometricError'],child['geometricError'])
                _,_,level_ids=read(out/child['content']['uri'])
                self.assertEqual(level_ids,ids)
                node=child
            _,leaves,_=read(out/node['content']['uri'])
            self.assertEqual(len(leaves[0]),len(coords))
            self.assertLessEqual(distance(leaves[0],coarse[0]).max(),manifest['root']['geometricError']+1e-5)
            self.assertEqual(node['geometricError'],0)
            report=json.loads((out/'conversion.json').read_text())
            self.assertEqual(report['leafTiles'],1)
            self.assertGreaterEqual(report['tiles'],2)
            self.assertLessEqual(report['tiles'],4)

    def test_mixed_features_route_with_bounded_content_and_preserve_properties(self):
        with tempfile.TemporaryDirectory() as tmp:
            vector.run(types.SimpleNamespace(input=str(ROOT/'tests/fixtures/vector.geojson'),output=tmp,max_features=2))
            root=json.loads((pathlib.Path(tmp)/'tileset.json').read_text())['root']
            def walk(node):
                ids=[]
                if 'content' in node:
                    doc,_,ids=read(pathlib.Path(tmp)/node['content']['uri'])
                    schema=doc['extensions']['EXT_structural_metadata']['schema']['classes']['feature']['properties']
                    self.assertIn('height',schema)
                    self.assertLessEqual(len(ids),4096 if node.get('children') else 2)
                    self.assertEqual(len(ids),len(set(ids)))
                else:
                    self.assertTrue(node['extras']['routing'])
                all_ids=[]
                for c in node.get('children',[]):
                    self.assertGreaterEqual(node['geometricError'],c['geometricError'])
                    all_ids.extend(walk(c))
                    b,bc=node['boundingVolume']['box'],c['boundingVolume']['box']
                    delta=np.array(c['transform'][12:15])
                    self.assertTrue((np.abs(np.array(bc[:3])+delta-b[:3])+np.array([bc[3],bc[7],bc[11]]) <= np.array([b[3],b[7],b[11]])+1e-8).all())
                if all_ids and ids:
                    # Coarse tiles may omit features within their error; each covers the rest.
                    self.assertLessEqual(set(ids),set(all_ids))
                return all_ids or ids
            self.assertEqual(len(walk(root)),4)

    def test_coarse_tiles_omit_small_features_within_a_shrinking_error(self):
        """Coarse tiles keep a subset; what they omit or simplify lies within their error."""
        rng=np.random.default_rng(7);features=[];a=np.linspace(0,2*np.pi,24,endpoint=False)
        for i in range(600):
            r,(x,y)=rng.uniform(.05,1.5),rng.uniform(0,300,2)
            features.append(np.column_stack([x+r*np.cos(a),y+.6*r*np.sin(a),0*a]))
        b=np.linspace(0,2*np.pi,64,endpoint=False)
        for x,y in [(40,40),(260,60),(150,250)]:
            features.append(np.column_stack([x+20*np.cos(b)*(1+.1*np.sin(7*b)),y+20*np.sin(b),0*b]))
        with tempfile.TemporaryDirectory() as tmp:
            source=pathlib.Path(tmp)/'field.geojson';out=pathlib.Path(tmp)/'out'
            source.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id=i,properties=dict(n=i),
                geometry=dict(type='Polygon',coordinates=[f.tolist()+[f[0].tolist()]])) for i,f in enumerate(features)])))
            vector.run(types.SimpleNamespace(input=str(source),output=str(out),source_crs='local',max_features=16))
            root=json.loads((out/'tileset.json').read_text())['root'];self.assertIn('content',root)
            def walk(node,offset,depth=0):
                if depth:offset=offset+node['transform'][12:15]
                tile=rings(out/node['content']['uri'],offset) if 'content' in node else None
                below=[walk(c,offset,depth+1) for c in node.get('children',[])]
                leaves={k:v for c in below for k,v in c[0].items()}
                if not below:return tile,node
                if tile is not None:
                    g=node['geometricError']+1e-4
                    self.assertTrue(set(tile)<=set(leaves))
                    if not depth:self.assertLess(len(tile),len(leaves))
                    for sid,exact in leaves.items():
                        points=np.concatenate(exact)
                        if sid not in tile:
                            self.assertLessEqual(np.linalg.norm(np.ptp(points,axis=0)),g)
                            continue
                        coarse=tile[sid]
                        for a,b in [(points,coarse),(np.concatenate(coarse),exact)]:
                            self.assertLessEqual(np.min([distance(a,np.vstack([r,r[:1]])) for r in b],axis=0).max(),g)
                    for c in node['children']:
                        if c.get('children') and 'content' in c:self.assertGreater(node['geometricError'],c['geometricError'])
                return leaves,node
            leaves,_=walk(root,np.zeros(3));self.assertEqual(len(leaves),603)

    def test_no_routing_tile_under_content_and_stand_ins_ignore_input_order(self):
        def square(x, y, s):
            c=[[x, y, 0], [x+s, y, 0], [x+s, y+s, 0], [x, y+s, 0]];return dict(type='Polygon', coordinates=[c+c[:1]])
        rng=np.random.default_rng(3)
        cluster=[square(*rng.uniform(0, 40, 2), 5.) for _ in range(480)]+[square(*rng.uniform(0, 2000, 2), .5) for _ in range(40)]
        grid=[square(3*(i % 20), 3*(i//20), 1) for i in range(400)]
        def bake(tmp, name, geometries, order, **options):
            source=pathlib.Path(tmp)/f'{name}.geojson';out=pathlib.Path(tmp)/name
            source.write_text(json.dumps(dict(type='FeatureCollection', features=[dict(type='Feature', id=i,
                properties=dict(n=i), geometry=geometries[i]) for i in order])))
            vector.run(types.SimpleNamespace(input=str(source), output=str(out), source_crs='local', max_features=16, **options))
            def walk(node, above=False):
                content='content' in node;self.assertTrue(content or not above)
                if content and node.get('children'):
                    self.assertLessEqual(node['geometricError'], node['extras']['toleranceMetres']+1e-3)
                ids=sorted(read(out/node['content']['uri'])[2]) if content else None
                return [ids]+[x for c in node.get('children', []) for x in walk(c, above or content)]
            return walk(json.loads((out/'tileset.json').read_text())['root'])
        with tempfile.TemporaryDirectory() as tmp:
            bake(tmp, 'cluster', cluster, range(520), max_parent_features=60)
            shuffled=list(range(400));np.random.default_rng(5).shuffle(shuffled)
            self.assertEqual(bake(tmp, 'ordered', grid, range(400)), bake(tmp, 'shuffled', grid, shuffled))


if __name__=='__main__':
    unittest.main()
