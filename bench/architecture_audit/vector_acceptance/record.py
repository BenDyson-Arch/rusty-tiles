#!/usr/bin/env python3
"""Condense completed independent executions without importing product code."""
import argparse
import hashlib
import json
import pathlib
import platform
import subprocess
import sys

parser = argparse.ArgumentParser()
parser.add_argument('--source-commit', required=True)
parser.add_argument('--oracle', type=pathlib.Path, required=True)
parser.add_argument('--resources', type=pathlib.Path, required=True)
parser.add_argument('--output', type=pathlib.Path, required=True)
args = parser.parse_args()
raw = json.loads(args.oracle.read_text())
resources = json.loads(args.resources.read_text())
assert raw['binary_sha256'] == resources['binary_sha256']
assert raw['all_assertions_passed']
observations = {}
combined_cases = dict(raw['cases'])
combined_cases.update(raw.get('native_gpkg', {}))
for name, case in combined_cases.items():
    item = {k: case[k] for k in ('command', 'exit_code', 'source_sha256', 'preserved_output_sha256', 'preserved_source_sha256') if k in case}
    if 'inventory' in case:
        inv = case['inventory']
        counts = inv['report']
        item.update(identities=inv['identities'], content_sha256=inv['payloads'],
                    root_transform=inv['root_transform'], diagnostics=inv['diagnostics'],
                    counts={k: counts[k] for k in ('features','fragments','skippedFeatures','fragmentedPolygons','maximumTileBytes','maximumTileVertices')},
                    reuse=counts['reuse'])
    else:
        try:
            error = json.loads(case.get('stdout', '{}')).get('error', {})
        except json.JSONDecodeError:
            error = {}
        item['error'] = error
    observations[name] = item
for case in resources['cases']:
    case.pop('report', None)
record = dict(source_commit=args.source_commit, binary_sha256=raw['binary_sha256'],
              environment=dict(platform=platform.platform(), python=sys.version,
                               rustc=subprocess.check_output(['rustc','--version'], text=True).strip()),
              provenance='Parent built this binary after freezing the stated commit; this recorder verifies oracle/resource binary hashes agree. Earlier candidate runs are not attributed to frozen source.',
              raw_evidence=dict(oracle=str(args.oracle), oracle_sha256=hashlib.sha256(args.oracle.read_bytes()).hexdigest(),
                                resources=str(args.resources), resources_sha256=hashlib.sha256(args.resources.read_bytes()).hexdigest()),
              replay_commands=[f'python tests/vector_acceptance_oracle.py --binary BINARY --evidence DIRECTORY' + (' --native' if 'native_gpkg' in raw else ''),
                               f'python bench/architecture_audit/vector_acceptance/measure.py --binary BINARY --output RESOURCE_JSON'],
              observations=observations, resources=resources['cases'], measurement_caveat=resources['caveat'])
args.output.write_text(json.dumps(record, indent=2)+'\n')
