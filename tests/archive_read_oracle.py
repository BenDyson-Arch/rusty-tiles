"""Run independently authored archive framing controls in an ordinary checkout.

Fixture and format truth belong to the separate archive_read_acceptance lane.
This runner supplies CLI execution and receipts without its historical local
snapshot dependency. It makes no Rust allocation or I/O injection claim.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    driver = root / 'bench/architecture_audit/implicit_rewrite/archive_read_acceptance/probe.py'
    spec = importlib.util.spec_from_file_location('independent_archive_fixtures', driver)
    oracle = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(oracle)
    args.work.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve()
    records = []
    for case in oracle.fixtures():
        raw = case['raw']
        truth = oracle.classify(raw)
        assert truth['kind'] == case['expected'], case['label']
        path = args.work / (case['label'] + '.3tz')
        path.write_bytes(raw)
        command = [str(binary), 'validate', '--json', str(path.resolve())]
        try:
            run = subprocess.run(command, capture_output=True, text=True, timeout=10,
                                 env=dict(os.environ, RAYON_NUM_THREADS='2'))
            stdout, stderr, code = run.stdout, run.stderr, run.returncode
            try:
                report = json.loads(stdout)
            except json.JSONDecodeError:
                report = None
            actual = ('admitted' if code == 0 and isinstance(report, dict) and report.get('ok') is True
                      else report.get('error', {}).get('code') if isinstance(report, dict)
                      else 'unparseable_output')
        except subprocess.TimeoutExpired as failure:
            decode = lambda value: value.decode(errors='replace') if isinstance(value, bytes) else value or ''
            stdout, stderr, code, actual = decode(failure.stdout), decode(failure.stderr), None, 'timeout'
        records.append({'label': case['label'], 'archive_sha256': sha(raw),
                        'expected': case['expected'], 'actual': actual,
                        'command': command, 'exit_code': code,
                        'stdout': stdout, 'stderr': stderr,
                        'read_only': path.is_file() and path.read_bytes() == raw})
    evidence = {'binary_sha256': sha(binary.read_bytes()),
                'fixture_driver_sha256': sha(driver.read_bytes()),
                'runner_sha256': sha(Path(__file__).read_bytes()),
                'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
                'read_only': all(r['read_only'] for r in records), 'records': records,
                'scope': 'Independent finite stored-3TZ CLI categories; no allocation, injected I/O, A2 or release claim.'}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + '\n')
    assert len(records) == 45
    assert evidence['read_only'], args.output
    assert all(r['actual'] == r['expected'] for r in records), args.output
    print(json.dumps({'ok': True, 'cases': len(records), 'evidence': str(args.output)}))


if __name__ == '__main__':
    main()
