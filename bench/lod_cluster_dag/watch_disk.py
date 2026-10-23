#!/usr/bin/env python3
"""Own one child group; terminate only that group if free disk drops below 60 GiB."""
import json,os,signal,subprocess,sys,time
from pathlib import Path
root=Path(sys.argv[1]);root.mkdir(parents=True,exist_ok=True)
limit=60*1024**3
free=lambda:os.statvfs('/').f_bavail*os.statvfs('/').f_frsize
if free()<limit:raise SystemExit('insufficient free disk before launch')
p=subprocess.Popen(sys.argv[2:],start_new_session=True);rows=[];stopped=False
try:
 while p.poll() is None:
  available=free();rows.append({'time':time.time(),'free_bytes':available})
  if available<limit:
   stopped=True;os.killpg(p.pid,signal.SIGTERM)
   try:p.wait(timeout=5)
   except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
   break
  time.sleep(2)
finally:
 (root/'disk-watchdog.json').write_text(json.dumps(dict(killed_for_low_disk=stopped,minimum_free_bytes=min(r['free_bytes'] for r in rows) if rows else free(),samples=rows),indent=2))
raise SystemExit(75 if stopped else p.wait())
