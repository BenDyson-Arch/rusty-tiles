"""Judge a deep-view A/B (run_real.py results, baseline first) against A-5.2's frozen requirements.

Usage: python3 -I judge_deep.py --cb0 RESULT_CB0.json --cb1 RESULT_CB1.json --parents PARENTS.json --errors ERRORS.json
  RESULT_CB0: PIMPL_CHILD_BOUNDS=0 (cullWithChildrenBounds false); RESULT_CB1: Cesium defaults.
  The first case in each result is the baseline (develop); every other case is judged against it.

Requirements (design §6 A-5.2, frozen in ../expectations.json):
  R1 every view of every run loaded, the overview loaded, no tile failures and no failed requests;
  R2 the same target-path tile in every view (both runs);
  R3 the same non-null picks in every view (both runs): a pick that is null on one side only fails,
     null on both sides is reported separately and is not agreement;
  R4 with cullWithChildrenBounds false: identical selections in every view;
  R5 with defaults: every view whose selections differ has the head selecting a strict ancestor of a
     baseline-selected tile, never the reverse, with equal pixel coverage; the share is reported.
"""
import argparse, importlib.util, json, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f'{name}.py')
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def ancestors(parents, k):
    out = set()
    while parents.get(k) is not None:
        k = parents[k]; out.add(k)
    return out


def judge_pair(base, other, parents, cb):
    problems, coarser, null_both, same_sel = [], 0, 0, 0
    for a, b in zip(base['views'], other['views']):
        A, B = set(a['selectedKeys']), set(b['selectedKeys'])
        if a['pickedKey'] is None and b['pickedKey'] is None:
            null_both += 1
        elif a['pickedKey'] != b['pickedKey']:
            problems.append(f"R3 {a['name']}: pick {a['pickedKey']} != {b['pickedKey']}")
        if A == B:
            same_sel += 1
            continue
        if not cb:
            problems.append(f"R4 {a['name']}: selections differ with cullWithChildrenBounds false")
            continue
        ob = any(ancestors(parents, x) & B for x in A)
        oa = any(ancestors(parents, x) & A for x in B)
        if not ob or oa:
            problems.append(f"R5 {a['name']}: differing selection is not the head selecting an ancestor")
        elif a['coverage'] != b['coverage']:
            problems.append(f"R5 {a['name']}: coverage {a['coverage']} != {b['coverage']}")
        else:
            coarser += 1
    return problems, {'views': len(base['views']), 'sameSelection': same_sel, 'headSelectsAncestor': coarser,
                      'headSelectsAncestorShare': round(coarser / max(1, len(base['views'])), 4), 'nullPickBoth': null_both}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--cb0', required=True)
    ap.add_argument('--cb1', required=True)
    ap.add_argument('--parents', required=True)
    ap.add_argument('--errors', required=True)
    a = ap.parse_args()
    tp = load('target_path')
    parents, errors = json.load(open(a.parents)), json.load(open(a.errors))
    verdict = {'requirements': 'design §6 A-5.2', 'runs': {}, 'problems': []}
    for tag, path, cb in (('cb0', a.cb0, False), ('cb1', a.cb1, True)):
        res = json.load(open(path))['results']
        names = list(res)
        for n in names:
            r = res[n]
            if 'views' not in r:
                verdict['problems'].append(f'R1 {tag} {n}: run failed: {str(r.get("error"))[:200]}')
                continue
            if r.get('failures') or r.get('failedRequests') or not r['overview']['loaded'] or not all(v['loaded'] for v in r['views']):
                verdict['problems'].append(f'R1 {tag} {n}: failures {r.get("failures")[:3]}, failedRequests {len(r.get("failedRequests") or [])}, '
                                           f'overview loaded {r["overview"]["loaded"]}, views loaded {sum(v["loaded"] for v in r["views"])}/{len(r["views"])}')
        if any('views' not in res[n] for n in names):
            continue
        t = tp.target_path(res, parents, errors)
        if t['sameTargetPathSelection'] != t['views']:
            verdict['problems'].append(f"R2 {tag}: same target path in {t['sameTargetPathSelection']}/{t['views']} views")
        pairs = {}
        for n in names[1:]:
            problems, summary = judge_pair(res[names[0]], res[n], parents, cb)
            verdict['problems'].extend(f'{tag} {names[0]}~{n} {p}' for p in problems)
            pairs[f'{names[0]}~{n}'] = summary
        verdict['runs'][tag] = {'result': path, 'cullWithChildrenBounds': cb, 'targetPath': {k: v for k, v in t.items() if k != 'perView'},
                                'pairs': pairs, 'requests': {n: res[n]['requests'] for n in names},
                                'viewSeconds': {n: round(sum(v['seconds'] for v in res[n]['views']), 2) for n in names}}
    verdict['verdict'] = 'pass' if not verdict['problems'] else 'fail'
    print(json.dumps(verdict, indent=1))
    sys.exit(0 if verdict['verdict'] == 'pass' else 1)


if __name__ == '__main__':
    main()
