"""Per-CPU busy share between two /proc/stat snapshots (stdlib). Usage: cpustat.py snap > F; cpustat.py diff F"""
import json, sys
def read():
    d = {}
    for line in open('/proc/stat'):
        if line.startswith('cpu') and line[3] != ' ':
            f = line.split(); v = list(map(int, f[1:])); d[f[0]] = [sum(v), v[3] + v[4]]
    return d
if sys.argv[1] == 'snap':
    print(json.dumps(read()))
else:
    a = json.load(open(sys.argv[2])); b = read()
    print(json.dumps({k: round(100 * (1 - (b[k][1] - a[k][1]) / max(1, b[k][0] - a[k][0])), 1) for k in sorted(a, key=lambda s: int(s[3:]))}))
