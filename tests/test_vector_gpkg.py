"""OGR placement, identity, budget and fragment surface checks."""
import json
import os
import subprocess
import pathlib
import struct
import sys
import tempfile
import types
import unittest
import zipfile

import numpy as np
from osgeo import ogr, osr
from test_vector_lod import vector, read, parts
from cli_bin import requires_bin



def details(path):
    from test_vector_reuse import details as decode
    return decode(path)


def nodes(node):
    yield node
    for child in node.get('children',[]):
        yield from nodes(child)


def gpkg(path, layers, spatial_index=True):
    ds=ogr.GetDriverByName('GPKG').CreateDataSource(str(path))
    for name,epsg,features in layers:
        srs=osr.SpatialReference();srs.ImportFromEPSG(epsg)
        layer=ds.CreateLayer(name,srs,ogr.wkbUnknown,options=[] if spatial_index else ['SPATIAL_INDEX=NO'])
        for key,kind in [('name',ogr.OFTString),('large',ogr.OFTInteger64)]:
            layer.CreateField(ogr.FieldDefn(key,kind))
        for fid,g,large in features:
            f=ogr.Feature(layer.GetLayerDefn());f.SetFID(fid);f.SetField('name',name)
            if large is not None:f.SetField('large',large)
            f.SetGeometry(ogr.CreateGeometryFromJson(json.dumps(g)))
            layer.CreateFeature(f)
    ds=None


