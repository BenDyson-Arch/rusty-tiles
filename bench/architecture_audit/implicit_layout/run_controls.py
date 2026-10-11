"""Run the forest checker over the generated fixtures and judge each against its frozen expectation.

Usage: python3 -I run_controls.py FIXTURE_DIR OUT.json
  FIXTURE_DIR is make_forest_fixtures.py's output (manifest.json plus archives).

Expectations (frozen in expectations.json):
  * positive: verdict pass, every rule pass or not-applicable, both with the --explicit reference and
    without it;
  * negative: every listed target rule has status fail (with the reference); the verdict is fail when
    a target rule is required. A negative that does not need the reference must also fail its
    non-A4 targets without it (A4.* rules apply only with the reference).
Agreement is the number of (case, mode) runs that meet their expectation.
"""
import hashlib, importlib.util, json, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent


def load_checker():
    spec = importlib.util.spec_from_file_location('forest_check', HERE / 'forest_check.py')
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def main():
    fx, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    chk = load_checker()
    sys.setrecursionlimit(10000)
    manifest = json.loads((fx / 'manifest.json').read_text())
    required = {rid: req for rid, req, _, _ in chk.RULES}
    runs, agree = [], 0
    for name, case in manifest['cases'].items():
        modes = [True, False] if case['kind'] == 'positive' or not case.get('needsReference') else [True]
        for with_ref in modes:
            ref = str(fx / case['reference']) if with_ref else None
            v = chk.run(str(fx / case['archive']), ref, 4)
            status = {r: x['status'] for r, x in v['rules'].items()}
            if case['kind'] == 'positive':
                bad = sorted(r for r, s in status.items() if s == 'fail')
                ok = v['verdict'] == 'pass' and not bad
                detail = {'unexpectedFailures': bad}
            else:
                # A4.* rules apply only with the --explicit reference; without it they are not judged
                targets = [r for r in case['expect']['failRules'] if with_ref or not r.startswith('A4.')]
                missed = [r for r in targets if status[r] != 'fail']
                want_fail = any(required[r] for r in targets)
                ok = not missed and (v['verdict'] == 'fail') == want_fail
                detail = {'targets': targets, 'missedTargets': missed,
                          'collateralFailures': sorted(r for r, s in status.items() if s == 'fail' and r not in targets)}
            agree += ok
            runs.append({'case': name, 'kind': case['kind'], 'withReference': with_ref, 'agree': ok, 'verdict': v['verdict'],
                         'failedRules': v['failedRules'], **detail,
                         'examples': {r: v['rules'][r]['examples'][:2] for r in (case['expect'].get('failRules') or [])},
                         'archiveSha256': v['archiveSha256'], 'stats': v['stats'] if case['kind'] == 'positive' else None})
            print(f"{'ok ' if ok else 'BAD'} {name:<28} ref={with_ref!s:<5} verdict={v['verdict']:<4} failed={v['failedRules']}", file=sys.stderr)
    summary = {'runs': len(runs), 'agree': agree,
               'positives': sum(1 for c in manifest['cases'].values() if c['kind'] == 'positive'),
               'negatives': sum(1 for c in manifest['cases'].values() if c['kind'] == 'negative'),
               'rulesTargeted': sorted({r for c in manifest['cases'].values() for r in c['expect'].get('failRules', [])})}
    receipt = {'checkerSha256': hashlib.sha256((HERE / 'forest_check.py').read_bytes()).hexdigest(),
               'generatorSha256': manifest['generatorSha256'],
               'runnerSha256': hashlib.sha256((HERE / 'run_controls.py').read_bytes()).hexdigest(),
               'manifestSha256': hashlib.sha256((fx / 'manifest.json').read_bytes()).hexdigest()}
    out.write_text(json.dumps({'receipt': receipt, 'summary': summary, 'runs': runs}, indent=1) + '\n')
    print(json.dumps(summary))
    sys.exit(0 if agree == len(runs) else 1)


if __name__ == '__main__':
    main()
