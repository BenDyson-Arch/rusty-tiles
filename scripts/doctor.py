"""Read-only dependency/capability inventory. Never downloads modules or grids."""
import importlib
import importlib.metadata
import json
import os
import pathlib
import sys

NATIVE=['mesh-to-3tz','glb-to-3tz','createTilesetJson','convert','point-cloud']
PYTHON=['vector','raster','terrain']


def report(selected=()):
    modules={};loaded={}
    for name,package in [('numpy','numpy'),('gdal','osgeo.gdal'),('ogr','osgeo.ogr'),
            ('osr','osgeo.osr')]:
        try:
            module=importlib.import_module(package);loaded[name]=module
            version=module.VersionInfo('--version') if name=='gdal' else getattr(module,'__version__',None)
            if version is None:
                try:version=importlib.metadata.version('GDAL' if name in ('ogr','osr') else name)
                except importlib.metadata.PackageNotFoundError:version='unknown'
            modules[name]=dict(available=True,version=version,path=getattr(module,'__file__',None))
        except Exception as error:
            modules[name]=dict(available=False,error=str(error))
    geos=dict(available=False,error='OGR unavailable');triangulation=False
    if 'ogr' in loaded:
        ogr=loaded['ogr']
        try:
            version='.'.join(str(getattr(ogr,'GetGEOSVersion'+part)()) for part in ('Major','Minor','Micro'))
            geos=dict(available=version!='0.0.0',version=version)
            shape=ogr.CreateGeometryFromWkt('POLYGON ((0 0,1 0,0 1,0 0))')
            triangulation=shape.ConstrainedDelaunayTriangulation().GetGeometryCount()==1
        except Exception as error:geos['error']=str(error)
    database={}
    if 'osr' in loaded:
        try:
            reference=loaded['osr'].SpatialReference()
            database['gdalAvailable']=reference.ImportFromEPSG(4326)==0
        except Exception as error:database.update(gdalAvailable=False,gdalError=str(error))
    paths=[]
    for name in ('PROJ_DATA','PROJ_LIB'):
        if os.environ.get(name):paths.extend(os.environ[name].split(os.pathsep))
    if 'osr' in loaded:
        try:paths.extend(loaded['osr'].GetPROJSearchPaths())
        except Exception:pass
    paths=list(dict.fromkeys(str(pathlib.Path(p).expanduser()) for p in paths if p))
    grids=[]
    for directory in paths:
        root=pathlib.Path(directory)
        if root.is_dir():
            for suffix in ('*.gtx','*.gsb','*.tif','*.bin'):
                grids.extend(str(p) for p in root.glob(suffix) if p.is_file())
    groups={'vector':['numpy','gdal','ogr','osr'], 'raster':['numpy','gdal'],
        'terrain':['numpy','gdal']}
    commands={name:dict(ready=True,requires=[]) for name in NATIVE}
    for name,requires in groups.items():
        missing=[m for m in requires if not modules[m]['available']]
        if name=='vector' and not triangulation:missing.append('GEOS constrained triangulation')
        if name in ('vector','raster','terrain') and not database.get('gdalAvailable'):missing.append('PROJ database')
        commands[name]=dict(ready=not missing,requires=requires,missing=missing)
    selected=list(selected) or NATIVE+PYTHON
    return dict(ready=all(commands[name]['ready'] for name in selected),selectedCommands=selected,
        python=dict(available=True,executable=sys.executable,version=sys.version.split()[0]),
        modules=modules,geos=geos,vectorTriangulation=triangulation,
        proj=dict(database=database,dataDirectories=paths,availableGrids=sorted(set(grids)),
            note='Grid inventory is not proof that a particular CRS/height operation is available; conversion validates it.'),
        commands=commands)


if __name__=='__main__':
    print(json.dumps(report(sys.argv[1:]),allow_nan=False))
