"""Finite resource witness preparation/monitor (never imported by the converter).

prepare: 100k mixed analytic features, metadata and a 4096-vertex line, outside
measurement. monitor: sample converter disk/FD/thread/swap high-waters; caller
uses measure.py for child-process RSS. This is not a worst-case allocation proof.
"""
import argparse
import hashlib
import json
import math
import pathlib
import subprocess
import time


def prepare(path):
    kinds, maximum_vertices, maximum_properties, maximum_feature = {}, 0, 0, 0
    with path.open('w') as stream:
        stream.write('{"type":"FeatureCollection","features":[')
        for i in range(100000):
            x, y = i % 400 * 20, i // 400 * 20
            if i == 0:
                geometry = dict(type='LineString', coordinates=[[x+j/512,y+math.sin(j/100),3+j/4096] for j in range(4096)])
                vertices = 4096
            elif i % 3 == 0:
                geometry = dict(type='Point', coordinates=[x,y,3])
                vertices = 1
            elif i % 3 == 1:
                geometry = dict(type='MultiLineString', coordinates=[[[x,y,3],[x+2,y+1,4],[x+4,y,3]],[[x+6,y,3],[x+9,y+2,5]]])
                vertices = 5
            else:
                outer = [[x,y,3],[x+10,y,3],[x+10,y+10,3],[x,y+10,3],[x,y,3]]
                hole = [[x+3,y+3,3],[x+3,y+7,3],[x+7,y+7,3],[x+7,y+3,3],[x+3,y+3,3]]
                geometry = dict(type='Polygon', coordinates=[outer,hole])
                vertices = 10
            properties = dict(name='survey-'+str(i), value=2**60+i, nullable=None if i%2 else i,
                              active=bool(i%2), tags=['survey',i,None])
            feature = dict(type='Feature',id=i,properties=properties,geometry=geometry)
            raw = json.dumps(feature,separators=(',',':'))
            stream.write((',' if i else '')+raw)
            maximum_feature = max(maximum_feature,len(raw.encode()))
            kinds[geometry['type']] = kinds.get(geometry['type'],0)+1
            maximum_vertices = max(maximum_vertices,vertices)
            maximum_properties = max(maximum_properties,len(json.dumps(properties,separators=(',',':')).encode()))
        stream.write(']}')
    return dict(source=str(path),sha256=hashlib.sha256(path.read_bytes()).hexdigest(),bytes=path.stat().st_size,
        features=100000,kinds=kinds,max_feature_json_bytes=maximum_feature,max_source_vertices=maximum_vertices,max_property_json_bytes=maximum_properties,
        fields=5,layers=1,provenance='analytic stress witness; distinct from the existing local-data realistic run')


def monitor(directory, receipt, command):
    directory.mkdir(parents=True,exist_ok=True)
    child = subprocess.Popen(command)
    result = dict(max_directory_bytes=0,max_fds=0,max_threads=0,max_swap_kib=0,samples=0)
    while True:
        try:
            proc = pathlib.Path('/proc')/str(child.pid)
            status = dict(line.split(':',1) for line in (proc/'status').read_text().splitlines() if ':' in line)
            result['max_fds'] = max(result['max_fds'],len(list((proc/'fd').iterdir())))
            result['max_threads'] = max(result['max_threads'],int(status['Threads']))
            result['max_swap_kib'] = max(result['max_swap_kib'],int(status.get('VmSwap','0 kB').split()[0]))
            size = 0
            for path in directory.rglob('*'):
                try:
                    if path.is_file(): size += path.stat().st_size
                except FileNotFoundError:
                    pass
            result['max_directory_bytes'] = max(result['max_directory_bytes'],size)
            result['samples'] += 1
        except FileNotFoundError:
            pass
        if child.poll() is not None:
            break
        time.sleep(.05)
    result.update(exit=child.returncode,remaining=[p.name for p in directory.iterdir()])
    receipt.write_text(json.dumps(result,indent=2)+'\n')
    raise SystemExit(child.returncode)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('mode',choices=['prepare','monitor'])
    parser.add_argument('path',type=pathlib.Path)
    parser.add_argument('receipt',type=pathlib.Path)
    parser.add_argument('command',nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.mode == 'prepare':
        args.receipt.write_text(json.dumps(prepare(args.path),indent=2)+'\n')
    else:
        monitor(args.path,args.receipt,args.command)
