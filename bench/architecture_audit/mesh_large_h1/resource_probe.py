import argparse, hashlib, json, os, pathlib, time

def digest(path):
    h=hashlib.sha256()
    with pathlib.Path(path).open('rb') as f:
        while block:=f.read(1<<20): h.update(block)
    return h.hexdigest()

def allocated(parent):
    total=0
    for root, _, files in os.walk(parent):
        for path in [pathlib.Path(root)]+[pathlib.Path(root)/name for name in files]:
            try: total+=path.stat().st_blocks*512
            except FileNotFoundError: pass
    return total

p=argparse.ArgumentParser()
p.add_argument('--binary',required=True)
p.add_argument('--input',required=True)
p.add_argument('--output',required=True)
p.add_argument('--scratch',required=True)
p.add_argument('--receipt',required=True)
p.add_argument('--revision',required=True)
a=p.parse_args()
output=pathlib.Path(a.output)
scratch=pathlib.Path(a.scratch)
assert not output.exists()
assert scratch.is_dir() and not list(scratch.iterdir())
inputs={str(path):digest(path) for path in pathlib.Path(a.input).parent.iterdir() if path.is_file()}
binary_hash=digest(a.binary)
env=dict(os.environ,PATH='',RAYON_NUM_THREADS='2',RUST_TEST_THREADS='2',
    H1_INPUT=a.input,H1_OUTPUT=a.output,H1_SCRATCH=a.scratch,H1_JOBS='2',
    H1_RUN_BYTES=str(64<<20),H1_FAN_IN='8',H1_BUFFER_BYTES=str(128<<10),H1_PROFILE='quality')
command=['/usr/bin/taskset','-c','4,5','/usr/bin/nice','-n','10',a.binary,
    '--exact','mesh_large::tests::external_driver','--ignored','--nocapture']
log=pathlib.Path(a.receipt).with_suffix('.log')
start=time.monotonic()
sampled_physical=0
with log.open('wb') as stream:
    pid=os.posix_spawn(command[0],command,env,file_actions=[
        (os.POSIX_SPAWN_DUP2,stream.fileno(),1),(os.POSIX_SPAWN_DUP2,stream.fileno(),2)])
    while True:
        reaped,status,usage=os.wait4(pid,os.WNOHANG)
        if reaped: break
        sampled_physical=max(sampled_physical,allocated(scratch))
        time.sleep(.1)
elapsed=time.monotonic()-start
code=os.waitstatus_to_exitcode(status)
report=json.loads((output/'report.json').read_text()) if code==0 else None
unchanged=binary_hash==digest(a.binary) and all(digest(path)==value for path,value in inputs.items())
receipt={'source_revision':a.revision,'binary_sha256':binary_hash,'runner_sha256':digest(__file__),
    'inputs_sha256':inputs,'config':{k:v for k,v in env.items() if k.startswith('H1_')},
    'command':command,'exit_code':code,'elapsed_seconds':elapsed,'cpu_seconds':usage.ru_utime+usage.ru_stime,
    'peak_rss_bytes':usage.ru_maxrss*1024,'sampled_physical_bytes':sampled_physical,
    'physical_measurement_limit':'periodic non-atomic allocation estimate; neither an atomic peak nor a rigorous lower bound; stable stopped witness is separate',
    'unchanged_inputs_and_binary':unchanged,'scratch_cleanup_entries':len(list(scratch.iterdir())),
    'producer_report':report}
if report:
    receipt['rss_within_declared_bound']=receipt['peak_rss_bytes']<=report['capture']['memory']['bound_bytes']
    receipt['logical_within_declared_bound']=report['capture']['scratch_logical_peak_bytes']<=report['capture']['scratch']['logical_need_bytes']
    receipt['observed_physical_within_allowance']=sampled_physical<=report['capture']['scratch']['need_bytes']
pathlib.Path(a.receipt).write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps({k:v for k,v in receipt.items() if k not in ['inputs_sha256','producer_report','command','config']},indent=2))
assert code==0 and unchanged and not list(scratch.iterdir())
assert all(receipt[k] for k in ['rss_within_declared_bound','logical_within_declared_bound','observed_physical_within_allowance'])
