#!/usr/bin/env python3
"""Production mutants for the meshopt property-table / OCTAHEDRAL NORMAL slice.

    python3 -I mutants.py --worktree W --cargo 'flock -o LOCK taskset -c 16-19 cargo' --report R

Each mutant patches one Rust source (exactly one occurrence), runs the named tests and
passes only when they FAIL. The source is always restored and its hash rechecked.
"""
import argparse, hashlib, json, shlex, subprocess, time
from pathlib import Path

# (name, file, old, new, test filters, target)
MUTANTS = [
    ('lod-table-decoding-skipped', 'src/mesh_large/lod/codec.rs',
     '.and_then(|e| e.get("EXT_meshopt_compression"))\n        {',
     '.and_then(|e| e.get("EXT_meshopt_compression_skipped"))\n        {',
     ['public_mesh_to_tileset_publishes_certified_coarse_tiles'], '--lib'),
    ('lod-uint64-column-misread', 'src/mesh_large/lod/codec.rs',
     '"UINT64" | "FLOAT64" => 8,\n                    _ => return Err(invalid("property component type")),',
     '"UINT64" | "FLOAT64" => 4,\n                    _ => return Err(invalid("property component type")),',
     ['mesh_large::lod::tests'], '--lib'),
    ('validator-octahedral-filter-skipped', 'src/content_integrity/meshopt.rs',
     'if filter == Filter::Octahedral {\n        octahedral(&mut decoded, count, stride)?;',
     'if filter == Filter::Octahedral && false {\n        octahedral(&mut decoded, count, stride)?;',
     ['independent_corpus_rust_cli_parity_and_read_only_inputs'], '--test c1_validate'),
    ('validator-octahedral-code-check-skipped', 'src/content_integrity/meshopt.rs',
     'if !valid_one\n', 'if false && !valid_one\n',
     ['independent_corpus_rust_cli_parity_and_read_only_inputs'], '--test c1_validate'),
    ('writer-table-columns-raw', 'src/mesh_large/leaves/writer.rs',
     'if width.is_multiple_of(4) {\n            self.streams[index]',
     'if width == 0 {\n            self.streams[index]',
     ['provenance_columns_are_lossless_meshopt_streams'], '--lib'),
    ('writer-octahedral-column-swap', 'src/glb.rs',
     'flat_map(|v| [v[0], v[1], v[2], 0.])', 'flat_map(|v| [v[1], v[0], v[2], 0.])',
     ['leaf_normals_follow_the_profile', 'octahedral_normals_stay_within'], '--lib'),
]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worktree', type=Path, required=True)
    parser.add_argument('--cargo', required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    results = []
    for name, file, old, new, tests, target in MUTANTS:
        path = args.worktree / file
        original = path.read_text(); before = digest(path)
        assert original.count(old) == 1, (name, 'mutation site must be unique')
        path.write_text(original.replace(old, new))
        try:
            command = shlex.split(args.cargo) + ['test', *shlex.split(target), '--', *tests]
            start = time.monotonic()
            run = subprocess.run(command, cwd=args.worktree, capture_output=True, text=True)
            tail = (run.stdout + run.stderr)[-1500:]
            killed = run.returncode != 0 and ('test result: FAILED' in run.stdout or 'panicked' in run.stdout)
            results.append({'name': name, 'file': file, 'tests': tests, 'exit': run.returncode,
                            'killed': killed, 'seconds': round(time.monotonic() - start, 1), 'tail': tail})
        finally:
            path.write_text(original)
            assert digest(path) == before, (name, 'source not restored')
        print(name, 'KILLED' if results[-1]['killed'] else 'SURVIVED', flush=True)
    args.report.write_text(json.dumps({'mutants': results, 'allKilled': all(r['killed'] for r in results)}, indent=2) + '\n')


if __name__ == '__main__':
    main()
