"""Validate published archives and reject independently repacked corruption."""
import json
import struct
import laspy
import numpy as np
import os
import pathlib
import subprocess
import tempfile
import unittest
import zipfile

BIN=os.environ.get('RUSTY_TILES_BIN')

@unittest.skipUnless(BIN,'set RUSTY_TILES_BIN for archive acceptance')
class ArchiveValidationTests(unittest.TestCase):
    def call(self,*args):
        return subprocess.run([BIN,*args],capture_output=True,text=True)

    def fixture(self,root):
        source=root/'source.geojson';out=root/'valid.3tz'
        source.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id=i,
            properties=dict(name=f'line-{i}'),geometry=dict(type='LineString',coordinates=[
                [i*100+j,float(j%3)*.01,i*.0001] for j in range(30)])) for i in range(4)])))
        result=self.call('vector','-i',str(source),'-o',str(out),'--sourceCrs','local','--maxFeatures','1','--jobs','2','--quantize','--meshopt')
        self.assertEqual(result.returncode,0,result.stderr)
        return out

    def repack(self,root,original,name,mutate):
        directory=root/name;directory.mkdir()
        with zipfile.ZipFile(original) as archive:
            for entry in archive.namelist():
                if entry=='@3dtilesIndex1@':continue
                path=directory/entry;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(archive.read(entry))
        mutate(directory)
        out=root/(name+'.3tz');result=self.call('convert','-i',str(directory),'-o',str(out))
        self.assertEqual(result.returncode,0,result.stderr);return out

    def change_json(self,directory,name,fn):
        path=directory/name;value=json.loads(path.read_text());fn(value);path.write_text(json.dumps(value))

    def test_valid_archive_and_corruption_categories(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);original=self.fixture(root);before=original.read_bytes()
            result=self.call('validate',str(original),'--json');self.assertEqual(result.returncode,0,result.stderr)
            report=json.loads(result.stdout);self.assertTrue(report['ok']);self.assertGreater(report['tiles'],4)
            self.assertEqual(before,original.read_bytes())
            def corrupt_payload(d):
                p=next((d/'t').glob('*.glb'));data=bytearray(p.read_bytes());data[-1]^=1;p.write_bytes(data)
            cases=[('hash',corrupt_payload,'checksum'),
                ('missing',lambda d:next((d/'t').glob('*.glb')).unlink(),'missing archive'),
                ('unused',lambda d:(d/'unrelated.txt').write_text('unreferenced'),'unreferenced'),
                ('bytes',lambda d:self.change_json(d,'conversion.json',lambda v:v['budgets'].update(bytes=1)),'byte budget'),
                ('vertices',lambda d:self.change_json(d,'conversion.json',lambda v:v['budgets'].update(vertices=1)),'vertex budget'),
                ('error',lambda d:self.change_json(d,'tileset.json',lambda v:v['root'].update(geometricError=v['geometricError']*2)),'geometricError'),
                ('bounds',lambda d:self.change_json(d,'tileset.json',lambda v:v['root']['boundingVolume'].update(box=[0,0,0,.001,0,0,0,.001,0,0,0,.001])),'bounds escape'),
                ('schema',lambda d:self.change_json(d,'tileset.json',lambda v:v['asset'].update(version='2.0')),'asset.version'),
                ('state',lambda d:(d/'vector-build.json').write_text('{}'),'build-state checksum')]
            for name,mutate,message in cases:
                with self.subTest(name=name):
                    out=self.repack(root,original,name,mutate);result=self.call('validate',str(out),'--json')
                    self.assertNotEqual(result.returncode,0,result.stdout)
                    self.assertIn(message,json.loads(result.stdout)['error']['message'])

    def test_point_archive_and_feature_attributes_use_valid_core_types(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'thin.las';out=root/'points.3tz'
            header=laspy.LasHeader(point_format=3,version='1.2');header.scales=[1e-7]*3
            cloud=laspy.LasData(header);cloud.x=np.linspace(0,10,60);cloud.y=np.linspace(0,1e-6,60);cloud.z=np.zeros(60)
            cloud.write(source)
            result=self.call('point-cloud','-i',str(source),'-o',str(out),'--sourceCrs','local','--maxPoints','10')
            self.assertEqual(result.returncode,0,result.stderr)
            result=self.call('validate',str(out),'--json');self.assertEqual(result.returncode,0,result.stdout)
            for archive_path in (out,self.fixture(root)):
                with zipfile.ZipFile(archive_path) as archive:
                    for name in archive.namelist():
                        if not name.endswith('.glb'):continue
                        data=archive.read(name);length=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+length])
                        metadata=doc['extensions']['EXT_structural_metadata']
                        self.assertRegex(metadata['schema']['id'],r'^[a-zA-Z_][a-zA-Z0-9_]*$')
                        for mesh in doc['meshes']:
                            for primitive in mesh['primitives']:
                                accessor=doc['accessors'][primitive['attributes']['_FEATURE_ID_0']]
                                self.assertIn(accessor['componentType'],(5123,5126))

    def test_directories_and_non_archives_get_actionable_errors(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);text=root/'notes.3tz';text.write_text('not an archive')
            for path,message in [(root,'is a directory'),(text,'is not a ZIP/.3tz archive')]:
                with self.subTest(path=path):
                    result=self.call('validate',str(path),'--json')
                    self.assertEqual(result.returncode,3,result.stdout)
                    error=json.loads(result.stdout)['error']
                    self.assertEqual(error['code'],'data');self.assertIn(message,error['message'])
                    self.assertIn('raster and terrain output directories are not validated yet',error['message'])
                    self.assertNotIn('os error',error['message'])

    def test_index_and_external_validator_failures(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);original=self.fixture(root)
            bad=root/'bad-index.3tz'
            with zipfile.ZipFile(original) as source,zipfile.ZipFile(bad,'w',compression=zipfile.ZIP_STORED) as target:
                for name in source.namelist():
                    data=source.read(name)
                    if name=='@3dtilesIndex1@':data=data[:-1]
                    target.writestr(name,data)
            result=self.call('validate',str(bad),'--json');self.assertNotEqual(result.returncode,0)
            self.assertIn('index',json.loads(result.stdout)['error']['message'])
            result=self.call('validate',str(original),'--json','--external-validator',str(root/'absent'))
            self.assertEqual(result.returncode,4);self.assertEqual(json.loads(result.stdout)['error']['code'],'environment')
            executable=root/'validator';executable.write_text("#!/usr/bin/env python3\nimport sys,json\nassert sys.argv[1]=='--tilesetFile'\njson.dump({'issues':[{'severity':'ERROR','message':'fixture'}]},open(sys.argv[4],'w'))\n")
            executable.chmod(0o755)
            result=self.call('validate',str(original),'--json','--external-validator',str(executable))
            self.assertNotEqual(result.returncode,0);self.assertIn('external validation',json.loads(result.stdout)['error']['message'])
