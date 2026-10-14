#!/usr/bin/env python3
"""Official Khronos core validation, with empty-string and padding controls."""
import argparse
import copy
import json
from pathlib import Path
import subprocess
import tempfile
import zipfile
import f1c2_oracle as oracle


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--validator-modules', required=True, type=Path)
    parser.add_argument('--json-output', required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    conversions, plan = [], []
    with tempfile.TemporaryDirectory(prefix='f1c2-khronos-') as temporary:
        root = Path(temporary)
        for variant in ('instances', 'viewer', 'empty-names', 'first-scene'):
            source = oracle.fixture(variant)
            source_name = variant+'/source.glb'
            (root/variant).mkdir()
            (root/source_name).write_bytes(source)
            plan.append({'input': source_name})
            archive = root/variant/'candidate.3tz'
            completed = subprocess.run([str(binary), '--json', 'mesh-local-to-3tz', '-i', str(root/source_name),
                '-o', str(archive), '--leaf-triangles', '5'], text=True, capture_output=True, timeout=60)
            oracle.require(completed.returncode == 0, 'validator conversion '+completed.stdout+completed.stderr)
            with zipfile.ZipFile(archive) as stream:
                members = {n: stream.read(n) for n in stream.namelist() if n != '@3dtilesIndex1@'}
            inspected = oracle.inspect_members(source, members, 5)
            inspected['archive_sha256'] = oracle.digest(archive.read_bytes())
            inspected['variant'] = variant
            conversions.append(inspected)
            for member, data in members.items():
                if member.endswith('.glb'):
                    name = variant+'/leaves/'+member
                    target = root/name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(data)
                    plan.append({'input': name})
        empty_members = oracle.synthetic_members(oracle.fixture('empty-names'))
        empty_leaf = empty_members['t/0.glb']
        (root/'synthetic-empty.glb').write_bytes(empty_leaf)
        plan.append({'input': 'synthetic-empty.glb'})
        doc, raw, _ = oracle.decode(empty_leaf)
        changed = copy.deepcopy(doc)
        prop = changed['extensions']['EXT_structural_metadata']['propertyTables'][0]['properties']['node_name']
        changed['bufferViews'][prop['values']]['byteLength'] = 0
        (root/'zero-string-view.glb').write_bytes(oracle.glb(changed, raw))
        plan.append({'input': 'zero-string-view.glb', 'expectedCode': 'VALUE_NOT_IN_RANGE'})
        (root/'terminal-four-padding.glb').write_bytes(oracle.terminal_overpadding(oracle.fixture('viewer')))
        plan.append({'input': 'terminal-four-padding.glb', 'expectedCode': 'BUFFER_GLB_CHUNK_TOO_BIG'})
        (root/'validation-plan.json').write_text(json.dumps(plan))
        helper = Path(__file__).parent/'fixtures/f1c2_validate.cjs'
        completed = subprocess.run(['node', str(helper), str(args.validator_modules.resolve(strict=True)), str(root)],
            text=True, capture_output=True, timeout=120)
        oracle.require(completed.returncode == 0, 'official validation '+completed.stdout+completed.stderr)
        receipt = json.loads(completed.stdout)
        receipt.update(binary_sha256=oracle.digest(binary.read_bytes()), conversions=conversions,
            authored_driver_sha256={p.name: oracle.digest(p.read_bytes()) for p in (Path(__file__), helper, Path(__file__).parent/'f1c2_oracle.py')},
            scope='Every emitted leaf for four independent sources ceiling5, exact empty STRING sentinel; official core errors and sensitive zero-view/four-byte padding controls. Extension semantics require independent decoder/Cesium proof.')
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(json.dumps(receipt, indent=2)+'\n')
        print(json.dumps({'validated': len(plan), 'warnings': sorted({m['code'] for r in receipt['results'] for m in r['report']['issues']['messages'] if m['severity'] == 1}), 'receipt': str(args.json_output)}))


if __name__ == '__main__':
    main()
