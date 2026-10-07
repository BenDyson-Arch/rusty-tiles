"""Repeated conversions compare every byte or only the documented volatile section."""
import json
import os
import pathlib
import subprocess
import tempfile
import unittest
import zipfile
import laspy
import numpy as np

from cli_bin import BIN, requires_bin

@requires_bin('set RUSTY_TILES_BIN for reproducibility acceptance')
class ReproducibilityTests(unittest.TestCase):
    def call(self,*args):
        result=subprocess.run([BIN,*args],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr)

    def test_vector_exact_archives_and_diagnostic_exception(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'source.geojson'
            source.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id=i,
                properties=dict(name=f'line-{i}'),geometry=dict(type='LineString',coordinates=[
                    [i*100+j,float(j%3)*.01,i*.0001] for j in range(30)])) for i in range(6)])))
            common=['vector','-i',str(source),'--sourceCrs','local','--maxFeatures','1','--quantize','--meshopt']
            outputs=[]
            for index,jobs in enumerate((1,2,2)):
                out=root/f'exact-{index}.3tz';outputs.append(out)
                self.call(*common,'-o',str(out),'--jobs',str(jobs),'--reproducible')
                self.call('validate',str(out))
            self.assertEqual(outputs[0].read_bytes(),outputs[1].read_bytes())
            self.assertEqual(outputs[1].read_bytes(),outputs[2].read_bytes())
            ordinary=[]
            for index in range(2):
                out=root/f'diagnostics-{index}.3tz';ordinary.append(out)
                self.call(*common,'-o',str(out),'--jobs','2')
            with zipfile.ZipFile(ordinary[0]) as a,zipfile.ZipFile(ordinary[1]) as b:
                self.assertEqual(a.namelist(),b.namelist())
                for name in a.namelist():
                    if name=='@3dtilesIndex1@':continue # Offsets change with performance JSON size.
                    if name=='conversion.json':
                        left=json.loads(a.read(name));right=json.loads(b.read(name))
                        self.assertIn('performance',left);self.assertIn('performance',right)
                        left.pop('performance');right.pop('performance');self.assertEqual(left,right)
                    else:self.assertEqual(a.read(name),b.read(name),name)
                for entry in a.infolist():self.assertEqual(entry.date_time,(1980,1,1,0,0,0))

    def test_point_cloud_and_pack_are_byte_identical(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'points.las'
            cloud=laspy.LasData(laspy.LasHeader(point_format=3,version='1.2'))
            cloud.x=np.linspace(0,10,40);cloud.y=np.sin(np.linspace(0,5,40));cloud.z=np.zeros(40)
            cloud.write(source);outputs=[]
            for i in range(2):
                out=root/f'points-{i}.3tz';outputs.append(out)
                self.call('point-cloud','-i',str(source),'-o',str(out),'--sourceCrs','local','--maxPoints','10')
            self.assertEqual(outputs[0].read_bytes(),outputs[1].read_bytes())
            folder=root/'tiles';folder.mkdir()
            with zipfile.ZipFile(outputs[0]) as archive:
                for name in archive.namelist():
                    if name=='@3dtilesIndex1@':continue
                    path=folder/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(archive.read(name))
            packed=[]
            for index in range(2):
                for path in folder.rglob('*'):
                    if path.is_file():os.utime(path,(100000+index*1000,100000+index*1000))
                out=root/f'packed-{index}.3tz';packed.append(out);self.call('convert','-i',str(folder),'-o',str(out))
            self.assertEqual(packed[0].read_bytes(),packed[1].read_bytes())
