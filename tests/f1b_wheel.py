#!/usr/bin/env python3
"""Installed-extension parity against independently decoded F1b fixtures."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import f1b_oracle as oracle


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--installed', required=True, type=Path)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    installed = args.installed.resolve()
    sys.path.insert(0, str(installed))
    import rusty_tiles
    native_modules = [module for name, module in sys.modules.items()
                      if name.startswith('rusty_tiles') and getattr(module, '__file__', '').endswith(('.so', '.pyd'))]
    oracle.require(len(native_modules) == 1, 'one loaded native extension')
    extension = Path(native_modules[0].__file__).resolve()
    oracle.require(extension.is_relative_to(installed), 'must use installed native extension')
    binary = args.binary.resolve()
    cases = []
    with tempfile.TemporaryDirectory(prefix='f1b-wheel-') as temporary:
        root = Path(temporary)
        for variant in oracle.VARIANTS:
            source = root / (variant + '.glb')
            source.write_bytes(oracle.fixture(variant=variant))
            source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
            for limit in (1, 3, 1000):
                cli = root / (variant + '-' + str(limit) + '-cli.3tz')
                python = root / (variant + '-' + str(limit) + '-python.3tz')
                result = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source), '-o', str(cli), '--leaf-triangles', str(limit)], capture_output=True, text=True, timeout=60)
                oracle.require(result.returncode == 0, result.stdout + result.stderr)
                cli_report = json.loads(result.stdout)['meshReport']
                events = []
                value = rusty_tiles.mesh_local_to_3tz(source, python, leaf_triangles=limit, callback=events.append)
                inspected = oracle.inspect(source, python, limit)
                oracle.require(value.report == cli_report == inspected['report'], 'full report parity')
                oracle.require(events and not value.cleanup_diagnostics, 'precommit events and clean successful receipt')
                oracle.require(hashlib.sha256(source.read_bytes()).hexdigest() == source_hash, 'source unchanged')
                cases.append({'variant': variant, 'leaf_limit': limit, 'status': 'passed', 'report': value.report})
        refusals = []
        for name, data, kind in oracle.rejection_fixtures():
            source = root / (name + '.glb')
            source.write_bytes(data)
            source_hash = hashlib.sha256(data).hexdigest()
            for limit in (1, 1000):
                output = root / (name + '-' + str(limit)) / 'result.3tz'
                try:
                    rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=limit)
                except (rusty_tiles.DataError, rusty_tiles.UnsupportedError) as failure:
                    oracle.require(failure.kind == kind, 'typed refusal parity: ' + name)
                else:
                    raise oracle.OracleError('ineligible source admitted: ' + name)
                oracle.require(not output.parent.exists(), 'no refused output work: ' + name)
                oracle.require(hashlib.sha256(source.read_bytes()).hexdigest() == source_hash, 'refused source unchanged')
                refusals.append({'case': name, 'leaf_limit': limit, 'kind': kind})
    receipt = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'extension_path': str(extension), 'extension_sha256': hashlib.sha256(extension.read_bytes()).hexdigest(), 'python': sys.version, 'positive_cases': cases, 'refusal_cases': refusals}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
