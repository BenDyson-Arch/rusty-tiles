import sys, json, time
def r():
    d = {}
    for l in open('/proc/stat'):
        if l.startswith('cpu') and l[3] != ' ':
            f = l.split(); v = list(map(int, f[1:])); d[f[0][3:]] = (sum(v), v[3] + v[4])
    return d
if sys.argv[1] == 'snap':
    print(json.dumps({'t': time.time(), 'cpu': r()}))
else:
    a = json.load(open(sys.argv[2])); b = r(); dt = time.time() - a['t']
    out = {k: round(100 * (1 - (b[k][1] - a['cpu'][k][1]) / max(1, b[k][0] - a['cpu'][k][0])), 1) for k in ('12', '13', '14', '15', '0', '1')}
    print(json.dumps({'seconds': round(dt, 1), 'busyPercent': out}))
