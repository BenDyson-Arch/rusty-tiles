#!/usr/bin/env python3
"""Frozen old/fixed transport comparison and independently framed HTTP probes."""
import hashlib,json,os,selectors,signal,socket,subprocess,tempfile,time
from pathlib import Path

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[3]
OLD_SHA='bef6518c234783caaa41a1d4c416e22dab2001d8067de379f537864cf61d503e'
NEW_SHA='a053b3ac570b13edd4cc56e937d10b1605325788382f78bce4a51346cc42a0ea'
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def collect(clients,duration):
    received={i:b'' for i in range(len(clients))}
    end=time.monotonic()+duration
    with selectors.DefaultSelector() as selected:
        for i,c in enumerate(clients):c.setblocking(False);selected.register(c,selectors.EVENT_READ,i)
        while time.monotonic()<end:
            for key,_ in selected.select(max(0,end-time.monotonic())):
                try:data=key.fileobj.recv(65536)
                except BlockingIOError:continue
                if data:received[key.data]+=data
                else:selected.unregister(key.fileobj)
    return received
def decode(raw):
    head,body=raw.split(b'\r\n\r\n',1)
    lines=head.decode().split('\r\n')
    headers=dict((k.lower(),v.strip()) for k,v in (line.split(':',1) for line in lines[1:]))
    assert lines[0].startswith('HTTP/1.1 200 '),head
    assert len(body)==int(headers['content-length'])
    assert headers['cache-control']=='no-cache'
    assert json.loads(body)=={'mesh':'/mesh/tileset.json'}
    return len(body)
def request(c,port,path='/config.json',method='GET'):
    c.settimeout(3)
    c.sendall(f'{method} {path} HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: keep-alive\r\n\r\n'.encode())
