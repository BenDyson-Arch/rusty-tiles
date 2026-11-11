#!/usr/bin/env python3
"""Replay cache invariance and pruning accept-preservation against sensitive mutants.

Uses source-extracted Grid/predicates and a tiny effort shim with the same additive
budget rule. The native unit test is authoritative for actual Budget/Error wiring.
OUT holds generated sources, binaries and logs; no producer or Cargo dependency.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess

from certificate_probe import harness


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('out', type=Path)
    out = parser.parse_args().out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    root = Path(__file__).resolve().parents[4]
    source = root / 'src/validate/hlod_delivery/drops.rs'
    shutil.copyfile(source, out / 'drops.rs')
    shutil.copyfile(source.with_suffix('') / 'certificate.rs', out / 'certificate.rs')
    harness(out, out / 'kernel.rs')
    code = (out / 'kernel.rs').read_text()
    start, end = code.index('#[derive(Default)] struct Effort'), code.index('fn point(')
    code = code[:start] + r'''struct Budget { value:u64 }
struct Effort { certificate_fast:AtomicU64,certificate_slow:AtomicU64,budget:Budget,base:u64,used:AtomicU64 }
impl Default for Effort {fn default()->Self {Self {certificate_fast:AtomicU64::new(0),certificate_slow:AtomicU64::new(0),budget:Budget{value:10_000_000},base:0,used:AtomicU64::new(0)}}}
impl Effort {fn charge(&self,n:usize)->Result<(),Error>{let old=self.used.fetch_add(n as u64,Ordering::Relaxed);let total=self.base+old+n as u64;if total>self.budget.value {Err(Error::ResourceLimit(format!("budget exceeded: {total}")))}else{Ok(())}}}
fn effort_for_tests()->Effort {Effort::default()}
''' + code[end:]
    src = source.read_text()
    test = src[src.index('    #[test]\n    fn cached_faces'):src.index('    /// A third component')]
    test = test.replace('        use super::super::tests::effort_for_tests;\n', '')
    test = test.replace('        use super::{point, Grid};\n', '')
    test = test.replace('        use super::{distance2, point, Grid};\n', '')
    code += (r'''#[cfg(test)] mod budget_control {use super::*;
type Face=[[f64;3];3];
const NEAR:Face=[[0.,0.,0.],[2.,0.,0.],[0.,2.,0.]];
const FAR:Face=[[5.,0.,0.],[6.,0.,0.],[5.,1.,0.]];
''' + test + '}\n')
    results = {}
    for variant in ['fixed', 'priority', 'in-cell-priority']:
        mutant = code
        if variant != 'fixed':
            loop = ('let last = self.certified.get().map(|(i, _)| i);\n'
                    'for i in last.into_iter().chain(cell.iter().copied().filter(|i| Some(*i) != last)) {')
            if variant == 'in-cell-priority':
                loop = loop.replace('.map(|(i, _)| i);', '.map(|(i, _)| i).filter(|i| cell.contains(i));')
            begin, end = mutant.index('    fn covers('), mutant.index('    /// Whether `p`')
            certificate = mutant[begin:end]
            assert certificate.count('for &i in cell {') == 1
            mutant = mutant[:begin] + certificate.replace('for &i in cell {', loop) + mutant[end:]
        path, binary = out / (variant + '.rs'), out / variant
        path.write_text(mutant)
        subprocess.run(['rustc', '--edition=2021', '--test', str(path), '-o', str(binary)], check=True)
        run = subprocess.run([str(binary), 'budget_control::cached_faces_keep_certificate_work_and_honest_point_queries',
                              '--exact', '--nocapture'], text=True, capture_output=True)
        (out / (variant + '.log')).write_text(run.stdout + run.stderr)
        assert 'running 1 test' in run.stdout, run.stdout
        results[variant] = run.returncode
    assert results == {'fixed': 0, 'priority': 101, 'in-cell-priority': 101}, results
    # Reconstruct the previous scan by removing only the pruning branch. Keep
    # geometry, numeric predicates, subdivision, candidate order and accounting.
    start = code.index('    fn covers(')
    end = code.index('    /// Whether `p`', start)
    original = code[start:end].replace('fn covers(', 'fn covers_unpruned(', 1)
    begin = original.index('                    if certificate::outside_box(center,')
    stop = original.index('                    let d = distance(center,', begin)
    pruning = original[begin:stop]
    assert 'charge_distance()?;' in pruning and 'continue;' in pruning
    original = original[:begin] + original[stop:]
    differential = code + '\nimpl Grid {\n' + original + '}\n'
    differential += Path(__file__).with_name('pruning_control.rs').read_text()
    for variant in ['pruning', 'pruning-reject-mutant']:
        mutant = differential
        if variant != 'pruning':
            assert mutant.count(pruning) == 1
            mutant = mutant.replace(pruning, pruning.replace('continue;', 'return Ok(false);'))
        path, binary = out / (variant + '.rs'), out / variant
        path.write_text(mutant)
        subprocess.run(['rustc', '--edition=2021', '--test', str(path), '-o', str(binary)], check=True)
        run = subprocess.run([str(binary), 'pruning_never_turns_an_accept_into_a_refusal',
                              '--nocapture'], text=True, capture_output=True)
        (out / (variant + '.log')).write_text(run.stdout + run.stderr)
        assert 'running 1 test' in run.stdout, run.stdout
        results[variant] = run.returncode
    assert results['pruning'] == 0 and results['pruning-reject-mutant'] == 101, results
    (out / 'result.json').write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results))


if __name__ == '__main__':
    main()
