#!/usr/bin/env python3
"""Linux /proc observations at fixed point settings; no universal memory bound."""
import argparse
import hashlib
import importlib.util
import json
import os
import pathlib
import subprocess
import tempfile
import time

ROOT=pathlib.Path(__file__).resolve().parents[3]
spec=importlib.util.spec_from_file_location('point_oracle',ROOT/'tests/p1_point_oracle.py')
oracle=importlib.util.module_from_spec(spec);spec.loader.exec_module(oracle)


def probe(binary,count,max_points,chunk_points,implicit,additional_extra=0,geographic=False):
    with tempfile.TemporaryDirectory(prefix='p1-resource-') as tmp:
        root=pathlib.Path(tmp);source=root/'source.las';output=root/'result.3tz'
        oracle.fixture(source,count=count,additional_extra=additional_extra,variant='geographic' if geographic else 'spread')
        command=[str(binary),'--json','point-cloud','-i',str(source),'-o',str(output),'--sourceCrs','EPSG:4326' if geographic else 'local','--maxPoints',str(max_points),'--chunkPoints',str(chunk_points)]
        if not implicit:command.append('--explicit')
        if geographic:command+=['--heightOffset','0']
        started=time.monotonic();process=subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        peak={'rss_bytes':0,'high_water_rss_bytes':0,'descriptors':0,'threads':0,'workspace_bytes':0,'scratch_bytes':0,'workspace_files':0};samples=0
        while process.poll() is None:
            try:
                status={line.split(':',1)[0]:line.split(':',1)[1].strip() for line in pathlib.Path(f'/proc/{process.pid}/status').read_text().splitlines() if ':' in line}
                for name,key in [('rss_bytes','VmRSS'),('high_water_rss_bytes','VmHWM')]:peak[name]=max(peak[name],int(status.get(key,'0 kB').split()[0])*1024)
                peak['threads']=max(peak['threads'],int(status.get('Threads','0')))
                peak['descriptors']=max(peak['descriptors'],len(list(pathlib.Path(f'/proc/{process.pid}/fd').iterdir())))
                workspace=scratch=files=0
                for directory,_,names in os.walk(root):
                    for name in names:
                        path=pathlib.Path(directory)/name
                        if path==source or path==output:continue
                        try:size=path.stat().st_size
                        except FileNotFoundError:continue
                        workspace+=size;files+=1
                        if 'scratch' in path.parts or path.suffix=='.bin':scratch+=size
                peak['workspace_bytes']=max(peak['workspace_bytes'],workspace);peak['scratch_bytes']=max(peak['scratch_bytes'],scratch);peak['workspace_files']=max(peak['workspace_files'],files);samples+=1
            except (FileNotFoundError,ProcessLookupError):pass
            time.sleep(.01)
        stdout,stderr=process.communicate();oracle.require(process.returncode==0,'resource conversion failed: '+stdout+stderr)
        result=json.loads(stdout);report={key:result.get(key) for key in ('counts','settings')}
        leftovers=[str(p.relative_to(root)) for p in root.iterdir() if p not in (source,output)]
        oracle.require(not leftovers,'workspace leak after success')
        return {'points':count,'additional_u64_fields':additional_extra,'point_record_bytes':oracle.unpack('H',source.read_bytes(),105)[0],'max_points':max_points,'chunk_points':chunk_points,'implicit':implicit,'coordinate_profile':'analytic_geographic_ECEF' if geographic else 'local_metre','elapsed_seconds':time.monotonic()-started,'samples':samples,'source_bytes':source.stat().st_size,'archive_bytes':output.stat().st_size,'peaks':peak,'cli_report':report,'workspace_after_exit':leftovers}


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',required=True);p.add_argument('--counts',default='1024,8192,32768');p.add_argument('--max-points',type=int,default=256);p.add_argument('--chunk-points',type=int,default=1024);p.add_argument('--explicit',action='store_true');p.add_argument('--geographic',action='store_true');p.add_argument('--wide-count',type=int,default=8192);p.add_argument('--extra-fields',default='32,128');p.add_argument('--json-output',required=True);a=p.parse_args()
    binary=pathlib.Path(a.binary).resolve();results=[probe(binary,int(count),a.max_points,a.chunk_points,not a.explicit,geographic=a.geographic) for count in a.counts.split(',')]
    results.extend(probe(binary,a.wide_count,a.max_points,a.chunk_points,not a.explicit,int(fields),a.geographic) for fields in a.extra_fields.split(',') if fields)
    evidence={'binary':str(binary),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'sampling_interval_seconds':.01,'limits':'Observed Linux process RSS/high-water RSS, descriptors, threads and on-disk workspace at fixed settings. Short peaks can be missed. Reader, transform, spool, leaves, hierarchy and serialization share this serial process; the observations do not attribute each allocation or establish a whole-job constant-memory bound. Hierarchy/member inventory and scratch grow with input and leaf count. No worker/native subprocess exists in the measured local profile.','runs':results}
    pathlib.Path(a.json_output).write_text(json.dumps(evidence,indent=2)+'\n');print(json.dumps(evidence,indent=2))
if __name__=='__main__':main()