def main():
    folder=Path('/home/bend/.cache/rusty-tiles-f1c1-evidence')
    fixed=json.loads((folder/'preview-fix-build-manifest.json').read_text())
    old=json.loads((folder/'build-manifest.json').read_text())
    binary=Path(fixed['portable_binary']['path']);oldbinary=Path(old['native_binary']['path'])
    assert sha(binary)==fixed['portable_binary']['sha256']==NEW_SHA
    assert sha(oldbinary)==old['native_binary']['sha256']==OLD_SHA
    pin=fixed['source_commit']
    assert pin=='5e4dbc30ef1de1f9231e5caf323ac72c52fffdd5'
    for name,digest in fixed['compiled_inputs_sha256'].items():
        assert sha(ROOT/name)==digest,(name,'worktree drift')
        assert hashlib.sha256(subprocess.check_output(['git','show',f'{pin}:{name}'],cwd=ROOT)).hexdigest()==digest,(name,'Git drift')
    cache=Path(tempfile.mkdtemp(prefix='rusty-tiles-preview-fixed-review-',dir='/home/bend/.cache'))
    (cache/'cesium').mkdir();(cache/'cesium/Cesium.js').write_text('// literal transport fixture')
    (cache/'mesh').mkdir();(cache/'mesh/tileset.json').write_text('{}')
    payload=bytes(i%251 for i in range(196608));(cache/'mesh/streamed.glb').write_bytes(payload)
    with (cache/'mesh/backpressure.glb').open('wb') as out:out.truncate(64*1024*1024)
    receipt={'source_commit':pin,'compiled_inputs_verified':len(fixed['compiled_inputs_sha256']),'binary_sha256':NEW_SHA,'old_control_binary_sha256':OLD_SHA,'probe_sha256':sha(Path(__file__)),'cache':str(cache),'executions':[]}
    def serve(executable,name):
        log=(cache/(name+'.stderr')).open('wb')
        p=subprocess.Popen([str(executable),'preview','--cesium',str(cache/'cesium'),'--mesh',str(cache/'mesh'),'--port','0','--json'],stdout=subprocess.PIPE,stderr=log,text=True,start_new_session=True,env=dict(os.environ,PATH=''))
        os.sched_setaffinity(p.pid,{min(os.sched_getaffinity(0))})
        with selectors.DefaultSelector() as ready:
            ready.register(p.stdout,selectors.EVENT_READ);assert ready.select(10)
            startup=json.loads(p.stdout.readline())
        return p,log,int(startup['url'].split(':')[-1].rstrip('/'))
    def close(p,log,clients):
        for c in clients:c.close()
        if p.poll() is None:os.killpg(p.pid,signal.SIGTERM);p.wait(timeout=5)
        log.close()
    for name,executable,expected in [('old-sensitive-control',oldbinary,4),('fixed-held-open-burst',binary,6)]:
        p,log,port=serve(executable,name);clients=[]
        try:
            time.sleep(.05)
            os.kill(p.pid,signal.SIGSTOP);os.waitpid(p.pid,os.WUNTRACED)
            try:
                for _ in range(6):
                    c=socket.create_connection(('127.0.0.1',port),timeout=2);clients.append(c);request(c,port)
            finally:os.kill(p.pid,signal.SIGCONT)
            raw=collect(clients,.5)
            complete=[i for i,v in raw.items() if v]
            for i in complete:decode(raw[i])
            assert len(complete)==expected,(name,complete)
            record={'name':name,'complete_while_all_six_keepalives_held':complete,'matches_exact_complete_requirement':len(complete)==6,'phase_seconds':.5}
            if expected==6:
                for c in clients:request(c,port)
                reused=collect(clients,.5)
                for value in reused.values():decode(value)
                record['all_six_connections_reused_without_close']=True
            else:
                clients[complete[0]].close()
                pending=[i for i,v in raw.items() if not v]
                active=[c for c in clients if c.fileno()>=0]
                progressed=collect(active,.5)
                assert any(progressed.values())
                record['pending_before_release']=pending
                record['response_released_by_closing_one_served_connection']=True
            receipt['executions'].append(record)
        finally:close(p,log,clients)
    p,log,port=serve(binary,'fixed-idle-partial-backpressure');clients=[]
    try:
        for _ in range(8):clients.append(socket.create_connection(('127.0.0.1',port),timeout=2))
        for _ in range(8):
            c=socket.create_connection(('127.0.0.1',port),timeout=2);c.sendall(b'GET /config.json HTTP/1.1\r\nHost:');clients.append(c)
        blocked=[]
        for _ in range(4):
            c=socket.create_connection(('127.0.0.1',port),timeout=2);c.setsockopt(socket.SOL_SOCKET,socket.SO_RCVBUF,4096);request(c,port,'/mesh/backpressure.glb');blocked.append(c);clients.append(c)
        for c in blocked:c.settimeout(3);assert c.recv(1,socket.MSG_PEEK)
        live=[]
        for _ in range(16):
            c=socket.create_connection(('127.0.0.1',port),timeout=2);request(c,port);live.append(c);clients.append(c)
        responses=collect(live,.5)
        for value in responses.values():decode(value)
        for c in live:request(c,port)
        responses=collect(live,.5)
        for value in responses.values():decode(value)
        # Directly frame pipelined file GET/HEAD/GET on a reused socket.
        c=live[0];c.settimeout(3);reader=c.makefile('rb')
        try:
            for method in ('GET','HEAD','GET'):request(c,port,'/mesh/streamed.glb',method)
            for method in ('GET','HEAD','GET'):
                status=reader.readline();assert status.startswith(b'HTTP/1.1 200 ')
                headers={}
                while True:
                    line=reader.readline();assert line
                    if line==b'\r\n':break
                    k,v=line.decode().split(':',1);headers[k.lower()]=v.strip()
                assert int(headers['content-length'])==len(payload)
                assert headers['content-type']=='model/gltf-binary'
                body=b'' if method=='HEAD' else reader.read(len(payload))
                assert body==(b'' if method=='HEAD' else payload)
        finally:reader.close()
        receipt['executions'].append({'name':'fixed-idle-partial-backpressure','idle_clients':8,'partial_clients':8,'unread_64MiB_receivers':4,'receiver_buffers_bytes':[c.getsockopt(socket.SOL_SOCKET,socket.SO_RCVBUF) for c in blocked],'complete_clients':16,'reused_clients':16,'pipelined_streamed_GET_HEAD_GET':'passed','all_other_clients_held_open':True})
    finally:close(p,log,clients)
    assert sha(binary)==NEW_SHA and sha(oldbinary)==OLD_SHA
    receipt['status']='passed'
    receipt['limits']=['Linux frozen portable artifact; no browser/native/hosted-platform execution in this probe.','Finite client/stream workload establishes progress here, not universal resource or liveness bounds.','Pinned old control is native and fixed artifact is portable, but preview is feature-independent and both previews use the same literal fixtures/HTTP sequence.']
    (HERE/'preview-fixed-results.json').write_text(json.dumps(receipt,indent=2)+'\n')
    print(json.dumps({'status':receipt['status'],'executions':receipt['executions']}))
if __name__=='__main__':main()
