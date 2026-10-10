#!/usr/bin/env python3
"""Installed placement API and CLI parity against independent world truth."""
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
import f1c1_oracle as oracle


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--installed', required=True, type=Path)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    installed, binary = args.installed.resolve(), args.binary.resolve()
    sys.path.insert(0, str(installed))
    import rusty_tiles
    modules = [module for name, module in sys.modules.items()
               if name.startswith('rusty_tiles') and
               str(getattr(module, '__file__', '')).endswith(('.so', '.pyd'))]
    oracle.require(len(modules) == 1, 'one installed extension')
    extension = Path(modules[0].__file__).resolve()
    oracle.require(extension.is_relative_to(installed), 'extension belongs to installed target')
    receipt = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
               'extension_sha256': hashlib.sha256(extension.read_bytes()).hexdigest(),
               'extension_path': str(extension), 'python': sys.version,
               'positive_cases': [], 'request_refusals': []}
    scenarios = [('all-slots', name, limit, False)
                 for name in oracle.PLACEMENTS for limit in (1, 1000)]
    scenarios += [(variant, 'mixed-offset', limit, variant in oracle.EXTERNAL_VARIANTS)
                  for variant in oracle.VARIANTS for limit in (1, 1000)]
    with tempfile.TemporaryDirectory(prefix='f1c1-wheel-') as temporary:
        root = Path(temporary)
        sources = {}
        for index, (variant, name, limit, external) in enumerate(scenarios):
            key = (variant, external)
            if key not in sources:
                sources[key] = oracle.write_fixture(root / (variant + str(external)), variant, external)
            source = sources[key]
            before = {str(path.relative_to(source.parent)): hashlib.sha256(path.read_bytes()).hexdigest()
                      for path in source.parent.rglob('*') if path.is_file()}
            placement = oracle.PLACEMENTS[name]
            cli, output = root / f'{index}-cli.3tz', root / f'{index}-python.3tz'
            completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz',
                                        '-i', str(source), '-o', str(cli), '--leaf-triangles', str(limit),
                                        *oracle.placement_cli_args(placement)],
                                       capture_output=True, text=True, timeout=60)
            oracle.require(completed.returncode == 0, completed.stdout + completed.stderr)
            events = []
            result = rusty_tiles.mesh_local_to_3tz(source, output, leaf_triangles=limit,
                                                   callback=events.append,
                                                   **{key: tuple(value) for key, value in placement.items()})
            inspected = oracle.inspect(source, output, limit, placement)
            oracle.require(result.report == json.loads(completed.stdout)['meshReport'] == inspected['report'],
                           'complete placement report parity')
            with zipfile.ZipFile(cli) as first, zipfile.ZipFile(output) as second:
                oracle.require(first.namelist() == second.namelist() and
                               all(first.read(member) == second.read(member) for member in first.namelist()),
                               'complete archive member parity')
            oracle.require(events and not result.cleanup_diagnostics, 'successful event and cleanup receipt')
            after = {str(path.relative_to(source.parent)): hashlib.sha256(path.read_bytes()).hexdigest()
                     for path in source.parent.rglob('*') if path.is_file()}
            oracle.require(after == before, 'source and resources remain unchanged')
            receipt['positive_cases'].append({'variant': variant, 'placement': name,
                                              'leaf_limit': limit, 'external': external,
                                              'world_precision': inspected['world_precision']})
        errors = {'invalid_request': rusty_tiles.InvalidRequestError,
                  'unsupported': rusty_tiles.UnsupportedError}
        for name, arguments, kind in oracle.refusal_cases():
            if name.startswith('partial-'):
                continue
            parameters = {}
            cursor = 0
            while cursor < len(arguments):
                flag = arguments[cursor]
                length = 4 if flag == '--orientation-xyzw' else 3
                parameters[flag[2:].replace('-', '_')] = tuple(
                    float(value) for value in arguments[cursor + 1:cursor + 1 + length])
                cursor += length + 1
            for prior in (False, True):
                output = root / (name + '.3tz') if prior else root / 'absent' / name / 'out.3tz'
                if prior:
                    output.write_bytes(b'previous artifact')
                try:
                    rusty_tiles.mesh_local_to_3tz(root / 'missing.glb', output,
                                                  leaf_triangles=1, force=prior, **parameters)
                except errors[kind] as failure:
                    oracle.require(failure.kind == kind and not failure.secondary_diagnostics
                                   and not failure.retained_paths and failure.recovery is None,
                                   'typed placement refusal metadata')
                else:
                    raise oracle.OracleError('placement request admitted: ' + name)
                oracle.require(output.read_bytes() == b'previous artifact' if prior else not output.parent.exists(),
                               'request refusal preserves publication state')
                receipt['request_refusals'].append({'case': name, 'kind': kind, 'prior_output': prior})
        oracle.require(not any(path.name.startswith(('.mesh-work-', '.tiles-stage-'))
                               for path in root.rglob('*')), 'no retained scratch')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
