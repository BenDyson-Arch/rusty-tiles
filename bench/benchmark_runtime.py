"""Compare native readiness/preview to frozen pre-migration Python helpers.
Runs only local invented files; downloads nothing. Preview startup includes an
HTTP config round trip. Peak preview RSS is sampled after a 1 MiB file round trip.
"""
import argparse
import hashlib
import http.client
import json
import os
import pathlib
import platform
import select
import statistics
import subprocess
import sys
import tempfile
import time
from urllib.parse import urlsplit

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASELINE = 'bf3346c4808d54df52548badabde7dc0d65c0723'

def fetch(url,path):
    address=urlsplit(url);client=http.client.HTTPConnection(address.hostname,address.port,timeout=10)
    try:
        client.request('GET',path);response=client.getresponse();data=response.read()
        if response.status!=200:raise AssertionError(response.status)
        return data
    finally:client.close()

def benchmark(binary,repeats):
    binary=binary.resolve()
    with tempfile.TemporaryDirectory() as tmp:
        root=pathlib.Path(tmp);legacy=root/'legacy';(legacy/'scripts').mkdir(parents=True);(legacy/'preview').mkdir()
        for file in ('scripts/doctor.py','scripts/preview.py','preview/index.html'):
            (legacy/file).write_bytes(subprocess.check_output(['git','show',f'{BASELINE}:{file}'],cwd=ROOT))
        runtime=root/'runtime';runtime.mkdir();(runtime/'Cesium.js').write_text('// invented runtime')
        tiles=root/'tiles';tiles.mkdir();(tiles/'tileset.json').write_text('{}')
        payload=bytes(range(256))*4096;(tiles/'payload.bin').write_bytes(payload)
        rows={name:{'doctorSeconds':[],'previewStartupSeconds':[],'previewRssKiB':[],'sequential32RequestsSeconds':[]}
              for name in ('python','rust')}
        for trial in range(repeats+1):
            for name in (('python','rust') if trial%2 else ('rust','python')):
                doctor=[str(binary),'doctor','--json'] if name=='rust' else [sys.executable,str(legacy/'scripts/doctor.py')]
                env=dict(os.environ,PATH='') if name=='rust' else dict(os.environ)
                start=time.perf_counter();report=json.loads(subprocess.check_output(doctor,env=env));elapsed=time.perf_counter()-start
                if not report['ready']:raise AssertionError(report)
                shared={'vector','terrain','raster','point-cloud','convert','mesh-to-3tz','glb-to-3tz','createTilesetJson'}
                if not all(report['commands'][command]['ready'] for command in shared):raise AssertionError(report)
                command=[str(binary),'preview'] if name=='rust' else [sys.executable,str(legacy/'scripts/preview.py')]
                command += ['--port','0','--cesium',str(runtime),'--mesh',str(tiles)]
                start=time.perf_counter();process=subprocess.Popen(command,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
                try:
                    if not select.select([process.stdout],[],[],10)[0]:raise AssertionError('startup timed out')
                    url=process.stdout.readline().strip().removeprefix('Preview: ')
                    config=json.loads(fetch(url,'/config.json'))
                    if config!={'mesh':'/mesh/tileset.json'}:raise AssertionError(config)
                    startup=time.perf_counter()-start
                    if fetch(url,'/mesh/payload.bin')!=payload:raise AssertionError('payload differs')
                    rss=next(int(line.split()[1]) for line in pathlib.Path(f'/proc/{process.pid}/status').read_text().splitlines() if line.startswith('VmHWM:'))
                    start=time.perf_counter()
                    for _ in range(32):
                        if fetch(url,'/mesh/payload.bin')!=payload:raise AssertionError('payload differs')
                    requests=time.perf_counter()-start
                    if trial:
                        rows[name]['doctorSeconds'].append(elapsed);rows[name]['previewStartupSeconds'].append(startup)
                        rows[name]['previewRssKiB'].append(rss);rows[name]['sequential32RequestsSeconds'].append(requests)
                finally:
                    process.terminate();process.communicate(timeout=5)
        return {'environment':{'platform':platform.platform(),'python':platform.python_version(),
            'binarySha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'version':subprocess.check_output([str(binary),'--version'],text=True).strip(),
            'doctor':json.loads(subprocess.check_output([str(binary),'doctor','--json'],env=dict(os.environ,PATH=''))),
            'baselineCommit':BASELINE,'repeats':repeats},'samples':rows,
            'medians':{name:{metric:statistics.median(samples) for metric,samples in values.items()} for name,values in rows.items()},
            'scope':'Warm local process startup/readiness and loopback HTTP, same invented 1 MiB bytes, rotated order, one warmup. Largest sampled server RSS, not browser memory or rendering throughput.'}

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('binary',type=pathlib.Path)
    parser.add_argument('output',type=pathlib.Path);parser.add_argument('--repeats',type=int,default=7)
    args=parser.parse_args();report=benchmark(args.binary,args.repeats)
    args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['medians'],indent=2))
