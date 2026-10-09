"""Independent analytic T1 source, topology, refusal and publication acceptance."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from osgeo import gdal, osr
import t1_terrain_oracle as oracle
from cli_bin import BIN, requires_bin

gdal.UseExceptions()


@requires_bin('set RUSTY_TILES_BIN for native terrain acceptance')
class NativeTerrainTests(unittest.TestCase):
    def call(self, source, output, *extra, cells=16, offset=10.25, fill=-999.125):
        result = subprocess.run([BIN, '--json', 'terrain', '-i', str(source), '-o', str(output),
            '--cells-per-leaf', str(cells), '--height-offset', str(offset), '--fill-height', str(fill),
            *extra], capture_output=True, text=True, env=dict(os.environ, PATH=''), timeout=60)
        return result, json.loads(result.stdout)

    def test_complete_analytic_plane_compressed_tiled_and_shared_edges(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for index, options in enumerate([[], ['COMPRESS=DEFLATE'], ['COMPRESS=LZW'],
                                            ['TILED=YES', 'BLOCKXSIZE=16', 'BLOCKYSIZE=16', 'COMPRESS=DEFLATE']]):
                source = root / f'plane-{index}.tif'
                original = root / 'original.tif'
                oracle.write_fixture(original)
                dataset = gdal.Open(str(original))
                copy = gdal.GetDriverByName('GTiff').CreateCopy(str(source), dataset, options=options)
                copy = None; dataset = None
                output = root / f'out-{index}'
                result, summary = self.call(source, output)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(summary['terrainReport']['profile'], 't1-terrain-mesh-3dtiles-1.1')
                self.assertLessEqual(summary['terrainReport']['positionErrorMetres'], .05)
                self.assertEqual(oracle.check_plane(output), {'nodes': 25, 'triangles': 32})
                oracle.corruption_controls(output)
                inventory = [p for p in output.rglob('*') if p.is_file()]
                self.assertEqual(sum(p.stat().st_size for p in inventory), summary['terrainReport']['generatedBytes'])
            source = root / 'multipatch.tif'
            oracle.write_fixture(source, width=33, height=17, pixel=.01)
            output = root / 'multipatch'
            result, _ = self.call(source, output)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(oracle.check_directory(output)['leaf_tiles'], 6)
            self.assertEqual(oracle.check_plane(output, width=33, height=17, pixel=.01), {'nodes': 612, 'triangles': 1122})

    def test_native_float64_fraction_earns_its_reported_storage_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); source = root / 'fraction.tif'
            oracle.write_fixture(source, 'constant', width=1, height=1, pixel=.000001)
            dataset = gdal.Open(str(source), gdal.GA_Update)
            literal = 1000.123456789
            dataset.GetRasterBand(1).Fill(literal); dataset = None
            output = root / 'output'; result, summary = self.call(source, output, offset=0)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            manifest = json.loads((output/'tileset.json').read_text())
            _, primitives = oracle.glb(output/manifest['root']['children'][0]['content']['uri'])
            positions = primitives[0][0]
            truth = [oracle.ecef(lon, lat, literal) for lat in [21-.000001,21] for lon in [10,10+.000001]]
            errors = []
            for position, expected in zip(positions, truth):
                actual = oracle.transform(manifest['root']['transform'], (position[0],-position[2],position[1]))
                errors.append(sum((a-b)**2 for a,b in zip(actual,expected))**.5)
            self.assertLessEqual(max(errors), summary['terrainReport']['positionErrorMetres']+2e-8)
            self.assertGreater(max(errors), .00001)  # A hidden Float32 source cast cannot earn a near-zero receipt.

    def test_source_refusals_preserve_replacement_and_leave_no_work(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / 'output'; output.mkdir(); (output / 'old').write_bytes(b'old')
            source = root / 'source.tif'
            for case in ['all-invalid', 'nonfinite', 'extreme-height']:
                oracle.write_fixture(source, case)
                result, summary = self.call(source, output, '--force')
                self.assertNotEqual(result.returncode, 0, summary)
                self.assertEqual((output / 'old').read_bytes(), b'old')
                self.assertEqual(set(root.iterdir()), {source, output})
            for alteration in ['unit', 'scale', 'offset', 'mask', 'rotation', 'point', 'crs', 'overview', 'collapsed']:
                oracle.write_fixture(source)
                dataset = gdal.Open(str(source), gdal.GA_Update)
                band = dataset.GetRasterBand(1)
                if alteration == 'unit': band.SetUnitType('feet')
                if alteration == 'scale': band.SetScale(2)
                if alteration == 'offset': band.SetOffset(1)
                if alteration == 'mask': band.CreateMaskBand(gdal.GMF_PER_DATASET)
                if alteration == 'rotation': dataset.SetGeoTransform([10,.25,.001,21,0,-.25])
                if alteration == 'point': dataset.SetMetadataItem('AREA_OR_POINT', 'Point')
                if alteration == 'crs':
                    crs = osr.SpatialReference(); crs.ImportFromEPSG(3857); dataset.SetProjection(crs.ExportToWkt())
                if alteration == 'overview': dataset.BuildOverviews('NEAREST', [2])
                if alteration == 'collapsed': dataset.SetGeoTransform([0,1e-60,0,.004,0,-.001])
                band = None; dataset = None
                result, summary = self.call(source, output, '--force')
                self.assertNotEqual(result.returncode, 0, (alteration, summary))
                self.assertEqual((output / 'old').read_bytes(), b'old')
                for sibling in root.glob('source.tif.*'): sibling.unlink()
                self.assertEqual(set(root.iterdir()), {source, output})

    def test_source_nodata_is_filled_only_in_footprint(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); source = root / 'source.tif'
            oracle.write_fixture(source, 'nodata')
            output = root / 'output'; result, summary = self.call(source, output)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            # All decoded positions are checked independently against literal
            # source contributors; scalar NoData invalidates nonzero weights.
            report = summary['terrainReport']; manifest = json.loads((output/'tileset.json').read_text())
            observed = []
            for leaf in manifest['root']['children']:
                _, primitives = oracle.glb(output/leaf['content']['uri'])
                observed += [oracle.transform(manifest['root']['transform'], (p[0],-p[2],p[1]))
                             for positions, _ in primitives for p in positions]
            def expected(i,j):
                x=max(0,min(3,i-.5)); y=max(0,min(3,4-j-.5))
                contributors=[]
                import math
                for row, wy in [(math.floor(y),1-y%1),(min(3,math.floor(y)+1),y%1)]:
                    for col, wx in [(math.floor(x),1-x%1),(min(3,math.floor(x)+1),x%1)]:
                        if wx*wy: contributors.append((row,col,wx*wy))
                if any(row==1 and col==1 for row,col,_ in contributors): return -999.125
                return sum((100+2*col+3*row)*w for row,col,w in contributors)+10.25
            wanted = [oracle.ecef(10+i*.25,20+j*.25,expected(i,j)) for j in range(5) for i in range(5)]
            self.assertEqual(len(observed), len(wanted))
            for point, truth in zip(observed, wanted):
                self.assertLessEqual(sum((a-b)**2 for a,b in zip(point,truth))**.5, report['positionErrorMetres']+1e-7)
            self.assertEqual(report['heightRange'][0], -999.125)
            self.assertFalse((output/'layer.json').exists())
            self.assertFalse(list(output.rglob('*.terrain')))


if __name__ == '__main__':
    unittest.main()
