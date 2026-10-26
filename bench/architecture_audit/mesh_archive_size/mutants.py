#!/usr/bin/env python3
"""Production and oracle mutants for the meshopt property-table / OCTAHEDRAL NORMAL slice.

    python3 -I mutants.py --worktree W --cargo 'flock -o LOCK taskset -c 16-19 cargo' --report R

Each mutant patches one or more sites (each exactly once), runs its named check and passes
only when that check FAILS. Sources are always restored and their hashes rechecked.
Production mutants run cargo tests; oracle mutants patch the C2 oracle's meshopt decoder and
run its self-test. A-H are the independent #226 audit's production mutants.
"""
import argparse, hashlib, json, shlex, subprocess, time
from pathlib import Path

MESHOPT = 'src/content_integrity/meshopt.rs'
PAYLOAD = 'src/content_integrity/payload.rs'
ORACLE = 'bench/architecture_audit/c2_decoded_containment/oracle/c2o/meshopt.py'
C1 = ('cargo', '--test c1_validate', ['independent_corpus_rust_cli_parity_and_read_only_inputs'])
SELF_TEST = ('python', 'bench/architecture_audit/c2_decoded_containment/oracle/self_test/run.py', [])

# (name, [(file, old, new)], check)
MUTANTS = [
    # The client-order decoder moved from H17's lod/codec.rs to hlod/decode.rs in S6.
    ('lod-table-decoding-skipped', [('src/mesh_large/hlod/decode.rs',
      '.and_then(|e| e.get("EXT_meshopt_compression"))\n        {',
      '.and_then(|e| e.get("EXT_meshopt_compression_skipped"))\n        {')],
     ('cargo', '--lib', ['mesh_large::hlod::tests::decoder_reads_leaf_provenance_columns_exactly'])),
    ('lod-uint64-column-misread', [('src/mesh_large/hlod/decode.rs',
      '"UINT64" | "FLOAT64" => 8,\n                    _ => return Err(invalid("property component type")),',
      '"UINT64" | "FLOAT64" => 4,\n                    _ => return Err(invalid("property component type")),')],
     ('cargo', '--lib', ['mesh_large::hlod::tests::decoder_reads_leaf_provenance_columns_exactly'])),
    ('validator-octahedral-filter-skipped', [(MESHOPT,
      'if filter == Filter::Octahedral {\n        octahedral(&mut decoded, count, stride)?;',
      'if filter == Filter::Octahedral && false {\n        octahedral(&mut decoded, count, stride)?;')], C1),
    ('validator-octahedral-one-check-skipped', [(MESHOPT, 'if !valid_one ||', 'if false && !valid_one ||')], C1),
    ('writer-table-columns-raw', [('src/mesh_large/leaves/writer.rs',
      'if width.is_multiple_of(4) {\n            self.streams[index]', 'if width == 0 {\n            self.streams[index]')],
     ('cargo', '--lib', ['provenance_columns_are_lossless_meshopt_streams'])),
    ('writer-octahedral-column-swap', [('src/glb.rs',
      'flat_map(|v| [v[0], v[1], v[2], 0.])', 'flat_map(|v| [v[1], v[0], v[2], 0.])')],
     ('cargo', '--lib', ['leaf_normals_follow_the_profile', 'octahedral_normals_stay_within'])),
    # Independent audit A-H.
    ('A-x-bound-off-by-one', [(MESHOPT, 'read(base).abs() > one', 'read(base).abs() >= one')], C1),
    ('B-y-bound-skipped', [(MESHOPT, ' || read(base + component).abs() > one', '')], C1),
    ('C-stride-12-accepted', [(MESHOPT, '(stride != 4 && stride != 8)', '(stride % 4 != 0)'),
      (PAYLOAD, '"ATTRIBUTES" if octahedral && stride != 4 && stride != 8 => {', '"ATTRIBUTES" if octahedral && stride % 4 != 0 => {')], C1),
    ('D-mode-check-removed', [(MESHOPT, '(!matches!(mode, Mode::Attributes) || (stride != 4 && stride != 8))', '(stride != 4 && stride != 8)'),
      (PAYLOAD, '"TRIANGLES" | "INDICES" if octahedral => {', '"TRIANGLES" | "INDICES" if false => {')], C1),
    ('E-filter-ignored', [(PAYLOAD, 'decode_filtered(compressed, count, stride, mode, filter)',
      'decode_filtered(compressed, count, stride, mode, super::meshopt::Filter::None)')], C1),
    ('F-quaternion-as-none', [(MESHOPT, '"OCTAHEDRAL" => Some(Self::Octahedral),',
      '"OCTAHEDRAL" => Some(Self::Octahedral),\n            "QUATERNION" => Some(Self::None),')], C1),
    ('G-width-swapped', [(MESHOPT, 'if component == 1 {', 'if component == 2 {')], C1),
    ('H-one-pow2-check-weakened', [(MESHOPT, '(one + 1).count_ones() == 1', '(one + 1).count_ones() <= 2')], C1),
    # Bound tolerance (audit SHOULD 3).
    ('bounds-tolerance-removed', [(PAYLOAD, 'Some(v) if filtered[v] && [5120, 5122].contains(&component) => 1.,',
      'Some(v) if filtered[v] && [5120, 5122].contains(&component) => 0.,')], C1),
    ('bounds-tolerance-on-every-view', [(PAYLOAD, 'Some(v) if filtered[v] && [5120, 5122].contains(&component) => 1.,',
      'Some(_) => 1.,')], C1),
    ('bounds-tolerance-two-units', [(PAYLOAD, 'Some(v) if filtered[v] && [5120, 5122].contains(&component) => 1.,',
      'Some(v) if filtered[v] && [5120, 5122].contains(&component) => 3.,')], C1),
    # C2 oracle decoder mutants (self-test must fail).
    ('oracle-mode-check-removed', [(ORACLE, 'if flt == "OCTAHEDRAL" and (mode != "ATTRIBUTES" or stride not in (4, 8)):',
      'if flt == "OCTAHEDRAL" and stride not in (4, 8):')], SELF_TEST),
    ('oracle-stride-checks-removed', [(ORACLE, 'if flt == "OCTAHEDRAL" and (mode != "ATTRIBUTES" or stride not in (4, 8)):',
      'if flt == "OCTAHEDRAL" and mode != "ATTRIBUTES":'),
      (ORACLE, '    if stride not in (4, 8) or len(raw) != count * stride:', '    if len(raw) != count * stride:'),
      (ORACLE, 'fmt, bits = ("<4b", 8) if stride == 4 else ("<4h", 16)', 'fmt, bits = ("<4b", 8) if stride == 4 else ("<4h" + "x" * (stride - 8), 16)')], SELF_TEST),
    ('oracle-y-bound-skipped', [(ORACLE, ' or abs(y) > one:', ':')], SELF_TEST),
    ('oracle-x-bound-off-by-one', [(ORACLE, 'abs(x) > one', 'abs(x) >= one')], SELF_TEST),
    ('oracle-filter-ignored', [(ORACLE, 'return octahedral(raw, count, stride) if flt == "OCTAHEDRAL" else raw', 'return raw')], SELF_TEST),
]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worktree', type=Path, required=True)
    parser.add_argument('--cargo', required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--only', nargs='*')
    args = parser.parse_args()
    results = []
    for name, edits, (kind, target, tests) in MUTANTS:
        if args.only and name not in args.only:
            continue
        originals = {}
        try:
            for file, old, new in edits:
                path = args.worktree / file
                if file not in originals:
                    originals[file] = (path.read_text(), digest(path))
                text = path.read_text()
                assert text.count(old) == 1, (name, file, 'mutation site must be unique')
                path.write_text(text.replace(old, new))
            if kind == 'cargo':
                command = shlex.split(args.cargo) + ['test', *shlex.split(target), '--', *tests]
            else:
                command = ['python3', '-I', target]
            start = time.monotonic()
            run = subprocess.run(command, cwd=args.worktree, capture_output=True, text=True)
            out = run.stdout + run.stderr
            if kind == 'cargo':
                # A failing test or an aborted test process (e.g. the native filter's stride
                # assertion) kills the mutant; a compile error does not.
                killed = run.returncode != 0 and 'could not compile' not in out and (
                    'test result: FAILED' in out or 'panicked' in out or 'process didn\'t exit successfully' in out)
            else:
                killed = run.returncode != 0 or 'FAIL ' in run.stdout
            results.append({'name': name, 'files': sorted(originals), 'check': [kind, target, tests], 'exit': run.returncode,
                            'killed': killed, 'seconds': round(time.monotonic() - start, 1),
                            'evidence': [l[:300] for l in out.splitlines() if 'panicked' in l or l.startswith('FAIL') or 'lacks' in l or 'Assertion' in l or 'signal' in l][:6]})
        finally:
            for file, (text, before) in originals.items():
                path = args.worktree / file
                path.write_text(text)
                assert digest(path) == before, (name, 'source not restored')
        print(name, 'KILLED' if results[-1]['killed'] else 'SURVIVED', flush=True)
    args.report.write_text(json.dumps({'mutants': results, 'allKilled': all(r['killed'] for r in results)}, indent=2) + '\n')


if __name__ == '__main__':
    main()
