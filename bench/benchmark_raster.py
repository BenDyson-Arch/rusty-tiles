"""Compare release Python/native raster CLIs on deterministic invented rasters.

Linux only: wait4 CPU includes waited descendants; peak RSS is the largest process,
not aggregate process-tree memory. Fixture generation and fidelity checks are untimed.
Both binaries and Python GDAL must use the same GDAL/PROJ/codec stack.
"""
import argparse, datetime, hashlib, importlib.util, json, os, pathlib, platform, shlex
import shutil, statistics, subprocess, sys
import numpy as np
from osgeo import gdal, osr

gdal.UseExceptions(); osr.UseExceptions()
ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('timing', ROOT/'bench/benchmark_terrain.py')
timing=importlib.util.module_from_spec(spec);spec.loader.exec_module(timing)
METHODS=['python-default','rust-native','python-one-worker']
CASES=[
    dict(name='small-rgba',pixels=256,crs=4326,transform=[12.5,.00001,0,41.9,0,-.00001],display='image',minZoom=15,maxZoom=16),
    dict(name='regional-rgba',pixels=4096,crs=4326,transform=[12,2/4096,0,42,0,-2/4096],display='image',minZoom=9,maxZoom=12),
    dict(name='regional-gray',pixels=4096,crs=4326,transform=[12,2/4096,0,42,0,-2/4096],display='gray',minZoom=9,maxZoom=12),
    dict(name='projected-rgba',pixels=2048,crs=32633,transform=[500000,100,0,4650000,0,-100],display='image',minZoom=8,maxZoom=11),
]

def fixture(path,case):
    size=case['pixels'];gray=case['display']=='gray';bands=1 if gray else 4
    ds=gdal.GetDriverByName('GTiff').Create(str(path),size,size,bands,gdal.GDT_Float32 if gray else gdal.GDT_Byte,
        options=['TILED=YES','BLOCKXSIZE=256','BLOCKYSIZE=256'])
    crs=osr.SpatialReference();crs.ImportFromEPSG(case['crs']);ds.SetProjection(crs.ExportToWkt());ds.SetGeoTransform(case['transform'])
    ds.SetMetadata({'fixture':'deterministic invented raster benchmark'})
    for i in range(1,bands+1):
        ds.GetRasterBand(i).SetDescription(f'fixture-band-{i}')
    if gray:
        ds.GetRasterBand(1).SetNoDataValue(-32768);ds.GetRasterBand(1).SetUnitType('m')
    else:
        for i,color in enumerate((gdal.GCI_RedBand,gdal.GCI_GreenBand,gdal.GCI_BlueBand,gdal.GCI_AlphaBand),1):
            ds.GetRasterBand(i).SetColorInterpretation(color)
    ds.GetRasterBand(1).CreateMaskBand(gdal.GMF_PER_DATASET)
    rng=np.random.default_rng(314159)
    x=np.arange(size,dtype=np.float64)[None,:]/size
    for start in range(0,size,256):
        y=np.arange(start,min(start+256,size),dtype=np.float64)[:,None]/size
        coverage=np.full((len(y),size),255,dtype=np.uint8)
        coverage[(abs(x-.25)<.07)&(abs(y-.25)<.07)]=0
        ds.GetRasterBand(1).GetMaskBand().WriteArray(coverage,0,start)
        noise=rng.integers(-12,13,size=(len(y),size))
        if gray:
            pixels=(100*x+50*y+400*np.exp(-((x-.5)**2+(y-.5)**2)*16)+noise*.2-100).astype(np.float32)
            pixels[(abs(x-.6)<.05)&(abs(y-.6)<.05)]=-32768
            ds.GetRasterBand(1).WriteArray(pixels,0,start)
        else:
            for i in range(1,4):
                pixels=np.clip(128+70*np.sin(x*(i*12+8))*np.cos(y*(i*8+12))+noise,0,255).astype(np.uint8)
                pixels[(x<.15)&(y<.15)]=0
                ds.GetRasterBand(i).WriteArray(pixels,0,start)
            alpha=np.full((len(y),size),255,dtype=np.uint8)
            alpha[(abs(x-.55)<.07)&(abs(y-.55)<.07)]=0
            alpha[(x>.8)&(y>.8)]=128
            ds.GetRasterBand(4).WriteArray(alpha,0,start)
    ds=None


