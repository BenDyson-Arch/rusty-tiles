"""Independent C1 adversarial measurements; stdlib-only fresh wait4 worker.
--binary BIN --work NEW_DIR --output JSON; observed resource use is not a universal ceiling.
"""
import argparse, hashlib, json, os, pathlib, signal, struct, subprocess, sys, time, zipfile
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[3]/'tests'))
from c1_validation_oracle import archive, generate, glb, snapshot


def worker(config_path):
    config=json.loads(pathlib.Path(config_path).read_text()); root=pathlib.Path(config_path).parent
    started=time.monotonic(); peak=0; samples=0
    with (root/'stdout').open('w') as out, (root/'stderr').open('w') as err:
        child=subprocess.Popen(config['command'],stdout=out,stderr=err)
        while True:
            pid,status,usage=os.wait4(child.pid,os.WNOHANG)
            if pid: child.returncode=os.waitstatus_to_exitcode(status); break
            if time.monotonic()-started>30:
                os.kill(child.pid,signal.SIGKILL);os.wait4(child.pid,0);raise RuntimeError('validation exceeded 30 seconds')
            try: peak=max(peak,len(list(pathlib.Path(f'/proc/{child.pid}/fd').iterdir())));samples+=1
            except FileNotFoundError: pass
            time.sleep(.002)
    print(json.dumps(dict(exitCode=child.returncode,maximumRssKiB=usage.ru_maxrss,userSeconds=usage.ru_utime,
       systemSeconds=usage.ru_stime,wallSeconds=time.monotonic()-started,peakDescriptors=peak,descriptorSamples=samples,
       stdout=(root/'stdout').read_text(),stderr=(root/'stderr').read_text())))


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=pathlib.Path,required=True)
    p.add_argument('--work',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
    a.work.mkdir(parents=True,exist_ok=False); corpus=a.work/'corpus';generate(corpus)
    with zipfile.ZipFile(corpus/'triangle.3tz') as z:
        tile=z.read('tile.glb');manifest=z.read('tileset.json')
    n=int.from_bytes(tile[12:16],'little');document=json.loads(tile[20:20+n]);binary=tile[28+n:]
    many=json.loads(json.dumps(document));many['meshes'][0]['primitives']*=80000
    archive(corpus/'shared_primitives.3tz',{'tileset.json':manifest,'tile.glb':glb(many,binary)})
    indices=json.loads(json.dumps(document));indices['buffers'][0]['byteLength']=4000036
    indices['bufferViews'][1].update(byteOffset=36,byteLength=4000000)
    indices['accessors'][1].update(count=1000000,componentType=5125)
    indices['meshes'][0]['primitives'][0]['mode']=0;indices['meshes'][0]['primitives']*=1000
    archive(corpus/'legal_shared_indices.3tz',{'tileset.json':manifest,'tile.glb':glb(indices,binary[:36]+bytes(4000000))})
    large=json.loads(json.dumps(document));large['extras']={'padding':'x'*(8*1024*1024)}
    archive(corpus/'json_limit.3tz',{'tileset.json':manifest,'tile.glb':glb(large,binary)})
    with zipfile.ZipFile(corpus/'implicit_no_content.3tz') as z:
        implicit_manifest=json.loads(z.read('tileset.json'))
    subtree={'tileAvailability':{'constant':1},'contentAvailability':[],'childSubtreeAvailability':{'constant':0}}
    for name,extras in [('implicit_json_limit',{'padding':'x'*(8*1024*1024)}),
                        ('implicit_json_items_limit',{'nodes':[0]*65537})]:
        doc=dict(subtree,extras=extras);text=json.dumps(doc).encode();text+=b' '*(-len(text)%8)
        data=struct.pack('<4sIQQ',b'subt',1,len(text),0)+text
        archive(corpus/(name+'.3tz'),{'tileset.json':json.dumps(implicit_manifest).encode(),'0-0-0.subtree':data})
    implicit_manifest['root']['implicitTiling'].update(subtreeLevels=16,availableLevels=16)
    # No subtree resource is supplied: capacity admission must precede reads.
    archive(corpus/'implicit_capacity_limit.3tz',{'tileset.json':json.dumps(implicit_manifest).encode()})
    before=snapshot(corpus); results=[]
    for name in ['triangle','count_limit','count_sum_overflow','meshopt_count_limit','json_limit','shared_primitives','legal_shared_indices','implicit_json_limit','implicit_json_items_limit','implicit_capacity_limit']:
        for repeat in range(2):
            run=a.work/f'{name}-{repeat}';run.mkdir();config=run/'config.json'
            config.write_text(json.dumps({'command':[str(a.binary.resolve()),'validate',str((corpus/(name+'.3tz')).resolve()),'--json']}))
            child=subprocess.Popen([sys.executable,str(pathlib.Path(__file__).resolve()),'--worker',str(config)],
                                   stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
            try: stdout,stderr=child.communicate(timeout=35)
            except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.wait();raise
            assert child.returncode==0,stderr
            result=json.loads(stdout);result.update(name=name,repetition=repeat);report=json.loads(result.pop('stdout'))
            result['report']=report
            assert result['exitCode'] in [0,2,3],result
            if name in ['count_limit','count_sum_overflow','meshopt_count_limit','json_limit','implicit_json_limit','implicit_json_items_limit','implicit_capacity_limit']:
                assert report['error']['code']=='resource_limit',result
            if name=='legal_shared_indices':assert report['ok'] is True,result
            results.append(result)
    assert snapshot(corpus)==before
    evidence={'binarySha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'runs':results,'readOnly':True,
      'limitations':'Fresh-process Linux wait4 observations only, not a universal RSS ceiling; shared geometry stresses bounded work without enormous payload.'}
    a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(evidence,indent=2)+'\n')
    print(json.dumps({'ok':True,'evidence':str(a.output),'runs':len(results)}))


if __name__=='__main__':
    if len(sys.argv)==3 and sys.argv[1]=='--worker':worker(sys.argv[2])
    else:main()
