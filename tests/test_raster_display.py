"""Finite R2 display bar: literal source values and an independent PNG decoder.

GDAL authors/reads the COG, but never supplies expected display pixels. The D1
stdlib PNG reader shares no decoder with GDAL or the converter's image crate.
"""
import json
import math
import pathlib
import os
import subprocess
import tempfile
import unittest

import numpy as np
from osgeo import gdal, osr

from cli_bin import BIN, requires_bin
from d1_raster_oracle import decode_png

gdal.UseExceptions()
osr.UseExceptions()


@requires_bin('set RUSTY_TILES_BIN for raster acceptance')
class RasterDisplayTests(unittest.TestCase):
    def call(self, *args, **kwargs):
        return subprocess.run(*args, env=dict(os.environ, PATH=''), **kwargs)

    def check_display(self, bands, expected, *, options=(), mask=None, nodata=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            source, output = root / 'literal.tif', root / 'output'
            dtype = gdal.GDT_Byte if bands.dtype == np.uint8 else gdal.GDT_Float32
            ds = gdal.GetDriverByName('GTiff').Create(str(source), 256, 256, len(bands), dtype)
            half = math.pi * 6378137
            step = 2 * half / (8 * 256)
            ds.SetGeoTransform([-half + 5 * 256 * step, step, 0,
                                half - 2 * 256 * step, 0, -step])
            srs = osr.SpatialReference()
            srs.ImportFromEPSG(3857)
            ds.SetProjection(srs.ExportToWkt())
            for i, values in enumerate(bands, 1):
                band = ds.GetRasterBand(i)
                band.WriteArray(values)
                if nodata is not None:
                    band.SetNoDataValue(nodata)
            if len(bands) == 4:
                ds.GetRasterBand(4).SetColorInterpretation(gdal.GCI_AlphaBand)
            if mask is not None:
                ds.GetRasterBand(1).CreateMaskBand(gdal.GMF_PER_DATASET)
                ds.GetRasterBand(1).GetMaskBand().WriteArray(mask)
            ds = None
            result = self.call([str(BIN), 'raster', '-i', str(source), '-o', str(output),
                                '--minZoom', '1', '--maxZoom', '3', '--json', *options],
                               capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(json.loads(result.stdout)['ok'])
            cog = gdal.Open(str(output / 'source.cog.tif'))
            for i, values in enumerate(bands, 1):
                np.testing.assert_array_equal(cog.GetRasterBand(i).ReadAsArray(), values)
                if nodata is not None:
                    self.assertEqual(cog.GetRasterBand(i).GetNoDataValue(), nodata)
                if mask is not None:
                    np.testing.assert_array_equal(cog.GetRasterBand(i).GetMaskBand().ReadAsArray(), mask)
                elif nodata is not None:
                    np.testing.assert_array_equal(cog.GetRasterBand(i).GetMaskBand().ReadAsArray(),
                                                  np.where(values == nodata, 0, 255).astype(np.uint8))
            cog = None
            manifest = json.loads((output / 'tilejson.json').read_text())
            self.assertEqual((manifest['minzoom'], manifest['maxzoom']), (1, 3))
            for actual, literal in zip(manifest['bounds'], [45, 40.97989806962013, 90, 66.51326044311186]):
                self.assertAlmostEqual(actual, literal, places=10)
            covered = {1: 0, 2: 0, 3: 0}
            padded = 0
            observed = set()
            for path in (output / 'tiles').rglob('*.png'):
                z, tx, ty = map(int, path.relative_to(output / 'tiles').with_suffix('').parts)
                channels, pixels = decode_png(path)
                k = 2 ** (3 - z)
                for py in range(256):
                    for px in range(256):
                        x = k * (tx * 256 + px) + k // 2 - 5 * 256
                        y = k * (ty * 256 + py) + k // 2 - 2 * 256
                        if 0 <= x < 256 and 0 <= y < 256:
                            want = expected(x, y)
                            covered[z] += 1
                            observed.add(want)
                        else:
                            want = (0, 0, 0, 0)
                            padded += 1
                        start = (py * 256 + px) * channels
                        actual = tuple(pixels[start:start + channels])
                        if channels == 3:
                            actual += (255,)
                        if actual != want:
                            self.fail(f'{path}: pixel {px},{py}, source {x},{y}: {actual} != {want}')
            # Full census prevents a missing tile or an empty iterator passing.
            self.assertEqual(covered, {1: 4096, 2: 16384, 3: 65536})
            self.assertGreater(padded, 0)
            return observed

    def test_literal_constant_and_rgb_ramp(self):
        for color in [(0, 0, 0), (23, 101, 239)]:
            with self.subTest(color=color):
                bands = np.empty((3, 256, 256), dtype=np.uint8)
                bands[:] = np.array(color)[:, None, None]
                self.check_display(bands, lambda x, y: (*color, 255))
        y, x = np.indices((256, 256))
        bands = np.array([x, y, (3 * x + 5 * y) % 256], dtype=np.uint8)
        self.check_display(bands, lambda x, y: (x, y, (3 * x + 5 * y) % 256, 255))

    def test_literal_impulses_distinguish_finest_and_coarse_phase(self):
        bands = np.zeros((3, 256, 256), dtype=np.uint8)
        impulses = {(128, 128): (255, 0, 0, 255),
                    (129, 129): (0, 255, 0, 255),
                    (130, 130): (0, 0, 255, 255)}
        for (x, y), rgba in impulses.items():
            bands[:, y, x] = rgba[:3]
        seen = self.check_display(bands, lambda x, y: impulses.get((x, y), (0, 0, 0, 255)))
        self.assertTrue(set(impulses.values()) <= seen)

    def test_selected_numeric_gray_literal_clipping_and_rounding(self):
        raw = [-20, -10, -5, 0, 5, 10, 20, -999]
        display = [0, 0, 64, 128, 191, 255, 255, None]
        bands = np.full((2, 256, 256), 123, dtype=np.float32)
        bands[1] = np.tile(np.repeat(raw, 32), (256, 1))
        def expected(x, y):
            value = display[x // 32]
            return (0, 0, 0, 0) if value is None else (value, value, value, 255)
        self.check_display(bands, expected, nodata=-999,
                           options=('--display', 'gray', '--band', '2',
                                    '--displayMin', '-10', '--displayMax', '10'))

    def test_literal_nodata_mask_and_partial_alpha_intersection(self):
        bands = np.zeros((4, 256, 256), dtype=np.uint8)
        bands[3] = 255
        bands[:3, :32, :32] = 99  # NoData, despite opaque alpha and a valid mask.
        bands[:3, 64:96, 64:96] = np.array([23, 101, 239])[:, None, None]
        bands[3, 64:96, 64:96] = 128
        bands[3, 128:160, 128:160] = 0
        bands[3, 192:224, 192:224] = 128
        mask = np.full((256, 256), 255, dtype=np.uint8)
        mask[192:224, 192:224] = 0  # Boolean mask overrides partial alpha.
        def expected(x, y):
            if ((x < 32 and y < 32) or (128 <= x < 160 and 128 <= y < 160)
                    or (192 <= x < 224 and 192 <= y < 224)):
                return (0, 0, 0, 0)
            if 64 <= x < 96 and 64 <= y < 96:
                return (23, 101, 239, 128)
            return (0, 0, 0, 255)  # Valid black is opaque.
        seen = self.check_display(bands, expected, mask=mask, nodata=99,
                                  options=('--alphaBand', '4'))
        self.assertTrue({(0, 0, 0, 255), (0, 0, 0, 0), (23, 101, 239, 128)} <= seen)


if __name__ == '__main__':
    unittest.main()
