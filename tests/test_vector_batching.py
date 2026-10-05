"""Batching retains topology and feature identity with bounded glTF overhead."""
import json
import pathlib
import struct
import tempfile
import types
import unittest

import numpy as np
from test_vector_lod import vector, accessors, parts


def feature(fid,kind,coordinates):
    return dict(properties=dict(_source_id=str(fid),_source_layer='survey',large=2**60+fid),
        geometry=dict(type=kind,coordinates=coordinates))


def square(x,size=10,y=0):
    return [[x,y,0],[x+size,y,0],[x+size,y+size,0],[x,y+size,0],[x,y,0]]


class VectorBatchingTests(unittest.TestCase):
    def emit(self,items,**kwargs):
        tmp=tempfile.TemporaryDirectory();self.addCleanup(tmp.cleanup)
        path=pathlib.Path(tmp.name)/'tile.glb';report={}
        vector.emit(items,path,lambda p:np.asarray(p,dtype=float),encoding_report=report,**kwargs)
        return path,report

    def test_polygon_batch_offsets_holes_multipart_and_feature_ids(self):
        items=[feature(10,'Polygon',[square(0),square(2,2,2)]),
            feature(11,'MultiPolygon',[[square(20)],[square(40)]])]
        path,report=self.emit(items);doc,decode=accessors(path)
        self.assertEqual(report['primitives'],1)
        prim=doc['meshes'][0]['primitives'][0];ext=prim['extensions']['EXT_mesh_polygon']
        self.assertEqual(ext['count'],3)
        offsets=decode(ext['indicesOffsets']);loops=decode(ext['loopIndices']);starts=decode(ext['loopIndicesOffsets'])
        indices=decode(prim['indices']);pos=decode(prim['attributes']['POSITION']);ids=decode(prim['attributes']['_FEATURE_ID_0'])
        self.assertEqual(len(offsets),3);self.assertEqual(len(starts),3)
        self.assertEqual(np.count_nonzero(loops==0xffffffff),3) # one hole plus two polygon separators
        self.assertNotEqual(int(loops[-1]),0xffffffff)
        areas=[]
        for fid,tri,loop in zip([0,1,1],np.split(indices,offsets[1:]),np.split(loops,starts[1:])):
            self.assertTrue(np.all(ids[tri]==fid))
            self.assertTrue(np.all(ids[loop[loop!=0xffffffff]]==fid))
            xyz=pos[tri.reshape(-1,3)]
            areas.append(np.linalg.norm(np.cross(xyz[:,1]-xyz[:,0],xyz[:,2]-xyz[:,0]),axis=1).sum()/2)
        np.testing.assert_allclose(areas,[96,100,100])
        self.assertEqual([fid for _,fid,_ in parts(path)],[0,1,1])

    def test_repaired_polygon_offsets_followed_by_another_feature(self):
        bow=[[0,0,0],[2,2,0],[2,0,0],[0,2,0],[0,0,0]]
        path,_=self.emit([feature(1,'Polygon',[bow]),feature(2,'Polygon',[square(10)])],repair=True)
        doc,decode=accessors(path);prim=doc['meshes'][0]['primitives'][0];ext=prim['extensions']['EXT_mesh_polygon']
        self.assertEqual(ext['count'],3)
        self.assertEqual([fid for _,fid,_ in parts(path)],[0,0,1])
        indices=decode(prim['indices']);starts=decode(ext['indicesOffsets'])
        pos=decode(prim['attributes']['POSITION']);areas=[]
        for group in np.split(indices,starts[1:]):
            tri=pos[group.reshape(-1,3)]
            areas.append(np.linalg.norm(np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0]),axis=1).sum()/2)
        np.testing.assert_allclose(areas,[1,1,100])

    def test_disconnected_lines_restart_without_cross_feature_segments(self):
        lines=[[[0,0,0],[1,0,0]],[[10,0,0],[11,1,0],[12,0,0]],[[20,0,0],[21,0,0]]]
        path,report=self.emit([feature(1,'LineString',lines[0]),feature(2,'MultiLineString',lines[1:])])
        doc,decode=accessors(path);prim=doc['meshes'][0]['primitives'][0]
        self.assertEqual(report['primitives'],1);self.assertEqual(prim['mode'],3)
        self.assertIn('KHR_mesh_primitive_restart',doc['extensionsUsed'])
        self.assertIn('KHR_mesh_primitive_restart',doc['extensionsRequired'])
        self.assertNotIn('KHR_mesh_primitive_restart',prim['extensions'])
        self.assertEqual(decode(prim['indices']).tolist(),[0,1,0xffffffff,2,3,4,0xffffffff,5,6])
        decoded=parts(path);self.assertEqual([fid for _,fid,_ in decoded],[0,1,1])
        for (_,_,actual),expected in zip(decoded,lines):np.testing.assert_array_equal(actual,expected)
        single,_=self.emit([feature(1,'LineString',lines[0])]);doc,_=accessors(single)
        self.assertNotIn('KHR_mesh_primitive_restart',doc.get('extensionsRequired',[]))

    def test_mixed_geometry_metadata_and_standard_fill(self):
        items=[feature(1,'Point',[0,0,0]),feature(2,'MultiPoint',[[1,1,1],[2,2,2]]),
            feature(3,'LineString',[[3,0,0],[4,0,0]]),feature(4,'Polygon',[square(20)])]
        path,report=self.emit(items);doc,decode=accessors(path)
        self.assertEqual(report['primitives'],3)
        self.assertEqual([p['mode'] for p in doc['meshes'][0]['primitives']],[0,3,4])
        self.assertEqual([fid for _,fid,_ in parts(path)],[0,1,2,3])
        table=doc['extensions']['EXT_structural_metadata']['propertyTables'][0]
        self.assertEqual(table['count'],4)
        for prim in doc['meshes'][0]['primitives']:
            self.assertEqual(prim['extensions']['EXT_mesh_features']['featureIds'][0]['propertyTable'],0)
        fill,_=self.emit([feature(1,'Polygon',[square(0)]),feature(2,'Polygon',[square(20)])],fill_only=True)
        doc,decode=accessors(fill);self.assertEqual(len(doc['meshes'][0]['primitives']),1)
        self.assertNotIn('EXT_mesh_polygon',doc['extensionsUsed']);self.assertEqual(len(doc['accessors']),3)
        self.assertEqual(len(parts(fill)),4) # four indexed triangles, not two vertex arrays

    def test_thousand_polygons_keep_json_constant_and_quantized_ids_exact(self):
        items=[feature(i,'Polygon',[square(i*20)]) for i in range(1000)]
        path,report=self.emit(items,quantize=True);doc,decode=accessors(path)
        self.assertEqual(report['primitives'],1);self.assertEqual(report['vertices'],4000)
        self.assertLess(struct.unpack_from('<I',path.read_bytes(),12)[0],4096)
        self.assertEqual(len(doc['accessors']),6)
        prim=doc['meshes'][0]['primitives'][0];ids=decode(prim['attributes']['_FEATURE_ID_0'])
        np.testing.assert_array_equal(ids,np.repeat(np.arange(1000),4))
        ac=doc['accessors'][prim['attributes']['POSITION']];self.assertTrue(ac['normalized'])
        actual=decode(prim['attributes']['POSITION'])/65535*np.array(doc['nodes'][0]['scale'])+np.array(doc['nodes'][0]['translation'])
        expected=np.array([p for i in range(1000) for p in square(i*20)[:-1]])
        self.assertLessEqual(np.linalg.norm(actual-expected,axis=1).max(),report['quantizationError']+1e-6)

    def test_pipeline_counts_match_contents_and_actual_budgets(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'small.geojson';out=root/'out'
            source.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id=i,
                properties=dict(name=str(i)),geometry=dict(type='Polygon',coordinates=[square(i*20)])) for i in range(16)])))
            vector.run(types.SimpleNamespace(input=str(source),output=str(out),source_crs='local',max_features=4,
                max_vertices=32,max_bytes=4096,lod_levels=2))
            manifest=json.loads((out/'tileset.json').read_text());report=json.loads((out/'conversion.json').read_text());counts=[]
            def walk(node):
                total=0;vertices=0;size=0
                for content in node.get('contents',[node['content']] if 'content' in node else []):
                    file=out/content['uri'];doc,decode=accessors(file);prims=doc['meshes'][0]['primitives'];total+=len(prims)
                    vertices+=sum(doc['accessors'][p['attributes']['POSITION']]['count'] for p in prims);size+=file.stat().st_size
                if total:
                    self.assertEqual(node['extras']['primitives'],total);self.assertEqual(node['extras']['vertices'],vertices)
                    self.assertLessEqual(vertices,32);self.assertLessEqual(size,4096)
                counts.append(total)
                for child in node.get('children',[]):walk(child)
            walk(manifest['root'])
            self.assertEqual(report['encoding']['primitiveReferences'],sum(counts))
            self.assertEqual(report['encoding']['maximumTilePrimitives'],max(counts))


if __name__=='__main__':unittest.main()
