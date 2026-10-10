#!/usr/bin/env python3
"""Independent bounded stored-3TZ catalog scaling; actual Linux CLI wait4 costs.

Coordinator schedules execution after builds. No product imports, ZIP library,
builds or installations. Whole-C1 peak RSS includes semantic work; it is not a
standalone-reader allocation bound. Tiny Rust limit tests must be run separately
by the Cargo owner; this driver does not substitute Python models for them.
"""
import argparse
import binascii
import hashlib
import json
import os
from pathlib import Path
import signal
import struct
import time

INDEX = b'@3dtilesIndex1@'
MAX_ENTRIES = 65536
MANIFEST = json.dumps({'asset': {'version': '1.1'}, 'geometricError': 0,
                      'root': {'boundingVolume': {'box': [0,0,0,1,0,0,0,1,0,0,0,1]},
                               'geometricError': 0, 'refine': 'REPLACE'}},
                     separators=(',', ':')).encode()


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(65536), b''):
            h.update(block)
    return h.hexdigest()


def identity(path):
    st = path.stat()
    return {'device': st.st_dev, 'inode': st.st_ino, 'bytes': st.st_size,
            'mtime_ns': st.st_mtime_ns, 'sha256': digest(path)}


def write_archive(path, total_entries):
    """Literal stored records, sorted raw-MD5 24-byte index, ZIP64 count end."""
    assert 2 <= total_entries <= MAX_ENTRIES + 1
    rows = []
    with path.open('xb') as f:
        def member(name, data):
            at = f.tell()
            crc = binascii.crc32(data) & 0xffffffff
            size = len(data)
            f.write(struct.pack('<4s5H3I2H', b'PK\x03\x04', 20, 0, 0, 0, 0,
                                crc, size, size, len(name), 0))
            f.write(name)
            f.write(data)
            rows.append((name, at, size, crc))
        member(b'tileset.json', MANIFEST)
        for i in range(total_entries - 2):
            member(('opaque/m%05d.bin' % i).encode(), b'')
        names = [(hashlib.md5(name).digest(), at) for name, at, _, _ in rows]
        names.sort(key=lambda x: struct.unpack('<QQ', x[0]))
        index = b''.join(h + struct.pack('<Q', at) for h, at in names)
        member(INDEX, index)
        cd_at = f.tell()
        for name, at, size, crc in rows:
            f.write(struct.pack('<4s6H3I5H2I', b'PK\x01\x02', 20, 20, 0, 0, 0, 0,
                                crc, size, size, len(name), 0, 0, 0, 0, 0, at))
            f.write(name)
        cd_size = f.tell() - cd_at
        if total_entries >= 0xffff:
            z_at = f.tell()
            f.write(struct.pack('<4sQ2H2I4Q', b'PK\x06\x06', 44, 45, 45, 0, 0,
                                total_entries, total_entries, cd_size, cd_at))
            f.write(struct.pack('<4sIQI', b'PK\x06\x07', 0, z_at, 1))
            f.write(struct.pack('<4s4H2IH', b'PK\x05\x06', 0, 0, 0xffff, 0xffff,
                                0xffffffff, 0xffffffff, 0))
        else:
            f.write(struct.pack('<4s4H2IH', b'PK\x05\x06', 0, 0, total_entries,
                                total_entries, cd_size, cd_at, 0))
    return {'entries_including_index': total_entries, 'central_offset': cd_at,
            'central_bytes': cd_size, 'index_bytes': len(index),
            'aggregate_raw_name_bytes': sum(len(x[0]) for x in rows),
            'archive_stored_bytes': len(MANIFEST) + len(index),
            'ordinary_empty_members': total_entries - 2,
            'index_order': 'MD5 digest interpreted as (little-endian u64, u64)',
            'sha256': digest(path), 'archive_bytes': path.stat().st_size}


