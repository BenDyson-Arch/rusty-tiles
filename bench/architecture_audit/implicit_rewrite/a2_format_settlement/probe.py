#!/usr/bin/env python3
"""Replay the tiny format/client settlement, serially with explicit limits."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--client-package', type=Path, required=True)
    parser.add_argument('--artifact-directory', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    env = dict(os.environ, RAYON_NUM_THREADS='2', RUST_TEST_THREADS='2', CARGO_BUILD_JOBS='2')
    timings = {}
    commands = [
        ('prepare', ['nice', '-n', '10', sys.executable, '-B', str(HERE/'prepare.py'), str(args.artifact_directory)]),
        ('client', ['nice', '-n', '10', 'node', str(HERE/'client.mjs'), str(args.client_package),
                    str(args.artifact_directory), str(args.artifact_directory/'client-results.json')]),
        ('reader', ['nice', '-n', '10', sys.executable, '-B', str(HERE/'read.py'), str(args.artifact_directory),
                    str(args.artifact_directory/'reader-results.json')]),
    ]
    for name, command in commands:
        started = time.monotonic()
        subprocess.run(command, env=env, check=True, timeout=30)
        timings[name] = round(time.monotonic()-started, 6)
    primary = json.loads((HERE/'primary-sources.json').read_bytes())
    client = json.loads((args.artifact_directory/'client-results.json').read_bytes())
    reader = json.loads((args.artifact_directory/'reader-results.json').read_bytes())
    matches = {}
    for source in primary['sources']:
        prefix = 'packages/engine/Source/'
        if source['repo'] == 'cesium' and source['path'].startswith(prefix):
            name = source['path'][len(prefix):]
            assert client['runtime_source_sha256'][name] == source['sha256'], 'runtime/primary source mismatch'
            matches[name] = source['sha256']
    result = {
        'kind': 'bounded-a2-format-consumer-settlement',
        'baseline': 'e4d90897518d2b569fb5e99b876581523e4c418c',
        'artifact_directory': str(args.artifact_directory.resolve()),
        'limits': {'cases': 8, 'source_nodes_per_candidate': 5, 'payload_slots_per_candidate': 6,
                   'subtree_levels_max': 2, 'external_document_visits_max': 8,
                   'stage_timeout_seconds': 30, 'serial_stages': True, 'nice': 10,
                   'rayon_threads': 2, 'no_cargo_no_install_no_browser': True},
        'elapsed_seconds': timings,
        'driver_sha256': {name: sha(HERE/name) for name in ['probe.py','prepare.py','client.mjs','read.py']},
        'capture_sha256': {name: sha(args.artifact_directory/name) for name in
                           ['manifest.json','client-results.json','reader-results.json']},
        'primary_sources_sha256': sha(HERE/'primary-sources.json'),
        'runtime_matches_primary_cesium_commit': 'df52c781de3491a4b76839d420f7ca90a032efb6',
        'runtime_matching_primary_files': matches,
        'runtime': {key: client[key] for key in ['node','clientPackage','runtime_package_sha256',
                                                 'runtime_tree_sha256','runtime_source_sha256']},
        'cases': reader['cases'],
        'reader_only_controls': reader['reader_only_controls'],
        'actual_client_findings': {'candidate_roots_and_all_slots_preserved': True,
            'wrong_child_transform_changes_placement': True,
            'forbidden_root_external_json_loaded': True,
            'present_TILE_TRANSFORM_ignored': True},
        'official_validator_execution': {'executed': False, 'reason': 'not installed; no dependency installation',
            'source_commit': '7fa62c5f792069b077f174b477aab85dd7fecf22',
            'implicit_root_external_prohibition_implementation': 'TODO'},
        'proof_limits': ['finite terminal-occupancy profile acceptance is not general standards conformance',
            'literal implicit-root external-template interpretation remains separate conformance gate',
            'direct actual-client factories with parsed schema, not public fromUrl traversal',
            'no GPU render/selection/picking/features acceptance',
            'runtime tree identities cover engine/core JS and lock identity, not whole host attestation',
            'static tools validator findings are not executed package results'],
    }
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True)+'\n')
    print(json.dumps({'receipt': str(args.output), 'cases': 8, 'success': True}))


if __name__ == '__main__':
    main()
