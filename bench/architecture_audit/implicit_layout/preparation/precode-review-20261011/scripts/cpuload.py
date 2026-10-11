import sys, time
def r():
    d = {}
    for l in open('/proc/stat'):
        if l.startswith('cpu') and l[3] != ' ':
            f = l.split(); v = list(map(int, f[1:])); d[int(f[0][3:])] = (sum(v), v[3] + v[4])
    return d
secs = float(sys.argv[1]) if len(sys.argv) > 1 else 5
a = r(); time.sleep(secs); b = r()
print(' '.join(f'{k}:{100*(1-(b[k][1]-a[k][1])/max(1,b[k][0]-a[k][0])):.0f}%' for k in sorted(a)))
