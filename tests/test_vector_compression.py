"""Position quantization has a measured bound; compression budgets final bytes."""
import json
import os
import pathlib
import struct
import subprocess
import tempfile
import unittest
import zipfile
import numpy as np
from test_vector_lod import vector, read

BIN=os.environ.get('RUSTY_TILES_BIN')

class VectorCompressionTests(unittest.TestCase):
    def test_quantized_positions_decode_within_reported_error_and_keep_metadata(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp)
            source=np.column_stack([np.linspace(0,100,3000),np.sin(np.linspace(0,20,3000)),np.cos(np.linspace(0,20,3000))])
            items=[dict(properties=dict(_source_id='1',_source_layer='lines',name='example',value=2**60+3),
                geometry=dict(type='LineString',coordinates=source.tolist()))]
            baseline=root/'baseline.glb';out=root/'quantized.glb';report={}
            vector.emit(items,baseline,lambda p:np.asarray(p))
            vector.emit(items,out,lambda p:np.asarray(p),encoding_report=report,quantize=True)
            data=out.read_bytes();n=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+n]);binary=data[28+n:]
            a=doc['accessors'][doc['meshes'][0]['primitives'][0]['attributes']['POSITION']]
            self.assertEqual(a['componentType'],5123);self.assertTrue(a['normalized'])
            view=doc['bufferViews'][a['bufferView']]
            raw=np.frombuffer(binary,dtype='<u2',count=a['count']*4,offset=view['byteOffset']).reshape(-1,4)[:,:3]
            decoded=raw.astype(float)/65535*np.array(doc['nodes'][0]['scale'])+doc['nodes'][0]['translation']
            self.assertLessEqual(np.linalg.norm(decoded-source,axis=1).max(),report['rounding']+report['quantizationError'])
            self.assertLess(out.stat().st_size,baseline.stat().st_size)
            self.assertIn('KHR_mesh_quantization',doc['extensionsRequired'])
            self.assertEqual(doc['extensions']['EXT_structural_metadata'],read(baseline)[0]['extensions']['EXT_structural_metadata'])

    @unittest.skipUnless(BIN,'set RUSTY_TILES_BIN for codec CLI acceptance')
    def test_cli_combinations_report_encoded_sizes_and_respect_budgets(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);source=root/'lines.geojson'
            coords=[[i*.01,float(np.sin(i*.01)),0] for i in range(2000)]
            source.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id=1,
                properties=dict(name='line'),geometry=dict(type='LineString',coordinates=coords))])))
            for index,flags in enumerate([[],['--quantize'],['--meshopt'],['--quantize','--meshopt']]):
                out=root/f'out-{index}.3tz'
                result=subprocess.run([BIN,'vector','-i',str(source),'-o',str(out),'--sourceCrs','local',
                    '--maxBytes','32768',*flags],capture_output=True,text=True)
                self.assertEqual(result.returncode,0,result.stderr)
                with zipfile.ZipFile(out) as archive:
                    report=json.loads(archive.read('conversion.json'))
                    self.assertEqual(report['encoding']['quantize'],'--quantize' in flags)
                    self.assertEqual(report['encoding']['meshopt'],'--meshopt' in flags)
                    self.assertLessEqual(report['maximumTileBytes'],32768)
                    manifest=json.loads(archive.read('tileset.json'))
                    def check(node):
                        if 'content' in node:
                            data=archive.read(node['content']['uri']);self.assertEqual(len(data),node['extras']['encodedBytes'])
                            n=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+n])
                            if '--meshopt' in flags:self.assertIn('EXT_meshopt_compression',doc['extensionsRequired'])
                        for child in node.get('children',[]):check(child)
                    check(manifest['root'])
