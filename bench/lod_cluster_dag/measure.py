#!/usr/bin/env python3
"""Portable wait4 CPU/RSS + sampled scratch census (GNU time is absent here)."""
import json
import os
from pathlib import Path
import sys
import time

report, scratch, *command = sys.argv[1:]
start = time.monotonic()
pid = os.fork()
if pid == 0:
    os.execvp(command[0], command)
peak = 0
while True:
    peak = max(peak, sum(p.stat().st_size for p in Path(scratch).rglob('*') if p.is_file()))
    result, status, usage = os.wait4(pid, os.WNOHANG)
    if result: break
    time.sleep(.25)
peak = max(peak, sum(p.stat().st_size for p in Path(scratch).rglob('*') if p.is_file()))
data = dict(exit=os.waitstatus_to_exitcode(status), user_s=usage.ru_utime, sys_s=usage.ru_stime,
            wall_s=time.monotonic()-start, max_rss_kib=usage.ru_maxrss, peak_scratch_bytes=peak,
            scratch_poll_seconds=.25, command=command)
Path(report).write_text(json.dumps(data, indent=2))
sys.exit(data['exit'])