class GeoPackageTests(unittest.TestCase):
    def test_declared_three_axis_crs_preserves_ellipsoidal_height(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp)
            for epsg in (4979,7843,4937):
                with self.subTest(epsg=epsg):
                    p=root/f'{epsg}.gpkg';coordinates=[12.,50.,120.] if epsg==4937 else [153.,-27.,120.]
                    gpkg(p,[('sites',epsg,[(1,dict(type='Point',coordinates=coordinates),None)])])
                    args=types.SimpleNamespace(input=str(p),output=str(root/f'out-{epsg}'),max_features=1)
                    vector.run(args)
                    decoded=details(args.output)
                    self.assertEqual(len(decoded),1)
                    report=json.loads((pathlib.Path(args.output)/'conversion.json').read_text())
                    self.assertEqual(report['layers'][0]['heightMode'],'declared CRS')
                    # Independent ellipsoidal-to-ECEF formula for the CRS ellipsoid.
                    srs=osr.SpatialReference();srs.ImportFromEPSG(epsg)
                    a=srs.GetSemiMajor();inverse=srs.GetInvFlattening();f=1/inverse;e2=f*(2-f)
                    lon,lat=np.radians(coordinates[:2]);height=coordinates[2]
                    radius=a/np.sqrt(1-e2*np.sin(lat)**2)
                    expected=[(radius+height)*np.cos(lat)*np.cos(lon),
                              (radius+height)*np.cos(lat)*np.sin(lon),(radius*(1-e2)+height)*np.sin(lat)]
                    np.testing.assert_allclose(next(iter(decoded.values()))[0][1][0],expected,atol=.01,rtol=0)
                    self.assertTrue((pathlib.Path(args.output)/'tileset.json').is_file())
                    args.height_offset=0
                    args.output += '-offset'
                    with self.assertRaisesRegex(ValueError,'already defines heights'):vector.run(args)

    def test_coincident_duplicate_source_ids_partition_and_retain_every_feature(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'duplicates.geojson'
            features=[dict(type='Feature',id=7,properties=dict(label='same'),
                geometry=dict(type='Point',coordinates=[0,0,0])) for _ in range(8)]
            source.write_text(json.dumps(dict(type='FeatureCollection',features=features)))
            args=types.SimpleNamespace(input=str(source),output=str(root/'out'),source_crs='local',max_features=1)
            vector.run(args)
            out=pathlib.Path(args.output);manifest=json.loads((out/'tileset.json').read_text())
            leaves=[n for n in nodes(manifest['root']) if not n.get('children')]
            self.assertEqual(len(leaves),8)
            for leaf in leaves:
                self.assertEqual(leaf['extras']['featureFragments'],1)
                self.assertEqual(read(out/leaf['content']['uri'])[2],['7'])
            report=json.loads((out/'conversion.json').read_text())
            self.assertEqual(report['features'],8);self.assertEqual(report['fragments'],8)
            previous=root/'previous.3tz'
            with zipfile.ZipFile(previous,'w') as archive:
                for file in out.rglob('*'):
                    if file.is_file():archive.write(file,file.relative_to(out).as_posix())
            # Every surviving row moves to one side of the saved cuts. They
            # must be recomputed while retaining duplicate source identities.
            features=features[:3]
            for feature in features:feature['geometry']['coordinates']=[100,0,0]
            source.write_text(json.dumps(dict(type='FeatureCollection',features=features)))
            args.output=str(root/'replacement');args.reuse_tileset=str(previous)
            vector.run(args)
            replacement=pathlib.Path(args.output)
            updated=json.loads((replacement/'tileset.json').read_text())['root']
            leaves=[n for n in nodes(updated) if not n.get('children')]
            self.assertEqual(len(leaves),3)
            for leaf in leaves:self.assertEqual(read(replacement/leaf['content']['uri'])[2],['7'])
            self.assertEqual(json.loads((replacement/'conversion.json').read_text())['features'],3)

    def test_projected_layers_selection_and_exact_nullable_int64(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=pathlib.Path(tmp)/'source.gpkg'
            g=lambda x:dict(type='Point',coordinates=[x,5000000])
            gpkg(p,[('roads',3857,[(7,g(1000000),2**60+3),(8,g(1000010),None)]),('sites',3857,[(7,g(1000020),11)])])
            with self.assertRaisesRegex(ValueError,'select --layer'):
                vector.run(types.SimpleNamespace(input=str(p),output=str(pathlib.Path(tmp)/'invalid')))
            args=types.SimpleNamespace(input=str(p),output=str(pathlib.Path(tmp)/'out'),max_features=64,layers=['roads'])
            vector.run(args)
            decoded=details(args.output)
            self.assertEqual({key[1] for key in decoded},{'7','8'})
            report=json.loads((pathlib.Path(args.output)/'conversion.json').read_text())
            self.assertEqual(report['layers'][0]['heightMode'],'2D ellipsoid zero')
            target=osr.SpatialReference();target.ImportFromEPSG(4978)
            source=osr.SpatialReference();source.ImportFromEPSG(3857);source.PromoteTo3D()
            expected=osr.CoordinateTransformation(source,target).TransformPoint(1000000,5000000,0)
            np.testing.assert_allclose(decoded[('roads','7',0)][0][1][0],expected,atol=1e-7)
            out=pathlib.Path(args.output);manifest=json.loads((out/'tileset.json').read_text())
            file=out/manifest['root']['content']['uri'];data=file.read_bytes();n=struct.unpack_from('<I',data,12)[0]
            doc=json.loads(data[20:20+n]);binary=data[28+n:]
            meta=doc['extensions']['EXT_structural_metadata'];schema=meta['schema']['classes']['feature']['properties']['large']
            self.assertEqual(schema['componentType'],'INT64')
            col=meta['propertyTables'][0]['properties']['large'];view=doc['bufferViews'][col['values']]
            actual=np.frombuffer(binary,'<i8',2,view.get('byteOffset',0))
            self.assertEqual(actual[0],2**60+3);self.assertEqual(actual[1],schema['noData'])
            args.output=str(pathlib.Path(tmp)/'all');args.layers=[];args.all_layers=True
            vector.run(args);r=json.loads((pathlib.Path(args.output)/'conversion.json').read_text())
            self.assertEqual(r['features'],3);self.assertEqual(len(r['layers']),2)

    def test_3d_horizontal_crs_requires_explicit_height_offset(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=pathlib.Path(tmp)/'z.gpkg'
            gpkg(p,[('sites',3857,[(1,dict(type='Point',coordinates=[1000000,5000000,20]),None)])])
            args=types.SimpleNamespace(input=str(p),output=str(pathlib.Path(tmp)/'out'))
            with self.assertRaisesRegex(ValueError,'height-offset'):
                vector.run(args)
            args.height_offset=5
            vector.run(args)
            actual=details(args.output)[('sites','1',0)][0][1][0]
            target=osr.SpatialReference();target.ImportFromEPSG(4979);target.SetAxisMappingStrategy(osr.OAMS_TRADITIONAL_GIS_ORDER)
            source=osr.SpatialReference();source.ImportFromEPSG(4978)
            self.assertAlmostEqual(osr.CoordinateTransformation(source,target).TransformPoint(*actual)[2],25,places=6)

    def test_fragmented_line_covers_every_source_segment_and_caps_every_glb(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=pathlib.Path(tmp)/'line.gpkg'
            coords=[[float(i),float(np.sin(i)),0.] for i in range(513)]
            gpkg(p,[('roads',3857,[(23,dict(type='LineString',coordinates=coords),2**60+3)])])
            args=types.SimpleNamespace(input=str(p),output=str(pathlib.Path(tmp)/'out'),max_features=64,
                source_crs='local',max_vertices=40,max_bytes=4096)
            vector.run(args)
            out=pathlib.Path(args.output);root=json.loads((out/'tileset.json').read_text())['root']
            segments=[]
            def walk(node,translation):
                translation=translation+np.array(node.get('transform',np.eye(4).T.flatten().tolist())[12:15])
                if 'content' in node:
                    file=out/node['content']['uri'];_,positions,ids=read(file)
                    self.assertLessEqual(file.stat().st_size,4096)
                    self.assertLessEqual(sum(map(len,positions)),40)
                    self.assertTrue(all(i=='23' for i in ids))
                    if not node.get('children'):
                        for _,_,line in parts(file):
                            xyz=line[:,[0,2,1]]*[1,-1,1]+translation
                            segments.extend(zip(xyz[:-1],xyz[1:]))
                for child in node.get('children',[]):walk(child,translation)
            walk(root,np.zeros(3))
            self.assertEqual(len(segments),len(coords)-1)
            pairs={round(float(a[0])):(a,b) for a,b in segments}
            self.assertEqual(set(pairs),set(range(512)))
            for i,(a,b) in pairs.items():
                np.testing.assert_allclose(a,coords[i],atol=2e-6);np.testing.assert_allclose(b,coords[i+1],atol=2e-6)

    def test_oversized_polygon_fragments_preserve_hole_area(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=pathlib.Path(tmp)/'poly.gpkg'
            rings=[[[0,0,0],[10,0,0],[10,10,0],[0,10,0],[0,0,0]],
                   [[3,3,0],[3,7,0],[7,7,0],[7,3,0],[3,3,0]]]
            gpkg(p,[('land',3857,[(9,dict(type='Polygon',coordinates=rings),None)])])
            args=types.SimpleNamespace(input=str(p),output=str(pathlib.Path(tmp)/'out'),max_features=1,
                source_crs='local',max_vertices=8,max_bytes=8192)
            vector.run(args);out=pathlib.Path(args.output);root=json.loads((out/'tileset.json').read_text())['root']
            area=0.;outline_length=0.
            for node in nodes(root):
                if node.get('children'):continue
                vertex_count=0;byte_count=0
                for content in node.get('contents',[node.get('content')]):
                    file=out/content['uri'];doc,positions,ids=read(file)
                    self.assertTrue(all(i=='9' for i in ids));vertex_count+=sum(map(len,positions));byte_count+=file.stat().st_size
                    if 'extensions' not in content:
                        for _,_,tri in parts(file):
                            area+=np.linalg.norm(np.cross(tri[1]-tri[0],tri[2]-tri[0]))/2
                    else:
                        # Every outline edge belongs to an original outer/hole edge.
                        source_edges=[(np.array(a),np.array(b)) for ring in rings for a,b in zip(ring[:-1],ring[1:])]
                        translation=np.zeros(3)
                        # This small fixture has root origin (0,0,0); recover each
                        # leaf's absolute translation through the hierarchy below.
                        def locate(current,delta):
                            delta=delta+np.array(current.get('transform',np.eye(4).T.flatten())[12:15])
                            if current is node:return delta
                            for c in current.get('children',[]):
                                result=locate(c,delta)
                                if result is not None:return result
                        translation=locate(root,np.zeros(3))
                        for _,_,line in parts(file):
                            xyz=line[:,[0,2,1]]*[1,-1,1]+translation
                            def on_edge(a,b):
                                ab=b-a;t=(xyz-a)@ab/max(np.dot(ab,ab),1e-30)
                                return ((t>=-1e-7)&(t<=1+1e-7)).all() and np.linalg.norm(xyz-a-t[:,None]*ab,axis=1).max()<1e-6
                            self.assertTrue(any(on_edge(a,b) for a,b in source_edges))
                            outline_length+=np.linalg.norm(np.diff(xyz,axis=0),axis=1).sum()
                self.assertLessEqual(vertex_count,8);self.assertLessEqual(byte_count,8192)
            self.assertAlmostEqual(area,84,places=5)
            self.assertAlmostEqual(outline_length,56,places=5)
            r=json.loads((out/'conversion.json').read_text());self.assertEqual(r['fragmentedPolygons'],1)
            self.assertIn('no internal',r['polygonFragmentPolicy'])

    def test_irreducible_metadata_fails_instead_of_violating_budget(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=pathlib.Path(tmp)/'huge.geojson'
            p.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',properties=dict(text='x'*8000),
                geometry=dict(type='Point',coordinates=[0,0]))])))
            with self.assertRaisesRegex(ValueError,'indivisible'):
                vector.run(types.SimpleNamespace(input=str(p),output=str(pathlib.Path(tmp)/'out'),max_features=1,max_bytes=4096))


class CliGeoPackageTests(unittest.TestCase):
    @requires_bin('set RUSTY_TILES_BIN to exercise native ingestion and archive publication')
    def test_cli_native_reader_and_atomic_failure(self):
        import zipfile
        with tempfile.TemporaryDirectory() as tmp:
            p=pathlib.Path(tmp)/'source.gpkg'
            gpkg(p,[('sites',3857,[(7,dict(type='Point',coordinates=[1,2]),2**60+3)])])
            binary=os.environ['RUSTY_TILES_BIN'];output=pathlib.Path(tmp)/'out.3tz'
            result=subprocess.run([binary,'vector','-i',str(p),'-o',str(output),'--layer','sites'],capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr.decode())
            with zipfile.ZipFile(output) as z:
                self.assertIn('@3dtilesIndex1@',z.namelist())
                report=json.loads(z.read('conversion.json'));self.assertEqual(report['inputDriver'],'GPKG')
            original=output.read_bytes()
            result=subprocess.run([binary,'vector','-i',str(p),'-o',str(output)],capture_output=True)
            self.assertNotEqual(result.returncode,0);self.assertEqual(output.read_bytes(),original)
            output.unlink()
            result=subprocess.run([binary,'vector','-i',str(p),'-o',str(output),'--layer','missing'],capture_output=True)
            self.assertNotEqual(result.returncode,0);self.assertFalse(output.exists())
            self.assertFalse(any(f.name.startswith('.tmp') for f in pathlib.Path(tmp).iterdir()))


if __name__=='__main__':unittest.main()
