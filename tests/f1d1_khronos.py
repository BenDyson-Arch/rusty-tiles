#!/usr/bin/env python3
"""Official core supplement for independently checked F1d1 artifacts.

Extension meaning is established by the independent decoder and Cesium queries.
This check makes no claim that Khronos understands the metadata extensions.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import zipfile

import f1d1_oracle as oracle


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--artifact-dir', type=Path, required=True)
    p.add_argument('--validator-modules', type=Path, required=True)
    p.add_argument('--json-output', type=Path, required=True)
    a = p.parse_args()
    checked, plan = [], []
    with tempfile.TemporaryDirectory(prefix='f1d1-khronos-') as temporary:
        root = Path(temporary)
        for variant, limit in [('grid', 16), ('materialless', 16), ('instances', 48), ('viewer', 48), ('coincident-keys', 16)]:
            source = (a.artifact_dir / variant / 'source.glb').read_bytes()
            archive = a.artifact_dir / variant / 'result.3tz'
            with zipfile.ZipFile(archive) as stream:
                members = {n: stream.read(n) for n in stream.namelist() if n != '@3dtilesIndex1@'}
            proof = oracle.inspect_members(source, members, 16, limit, 8)
            checked.append({'case': variant, 'archive_sha256': oracle.digest(archive.read_bytes()), 'proof': proof})
            for name, data in [('source.glb', source), *members.items()]:
                if not name.endswith('.glb'):
                    continue
                filename = variant + '/' + name
                target = root / filename
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
                plan.append({'input': filename})
        doc, binary, _ = oracle.decode((root / 'grid/t/root.glb').read_bytes())
        position = doc['accessors'][doc['meshes'][0]['primitives'][0]['attributes']['POSITION']]
        doc['bufferViews'][position['bufferView']]['byteLength'] = 0
        (root / 'zero-position-view.glb').write_bytes(oracle.glb(doc, binary))
        plan.append({'input': 'zero-position-view.glb', 'expectedCode': 'VALUE_NOT_IN_RANGE'})
        (root / 'validation-plan.json').write_text(json.dumps(plan))
        helper = Path(__file__).parent / 'fixtures/f1c2_validate.cjs'
        completed = subprocess.run(['node', str(helper), str(a.validator_modules.resolve(strict=True)), str(root)], text=True, capture_output=True, timeout=120)
        oracle.require(completed.returncode == 0, 'official core validation ' + completed.stdout + completed.stderr)
        receipt = json.loads(completed.stdout)
    paths = [Path(__file__), helper, Path(__file__).with_name('f1d1_oracle.py'), Path(__file__).with_name('f1c2_oracle.py')]
    receipt.update(artifacts=checked, authored_driver_sha256={p.name: oracle.digest(p.read_bytes()) for p in paths},
        scope='Official core errors/warnings for independent sources, roots and leaves; zero-position-view sensitive control. Metadata meaning belongs to the independent decoder and public Cesium queries.')
    a.json_output.parent.mkdir(parents=True, exist_ok=True)
    a.json_output.write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps({'validated': len(plan), 'receipt': str(a.json_output)}))


if __name__ == '__main__':
    main()
