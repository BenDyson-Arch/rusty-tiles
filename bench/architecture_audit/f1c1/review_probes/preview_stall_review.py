#!/usr/bin/env python3
"""Independent HTTP connection-ownership probe; no browser or converter fixtures."""
import hashlib,json,os,selectors,signal,socket,subprocess,tempfile,time
from pathlib import Path

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[3]
PIN_SHA='bef6518c234783caaa41a1d4c416e22dab2001d8067de379f537864cf61d503e'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    manifest=json.loads(Path('/home/bend/.cache/rusty-tiles-f1c1-evidence/build-manifest.json').read_text())
    binary=Path(manifest['native_binary']['path'])
    assert sha(binary)==manifest['native_binary']['sha256']==PIN_SHA
    assert sha(ROOT/'src/preview.rs')==manifest['compiled_inputs_sha256']['src/preview.rs']
    cache=Path(tempfile.mkdtemp(prefix='rusty-tiles-preview-owner-review-',dir='/home/bend/.cache'))
    (cache/'cesium').mkdir();(cache/'cesium/Cesium.js').write_text('// request ownership probe stub')
    (cache/'mesh').mkdir();(cache/'mesh/tileset.json').write_text('{}')
    results={'artifact_sha256':PIN_SHA,'preview_source_sha256':sha(ROOT/'src/preview.rs'),'probe_sha256':sha(Path(__file__)),'cache':str(cache),'cases':[]}
    def case(count):
        log=(cache/f'preview-{count}.stderr').open('wb')
        process=subprocess.Popen([str(binary),'preview','--cesium',str(cache/'cesium'),'--mesh',str(cache/'mesh'),'--port','0','--json'],stdout=subprocess.PIPE,stderr=log,text=True,start_new_session=True)
        clients=[]
        try:
            cpu=min(os.sched_getaffinity(0))
            os.sched_setaffinity(process.pid,{cpu})
            with selectors.DefaultSelector() as ready:
                ready.register(process.stdout,selectors.EVENT_READ)
                assert ready.select(10),'startup timeout'
                startup=json.loads(process.stdout.readline())
            port=int(startup['url'].split(':')[-1].rstrip('/'))
            time.sleep(.05) # Establish four idle tiny_http connection-reader workers.
            os.kill(process.pid,signal.SIGSTOP)
            for i in range(count):
                client=socket.create_connection(('127.0.0.1',port),timeout=2)
                client.setblocking(False)
                client.sendall(b'GET /config.json HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n')
                clients.append(client)
            os.kill(process.pid,signal.SIGCONT)
            responses={i:b'' for i in range(count)}
            def collect(duration):
                end=time.monotonic()+duration
                with selectors.DefaultSelector() as selected:
                    for i,c in enumerate(clients):
                        if c.fileno()>=0:selected.register(c,selectors.EVENT_READ,i)
                    while time.monotonic()<end:
                        for key,_ in selected.select(max(0,end-time.monotonic())):
                            try:data=key.fileobj.recv(65536)
                            except BlockingIOError:continue
                            if data:responses[key.data]+=data
                            else:selected.unregister(key.fileobj)
            collect(.5)
            completed=[i for i,v in responses.items() if b'HTTP/1.1 200' in v and b'"mesh"' in v]
            pending=[i for i,v in responses.items() if not v]
            threads={p.name:(p/'wchan').read_text().strip() for p in Path(f'/proc/{process.pid}/task').iterdir()}
            record={'connection_count':count,'cpu_affinity':[cpu],'completed_while_keepalive_held':completed,'pending_while_keepalive_held':pending,'thread_wait_channels':threads}
            if pending and completed:
                # Releasing the owning client reader must service a queued client.
                clients[completed[0]].close()
                collect(.5)
                advanced=[i for i in pending if b'HTTP/1.1 200' in responses[i] and b'"mesh"' in responses[i]]
                record['completed_after_one_active_connection_closed']=advanced
                assert advanced,'queued connection did not release with an active reader'
            results['cases'].append(record)
        finally:
            for c in clients:c.close()
            if process.poll() is None:
                os.killpg(process.pid,signal.SIGTERM)
                process.wait(timeout=5)
            log.close()
    case(4)
    case(6)
    assert sha(binary)==PIN_SHA
    (HERE/'preview-stall-results.json').write_text(json.dumps(results,indent=2)+'\n')
    print(json.dumps(results,indent=2))
if __name__=='__main__':main()
