"""Retain real C1 context executions from the actual Cargo test log."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def sha(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--log', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    raw = args.log.read_text()
    rows = []
    for line in raw.splitlines():
        start = line.find('{"actualReportSha256"')
        if start >= 0:
            row = json.JSONDecoder().raw_decode(line[start:])[0]
            if 'independentContextControl' in row:
                rows.append(row)
    assert len(rows) == 82
    # Each execution is distinct by its control, fact and selected limits.
    assert len({(r['independentContextControl'], r['fact'], json.dumps(r['selectedLimits'], sort_keys=True))
                for r in rows}) == 82
    # I1 amendment (OD-4, X-7): the nested-metadata-extras control's three
    # one-less runs are admitted, because implicit tile extras are opaque.
    opaque = [r for r in rows if r['independentContextControl'] == 'nested-metadata-extras']
    assert len(opaque) == 6 and all(r['outcome'] == 'admitted' for r in opaque)
    # H7 amendment: the three b3dm controls' 18 runs are legacy Unsupported.
    legacy = [r for r in rows if r['independentContextControl'].startswith('b3dm-')]
    assert len(legacy) == 18 and all(r['outcome'] == 'unsupported' for r in legacy)
    assert sum(r['outcome'] == 'admitted' for r in rows) == 35
    assert sum(r['outcome'] == 'resource_limit' for r in rows) == 29
    assert all(r['readOnly'] for r in rows)
    unit = Path(re.search(r'Running unittests src/lib.rs \(([^)]+)\)', raw)[1])
    if not unit.is_absolute():
        unit = root / unit
    files = subprocess.check_output(['git', 'ls-files'], cwd=root, text=True).splitlines()
    selected = [name for name in files if name in ('Cargo.toml', 'Cargo.lock', 'build.rs', 'bindings/python/Cargo.toml')
                or name.startswith(('src/', 'bindings/python/src/')) and name.endswith('.rs')]
    controls = 'bench/architecture_audit/c1_payload_integrity/production_controls/'
    selected += [name for name in files if name.startswith(controls)]
    evidence = {
        'sourceCommit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'sourceTree': subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=root, text=True).strip(),
        'sourceAndInputSha256': {name: sha(root / name) for name in selected},
        'runnerSha256': sha(Path(__file__)), 'logSha256': sha(args.log),
        'unitArtifact': str(unit), 'unitSha256': sha(unit),
        'executions': rows, 'passes': 82,
        'scope': 'Actual finite C1 context/cache test executions; no complete semantic or universal resource claim.',
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps({'ok': True, 'executions': 82, 'evidence': str(args.output)}))


if __name__ == '__main__':
    main()
