"""Independent exact re-derivation of the prior C2 literal witness expectations.

Authored separately from the prior materialize.py/model.py: no import, no shared
helpers. Uses Fraction over the represented binary64 value of every JSON number.
Evaluates both quaternion meanings (raw polynomial vs normalized H(q)/|q|^2)
so that decision-dependent witnesses are visible. Reads witnesses.json given as
argv[1]; writes witness-check.json beside this script. Single-threaded, tiny.
"""
import json
import re
import sys
from fractions import Fraction as F
from pathlib import Path


def fx(v):
    return F(float(v)) if not isinstance(v, str) else F(v)


def col_major(m):
    m = [fx(v) for v in m]
    if m[3] != 0 or m[7] != 0 or m[11] != 0 or m[15] != 1:
        raise ValueError('non-affine')
    return [[m[c * 4 + r] for c in range(4)] for r in range(3)]  # rows r, cols c


def apply(a, p):
    return [sum(a[r][c] * p[c] for c in range(3)) + a[r][3] for r in range(3)]


def compose(a, b):  # a after b
    out = []
    for r in range(3):
        row = [sum(a[r][k] * b[k][c] for k in range(3)) for c in range(3)]
        row.append(sum(a[r][k] * b[k][3] for k in range(3)) + a[r][3])
        out.append(row)
    return out


def rot(q, mode):
    x, y, z, w = [fx(v) for v in q]
    if mode == 'raw':
        return [[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]]
    n = x * x + y * y + z * z + w * w
    h = [[w * w + x * x - y * y - z * z, 2 * (x * y - z * w), 2 * (x * z + y * w)],
         [2 * (x * y + z * w), w * w - x * x + y * y - z * z, 2 * (y * z - x * w)],
         [2 * (x * z - y * w), 2 * (y * z + x * w), w * w - x * x - y * y + z * z]]
    return [[v / n for v in row] for row in h]


def local(node, mode):
    if 'matrix' in node:
        return col_major(node['matrix'])
    t = [fx(v) for v in node.get('translation', [0, 0, 0])]
    s = [fx(v) for v in node.get('scale', [1, 1, 1])]
    r = rot(node.get('rotation', [0, 0, 0, 1]), mode)
    return [[r[i][j] * s[j] for j in range(3)] + [t[i]] for i in range(3)]


def decode(raw, comp, normalized):
    if not normalized or comp == 5126:
        return [fx(v) for v in raw]
    d = {5120: 127, 5121: 255, 5122: 32767, 5123: 65535}[comp]
    return [max(F(int(v), d), F(-1)) for v in raw]


IDENT = [[F(1), F(0), F(0), F(0)], [F(0), F(1), F(0), F(0)], [F(0), F(0), F(1), F(0)]]


def instances(case, mode):
    nodes = case['nodes']
    scene = case['scenes'][case['scene']]
    out = []
    stack = [(i, IDENT) for i in reversed(scene['nodes'])]
    while stack:
        i, parent = stack.pop()
        g = compose(parent, local(nodes[i], mode))
        if 'mesh' in nodes[i]:
            out.append((i, g))
        for c in reversed(nodes[i].get('children', [])):
            stack.append((c, g))
    return out


def U(p):
    return [p[0], -p[2], p[1]]


def run(case, mode):
    pos = case['positions']
    drawn = case.get('indices', range(len(pos)))
    rtc = [fx(v) for v in case.get('rtc', [0, 0, 0])]
    res = {'own_tile_local': [], 'selected_nodes': [], 'draw_visits': 0}
    for node_index, g in instances(case, mode):
        res['selected_nodes'].append(node_index)
        for k in drawn:
            raw = [0, 0, 0] if case.get('viewless') else pos[k]
            p = decode(raw, case['componentType'], case.get('normalized', False))
            yp = apply(g, p)
            tl = [a + b for a, b in zip(U(yp), rtc)]
            res['draw_visits'] += 1
            res['own_tile_local'].append(tl)
            res.setdefault('node_yp', []).append(yp)
            chain = case.get('tile_chain_root_first')
            if chain:
                mats = [col_major(m) for m in chain]
                q = tl
                ups = []
                for m in reversed(mats):
                    q = apply(m, q)
                    ups.append(q)
                res.setdefault('chain_up', []).append(ups)
    return res


def eq(expected, got):
    return [[fx(v) if not isinstance(v, str) else None for v in row] for row in expected] == got


def main():
    w = json.loads(Path(sys.argv[1]).read_text())
    report = []
    for case in w['cases']:
        rec = {'id': case['id']}
        for mode in ('raw', 'normalized'):
            r = run(case, mode)
            e = case['expected']
            ok = {}
            if 'own_tile_local' in e:
                ok['own_tile_local'] = eq(e['own_tile_local'], r['own_tile_local'])
            if 'node_yp' in e:
                ok['node_yp'] = eq(e['node_yp'], r['node_yp'])
            if 'draw_visits' in e:
                ok['draw_visits'] = e['draw_visits'] == r['draw_visits']
            if 'selected_nodes' in e:
                ok['selected_nodes'] = e['selected_nodes'] == r['selected_nodes']
            if 'own_tile_local_rational' in e:
                exp = [[F(eval(re.sub(r'(\d+)', r'F(\1)', v), {'__builtins__': {}}, {'F': F})) for v in row] for row in e['own_tile_local_rational']]
                ok['own_tile_local_rational'] = exp == r['own_tile_local']
            if 'own_tile_local_exact_expression' in e:
                a = F(e['constants']['a']); b = F(e['constants']['b'])
                exp = [[F(eval(v, {'__builtins__': {}}, {'a': a, 'b': b})) for v in row]
                       for row in e['own_tile_local_exact_expression']]
                ok['own_tile_local_exact_expression'] = exp == r['own_tile_local']
                ok['max_abs_difference_vs_expected'] = str(max(abs(x - y) for x, y in zip(exp[0], r['own_tile_local'][0])))
            if 'chain_up' in r:
                ups = r['chain_up'][0]
                picks = {'world': ups[-1], 'parent_tile_local': ups[0] if len(ups) == 2 else ups[1]}
                if len(ups) == 3:
                    picks['external_root_local'] = ups[0]
                for key, got in picks.items():
                    if key in e:
                        ok[key] = [[fx(v) for v in e[key][0]]] == [got]
            rec[mode] = ok
        report.append(rec)
    Path(__file__).with_name('witness-check.json').write_text(json.dumps(report, indent=2) + '\n')
    for rec in report:
        flat = {m: all(v for k, v in rec[m].items() if isinstance(v, bool)) for m in ('raw', 'normalized')}
        print(rec['id'], flat)


if __name__ == '__main__':
    main()
