#!/usr/bin/env python3
"""Run a coordinator-supplied, hash-pinned CLI over the KTX2 controls.

Archives are not tracked: they are regenerated into the run directory by generator.py
and must match fixtures/manifest.json byte for byte before anything runs."""
import argparse, copy, hashlib, importlib.util, json, os, subprocess, time
from pathlib import Path
HERE = Path(__file__).resolve().parent


def sha(raw): return hashlib.sha256(raw).hexdigest()


def controlled_launch():
    os.setpriority(os.PRIO_PROCESS, 0, 10)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:2]))


def run(args):
    binary = args.binary.resolve(); pin = args.source_pin.resolve()
    assert sha(binary.read_bytes()) == args.binary_sha256, 'binary hash mismatch'
    assert sha(pin.read_bytes()) == args.source_pin_sha256, 'source pin hash mismatch'
    manifest = json.loads((HERE / 'fixtures/manifest.json').read_text())
    for name, key in (('runner.py', 'runnerSha256'), ('generator.py', 'generatorSha256'), ('ktx2_oracle.py', 'oracleSha256')):
        assert manifest[key] == sha((HERE / name).read_bytes()), name + ' hash mismatch'
    for ref in manifest['referencePins']['files']:
        raw = (HERE / 'primary' / ref['file']).read_bytes(); assert sha(raw) == ref['sha256'] and len(raw) == ref['bytes']
    args.run_dir.mkdir(parents=True, exist_ok=False)
    spec = importlib.util.spec_from_file_location('ktx2_generator', HERE / 'generator.py')
    generator = importlib.util.module_from_spec(spec); spec.loader.exec_module(generator)
    fixtures = args.run_dir / 'fixtures'
    generated = generator.generate(fixtures)
    assert generated == manifest['cases'], 'regenerated fixtures differ from the frozen manifest'
    env = dict(os.environ, RAYON_NUM_THREADS='2', RUST_TEST_THREADS='2', CARGO_BUILD_JOBS='2')
    records = []
    for case in manifest['cases']:
        path = (fixtures / case['path']).resolve(); assert sha(path.read_bytes()) == case['sha256']
        command = [str(binary), '--json', 'validate', str(path)]; started = time.monotonic()
        result = subprocess.run(command, env=env, preexec_fn=controlled_launch, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=60, check=False)
        (args.run_dir / (case['name'] + '.stdout')).write_bytes(result.stdout)
        (args.run_dir / (case['name'] + '.stderr')).write_bytes(result.stderr)
        try: response = json.loads(result.stdout)
        except (ValueError, UnicodeError): response = None
        observed = 'admitted' if result.returncode == 0 else (response or {}).get('error', {}).get('code', 'unknown')
        expected = copy.deepcopy(case['expectedReport'])
        if expected: expected['archive'] = str(path)
        passed = (result.returncode == case['expectedExitCode'] and observed == case['expectedCategory']
                  and not result.stderr and (expected is None or expected == response))
        records.append(dict(name=case['name'], passed=passed, expectedCategory=case['expectedCategory'],
                            observedCategory=observed, expectedReport=expected, response=response,
                            exitCode=result.returncode, command=command, elapsedSeconds=time.monotonic() - started,
                            stdoutSha256=sha(result.stdout), stderrSha256=sha(result.stderr), archiveSha256=case['sha256']))
        print(json.dumps(dict(name=case['name'], passed=passed, expected=case['expectedCategory'], observed=observed)), flush=True)
    unchanged = all(sha((fixtures / c['path']).read_bytes()) == c['sha256'] for c in manifest['cases'])
    identities = sha(binary.read_bytes()) == args.binary_sha256 and sha(pin.read_bytes()) == args.source_pin_sha256
    receipt = dict(schemaVersion=1, binary=str(binary), binarySha256=args.binary_sha256,
                   sourcePinSha256=args.source_pin_sha256, sourcePin=json.loads(pin.read_text()),
                   manifestSha256=sha((HERE / 'fixtures/manifest.json').read_bytes()),
                   runnerSha256=sha(Path(__file__).read_bytes()), fixturesUnchanged=unchanged,
                   artifactIdentitiesUnchanged=identities, cases=records,
                   passes=sum(r['passed'] for r in records), failures=sum(not r['passed'] for r in records),
                   executionPolicy=dict(nice=10, workerEnv=2, affinity='first two allowed CPUs'))
    (args.run_dir / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return 0 if unchanged and identities and all(r['passed'] for r in records) else 1


if __name__ == '__main__':
    p = argparse.ArgumentParser(); p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--binary-sha256', required=True); p.add_argument('--source-pin', type=Path, required=True)
    p.add_argument('--source-pin-sha256', required=True); p.add_argument('--run-dir', type=Path, required=True)
    raise SystemExit(run(p.parse_args()))
