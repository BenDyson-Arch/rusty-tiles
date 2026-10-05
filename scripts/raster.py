"""Source-preserving raster conversion with an explicit display recipe."""
import argparse
import json
import math
import pathlib
import subprocess
import sys
from osgeo import gdal

gdal.UseExceptions()

def run(a):
    out = pathlib.Path(a.output)
    source = gdal.Open(a.input)
    if source.GetSpatialRef() is None:
        raise ValueError('raster requires a declared CRS')
    if not 0 <= a.min_zoom <= a.max_zoom <= 24:
        raise ValueError('require 0 <= minZoom <= maxZoom <= 24')
    cog = out / 'source.cog.tif'
    gdal.Translate(str(cog), source, format='COG', creationOptions=['COMPRESS=DEFLATE'])
    info = gdal.Info(str(cog), format='json')
    ring = info['wgs84Extent']['coordinates'][0]
    bounds = [min(p[0] for p in ring), min(p[1] for p in ring), max(p[0] for p in ring), max(p[1] for p in ring)]
    if not (-180 <= bounds[0] < bounds[2] <= 180 and -85.05112878 <= bounds[1] < bounds[3] <= 85.05112878):
        raise ValueError('split antimeridian rasters or reproject polar coverage before imagery tiling')
    total = 0
    for z in range(a.min_zoom, a.max_zoom + 1):
        n = 2**z
        x0, x1 = [math.floor((lon+180)/360*n) for lon in (bounds[0], bounds[2])]
        y0, y1 = [math.floor((1-math.asinh(math.tan(math.radians(lat)))/math.pi)/2*n) for lat in (bounds[3], bounds[1])]
        total += (x1-x0+1)*(y1-y0+1)
    if total > 100000: raise ValueError('display would exceed 100000 tiles; reduce maximum zoom')
    display = out / 'display.tif'
    if a.display == 'gray':
        if a.alpha_band:
            raise ValueError('gray display uses the selected band mask/NoData; alphaBand is for image display')
        if not 1 <= a.band <= source.RasterCount:
            raise ValueError('selected band does not exist')
        if a.display_min is None or a.display_max is None or not (math.isfinite(a.display_min) and math.isfinite(a.display_max) and a.display_min < a.display_max):
            raise ValueError('numeric display requires a finite increasing range')
        band = source.GetRasterBand(a.band)
        if band.GetScale() not in (None, 1) or band.GetOffset() not in (None, 0):
            raise ValueError('scaled bands must be converted to actual values before styling')
        selected = gdal.Translate('', source, format='VRT', bandList=[a.band])
        colours = out / 'colors.txt'
        colours.write_text(f'{a.display_min} 0 0 0 255\n{a.display_max} 255 255 255 255\nnv 0 0 0 0\n')
        gdal.DEMProcessing(str(display), selected, 'color-relief', colorFilename=str(colours), addAlpha=True, creationOptions=['TILED=YES', 'COMPRESS=DEFLATE'])
        colours.unlink()
        # Color relief respects NoData but does not apply an explicit source mask.
        # Combine it into alpha in bounded windows before display resampling.
        mask = selected.GetRasterBand(1).GetMaskBand()
        styled = gdal.Open(str(display), gdal.GA_Update)
        alpha = styled.GetRasterBand(4)
        for y in range(0, styled.RasterYSize, 256):
            for x in range(0, styled.RasterXSize, 256):
                width = min(256, styled.RasterXSize-x)
                height = min(256, styled.RasterYSize-y)
                values = alpha.ReadAsArray(x, y, width, height)
                coverage = mask.ReadAsArray(x, y, width, height)
                masked = coverage < values
                values[masked] = coverage[masked]
                alpha.WriteArray(values, x, y)
        styled.FlushCache()
        alpha = None
        styled = None
    else:
        if a.display_min is not None or a.display_max is not None:
            raise ValueError('displayMin/displayMax require gray display')
        bands = [1, 2, 3] if source.RasterCount >= 3 else [1]
        if a.alpha_band:
            if not 1 <= a.alpha_band <= source.RasterCount:
                raise ValueError('alpha band does not exist')
            bands.append(a.alpha_band)
        if any(source.GetRasterBand(i).DataType != gdal.GDT_Byte for i in bands):
            raise ValueError('image display requires byte imagery; use gray with an explicit range for numeric bands')
        interpretations = ['red', 'green', 'blue'] if len(bands) - bool(a.alpha_band) == 3 else ['gray']
        if a.alpha_band:
            interpretations.append('alpha')
        # The source mask is kept separately: valid black pixels must not become NoData.
        vrt = gdal.Translate('', source, format='VRT', bandList=bands, outputType=gdal.GDT_Byte,
                             scaleParams=[[0, 255, 0, 255]], noData='none', maskBand='mask,1')
        for i, name in enumerate(interpretations, 1):
            vrt.GetRasterBand(i).SetColorInterpretation(gdal.GCI_GrayIndex if name == 'gray' else getattr(gdal, 'GCI_'+name.capitalize()+'Band'))
        gdal.Warp(str(display), vrt, format='GTiff', dstSRS='EPSG:3857', dstAlpha=not bool(a.alpha_band),
                  creationOptions=['TILED=YES', 'COMPRESS=DEFLATE'])
    # gdal2tiles is available in the supported Debian GDAL runtime as well as newer GDAL.
    if int(gdal.VersionInfo('VERSION_NUM')) >= 3110000:
        command = ['gdal', 'raster', 'tile', '--webviewer=none', '--tiling-scheme=WebMercatorQuad',
                   '--convention=xyz', '--output-format=PNG', f'--min-zoom={a.min_zoom}', f'--max-zoom={a.max_zoom}']
    else:
        command = [sys.executable, '-m', 'osgeo_utils.gdal2tiles', '--xyz', '--webviewer=none',
                   '--processes=1', '-z', f'{a.min_zoom}-{a.max_zoom}']
    subprocess.run(command + [str(display), str(out/'tiles')], check=True)
    display.unlink()
    (out/'tilejson.json').write_text(json.dumps(dict(tilejson='3.0.0', scheme='xyz', tiles=['tiles/{z}/{x}/{y}.png'],
        minzoom=a.min_zoom, maxzoom=a.max_zoom, bounds=bounds)))
    (out/'conversion.json').write_text(json.dumps(dict(display=a.display, band=a.band, displayMin=a.display_min,
        displayMax=a.display_max, alphaBand=a.alpha_band, sourceBands=source.RasterCount, gdalVersion=gdal.VersionInfo())))

if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('input'); p.add_argument('output')
    p.add_argument('--min-zoom', type=int, default=0); p.add_argument('--max-zoom', type=int, required=True)
    p.add_argument('--display', choices=['image', 'gray'], default='image')
    p.add_argument('--band', type=int, default=1); p.add_argument('--alpha-band', type=int, default=0)
    p.add_argument('--display-min', type=float); p.add_argument('--display-max', type=float)
    run(p.parse_args())
