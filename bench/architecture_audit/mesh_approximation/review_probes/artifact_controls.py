#!/usr/bin/env python3
"""Small independent reviewer fixture executions; pass a frozen CLI binary."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'tests'))
import f1d1_oracle as oracle


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--json-output', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    records = []
    with tempfile.TemporaryDirectory(prefix='f1d-review-') as temporary:
        work = Path(temporary)
        for name, variant, size, limit in [
            ('materialless', 'materialless', 4, 8),
            ('coincident-key-floor', 'coincident-keys', 2, 1),
            ('coincident-key-reduction', 'coincident-keys', 2, 8),
        ]:
            source = oracle.fixture(variant, size)
            input_path = work / (name + '.glb')
            input_path.write_bytes(source)
            output = work / name / 'result.3tz'
            command = [str(binary), '--json', 'mesh-local-to-3tz', '-i', str(input_path),
                       '-o', str(output), '--leaf-triangles', '8', '--root-proxy-triangles',
                       str(limit), '--max-proxy-error-metres', '8']
            execution = subprocess.run(command, capture_output=True, text=True, timeout=60)
            record = {'case': name, 'source_sha256': oracle.digest(source),
                      'exit_code': execution.returncode, 'output_parent_exists': output.parent.exists()}
            if name == 'coincident-key-floor':
                assert execution.returncode != 0 and not output.parent.exists()
                record['error'] = json.loads(execution.stdout)['error']
                assert record['error']['kind'] == 'unsupported'
            else:
                assert execution.returncode == 0, execution.stdout + execution.stderr
                with zipfile.ZipFile(output) as archive:
                    members = {n: archive.read(n) for n in archive.namelist() if n != '@3dtilesIndex1@'}
                result = oracle.inspect_members(source, members, 8, limit, 8)
                record['archive_sha256'] = oracle.digest(output.read_bytes())
                record['checked'] = {k: result[k] for k in
                                     ('source_triangles', 'proxy_triangles', 'ideal_certificate_squared')}
            records.append(record)
    result = {'production_binary_sha256': oracle.digest(binary.read_bytes()),
              'driver_sha256': oracle.digest(Path(__file__).read_bytes()),
              'oracle_sha256': oracle.digest(Path(oracle.__file__).read_bytes()), 'cases': records}
    encoded = json.dumps(result, indent=2) + '\n'
    if args.json_output:
        args.json_output.write_text(encoded)
    else:
        print(encoded, end='')


if __name__ == '__main__':
    main()
