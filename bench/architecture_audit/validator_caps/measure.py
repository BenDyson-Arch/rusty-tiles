"""Run `validate --json` under `taskset -c 0,1 nice -n 10` and record wait4 costs.

Usage: python3 -I measure.py --binary BIN --label L --runs 3 --out RESULT.json ARCHIVE...

Each run is one fresh child of a small launcher interpreter; taskset and nice
exec into the CLI, so wait4's ru_maxrss is the CLI's peak RSS (the launcher's
own ~10 MiB inherited high-water mark is below the CLI's ~28 MiB floor). Results append to RESULT.json.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time


def sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


# A vfork/fork child inherits its parent's RSS high-water mark at exec, so
# ru_maxrss would carry this (report-parsing) process's RSS as a floor. A fresh
# small interpreter spawns and reaps the CLI instead and prints its wait4 usage.
LAUNCHER = """
import json, os, sys, time
out, err, command = sys.argv[1], sys.argv[2], sys.argv[3:]
flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
actions = [(os.POSIX_SPAWN_OPEN, 1, out, flags, 0o600), (os.POSIX_SPAWN_OPEN, 2, err, flags, 0o600)]
started = time.monotonic()
pid = os.posix_spawnp(command[0], command, dict(os.environ, RAYON_NUM_THREADS='2'), file_actions=actions)
_, status, usage = os.wait4(pid, 0)
print(json.dumps([os.waitstatus_to_exitcode(status), time.monotonic() - started,
                  usage.ru_utime, usage.ru_stime, usage.ru_maxrss]))
"""


def run(binary, archive, work):
    out, err = work / 'stdout', work / 'stderr'
    command = ['taskset', '-c', '0,1', 'nice', '-n', '10', str(binary), 'validate', '--json', str(archive)]
    for path in (out, err):
        path.unlink(missing_ok=True)
    launched = subprocess.run([sys.executable, '-I', '-c', LAUNCHER, str(out), str(err), *command],
                              capture_output=True, text=True, check=True)
    returncode, elapsed, user, system, maxrss = json.loads(launched.stdout)
    stdout, stderr = out.read_bytes(), err.read_bytes()
    try:
        parsed = json.loads(stdout)
    except ValueError:
        parsed = {}
    error = parsed.get('error') or {}
    return {'exitCode': returncode, 'elapsedSeconds': round(elapsed, 4),
            'userSeconds': round(user, 4), 'systemSeconds': round(system, 4),
            'maxRssKiB': maxrss, 'ok': parsed.get('ok'),
            'tiles': parsed.get('tiles'), 'entries': parsed.get('entries'),
            'contentReferences': parsed.get('contentReferences'),
            'contained': ('decodedContentContainment' in parsed.get('checks', [])) if parsed.get('ok') else None,
            'errorCode': error.get('code'), 'errorMessage': error.get('message'),
            'probe': [json.loads(line) for line in stderr.decode(errors='replace').splitlines()
                      if line.startswith('{"vcapsProbe"')],
            'stdoutSha256': hashlib.sha256(stdout).hexdigest()}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--label', required=True)
    p.add_argument('--runs', type=int, default=3)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--work', type=Path, default=Path('/home/bend/.cache/vcaps/work/measure'))
    p.add_argument('archives', nargs='+', type=Path)
    a = p.parse_args()
    a.work.mkdir(parents=True, exist_ok=True)
    results = json.loads(a.out.read_text()) if a.out.exists() else []
    binary_sha = sha(a.binary)
    for archive in a.archives:
        runs = [run(a.binary, archive, a.work) for _ in range(a.runs)]
        row = {'label': a.label, 'binary': str(a.binary), 'binarySha256': binary_sha,
               'archive': archive.name, 'archiveSha256': sha(archive), 'archiveBytes': archive.stat().st_size,
               'loadavg': os.getloadavg(),
               'medianElapsedSeconds': statistics.median(r['elapsedSeconds'] for r in runs),
               'medianUserSeconds': statistics.median(r['userSeconds'] for r in runs),
               'maxRssKiB': max(r['maxRssKiB'] for r in runs), 'runs': runs}
        results.append(row)
        a.out.write_text(json.dumps(results, indent=1) + '\n')
        last = runs[-1]
        print(json.dumps({k: row[k] for k in ('label', 'archive', 'medianElapsedSeconds', 'maxRssKiB')}
                         | {k: last[k] for k in ('exitCode', 'ok', 'tiles', 'entries', 'contained', 'errorCode',
                                                 'errorMessage', 'probe')}), flush=True)


if __name__ == '__main__':
    main()