def compare_source(source,output):
    a=gdal.Open(str(source));b=gdal.Open(str(output/'source.cog.tif'))
    assert a.RasterCount==b.RasterCount
    assert a.GetGeoTransform()==b.GetGeoTransform() and a.GetProjection()==b.GetProjection()
    assert a.GetMetadata()==b.GetMetadata()
    assert b.GetMetadata('IMAGE_STRUCTURE')['LAYOUT']=='COG'
    for i in range(1,a.RasterCount+1):
        ba,bb=a.GetRasterBand(i),b.GetRasterBand(i)
        for name in ('GetDescription','GetNoDataValue','GetScale','GetOffset','GetUnitType','GetColorInterpretation','GetMetadata'):
            assert getattr(ba,name)()==getattr(bb,name)(),(i,name)
        for y in range(0,a.RasterYSize,256):
            height=min(256,a.RasterYSize-y)
            np.testing.assert_array_equal(ba.ReadAsArray(0,y,a.RasterXSize,height),bb.ReadAsArray(0,y,a.RasterXSize,height))
            np.testing.assert_array_equal(ba.GetMaskBand().ReadAsArray(0,y,a.RasterXSize,height),bb.GetMaskBand().ReadAsArray(0,y,a.RasterXSize,height))


def compare(folder,source):
    baseline=folder/METHODS[0]
    paths={str(p.relative_to(baseline)) for p in (baseline/'tiles').rglob('*.png')}
    assert paths
    baseline_docs={name:json.loads((baseline/name).read_text()) for name in ('conversion.json','tilejson.json')}
    for method in METHODS:
        output=folder/method;compare_source(source,output)
        assert paths=={str(p.relative_to(output)) for p in (output/'tiles').rglob('*.png')}
        for name,doc in baseline_docs.items():assert json.loads((output/name).read_text())==doc,(method,name)
    mismatched={method:dict(tiles=0,samples=0,maxChannelDifference=0) for method in METHODS[1:]}
    matching_bytes={method:True for method in METHODS[1:]}
    for relative in sorted(paths):
        data=gdal.Open(str(baseline/relative)).ReadAsArray()
        for method in METHODS[1:]:
            matching_bytes[method]&=(baseline/relative).read_bytes()==(folder/method/relative).read_bytes()
            other=gdal.Open(str(folder/method/relative)).ReadAsArray()
            assert data.shape==other.shape
            differences=np.abs(data.astype(np.int16)-other.astype(np.int16))
            stats=mismatched[method]
            if np.any(differences):
                stats['tiles']+=1;stats['samples']+=int(np.count_nonzero(differences))
                stats['maxChannelDifference']=max(stats['maxChannelDifference'],int(differences.max()))
    return dict(sourceValuesMasksAndMetadataPreserved=True,manifestsAndRecipesMatch=True,tilePathsMatch=True,
        decodedPixelDifferences=mismatched,pngBytesMatch=matching_bytes)


