#!/usr/bin/env python3
"""Run the H7 legacy-content controls. Never builds a target, never writes into fixtures/.

    python3 -I check.py --oracle                     # C2/I1 oracle verdicts (c2_oracle.py --implicit)
    python3 -I check.py --binary BIN                 # CLI verdicts against expectedCategory (H7 rule)
    python3 -I check.py --binary BIN --develop       # CLI verdicts against developCategory (pre-H7)
    python3 -I check.py --regenerate                 # generate.py into a temp dir; byte-identical?

Each mode prints one JSON line per case and exits 1 on any mismatch. --develop is the
sensitivity record: run it on a pre-H7 binary, it must pass, and every case whose
developCategory differs from expectedCategory is a control that the H7 change flips.
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / 'fixtures'
ORACLE = HERE.parent / 'c2_decoded_containment' / 'oracle' / 'c2_oracle.py'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def manifest():
    m = json.loads((FIXTURES / 'manifest.json').read_text())
    assert m['generatorSha256'] == sha((HERE / 'generate.py').read_bytes()), 'generator changed: regenerate'
    for case in m['cases']:
        assert sha((FIXTURES / case['path']).read_bytes()) == case['sha256'], case['name']
    return m


def run_cli(binary, case, develop):
    path = FIXTURES / case['path']
    result = subprocess.run([str(binary), 'validate', str(path), '--json'], capture_output=True, timeout=60)
    report = json.loads(result.stdout)
    error = report.get('error') or {}
    observed = 'admitted' if report.get('ok') else error.get('code', 'unknown')
    message = error.get('message', '')
    if develop:
        passed = observed == case['developCategory']
    else:
        passed = (observed == case['expectedCategory'] and result.returncode == case['expectedExitCode']
                  and all(part in message for part in case['messageIncludes']))
    return dict(name=case['name'], passed=passed, observed=observed, exitCode=result.returncode,
                message=message, expected=case['developCategory' if develop else 'expectedCategory'])


def run_oracle(case):
    path = FIXTURES / case['path']
    result = subprocess.run([sys.executable, '-I', str(ORACLE), str(path), '--implicit', '--no-detail'],
                            capture_output=True, timeout=600)
    report = json.loads(result.stdout)
    observed = report.get('outcome')
    facts = report.get('facts') or {}
    passed = observed == case['c2OracleOutcome']
    if passed and observed == 'Unsupported' and case['rule'] in ('L1', 'L2', 'T'):
        why = facts.get('why', '')
        fmt = case['messageIncludes'][1]
        passed = report.get('phase') == ('I1' if case['rule'] == 'T' else 'C1') and fmt in why
        if case['rule'] != 'T':
            passed = passed and facts.get('rule') == {'L1': 'suffix', 'L2': 'magic'}[case['rule']]
    return dict(name=case['name'], passed=passed, observed=observed, phase=report.get('phase'),
                why=facts.get('why') or report.get('why'), expected=case['c2OracleOutcome'])


def regenerate():
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / 'fixtures'
        subprocess.run([sys.executable, '-I', str(HERE / 'generate.py'), '--out', str(out)], check=True,
                       capture_output=True)
        theirs = {p.name: sha(p.read_bytes()) for p in sorted(out.iterdir())}
        ours = {p.name: sha(p.read_bytes()) for p in sorted(FIXTURES.iterdir())}
        same = theirs == ours
        print(json.dumps(dict(regenerated=len(theirs), identical=same)))
        return same


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--develop', action='store_true')
    parser.add_argument('--oracle', action='store_true')
    parser.add_argument('--regenerate', action='store_true')
    args = parser.parse_args()
    ok = True
    if args.regenerate:
        ok &= regenerate()
    m = manifest()
    for case in m['cases']:
        rows = []
        if args.binary:
            rows.append(run_cli(args.binary.resolve(), case, args.develop))
        if args.oracle:
            rows.append(run_oracle(case))
        for row in rows:
            ok &= row['passed']
            print(json.dumps(row, sort_keys=True), flush=True)
    return 0 if ok else 1


if __name__ == '__main__':
    os.environ.setdefault('RAYON_NUM_THREADS', '2')
    sys.exit(main())
