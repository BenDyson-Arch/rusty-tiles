"""Judge run.py's fixture-suite result against A-5.1's frozen expectations (../expectations.json).

Usage: python3 -I judge_fixtures.py FIXTURES_RESULT.json
  E, S, X, F, D: 8/8 leaves selected and picked, the in-cell control view passes, no tile failures;
  A: 0/8 leaves selected and picked (the derived-cell placeholder cull; must fail 8/8);
  N: tile failures reading the metadata schema and 0/8 leaves (F-1 sensitivity: no root schema).
"""
import json, sys

EXPECT = {
    'E': 'pass', 'S': 'pass', 'X': 'pass', 'F': 'pass', 'D': 'pass',
    'A': 'fail-8/8', 'N': 'fail-schema',
}


def judge(results):
    out, ok_all = {}, True
    for v, want in EXPECT.items():
        r = results.get(v)
        if r is None or 'views' not in r:
            out[v] = {'expect': want, 'ok': False, 'why': 'variant missing or run failed'}
            ok_all = False
            continue
        leaves = [w for w in r['views'] if not w['control']]
        controls = [w for w in r['views'] if w['control']]
        sel = sum(w['leafSelected'] for w in leaves)
        pick = sum(w['pickedLeaf'] for w in leaves)
        fails = r.get('failures') or []
        if want == 'pass':
            ok = sel == pick == len(leaves) == 8 and not fails and all(w['leafSelected'] and w['pickedLeaf'] for w in controls)
        elif want == 'fail-8/8':
            ok = sel == 0 and pick == 0 and len(leaves) == 8 and not fails
        else:
            ok = sel == 0 and pick == 0 and len(leaves) == 8 and any("reading 'classes'" in f for f in fails)
        ok_all &= ok
        out[v] = {'expect': want, 'ok': ok, 'leafSelected': f'{sel}/{len(leaves)}', 'leafPicked': f'{pick}/{len(leaves)}',
                  'controls': [(w['leafSelected'], w['pickedLeaf']) for w in controls], 'failures': fails[:2],
                  'requests': r.get('requests'), 'cesium': r.get('version')}
    return ok_all, out


if __name__ == '__main__':
    ok, out = judge(json.load(open(sys.argv[1]))['results'])
    print(json.dumps({'verdict': 'pass' if ok else 'fail', 'variants': out}, indent=1))
    sys.exit(0 if ok else 1)
