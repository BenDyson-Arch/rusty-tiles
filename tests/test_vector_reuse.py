"""Unchanged encoders stay idle; edits invalidate affected partitions and dependencies."""
import copy
import json
import os
import subprocess
import pathlib
import struct
import tempfile
import types
import unittest
import zipfile

import numpy as np
from osgeo import ogr
from test_vector_gpkg import gpkg
from test_vector_lod import vector, read, parts


def archive(directory,path):
    with zipfile.ZipFile(path,'w') as z:
        for f in pathlib.Path(directory).rglob('*'):
            if f.is_file():z.write(f,f.relative_to(directory).as_posix())


def report(path):return json.loads((pathlib.Path(path)/'conversion.json').read_text())


def payloads(path):
    return {p.name:p.read_bytes() for p in (pathlib.Path(path)/'t').iterdir()}


def details(path):
    """Independent world-coordinate decode of full-detail content and properties."""
    root=json.loads((pathlib.Path(path)/'tileset.json').read_text())['root'];result={}
    def walk(node,parent):
        m=np.array(node.get('transform',np.eye(4).T.flatten())).reshape(4,4).T;world=parent@m
        if not node.get('children'):
            for content in node.get('contents',[node['content']] if 'content' in node else []):
                file=pathlib.Path(path)/content['uri'];doc,_,ids=read(file)
                data=file.read_bytes()
                if data[:4]==b'b3dm':data=data[28+sum(struct.unpack_from('<4s6I',data)[3:]):]
                n=struct.unpack_from('<I',data,12)[0];binary=data[28+n:]
                meta=doc['extensions']['EXT_structural_metadata'];table=meta['propertyTables'][0];schema=meta['schema']['classes']['feature']['properties']
                props=[{} for _ in ids]
                def view(i,dtype):
                    v=doc['bufferViews'][i];return np.frombuffer(binary[v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']],dtype)
                for name,col in table['properties'].items():
                    sc=schema[name]
                    if sc['type']=='STRING':
                        text=view(col['values'],'u1').tobytes();off=view(col['stringOffsets'],'<u4');values=[text[a:b].decode() for a,b in zip(off[:-1],off[1:])]
                    elif sc['type']=='BOOLEAN':values=np.unpackbits(view(col['values'],'u1'),bitorder='little')[:len(ids)].astype(bool).tolist()
                    else:values=view(col['values'],{'INT64':'<i8','FLOAT64':'<f8'}[sc['componentType']]).tolist()
                    for p,value in zip(props,values):p[name]=None if value==sc.get('noData','not a numeric sentinel') else value
                for mode,fid,pos in parts(file):
                    prop=props[fid]
                    xyz=pos[:,[0,2,1]]*[1,-1,1];xyz=(np.c_[xyz,np.ones(len(xyz))]@world.T)[:,:3]
                    key=(prop['_source_layer'],prop['_source_id'],mode)
                    result.setdefault(key,[]).append((prop,xyz))
        for child in node.get('children',[]):walk(child,world)
    walk(root,np.eye(4));return result


class ReuseTests(unittest.TestCase):
    def make_source(self,tmp,count=16,spatial_index=True):
        p=pathlib.Path(tmp)/'roads.gpkg'
        features=[(i+1,dict(type='LineString',coordinates=[[i*100+j,float(np.sin(j)*.1),0.] for j in range(61)]),2**60+3) for i in range(count)]
        gpkg(p,[('roads',3857,features)],spatial_index=spatial_index);return p

    def args(self,p,out,previous=None,**kwargs):
        return types.SimpleNamespace(input=str(p),output=str(out),source_crs='local',max_features=1,max_parent_features=1,
            max_vertices=128,max_bytes=16384,lod_tolerance=.2,lod_levels=3,reuse_tileset=str(previous) if previous else None,**kwargs)

    def assert_same_details(self,a,b):
        a,b=details(a),details(b);self.assertEqual(set(a),set(b))
        for key in a:
            self.assertEqual(len(a[key]),len(b[key]))
            for (pa,xa),(pb,xb) in zip(a[key],b[key]):
                self.assertEqual(pa,pb);np.testing.assert_allclose(xa,xb,atol=2e-5,rtol=0)

    def test_unchanged_input_never_invokes_encoder_and_content_is_identical(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=self.make_source(tmp);a=pathlib.Path(tmp)/'a';b=pathlib.Path(tmp)/'b';previous=pathlib.Path(tmp)/'previous.3tz'
            vector.run(self.args(p,a));archive(a,previous)
            vector.run(self.args(p,b,previous))
            self.assertEqual(payloads(a),payloads(b));self.assert_same_details(a,b)
            for key in ('primitiveReferences','maximumTilePrimitives'):
                self.assertEqual(report(a)['encoding'][key],report(b)['encoding'][key])
            r=report(b)['reuse'];self.assertEqual(r['rebuiltContents'],0);self.assertGreater(r['reusedContents'],0)
            for name,data in payloads(b).items():
                import hashlib
                self.assertEqual(pathlib.Path(name).stem,hashlib.sha256(data).hexdigest())

    def test_attribute_edit_rebuilds_one_leaf_and_reuses_other_subtrees(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=self.make_source(tmp);a=pathlib.Path(tmp)/'a';b=pathlib.Path(tmp)/'b';fresh=pathlib.Path(tmp)/'fresh';previous=pathlib.Path(tmp)/'previous.3tz'
            vector.run(self.args(p,a));archive(a,previous);old=previous.read_bytes()
            ds=ogr.Open(str(p),1);layer=ds.GetLayer(0);f=layer.GetFeature(7);f.SetField('name','edited');layer.SetFeature(f);ds=None
            vector.run(self.args(p,b,previous));vector.run(self.args(p,fresh))
            self.assertEqual(previous.read_bytes(),old);self.assert_same_details(b,fresh)
            r=report(b)['reuse'];self.assertGreaterEqual(r['reusedSubtrees'],3);self.assertLessEqual(r['rebuiltContents'],4)
            common=set(payloads(a))&set(payloads(b));self.assertGreater(len(common),len(payloads(a))*.8)
            self.assertTrue(all(payloads(a)[k]==payloads(b)[k] for k in common))

    def test_insert_move_delete_and_deleted_anchor_match_fresh_world_geometry(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=self.make_source(tmp,8);a=pathlib.Path(tmp)/'a';b=pathlib.Path(tmp)/'b';fresh=pathlib.Path(tmp)/'fresh';previous=pathlib.Path(tmp)/'previous.3tz'
            vector.run(self.args(p,a));archive(a,previous)
            ds=ogr.Open(str(p),1);layer=ds.GetLayer(0);layer.DeleteFeature(1);layer.DeleteFeature(6)
            f=layer.GetFeature(3);f.SetGeometry(ogr.CreateGeometryFromJson(json.dumps(dict(type='LineString',coordinates=[[1200,0,0],[1210,1,0],[1220,0,0]]))));layer.SetFeature(f)
            f=ogr.Feature(layer.GetLayerDefn());f.SetFID(22);f.SetField('name','inserted');f.SetField('large',None);f.SetGeometry(ogr.CreateGeometryFromJson(json.dumps(dict(type='Point',coordinates=[-100,0,0]))));layer.CreateFeature(f);ds=None
            vector.run(self.args(p,b,previous));vector.run(self.args(p,fresh));self.assert_same_details(b,fresh)
            r=report(b)['reuse'];self.assertGreater(r['reusedContents'],0)
            self.assertNotIn(('roads','1',3),details(b));self.assertIn(('roads','22',0),details(b))

    def test_shared_vertex_change_invalidates_unchanged_neighbor(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=pathlib.Path(tmp)/'shared.gpkg'
            gpkg(p,[('roads',3857,[(1,dict(type='LineString',coordinates=[[0,0],[1,.1],[2,0]]),1),
                (2,dict(type='LineString',coordinates=[[1,.1],[1,2]]),2),
                (3,dict(type='LineString',coordinates=[[100,0],[101,0]]),3)])])
            a=pathlib.Path(tmp)/'a';b=pathlib.Path(tmp)/'b';fresh=pathlib.Path(tmp)/'fresh';previous=pathlib.Path(tmp)/'previous.3tz'
            vector.run(self.args(p,a));archive(a,previous)
            ds=ogr.Open(str(p),1);layer=ds.GetLayer(0);f=layer.GetFeature(2);f.SetGeometry(ogr.CreateGeometryFromJson(json.dumps(dict(type='LineString',coordinates=[[1,1],[1,2]]))));layer.SetFeature(f);ds=None
            vector.run(self.args(p,b,previous));vector.run(self.args(p,fresh));self.assert_same_details(b,fresh)
            def all_content_ids(path):
                result={}
                for f in (path/'t').iterdir():
                    _,pos,ids=read(f);result.setdefault(ids[0],[]).append(sum(map(len,pos)))
                return result
            self.assertEqual(all_content_ids(a)['1'],[3])
            self.assertEqual(sorted(all_content_ids(b)['1']),[2,3])
            self.assertGreater(report(b)['reuse']['reusedContents'],0)

    def test_settings_changes_and_corrupt_previous_content_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=self.make_source(tmp,2);a=pathlib.Path(tmp)/'a';previous=pathlib.Path(tmp)/'previous.3tz';vector.run(self.args(p,a));archive(a,previous)
            args=self.args(p,pathlib.Path(tmp)/'bad-settings',previous);args.max_vertices=129
            with self.assertRaisesRegex(ValueError,'settings differ'):vector.run(args)
            bad=pathlib.Path(tmp)/'corrupt.3tz'
            with zipfile.ZipFile(previous) as src,zipfile.ZipFile(bad,'w') as dst:
                changed=False
                for name in src.namelist():
                    data=src.read(name)
                    if name.startswith('t/') and not changed:data=data[:-1]+bytes([data[-1]^1]);changed=True
                    dst.writestr(name,data)
            with self.assertRaisesRegex(ValueError,'checksum failed'):
                vector.run(self.args(p,pathlib.Path(tmp)/'bad-content',bad))

    def test_mixed_polygon_fill_and_outline_contents_are_reused(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=pathlib.Path(tmp)/'parcels.gpkg'
            rings=[[[0,0,0],[10,0,0],[10,10,0],[0,10,0],[0,0,0]],
                   [[3,3,0],[3,7,0],[7,7,0],[7,3,0],[3,3,0]]]
            gpkg(p,[('land',3857,[(1,dict(type='Polygon',coordinates=rings),None)])])
            a=pathlib.Path(tmp)/'a';b=pathlib.Path(tmp)/'b';previous=pathlib.Path(tmp)/'previous.3tz'
            args=self.args(p,a);args.max_vertices=8;args.max_bytes=8192
            vector.run(args);archive(a,previous)
            args.output=str(b);args.reuse_tileset=str(previous)
            vector.run(args)
            self.assertEqual(payloads(a),payloads(b));self.assert_same_details(a,b)
            self.assertTrue(any(name.endswith('.b3dm') for name in payloads(b)))
            self.assertEqual(report(b)['reuse']['rebuiltContents'],0)

    def test_delete_all_publishes_empty_tileset(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=self.make_source(tmp,2);a=pathlib.Path(tmp)/'a';b=pathlib.Path(tmp)/'b';previous=pathlib.Path(tmp)/'previous.3tz'
            vector.run(self.args(p,a));archive(a,previous)
            ds=ogr.Open(str(p),1);layer=ds.GetLayer(0);layer.DeleteFeature(1);layer.DeleteFeature(2);ds=None
            vector.run(self.args(p,b,previous));self.assertEqual(payloads(b),{});self.assertEqual(report(b)['features'],0)
            self.assertEqual(details(b),{})


class CliReuseTests(unittest.TestCase):
    @unittest.skipUnless(os.environ.get('RUSTY_TILES_BIN'),'set RUSTY_TILES_BIN for native reuse acceptance')
    def test_cli_reuses_archive_and_failed_update_does_not_publish(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);p=ReuseTests().make_source(tmp,8);baseline=root/'baseline.3tz';updated=root/'updated.3tz'
            binary=os.environ['RUSTY_TILES_BIN'];command=[binary,'vector','-i',str(p),'--sourceCrs','local','--maxFeatures','1','--maxVertices','128','--maxBytes','16384']
            result=subprocess.run(command+['-o',str(baseline)],capture_output=True);self.assertEqual(result.returncode,0,result.stderr.decode())
            original=baseline.read_bytes()
            result=subprocess.run(command+['-o',str(updated),'--reuseTileset',str(baseline)],capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr.decode());self.assertEqual(baseline.read_bytes(),original)
            with zipfile.ZipFile(updated) as z:
                r=json.loads(z.read('conversion.json'))['reuse'];self.assertEqual(r['rebuiltContents'],0);self.assertGreater(r['reusedContents'],0)
            updated.unlink()
            result=subprocess.run(command+['-o',str(updated),'--reuseTileset',str(baseline),'--lodLevels','2'],capture_output=True)
            self.assertNotEqual(result.returncode,0);self.assertFalse(updated.exists());self.assertEqual(baseline.read_bytes(),original)


if __name__=='__main__':unittest.main()
