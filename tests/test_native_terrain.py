"""Independent analytic T1 source, topology, refusal and publication acceptance."""
import json
import math
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
                self.assertEqual(summary['terrainReport']['profile'], 't1-terrain-lod-mesh-3dtiles-1.1')
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
            oracle.corruption_controls(output, width=33, height=17, pixel=.01)

    def validate(self, output):
        archive = output.with_suffix('.3tz')
        for argv in (['convert', '-i', str(output), '-o', str(archive)], ['validate', str(archive)]):
            result = subprocess.run([BIN, '--json', *argv], capture_output=True, text=True,
                                    env=dict(os.environ, PATH=''), timeout=60)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)

    def test_lod_pyramid_errors_skirts_and_forest_validation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            # 5x3 leaves, four levels, clipped end strips: certified errors,
            # skirts and every pyramid corruption control.
            source = root / 'bump.tif'
            oracle.write_fixture(source, 'bump', width=67, height=45, pixel=.001)
            output = root / 'bump'
            result, summary = self.call(source, output)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            report = summary['terrainReport']
            self.assertEqual((report['levels'], report['tilesPerLevel']), (4, [1, 2, 6, 15]))
            checked = oracle.check_directory(output)
            # Absolute source truth as well as relative coarse-to-leaf errors:
            # bilinear pixel-centre samples of the literal nonplanar fixture.
            _, frame, nodes = oracle.tree(output)
            for node in (n for n in nodes if not n['children']):
                patch = oracle.Patch(output, node, report, frame['transform'])
                for local, (i, j), _ in patch.points:
                    x = max(0., min(66., i-.5))
                    y = max(0., min(44., 45-j-.5))
                    height = 0.
                    for row, wy in [(math.floor(y), 1-y%1), (min(44, math.floor(y)+1), y%1)]:
                        for col, wx in [(math.floor(x), 1-x%1), (min(66, math.floor(x)+1), x%1)]:
                            height += (300+40*math.sin(col/5)*math.cos(row/7)+.5*col)*wx*wy
                    expected = oracle.ecef(10+i*.001, 21-45*.001+j*.001, height+10.25)
                    actual = oracle.transform(frame['transform'], local)
                    self.assertLessEqual(math.dist(actual, expected), report['positionErrorMetres']+1e-7)
            self.assertLessEqual(report['positionErrorMetres'], .05)
            self.assertGreater(checked['certified']['1'], .1)
            self.assertLessEqual(report['skirtDepthMetres'][0], report['skirtDepthMetres'][1])
            oracle.corruption_controls(output, plane=False)
            checks = self.validate(output)['checks']
            for check in ('implicitAddressingAndAvailability', 'hierarchyBounds', 'geometricErrorOrder'):
                self.assertIn(check, checks)
            # 17x3 leaves, six levels: skeleton tiles above implicit roots.
            source = root / 'strip.tif'
            oracle.write_fixture(source, 'bump', width=260, height=40, pixel=.001)
            output = root / 'strip'
            result, summary = self.call(source, output)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            manifest = json.loads((output/'tileset.json').read_text())
            self.assertNotIn('implicitTiling', manifest['root'])
            self.assertEqual(oracle.check_directory(output, samples=1)['leaf_tiles'], 51)
            checks = self.validate(output)['checks']
            self.assertIn('implicitAddressingAndAvailability', checks)

    def test_native_float64_fraction_earns_its_reported_storage_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); source = root / 'fraction.tif'
            oracle.write_fixture(source, 'constant', width=1, height=1, pixel=.000001)
            dataset = gdal.Open(str(source), gdal.GA_Update)
            literal = 1000.123456789
            dataset.GetRasterBand(1).Fill(literal); dataset = None
            output = root / 'output'; result, summary = self.call(source, output, offset=0)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            _, root_tile, tiles = oracle.tree(output)
            _, primitives = oracle.glb(output/tiles[0]['uri'])
            positions = primitives[0][0]
            truth = [oracle.ecef(lon, lat, literal) for lat in [21-.000001,21] for lon in [10,10+.000001]]
            errors = []
            for position, expected in zip(positions, truth):
                actual = oracle.transform(root_tile['transform'], (position[0],-position[2],position[1]))
                errors.append(sum((a-b)**2 for a,b in zip(actual,expected))**.5)
            oracle.check_directory(output)
            # The leaf is quantized on a micrometre grid, finer than Float32 at
            # 1000 m, so a hidden Float32 source cast (about 1.7e-5 m here) would
            # exceed the receipt.
            self.assertIsNotNone(summary['terrainReport']['quantizationStepMetres'][-1])
            self.assertLess(summary['terrainReport']['positionErrorMetres'], 5e-6)
            self.assertLessEqual(max(errors), summary['terrainReport']['positionErrorMetres']+2e-8)

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
            report = summary['terrainReport']; _, root, tiles = oracle.tree(output)
            observed = []
            for leaf in tiles:
                _, primitives = oracle.glb(output/leaf['uri'])
                observed += [oracle.transform(root['transform'], (p[0],-p[2],p[1]))
                             for p in primitives[0][0]]
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
            oracle.check_directory(output)
            self.assertEqual(report['heightRange'][0], -999.125)
            self.assertFalse((output/'layer.json').exists())
            self.assertFalse(list(output.rglob('*.terrain')))


if __name__ == '__main__':
    unittest.main()
