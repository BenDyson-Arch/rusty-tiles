#!/usr/bin/env python3
"""Explicitly download public validation data outside the repository.
Usage: python public_data.py roads|autzen /path/to/cache
GDAL is required for roads; laspy[lazrs], NumPy and pyproj for Autzen.
"""
import argparse
import hashlib
import json
import pathlib
import urllib.request

SOURCES={
 'roads':dict(url='https://naciscdn.org/naturalearth/10m/cultural/ne_10m_roads.zip',
              filename='ne_10m_roads.zip',license='Public domain',
              attribution='Natural Earth',licenseUrl='https://www.naturalearthdata.com/about/terms-of-use/'),
 'autzen':dict(url='https://media.githubusercontent.com/media/PDAL/data/main/autzen/autzen-classified.laz',
              filename='autzen-classified.laz',license='CC BY 4.0',
              attribution='Aaron Reyna / Watershed Sciences (2010); classifications Max Sampson / Hobu (2021)',
              licenseUrl='https://github.com/PDAL/data/blob/main/LICENSE',
              provenanceUrl='https://github.com/PDAL/data/blob/main/autzen/README.md')}


def sha256(path):
    digest=hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda:stream.read(1048576),b''):digest.update(block)
    return digest.hexdigest()


def prepare(dataset, cache):
    cache.mkdir(parents=True,exist_ok=True)
    source=SOURCES[dataset];download=cache/source['filename']
    if not download.exists():
        temporary=download.with_suffix(download.suffix+'.part')
        urllib.request.urlretrieve(source['url'],temporary)
        temporary.replace(download)
    report=dict(**source,downloadSha256=sha256(download))
    if dataset=='roads':
        from osgeo import gdal
        output=cache/'natural-earth-roads.gpkg'
        if not output.exists():
            ds=gdal.VectorTranslate(str(output),'/vsizip/'+str(download.resolve())+'/ne_10m_roads.shp',
                format='GPKG',layerName='roads',geometryType='MULTILINESTRING')
            if ds is None:raise RuntimeError('GeoPackage conversion failed')
            ds=None
        report.update(prepared=str(output),preparation='OGR Shapefile to GeoPackage, multi-line layer roads; source CRS and scalar fields retained')
    else:
        import laspy
        import numpy as np
        output=cache/'autzen-local-metres.las'
        if not output.exists():
            with laspy.open(download) as reader:
                crs=reader.header.parse_crs()
                if crs is None or [s.to_epsg() for s in crs.sub_crs_list]!=[2992,6360]:
                    raise ValueError('unexpected Autzen CRS; inspect units before preparing local metres')
                header=reader.header.copy();factor=np.array([.3048,.3048,1200/3937])
                header.scales*=factor;header.offsets*=factor
                header.vlrs=[v for v in header.vlrs if v.user_id!='LASF_Projection']
                if header.evlrs is not None:
                    header.evlrs=[v for v in header.evlrs if v.user_id!='LASF_Projection']
                with laspy.open(output,mode='w',header=header) as writer:
                    for points in reader.chunk_iterator(100000):
                        writer.write_points(laspy.ScaleAwarePointRecord(points.array,header.point_format,header.scales,header.offsets))
        report.update(prepared=str(output),preparation='Horizontal international feet and vertical US survey feet converted to local metre XYZ by header scales/offsets; raw point records unchanged; CRS VLRs removed. No vertical datum transformation or ellipsoidal placement.')
    report['preparedSha256']=sha256(output)
    (cache/f'{dataset}-provenance.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('dataset',choices=SOURCES);parser.add_argument('cache',type=pathlib.Path)
    args=parser.parse_args();prepare(args.dataset,args.cache)
