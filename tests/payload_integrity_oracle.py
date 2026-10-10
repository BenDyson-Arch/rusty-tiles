"""Run the frozen independent payload corpus and retain exact execution bindings."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tarfile


def sha(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    controls = root / 'bench/architecture_audit/c1_payload_integrity/production_controls'
    args.work.mkdir(parents=True, exist_ok=False)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    receipt = args.work / 'runs/receipt.json'
    status = 1
    failure = None
    try:
        bundle = controls / 'fixtures.tar.gz'
        assert sha(bundle) == '4cf05918b328821893fb63198d730ec76cec790cac8aa8e580051c33efc38c79'
        driver = controls / 'driver.py'
        assert sha(driver) == '5bcd8e1e0a34e86fa39339eb1d499e95851a871a2841d142145dc8cbb911f5d7'
        fixtures = args.work / 'fixtures'
        with tarfile.open(bundle) as archive:
            archive.extractall(fixtures, filter='data')
        assert sha(fixtures / 'manifest.json') == '1b3913464d6152f10b267506a41288492baba38a56a79bdd1cda1c42860dc4e5'
        files = subprocess.check_output(['git', 'ls-files'], cwd=root, text=True).splitlines()
        selected = [name for name in files if name in ('Cargo.toml', 'Cargo.lock', 'build.rs', 'bindings/python/Cargo.toml')
                    or name.startswith(('src/', 'bindings/python/src/')) and name.endswith('.rs')]
        pin = args.work / 'source-pin.json'
        pin.write_text(json.dumps({
            'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
            'source_tree': subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=root, text=True).strip(),
            'production_sha256': {name: sha(root / name) for name in selected},
            'runner_sha256': sha(Path(__file__)),
            'fixture_bundle_sha256': sha(bundle),
            'scope': 'Actual selected checkout and supplied CLI; finite controls, no release acceptance.',
        }, indent=2) + '\n')
        spec = importlib.util.spec_from_file_location('independent_payload_controls', driver)
        oracle = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(oracle)
        status = oracle.run(args.binary.resolve(), sha(args.binary), pin, sha(pin), fixtures, args.work / 'runs')
    except Exception as error:
        failure = {'type': type(error).__name__, 'message': str(error)}
    finally:
        evidence = json.loads(receipt.read_text()) if receipt.exists() else {'cases': [], 'passes': 0}
        evidence['runnerSha256'] = sha(Path(__file__))
        evidence['runnerExitCode'] = status
        if failure:
            evidence['runnerFailure'] = failure
        args.output.write_text(json.dumps(evidence, indent=2) + '\n')
    assert status == 0 and len(evidence['cases']) == 205, args.output
    print(json.dumps({'ok': True, 'cases': 205, 'evidence': str(args.output)}))


if __name__ == '__main__':
    main()
