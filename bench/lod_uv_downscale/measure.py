"""External process resource record, independent of instrumented build timer."""
import subprocess,resource,time,json,sys
start=time.monotonic();p=subprocess.Popen(sys.argv[2:]);_,status,usage=__import__('os').wait4(p.pid,0)
r=dict(exit=__import__('os').waitstatus_to_exitcode(status),wall_s=time.monotonic()-start,user_s=usage.ru_utime,sys_s=usage.ru_stime,max_rss_bytes=usage.ru_maxrss*1024)
open(sys.argv[1],'w').write(json.dumps(r,indent=2));sys.exit(r['exit'])