def output_stats(output):
    tiles=list((output/'tiles').rglob('*.png'))
    return dict(tiles=len(tiles),pngBytes=sum(p.stat().st_size for p in tiles),
        cogBytes=(output/'source.cog.tif').stat().st_size,
        totalBytes=sum(p.stat().st_size for p in output.rglob('*') if p.is_file()))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--old-bin',type=pathlib.Path,required=True)
    parser.add_argument('--new-bin',type=pathlib.Path,required=True)
    parser.add_argument('--work',type=pathlib.Path,required=True)
    parser.add_argument('--repeats',type=int,default=3)
    parser.add_argument('--old-ref',help='baseline revision recorded with results')
    args=parser.parse_args()
    if args.repeats<3:parser.error('use at least three measured repetitions')
    WORK=args.work.resolve();WORK.mkdir(parents=True,exist_ok=True)
    OLD=args.old_bin.resolve();NEW=args.new_bin.resolve();REPEATS=args.repeats
    env=dict(os.environ,PROJ_NETWORK='OFF')
    for name in ('GDAL_CACHEMAX','GDAL_NUM_THREADS','RAYON_NUM_THREADS'):
        env.pop(name,None)
    gdal_exe=shutil.which('gdal',path=env['PATH'])
    if not gdal_exe:parser.error('Python baseline requires the gdal executable')
    shim=WORK/'one-worker-bin';shim.mkdir(exist_ok=True)
    (shim/'gdal').write_text(f'#!/bin/sh\nexec {shlex.quote(gdal_exe)} "$@" --num-threads=1\n')
    (shim/'gdal').chmod(0o755)
    environments={METHODS[0]:env,METHODS[1]:dict(env,PATH=''),METHODS[2]:dict(env,PATH=str(shim)+os.pathsep+env['PATH'])}
    versions=dict(recordedAtUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(),platform=platform.platform(),
        cpu=next(line.split(':',1)[1].strip() for line in pathlib.Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),
        logicalCpus=os.cpu_count(),python=platform.python_version(),numpy=np.__version__,gdal=gdal.VersionInfo('--version'),
        proj=[osr.GetPROJVersionMajor(),osr.GetPROJVersionMinor(),osr.GetPROJVersionMicro()],
        rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),
        baselineRevision=args.old_ref,
        nativeRevision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()+' (working tree)',
        nativeSourceSha256={str(p):hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in [pathlib.Path('src/raster.rs'),pathlib.Path('src/raster/native.rs')]},
        oldBinarySha256=hashlib.sha256(OLD.read_bytes()).hexdigest(),newBinarySha256=hashlib.sha256(NEW.read_bytes()).hexdigest(),
        repeats=REPEATS,build='cargo build --release --locked --features native-geospatial',
        methodology='Full release CLI conversions including COG, styling, tiling, startup and safe publication. One warmup per method, then three measured repetitions in rotated order; warm filesystem cache, conversions sequential. Linux wait4 CPU includes waited descendants; peak RSS is largest process, not summed simultaneous process-tree memory. Fixture generation and decoded-output checks excluded.',
        configurations={'python-default':'Original Python path; default GDAL cache, native GDAL tile ALL_CPUS.',
            'rust-native':'New Rust path; 64 MiB raster cache, 64 MiB explicit display warp budget, up to four native GDAL tile/COG workers, uncompressed temporary TIFFs; no executable PATH.',
            'python-one-worker':'Original path except gdal raster tile explicitly receives --num-threads=1 via an exec-only shell shim; default GDAL cache retained.'})
    results=[]
    for case in CASES:
        folder=WORK/case['name'];folder.mkdir(exist_ok=True);source=folder/'source.tif'
        print(f'Creating {case["name"]}',flush=True);fixture(source,case)
        commands={}
        for method in METHODS:
            command=[str(NEW if method=='rust-native' else OLD),'raster','-i',str(source),'-o',str(folder/method),
                '--minZoom',str(case['minZoom']),'--maxZoom',str(case['maxZoom']),'--display',case['display'],'--force']
            command+=['--alphaBand','4'] if case['display']=='image' else ['--displayMin','-100','--displayMax','500']
            commands[method]=command
        samples={method:[] for method in METHODS}
        for method in METHODS:
            print(f'{case["name"]}: {method} warmup',flush=True)
            timing.measured(commands[method],folder/f'{method}-warmup.log',environments[method])
        for repeat in range(REPEATS):
            for method in METHODS[repeat%len(METHODS):]+METHODS[:repeat%len(METHODS)]:
                print(f'{case["name"]}: {method} repeat {repeat+1}',flush=True)
                samples[method].append(timing.measured(commands[method],folder/f'{method}-{repeat}.log',environments[method]))
        print(f'{case["name"]}: checking every tile pixel and source sample',flush=True)
        comparison=compare(folder,source)
        result=dict(case,sourceBytes=source.stat().st_size,sourceSha256=hashlib.sha256(source.read_bytes()).hexdigest(),methods={},comparison=comparison)
        for method in METHODS:
            values=samples[method]
            result['methods'][method]=dict(samples=values,
                medianWallSeconds=statistics.median(v['wallSeconds'] for v in values),
                medianCpuSeconds=statistics.median(v['cpuSeconds'] for v in values),
                medianPeakRssMiB=statistics.median(v['peakRssMiB'] for v in values),output=output_stats(folder/method))
        results.append(result)
        (WORK/'results.json').write_text(json.dumps(dict(environment=versions,cases=results),indent=2)+'\n')
        print(json.dumps(dict(case=case['name'],results={m:result['methods'][m]['medianWallSeconds'] for m in METHODS},comparison=comparison)),flush=True)

if __name__=='__main__':main()
