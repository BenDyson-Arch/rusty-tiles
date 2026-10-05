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
spec = importlib.util.spec_from_file_location('vector_lod', ROOT/'scripts/vector.py')
vector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vector)


def distance(points, path):
    points, path = np.asarray(points), np.asarray(path)
    a,b = path[:-1],path[1:]
    ab = b-a
    t = np.clip(np.einsum('pki,ki->pk',points[:,None,:]-a,ab)/np.maximum(np.sum(ab*ab,axis=1),1e-30),0,1)
    closest = a+t[:,:,None]*ab
    return np.linalg.norm(points[:,None,:]-closest,axis=2).min(axis=1)


def read(path):
    data = path.read_bytes()
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


class VectorLodTests(unittest.TestCase):
    def test_3d_rdp_error_measured_independently_in_both_directions(self):
        x=np.linspace(0,30,301)
        source=np.column_stack([x,.1*np.sin(x),.07*np.cos(x)])
        out,error=vector.simplify_path(source,.2,set())
        self.assertLess(len(out),len(source)//10)
        dense=np.concatenate([a+(b-a)*np.linspace(0,1,10)[:,None] for a,b in zip(source[:-1],source[1:])])
        self.assertLessEqual(distance(dense,out).max(),error+1e-12)
        out=np.asarray(out)
        chord=np.concatenate([a+(b-a)*np.linspace(0,1,100)[:,None] for a,b in zip(out[:-1],out[1:])])
        self.assertLessEqual(distance(chord,source).max(),error+1e-12)

    def test_vertical_polygon_with_hole_simplifies_and_retains_topology(self):
        def square(x0,y0,x1,y1):
            points=[]
            corners=np.array([[x0,0,y0],[x1,0,y0],[x1,0,y1],[x0,0,y1]])
            for a,b in zip(corners,np.roll(corners,-1,axis=0)):
                points.extend((a+(b-a)*np.linspace(0,1,11,endpoint=False)[:,None]).tolist())
            return points+[points[0]]
        rings=[square(0,0,20,20),square(5,5,10,10)]
        out,error,reason=vector.simplify_polygon(rings,.01,set())
        self.assertIsNone(reason)
        self.assertEqual(len(out),2)
        self.assertLess(sum(map(len,out)),sum(map(len,rings))//3)
        p,i,loops,*_=vector.polygon(out)
        tri=np.asarray(p)[np.asarray(i).reshape(-1,3)]
        area=np.linalg.norm(np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0]),axis=1).sum()/2
        self.assertAlmostEqual(area,375,places=8)
        self.assertEqual(loops.count(0xffffffff),1)
        for a,b in zip(rings,out):
            self.assertLessEqual(distance(a,b).max(),error+1e-10)

    def test_shared_vertices_and_nonplanar_polygons_stay_fixed(self):
        source=[[0,0,0],[1,.1,0],[2,0,0],[3,0,0]]
        out,_=vector.simplify_path(source,1,{tuple(source[1])})
        self.assertIn(source[1],out)
        nonplanar=[[[0,0,0],[1,0,0],[1,1,1],[0,1,0],[0,0,0]]]
        out,error,reason=vector.simplify_polygon(nonplanar,10,set())
        self.assertEqual(out,nonplanar)
        self.assertEqual(error,0)
        self.assertIn('nonplanar',reason)

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

    def test_mixed_features_have_content_and_properties_at_every_level(self):
        with tempfile.TemporaryDirectory() as tmp:
            vector.run(types.SimpleNamespace(input=str(ROOT/'tests/fixtures/vector.geojson'),output=tmp,max_features=2))
            root=json.loads((pathlib.Path(tmp)/'tileset.json').read_text())['root']
            def walk(node):
                self.assertIn('content',node)
                doc,_,ids=read(pathlib.Path(tmp)/node['content']['uri'])
                schema=doc['extensions']['EXT_structural_metadata']['schema']['classes']['feature']['properties']
                self.assertIn('height',schema)
                self.assertEqual(len(ids),len(set(ids)))
                all_ids=[]
                for c in node.get('children',[]):
                    self.assertGreaterEqual(node['geometricError'],c['geometricError'])
                    all_ids.extend(walk(c))
                    b,bc=node['boundingVolume']['box'],c['boundingVolume']['box']
                    self.assertTrue((np.abs(np.array(bc[:3])-b[:3])+np.array([bc[3],bc[7],bc[11]]) <= np.array([b[3],b[7],b[11]])+1e-8).all())
                if all_ids:
                    self.assertEqual(sorted(ids),sorted(all_ids))
                return ids
            self.assertEqual(len(walk(root)),4)


if __name__=='__main__':
    unittest.main()
