"""Timing helper shared by the native benchmarks."""
import json
import subprocess
import sys


def measured(command, log, environment):
    # A lightweight worker prevents the fixture generator's NumPy/GDAL memory
    # from becoming an inherited RSS floor in the measured child at exec.
    worker = """
import json, os, sys, time
command = sys.argv[2:]
start = time.perf_counter()
with open(sys.argv[1], 'wb') as output:
    pid = os.posix_spawn(command[0], command, dict(os.environ), file_actions=[
        (os.POSIX_SPAWN_DUP2, output.fileno(), 1),
        (os.POSIX_SPAWN_DUP2, output.fileno(), 2)])
    _, status, usage = os.wait4(pid, 0)
elapsed = time.perf_counter() - start
if os.waitstatus_to_exitcode(status):
    sys.exit(os.waitstatus_to_exitcode(status))
print(json.dumps(dict(wallSeconds=elapsed, cpuSeconds=usage.ru_utime+usage.ru_stime,
    peakRssMiB=usage.ru_maxrss/1024)))
"""
    result = subprocess.run([sys.executable, '-c', worker, str(log), *command],
        env=environment, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f'Conversion failed; see {log}: {result.stderr}')
    return json.loads(result.stdout)
