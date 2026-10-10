"""Retain exact executions of the independently authored meshopt stride lane."""
import argparse
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    lane = root / 'bench/architecture_audit/c1_payload_integrity/production_controls/stride_controls/2026-10-10'
    assert sha(lane / 'fixtures/manifest.json') == 'a3c07122e5fc902272bcf668a85e694601551a66556a7be3806132febd592aee'
    frozen = json.loads((lane / 'integrity.json').read_text())
    for name, record in frozen['files'].items():
        assert sha(lane / name) == record['sha256']
        assert (lane / name).stat().st_size == record['bytes']
    args.work.mkdir(parents=True, exist_ok=False)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    files = subprocess.check_output(['git', 'ls-files'], cwd=root, text=True).splitlines()
    selected = [name for name in files if name in ('Cargo.toml', 'Cargo.lock', 'build.rs', 'bindings/python/Cargo.toml')
                or name.startswith(('src/', 'bindings/python/src/')) and name.endswith('.rs')]
    pin = args.work / 'source-pin.json'
    pin.write_text(json.dumps({
        'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'source_tree': subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=root, text=True).strip(),
        'production_sha256': {name: sha(root / name) for name in selected},
        'runner_sha256': sha(Path(__file__)),
        'scope': 'Actual selected checkout and supplied CLI; finite independent meshopt controls.',
    }, indent=2) + '\n')
    spec = importlib.util.spec_from_file_location('independent_stride_lane', lane / 'runner.py')
    oracle = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(oracle)
    status = oracle.run(argparse.Namespace(binary=args.binary, binary_sha256=sha(args.binary),
                                          source_pin=pin, source_pin_sha256=sha(pin), run_dir=args.work / 'runs'))
    receipt = json.loads((args.work / 'runs/receipt.json').read_text())
    for case in receipt['cases']:
        for stream in ('stdout', 'stderr'):
            raw = (args.work / 'runs' / (case['name'] + '.' + stream)).read_bytes()
            assert hashlib.sha256(raw).hexdigest() == case[stream + 'Sha256']
            case[stream + 'Base64'] = base64.b64encode(raw).decode('ascii')
    receipt['coordinatorRunnerSha256'] = sha(Path(__file__))
    receipt['runnerExitCode'] = status
    args.output.write_text(json.dumps(receipt, indent=2) + '\n')
    assert status == 0 and receipt['passes'] == 6 and receipt['failures'] == 0, args.output


if __name__ == '__main__':
    main()