def run_cli(binary, archive, timeout, out, err):
    """One fresh, reaped process; wait4 returns this child's Linux ru_maxrss."""
    command = [str(binary), 'validate', '--json', str(archive)]
    started = time.monotonic()
    # glibc Linux posix_spawn uses a vfork-style child. Avoid inheriting the
    # Python fixture builder's address-space RSS as a fork-before-exec floor.
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    actions = [(os.POSIX_SPAWN_OPEN, 1, str(out), flags, 0o600),
               (os.POSIX_SPAWN_OPEN, 2, str(err), flags, 0o600)]
    pid = os.posix_spawn(str(binary), command, dict(os.environ, RAYON_NUM_THREADS='2'),
                         file_actions=actions, setsid=True)
    timed_out = False
    try:
        while True:
            got, status, usage = os.wait4(pid, os.WNOHANG)
            if got:
                break
            if time.monotonic() - started >= timeout:
                timed_out = True
                os.killpg(pid, signal.SIGKILL)
                _, status, usage = os.wait4(pid, 0)
                break
            time.sleep(0.02)
    except BaseException:
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.wait4(pid, 0)
        raise
    elapsed = time.monotonic() - started
    stdout = out.read_bytes()
    stderr = err.read_bytes()
    parsed = None
    try:
        parsed = json.loads(stdout)
    except (ValueError, UnicodeDecodeError):
        pass
    code = os.waitstatus_to_exitcode(status)
    category = 'admitted' if code == 0 and isinstance(parsed, dict) and parsed.get('ok') else (
        parsed.get('error', {}).get('code') if isinstance(parsed, dict) else None)
    message = parsed.get('error', {}).get('message', '') if isinstance(parsed, dict) else ''
    return {'command': command, 'exit_code': code, 'timed_out': timed_out,
            'elapsed_seconds': elapsed, 'user_seconds': usage.ru_utime,
            'system_seconds': usage.ru_stime, 'max_rss_kib_linux_wait4': usage.ru_maxrss,
            'category': category, 'error_message_prefix': message[:240],
            'semantic_unused_gate_reached': message.startswith('unreferenced archive entries:'),
            'stdout_path': str(out), 'stdout_bytes': len(stdout), 'stdout_sha256': digest(out),
            'stderr_path': str(err), 'stderr_bytes': len(stderr), 'stderr_sha256': digest(err),
            'success_limits': parsed.get('limits') if category == 'admitted' else None}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--work', required=True, type=Path)
    p.add_argument('--binary', required=True, type=Path)
    p.add_argument('--binary-sha256', required=True)
    p.add_argument('--source-pin', required=True, type=Path)
    p.add_argument('--source-pin-sha256', required=True)
    p.add_argument('--timeout', type=float, default=45)
    args = p.parse_args()
    assert os.name == 'posix' and hasattr(os, 'wait4') and hasattr(os, 'sched_getaffinity')
    assert 0 < args.timeout <= 60
    affinity = sorted(os.sched_getaffinity(0))
    assert 1 <= len(affinity) <= 2, 'Coordinator must invoke with at most two taskset CPUs'
    inherited_nice = os.getpriority(os.PRIO_PROCESS, 0)
    assert inherited_nice >= 10, 'Coordinator must invoke at nice 10 or higher'
    binary = args.binary.resolve()
    assert digest(binary) == args.binary_sha256, 'Candidate binary pin mismatch'
    assert digest(args.source_pin) == args.source_pin_sha256, 'Coordinator source pin mismatch'
    source_pin = json.loads(args.source_pin.read_bytes())
    work = args.work.resolve()
    assert work.parent == Path('/tmp') and work.name.startswith('rusty-tiles-archive-catalog-')
    work.mkdir(mode=0o700)  # Require fresh directory; never overwrite prior artifacts.
    receipt = {'probe_sha256': digest(Path(__file__)), 'binary_path': str(binary),
               'binary_sha256': args.binary_sha256, 'coordinator_source_pin': source_pin,
               'coordinator_source_pin_sha256': args.source_pin_sha256,
               'method': {'affinity_cpus': affinity, 'rayon_threads': 2, 'inherited_child_nice': inherited_nice,
                          'spawn': 'Linux glibc posix_spawn, separate session, no Python fork RSS floor',
                          'measurement': 'fresh Linux child wait4 ru_maxrss in KiB, whole C1',
                          'serial_processes': True, 'timeout_seconds_per_child': args.timeout,
                          'default_entries_cap': MAX_ENTRIES,
                          'semantic_gate_evidence': 'unreferenced message implies catalog/index and full member hash loop completed, based on reviewed C1 control flow; no instrumented stage claim',
                          'rust_small_boundary_test': 'archive3tz::read::tests::source_directory_count_member_and_total_limits_are_independent; coordinator must execute actual Rust test',
                          'limits': 'No universal RAM acceptance, standalone-facade RSS, source stability under concurrent replacement, or instrumented I/O/allocation claim'},
               'records': [], 'failures': []}
    for count in [2, 3, 1024, MAX_ENTRIES, MAX_ENTRIES + 1]:
        archive = work / ('entries-%d.3tz' % count)
        layout = write_archive(archive, count)
        before = identity(archive)
        initial_names = sorted(x.name for x in work.iterdir())
        out = work / ('entries-%d.stdout.json' % count)
        err = work / ('entries-%d.stderr.txt' % count)
        result = run_cli(binary, archive, args.timeout, out, err)
        after = identity(archive)
        new_names = sorted(set(x.name for x in work.iterdir()) - set(initial_names))
        expected = 'admitted' if count == 2 else ('resource_limit' if count > MAX_ENTRIES else 'invalid_input')
        passed = (result['category'] == expected and not result['timed_out'] and before == after
                  and new_names == sorted([out.name, err.name])
                  and (count in [2, MAX_ENTRIES + 1] or result['semantic_unused_gate_reached'])
                  and (count != 2 or result['success_limits'].get('archiveEntries') == MAX_ENTRIES))
        record = {'layout': layout, 'source_before': before, 'source_after': after,
                  'expected_category': expected, 'cli': result, 'only_expected_output_files_created':
                  new_names == sorted([out.name, err.name]), 'passed': passed}
        receipt['records'].append(record)
        if not passed:
            receipt['failures'].append(count)
        (work / 'receipt.json').write_text(json.dumps(receipt, indent=2, sort_keys=True) + '\n')
        print(json.dumps({'entries': count, 'expected': expected, 'actual': result['category'],
                          'rss_kib': result['max_rss_kib_linux_wait4'], 'elapsed': result['elapsed_seconds'],
                          'semantic_unused_gate_reached': result['semantic_unused_gate_reached'],
                          'passed': passed}), flush=True)
    assert not receipt['failures'], 'Catalog resource candidate acceptance failed; see receipt'


if __name__ == '__main__':
    main()
