#!/usr/bin/env python3
"""Installed F1b2 extension parity with independent dependency/geometry truth."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import f1b2_oracle as oracle


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--installed', required=True, type=Path)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    installed, binary = args.installed.resolve(), args.binary.resolve()
    sys.path.insert(0, str(installed))
    import rusty_tiles
    modules = [module for name, module in sys.modules.items() if name.startswith('rusty_tiles') and getattr(module, '__file__', '').endswith(('.so', '.pyd'))]
    oracle.require(len(modules) == 1, 'one installed native extension')
    extension = Path(modules[0].__file__).resolve()
    oracle.require(extension.is_relative_to(installed), 'extension must load from installed target')
    receipt = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'extension_sha256': hashlib.sha256(extension.read_bytes()).hexdigest(), 'extension_path': str(extension), 'python': sys.version, 'positive_cases': [], 'refusal_cases': []}
    errors = (rusty_tiles.DataError, rusty_tiles.UnsupportedError, rusty_tiles.TilesIOError, rusty_tiles.InvalidRequestError)
    with tempfile.TemporaryDirectory(prefix='f1b2-wheel-') as temporary:
        root = Path(temporary)
        for variant in oracle.VARIANTS:
            source = oracle.write_fixture(root / 'sources' / variant, variant)
            before = {str(p.relative_to(source.parent)): hashlib.sha256(p.read_bytes()).hexdigest() for p in source.parent.rglob('*') if p.is_file()}
            for limit in (1, 3, 1000):
                cli, python = (root / (variant + '-' + str(limit) + suffix + '.3tz') for suffix in ('-cli', '-python'))
                completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(cli), '--leaf-triangles', str(limit)], capture_output=True, text=True, timeout=60)
                oracle.require(completed.returncode == 0, completed.stdout + completed.stderr)
                cli_report = json.loads(completed.stdout)['meshReport']
                events = []
                value = rusty_tiles.mesh_local_to_3tz(source, python, leaf_triangles=limit, callback=events.append)
                inspected = oracle.inspect(source, python, limit)
                oracle.require(value.report == cli_report == inspected['report'], 'complete frontend/report parity')
                with zipfile.ZipFile(cli) as c, zipfile.ZipFile(python) as p:
                    oracle.require(c.namelist() == p.namelist() and all(c.read(name) == p.read(name) for name in c.namelist()), 'archive member byte parity')
                oracle.require(events and not value.cleanup_diagnostics, 'events and clean successful result')
                after = {str(p.relative_to(source.parent)): hashlib.sha256(p.read_bytes()).hexdigest() for p in source.parent.rglob('*') if p.is_file()}
                oracle.require(after == before, 'all admitted sources unchanged')
                receipt['positive_cases'].append({'variant': variant, 'leaf_limit': limit, 'status': 'passed', 'report': value.report})
        for name, bundle, kind in oracle.refusal_bundles():
            source = oracle.write_bundle(root / 'refusals' / name, bundle)
            for limit in (1, 1000):
                output = root / 'absent' / name / (str(limit) + '.3tz')
                try:
                    rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=limit)
                except errors as failure:
                    oracle.require(failure.kind == kind, 'typed refusal ' + name)
                else:
                    raise oracle.OracleError('invalid source admitted: ' + name)
                oracle.require(not output.parent.exists(), 'refusal created output work')
                receipt['refusal_cases'].append({'case': name, 'leaf_limit': limit, 'kind': kind})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
