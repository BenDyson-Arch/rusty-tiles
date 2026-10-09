#!/usr/bin/env python3
"""Linux F1a boundary evidence. RSS uses wait4; FD/thread/scratch peaks are sampled."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('f1a_oracle', ROOT / 'tests/f1a_oracle.py')
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)


def fixture(kind):
    if kind in ('geometry', 'leaves'):
        return oracle.fixture(100000), 100000 if kind == 'geometry' else 25, True
    if kind == 'rejected-leaves':
        return oracle.fixture(4097, variant='coincident'), 1, False
    if kind == 'rejected-geometry':
        return oracle.fixture(100001), 100000, False
    raw = oracle.fixture(100000 if kind == 'combined' else 8)
    doc, binary = oracle.decode_glb(raw)
    if kind in ('json-bytes', 'combined'):
        doc['asset']['generator'] = 'x' * (1024 * 1024 - 2048)
    if kind in ('source-bytes', 'combined'):
        # Find the largest aligned embedded BIN that fits the32MiB envelope.
        target = 32 * 1024 * 1024
        binary += b'\0' * (target - len(oracle.encode_glb(doc, binary)) - 32)
        while len(oracle.encode_glb(doc, binary)) + 4 <= target:
            binary += b'\0' * 4
    return oracle.encode_glb(doc, binary), 25 if kind == 'combined' else 3, True


def measure(binary, kind, root, launcher):
    work = root / kind
    work.mkdir()
    source, limit, succeeds = fixture(kind)
    input_path = work / 'source.glb'
    input_path.write_bytes(source)
    output = work / 'result.3tz'
    timing = work / 'peak-rss.txt'
    command = [str(binary), '--json', 'mesh-local-to-3tz', '-i', str(input_path),
               '-o', str(output), '--leaf-triangles', str(limit)]
    started = time.monotonic()
    with (work / 'stdout.json').open('w') as stdout, (work / 'stderr.txt').open('w') as stderr:
        child = subprocess.Popen([str(launcher), str(timing), *command], stdout=stdout, stderr=stderr)
        maxima = dict(file_descriptors=0, threads=0, scratch_files=0, scratch_bytes=0)
        while child.poll() is None:
            try:
                children = Path(f'/proc/{child.pid}/task/{child.pid}/children').read_text().split()
                for pid in children:
                    status = Path(f'/proc/{pid}/status').read_text().splitlines()
                    threads = next(int(line.split()[1]) for line in status if line.startswith('Threads:'))
                    maxima['threads'] = max(maxima['threads'], threads)
                    maxima['file_descriptors'] = max(maxima['file_descriptors'], len(list(Path(f'/proc/{pid}/fd').iterdir())))
                paths = [p for p in work.glob('.mesh-work-*/**/*') if p.is_file()]
                maxima['scratch_files'] = max(maxima['scratch_files'], len(paths))
                maxima['scratch_bytes'] = max(maxima['scratch_bytes'], sum(p.stat().st_size for p in paths))
            except (FileNotFoundError, ProcessLookupError, PermissionError):
                pass  # A sampled object can finish between inspection calls.
            time.sleep(0.002)
    payload = json.loads((work / 'stdout.json').read_text())
    assert (child.returncode == 0) == succeeds, (kind, child.returncode, payload)
    assert output.exists() == succeeds
    assert not list(work.glob('.mesh-work-*'))
    if not succeeds:
        assert 'unsupported' in json.dumps(payload).lower(), payload
    return dict(case=kind, input_bytes=len(source), leaf_triangles=limit,
                exit_code=child.returncode, elapsed_seconds=time.monotonic()-started,
                peak_rss_kib=int(timing.read_text()), sampled_peaks=maxima,
                output_bytes=output.stat().st_size if succeeds else 0, result=payload)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='rusty-f1a-resources-') as work:
        launcher = Path(work) / 'measure-child'
        subprocess.run(['cc', '-O2', '-Wall', '-Wextra', '-Werror', str(Path(__file__).with_name('measure_child.c')), '-o', str(launcher)], check=True)
        results = [measure(args.binary.resolve(), case, Path(work), launcher) for case in
                   ['small', 'geometry', 'leaves', 'source-bytes', 'json-bytes', 'combined', 'rejected-leaves', 'rejected-geometry']]
    report = dict(binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(), platform=platform.platform(), python=sys.version, cpu_count=os.cpu_count(),
                  caveat='Linux wait4 process peak RSS; descriptors, threads and scratch sampled at >=2ms, not proven maxima. Serial materialization, no constant-memory claim.', results=results)
    args.output.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
