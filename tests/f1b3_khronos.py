#!/usr/bin/env python3
"""Khronos validation of every static F1b3 source and its complete leaf inventory."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile

import f1b3_oracle as oracle


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--validator-modules', required=True, type=Path)
    parser.add_argument('--fixture-dir', type=Path, default=Path(__file__).parent / 'fixtures/f1b3')
    parser.add_argument('--source-commit')
    parser.add_argument('--json-output', required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    fixtures = args.fixture_dir.resolve(strict=True)
    helper = Path(__file__).parent / 'fixtures/f1b3_validate.cjs'
    manifest = json.loads((fixtures / 'manifest.json').read_text())
    conversions = []
    with tempfile.TemporaryDirectory(prefix='f1b3-khronos-') as temporary:
        root = Path(temporary)
        shutil.copytree(fixtures, root / 'sources')
        inputs = []
        for record in manifest['fixtures']:
            variant = record['variant']
            source = root / 'sources' / record['source']
            archive = root / 'outputs' / (variant + '.3tz')
            archive.parent.mkdir(parents=True, exist_ok=True)
            completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(source),
                                        '-o', str(archive), '--leaf-triangles', '3'],
                                       capture_output=True, text=True, timeout=60)
            oracle.require(completed.returncode == 0, 'Khronos candidate ' + variant + ': ' + completed.stdout + completed.stderr)
            evidence = oracle.inspect(source, archive, 3)
            unpacked = root / 'unpacked' / variant
            # Complete independent archive inventory/path checks precede extraction.
            with zipfile.ZipFile(archive) as archive_file:
                archive_file.extractall(unpacked)
            tileset = json.loads((unpacked / 'tileset.json').read_text())
            leaves = [child['content']['uri'] for child in tileset['root']['children']]
            inputs.append(str(source.relative_to(root)))
            inputs.extend(str((unpacked / leaf).relative_to(root)) for leaf in leaves)
            conversions.append({'variant': variant, 'leaves': len(leaves), 'archive_sha256': evidence['archive_sha256'],
                                'source_sha256': evidence['source_sha256'], 'published_images': evidence['published_images']})
        completed = subprocess.run(['node', str(helper), str(args.validator_modules.resolve(strict=True)), str(root), *inputs],
                                   capture_output=True, text=True, timeout=120)
        oracle.require(bool(completed.stdout), 'Khronos produced no receipt: ' + completed.stderr)
        receipt = json.loads(completed.stdout)
        drivers = (Path(__file__), helper, Path(__file__).parent / 'f1b3_oracle.py',
                   Path(__file__).parent / 'f1b_oracle.py', Path(__file__).parent / 'f1b2_oracle.py')
        receipt.update(binary_sha256=sha(binary), source_commit=args.source_commit,
                       authored_driver_sha256={str(path.relative_to(Path(__file__).parent)): sha(path) for path in drivers},
                       source_manifest_sha256=sha(fixtures / 'manifest.json'), conversions=conversions,
                       scope='All static F1b3 sources and every emitted leaf at ceiling3; independent external callbacks; no production validator')
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(json.dumps(receipt, indent=2) + '\n')
        oracle.require(completed.returncode == 0, 'Khronos errors: ' + completed.stderr)
        oracle.require(all(result['report']['issues']['numWarnings'] == 0 for result in receipt['results']), 'unexpected official validator warning')
    print(json.dumps({'sources': len(conversions), 'leaves': sum(c['leaves'] for c in conversions),
                      'errors': 0, 'warnings': 0, 'receipt': str(args.json_output)}))


if __name__ == '__main__':
    main()
