"""Measurement launcher: fork + exec CMD, then write CMD's own wait4 rusage (and, inside a
cgroup scope, the scope's memory.peak) to OUT.json. Exits with CMD's status.

  python3 -I -S launch.py OUT.json CMD [ARGS...]

Linux keeps a process's resident high-water mark across exec, so a measured command started
straight from a large Python runner inherits the runner's RSS as a floor. Run from this small
interpreter (-I -S, stdlib os/sys/json only) the floor is the launcher's few MiB; run_case.py
records that floor by launching /bin/true the same way.
"""
import json, os, sys


def main():
    out, argv = sys.argv[1], sys.argv[2:]
    pid = os.fork()
    if pid == 0:
        try:
            os.execv(argv[0], argv)
        finally:
            os._exit(127)
    while True:
        try:
            _, status, ru = os.wait4(pid, 0)
            break
        except InterruptedError:
            continue
    record = dict(maxrss_bytes=ru.ru_maxrss * 1024, utime=ru.ru_utime, stime=ru.ru_stime,
                  exit_code=os.waitstatus_to_exitcode(status))
    try:
        with open('/proc/self/cgroup') as f:
            group = f.read().strip().splitlines()[0].split('::', 1)[1]
        with open(f'/sys/fs/cgroup{group}/memory.peak') as f:
            record['cgroup'] = group
            record['cgroup_memory_peak_bytes'] = int(f.read().split()[0])
    except (OSError, IndexError, ValueError):
        pass
    with open(out, 'w') as f:
        json.dump(record, f)
    code = record['exit_code']
    sys.exit(code if code >= 0 else 128 - code)


if __name__ == '__main__':
    main()
