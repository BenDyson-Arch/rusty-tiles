#!/usr/bin/env python3
"""Reproduce the finite 0.4 point resource receipt (Linux, no universal cap).

generate SOURCE: extend the repo's deterministic format-3 hill to 10M rows,
with every fourth row's XY compressed by 10 to add a dense region.
measure BINARY SOURCE WORK RECEIPT: child stdout passes through to measure.py;
WORK must be empty. Run inside the owner's idle-priority measurement wrapper.
"""
import argparse
import hashlib
import importlib.util
import json
import mmap
import os
from pathlib import Path
import platform
import resource
import struct
import subprocess
import sys
import time


def sha256(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def generate(path):
    root = Path(__file__).resolve().parents[3]
    spec = importlib.util.spec_from_file_location('corpus', root / 'bench/perf_lane/corpus.py')
    corpus = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(corpus)
    corpus.write_las(path, count=10_000_000)
    with path.open('r+b') as stream, mmap.mmap(stream.fileno(), 0) as data:
        lo, hi = [2**31] * 2, [-2**31] * 2
        for index in range(10_000_000):
            offset = 227 + index * 34
            x, y = struct.unpack_from('<ii', data, offset)
            if index % 4 == 0:
                x, y = x // 10, y // 10
                struct.pack_into('<ii', data, offset, x, y)
            lo = [min(lo[0], x), min(lo[1], y)]
            hi = [max(hi[0], x), max(hi[1], y)]
        for axis in range(2):
            origin = struct.unpack_from('<d', data, 155 + axis * 8)[0]
            struct.pack_into('<dd', data, 179 + axis * 16,
                             origin + hi[axis] * .01, origin + lo[axis] * .01)
    print(json.dumps(dict(source=str(path), sha256=sha256(path), points=10_000_000,
                          bytes=path.stat().st_size)))


def measure(binary, source, work, receipt):
    work.mkdir(parents=True, exist_ok=True)
    if any(work.iterdir()):
        raise ValueError('measurement workspace must be empty')
    output = work / 'cloud.3tz'
    command = [str(binary), '--json', 'point-cloud', '-i', str(source), '-o', str(output),
               '--sourceCrs', 'EPSG:32756', '--heightOffset', '0']
    # Compute identities before the converter starts. Fixture generation and
    # hashing are outside the producer's timer and fresh-child RSS high water.
    identities = dict(binary_sha256=sha256(binary), source_sha256=sha256(source))
    started = time.monotonic()
    child = subprocess.Popen(command, stdout=sys.stdout, stderr=sys.stderr)
    peaks = dict(workspace_and_output_bytes=0, workspace_and_output_allocated_bytes=0,
                 open_descriptors=0, threads=0, sampled_rss_kib=0, sampled_hwm_kib=0,
                 sampled_swap_kib=0)
    samples = 0
    while child.poll() is None:
        logical = allocated = 0
        for path in work.rglob('*'):
            try:
                stat = path.stat()
                if path.is_file():
                    logical += stat.st_size
                    allocated += stat.st_blocks * 512
            except FileNotFoundError:
                pass  # producer legitimately removes a finished partition
        peaks['workspace_and_output_bytes'] = max(peaks['workspace_and_output_bytes'], logical)
        peaks['workspace_and_output_allocated_bytes'] = max(
            peaks['workspace_and_output_allocated_bytes'], allocated)
        try:
            status = dict(line.split(':', 1) for line in Path(f'/proc/{child.pid}/status').read_text().splitlines())
            for key, field in (('sampled_rss_kib', 'VmRSS'), ('sampled_hwm_kib', 'VmHWM'),
                               ('sampled_swap_kib', 'VmSwap'), ('threads', 'Threads')):
                peaks[key] = max(peaks[key], int(status.get(field, '0').split()[0]))
            peaks['open_descriptors'] = max(peaks['open_descriptors'],
                                            len(list(Path(f'/proc/{child.pid}/fd').iterdir())))
        except (FileNotFoundError, ProcessLookupError):
            pass
        samples += 1
        time.sleep(.05)
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    result = dict(**identities, command=command, exit=child.returncode,
                  wall_seconds=time.monotonic() - started, user_seconds=usage.ru_utime,
                  system_seconds=usage.ru_stime, max_rss_kib=usage.ru_maxrss,
                  peaks=peaks, samples=samples, sample_interval_seconds=.05,
                  source_bytes=source.stat().st_size,
                  output_bytes=output.stat().st_size if output.exists() else None,
                  remaining_paths=sorted(str(p.relative_to(work)) for p in work.rglob('*')),
                  source_unchanged=sha256(source) == identities['source_sha256'],
                  platform=platform.platform(), cpus=sorted(os.sched_getaffinity(0)),
                  worker_pool=False, environment={k: os.environ.get(k) for k in
                      ('RT_MEASURE_SLOT', 'RAYON_NUM_THREADS', 'TMPDIR')})
    receipt.write_text(json.dumps(result, indent=2) + '\n')
    if child.returncode:
        raise SystemExit(child.returncode)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest='operation', required=True)
    generation = subparsers.add_parser('generate')
    generation.add_argument('source', type=Path)
    measurement = subparsers.add_parser('measure')
    for arg in ('binary', 'source', 'work', 'receipt'):
        measurement.add_argument(arg, type=Path)
    args = parser.parse_args()
    if args.operation == 'generate':
        generate(args.source.resolve())
    else:
        measure(*(getattr(args, arg).resolve() for arg in ('binary', 'source', 'work', 'receipt')))


if __name__ == '__main__':
    main()
