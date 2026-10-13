#!/usr/bin/env python3
"""Mutants of the oracle and of production; each must flip its named control.

    python3 -I mutants.py oracle --report mutants/oracle-report.json
    python3 -I mutants.py production --worktree W --build-command '...' --binary B \
        --work DIR --report mutants/production-report.json

Oracle mutants patch ktx2_oracle.py text in a scratch copy and recompute every verdict;
a mutant passes when its named case's category differs from the frozen expectation.
Production mutants patch one Rust source in the worktree, rebuild with the supplied
command, run runner.py and pass when the named case fails; the source is always
restored and its hash rechecked.
"""
import argparse, hashlib, importlib.util, json, shlex, subprocess, sys, tempfile, shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent
ORACLE = [
    ('skip-identifier', 'if raw[:12] != IDENTIFIER:', 'if False:', 'bad-identifier'),
    ('skip-2d', 'if depth or layers or faces != 1:', 'if False:', 'non-2d-no-material'),
    ('non-2d-always-unsupported', 'INVALID if material else UNSUPPORTED', 'UNSUPPORTED', 'non-2d-material'),
    ('skip-vkformat', 'if vk != 0 or type_size != 1:', 'if False:', 'vk-format-defined'),
    ('skip-multiple-of-4', 'if width % 4 or height % 4:', 'if False:', 'not-multiple-of-4'),
    ('level-count-plus-one', 'level_count - 1 > max(width, height).bit_length() - 1',
     'level_count - 2 > max(width, height).bit_length() - 1', 'level-count-over-dimensions'),
    ('allow-zlib', 'if scheme not in (0, 1, 2):', 'if scheme not in (0, 1, 2, 3):', 'uastc-zlib-scheme'),
    ('allow-rrrg', 'channels not in ((0,), (3,), (4,), (6,))', 'channels not in ((0,), (3,), (4,), (5,), (6,))',
     'uastc-rrrg-channel'),
    ('any-color-model', "        bad('color-model')", '        pass', 'not-basis-color-model'),
    ('any-texel-block', 'if dfd[16:20] != bytes([3, 3, 0, 0]):', 'if False:', 'texel-block-6x4'),
    ('skip-dfd-size', 'if u32(dfd, 0) != dfd_length or dfd_length % 4:', 'if dfd_length % 4:', 'dfd-total-size'),
    ('skip-kvd-metadata', "                bad('kvd-metadata')", '                pass', 'orientation-ru'),
    ('skip-kvd-framing', "        found.append((category, rule))\n",
     "        if rule != 'kvd-pairs':\n            found.append((category, rule))\n", 'kvd-pair-length'),
    ('global-offset-ignored', 'if sgd_offset != 0 or scheme == 1:', 'if scheme == 1:', 'global-data-for-uastc'),
    ('scheme-0-unaligned', 'alignment = 16 if scheme == 0 else 1', 'alignment = 1', 'uastc-none-upstream'),
    ('skip-level-offset', 'if offset != align(cursor, alignment):', 'if False:', 'overlapping-levels'),
    ('skip-zstd-length', '(scheme == 2 and inflated != 16 * blocks)', '(scheme == 2 and False)', 'uncompressed-length'),
    ('allow-trailing', "        bad('trailing-bytes')", '        pass', 'trailing-bytes'),
    ('skip-basislz-length', 'or extended:\n            return found + [(INVALID, \'basislz-length\')]',
     'and False:\n            return found + [(INVALID, \'basislz-length\')]', 'basislz-global-length'),
    ('skip-image-descriptor', "                bad('image-descriptor')", '                pass', 'basislz-slice-past-level'),
    ('skip-mime', "if index in basisu_sources and (image.get('mimeType', 'image/ktx2')",
     "if False and (image.get('mimeType', 'image/ktx2')", 'mime-png'),
    ('suffix-instead-of-identifier', '    return data[:12] == IDENTIFIER',
     "    return image.get('uri', '').lower().endswith('.ktx2')", 'fallback-ktx2-by-identifier'),
    ('fallback-may-be-ktx2', 'if index in core_sources and EXT in used and declared_ktx2(image, data):', 'if False:',
     'fallback-is-ktx2'),
    ('data-uri-not-first', "        if image.get('uri', '').startswith('data:'):", '        if False:', 'data-uri-no-mime'),
    ('allow-red-srgb', "        bad('red-srgb')", '        pass', 'red-srgb'),
    ('allow-premultiplied', "        bad('premultiplied')", '        pass', 'premultiplied-material'),
    ('global-data-unaligned', 'if scheme != 1 or sgd_offset != align(end, 8):', 'if scheme != 1:', 'global-data-8-late'),
    ('kvd-anywhere', "        if kvd_offset != end:\n            bad('kvd-position')",
     '        if False:\n            pass', 'kvd-misplaced'),
    ('allow-empty-key', " or pair[0] == 0:", ':', 'kvd-empty-key'),
    ('allow-pframe-flag', 'if flags or rgb_length == 0', 'if rgb_length == 0', 'basislz-pframe-flag'),
    ('allow-extended', '+ tables + extended or extended:', '+ tables + extended:', 'basislz-extended'),
    ('alpha-may-be-empty', '(alpha_length != 0 and alpha_offset', '(alpha_offset', 'basislz-alpha-missing'),
    ('allow-etc1s-rgb-ggg', 'channels not in ((0,), (0, 15), (3,), (3, 4))',
     'channels not in ((0,), (0, 15), (0, 4), (3,), (3, 4))', 'etc1s-rgb-ggg'),
    ('any-dfd-vendor', 'if u32(dfd, 4) != 0 or', 'if u32(dfd, 4) > 1 or', 'dfd-vendor'),
    ('any-cubemap', ' or (faces == 6 and (height != width or depth))', '', 'cubemap-not-square-no-material'),
    ('fallback-not-required', "if 'source' not in texture and EXT not in required:", 'if False:', 'no-fallback-not-required'),
    ('skip-declaration', 'if EXT not in used:', 'if False:', 'undeclared-extension'),
]
# Each Rust mutant: (name, file, old, new, named case).
PRODUCTION = [
    ('skip-identifier', 'src/content_integrity/payload/ktx2.rs', 'if bytes.get(..12) != Some(&IDENTIFIER[..]) {',
     'if bytes.len() < 12 {', 'bad-identifier'),
    ('non-2d-always-unsupported', 'src/content_integrity/payload/ktx2.rs',
     'return Err(if material {', 'return Err(if material && false {', 'non-2d-material'),
    ('skip-multiple-of-4', 'src/content_integrity/payload/ktx2.rs', 'if width % 4 != 0 || height % 4 != 0 {',
     'if false {', 'not-multiple-of-4'),
    ('level-count-plus-one', 'src/content_integrity/payload/ktx2.rs', 'level_count > max_levels',
     'level_count > max_levels + 1', 'level-count-over-dimensions'),
    ('allow-zlib', 'src/content_integrity/payload/ktx2.rs', 'if !matches!(scheme, NONE | ZSTD) {',
     'if !matches!(scheme, NONE | ZSTD | 3) {', 'uastc-zlib-scheme'),
    ('scheme-0-unaligned', 'src/content_integrity/payload/ktx2.rs',
     'let alignment = if scheme == NONE { UASTC_BLOCK } else { 1 };', 'let alignment = 1;', 'uastc-none-upstream'),
    ('skip-level-offset', 'src/content_integrity/payload/ktx2.rs', 'if offset != align_up(cursor, alignment)? {',
     'if offset < align_up(0, alignment)? {', 'overlapping-levels'),
    ('allow-trailing', 'src/content_integrity/payload/ktx2.rs', 'if cursor != bytes.len() as u64 {',
     'if cursor > bytes.len() as u64 {', 'trailing-bytes'),
    ('skip-zstd-length', 'src/content_integrity/payload/ktx2.rs', '_ => u128::from(inflated) == uastc,',
     '_ => true,', 'uncompressed-length'),
    ('skip-image-descriptor', 'src/content_integrity/payload/ktx2.rs',
     'if flags != 0 || rgb_length == 0 || rgb_offset + rgb_length > *length || !alpha_ok {',
     'if flags != 0 || rgb_length == 0 || !alpha_ok {', 'basislz-slice-past-level'),
    ('skip-kvd-metadata', 'src/content_integrity/payload/ktx2.rs', '|| (key == b"KTXorientation" && value != b"rd")',
     '', 'orientation-ru'),
    ('any-texel-block', 'src/content_integrity/payload/ktx2.rs', 'if dfd[16..20] != [3, 3, 0, 0] {',
     'if dfd[17..20] != [3, 0, 0] {', 'texel-block-6x4'),
    ('skip-mime', 'src/content_integrity/payload.rs', '|| (mime.is_none() && image.view.is_some()))',
     '|| false)', 'mime-missing-buffer-view'),
    ('fallback-may-be-ktx2', 'src/content_integrity/payload.rs', 'if typed && roles & 1 != 0 && ktx2 {',
     'if typed && roles & 1 != 0 && false {', 'fallback-is-ktx2'),
    ('suffix-free-identifier-ignored', 'src/content_integrity/payload.rs',
     '|| data.starts_with(&ktx2::IDENTIFIER),', '|| false,', 'fallback-ktx2-by-identifier'),
    ('data-uri-not-first', 'src/content_integrity/payload.rs',
     'if uri.as_deref().is_some_and(|uri| uri.starts_with("data:")) {', 'if false {', 'data-uri-no-mime'),
    ('per-image-inspection', 'src/content_integrity/payload.rs',
     'Some(view) => ktx2_views[view] |= if roles & 4 != 0 { 3 } else { 1 },',
     'Some(_) => ktx2::inspect(&data, roles & 4 != 0)?,', 'shared-view-heavy-kvd'),
    ('global-data-unaligned', 'src/content_integrity/payload/ktx2.rs',
     'if scheme != BASIS_LZ || sgd_offset != align_up(end, 8)? {', 'if scheme != BASIS_LZ {', 'global-data-8-late'),
    ('allow-red-srgb', 'src/content_integrity/payload/ktx2.rs', 'if red && dfd[14] == TRANSFER_SRGB {',
     'if red && false {', 'red-srgb'),
    ('allow-premultiplied', 'src/content_integrity/payload/ktx2.rs',
     'if material && dfd[15] & FLAG_PREMULTIPLIED != 0 {', 'if material && false {', 'premultiplied-material'),
    ('kvd-anywhere', 'src/content_integrity/payload/ktx2.rs', 'if kvd_offset != end {', 'if false {', 'kvd-misplaced'),
    ('allow-empty-key', 'src/content_integrity/payload/ktx2.rs', 'nul.is_none_or(|nul| nul == 0)', 'nul.is_none()',
     'kvd-empty-key'),
    ('allow-pframe-flag', 'src/content_integrity/payload/ktx2.rs', 'if flags != 0 || rgb_length == 0',
     'if rgb_length == 0', 'basislz-pframe-flag'),
    ('allow-extended', 'src/content_integrity/payload/ktx2.rs', '|| extended != 0 {', '{', 'basislz-extended'),
    ('alpha-may-be-empty', 'src/content_integrity/payload/ktx2.rs',
     'alpha_length != 0 && alpha_offset + alpha_length <= *length', 'alpha_offset + alpha_length <= *length',
     'basislz-alpha-missing'),
    ('allow-etc1s-rgb-ggg', 'src/content_integrity/payload/ktx2.rs', '| (2, [0, 15]) |', '| (2, [0, 15 | 4]) |',
     'etc1s-rgb-ggg'),
    ('any-dfd-vendor', 'src/content_integrity/payload/ktx2.rs', 'if word(dfd, 4)? != 0', 'if word(dfd, 4)? > 1',
     'dfd-vendor'),
    ('any-cubemap', 'src/content_integrity/payload/ktx2.rs', 'if faces == 6 && (height != width || depth != 0) {',
     'if false {', 'cubemap-not-square-no-material'),
    ('fallback-not-required', 'src/content_integrity/payload.rs',
     'if !fallback && !required.contains("KHR_texture_basisu") {', 'if !fallback && false {',
     'no-fallback-not-required'),
    ('skip-container', 'src/content_integrity/payload.rs', 'ktx2::inspect(&views[view], flags & 2 != 0)?;',
     'let _ = view;', 'truncated-base-level'),
]


def sha(raw): return hashlib.sha256(raw).hexdigest()


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module); return module


def oracle_mutants(args):
    manifest = json.loads((HERE / 'fixtures/manifest.json').read_text())
    expected = {c['name']: c['expectedCategory'] for c in manifest['cases']}
    source = (HERE / 'ktx2_oracle.py').read_text()
    generator = load(HERE / 'generator.py', 'ktx2_generator')
    rows = []
    for name, old, new, named in ORACLE:
        assert source.count(old) == 1, name
        work = Path(tempfile.mkdtemp(dir=Path.home() / '.cache'))
        try:
            (work / 'ktx2_oracle.py').write_text(source.replace(old, new))
            mutant = load(work / 'ktx2_oracle.py', 'mutant_' + name.replace('-', '_'))
            cases = generator.generate(work / 'fixtures', mutant)
        finally:
            shutil.rmtree(work)
        flipped = sorted(c['name'] for c in cases if c['oracleCategory'] != expected[c['name']])
        rows.append(dict(mutant=name, namedCase=named, flipped=flipped, pass_=named in flipped,
                         patch=dict(old=old, new=new)))
        print(json.dumps(dict(mutant=name, named=named, passed=named in flipped, flipped=len(flipped))), flush=True)
    report = dict(kind='oracle', oracleSha256=sha((HERE / 'ktx2_oracle.py').read_bytes()),
                  manifestSha256=sha((HERE / 'fixtures/manifest.json').read_bytes()), mutants=rows,
                  passes=sum(r['pass_'] for r in rows), failures=sum(not r['pass_'] for r in rows))
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    return 0 if report['failures'] == 0 else 1


def production_mutants(args):
    runner = load(HERE / 'runner.py', 'ktx2_runner')
    args.work.mkdir(parents=True, exist_ok=False)
    pin = args.work / 'source-pin.json'
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=args.worktree, text=True).strip()
    def snapshot():
        return {str(f): sha(f.read_bytes()) for f in sorted((args.worktree / 'src').rglob('*.rs'))}
    before = snapshot()
    rows = []
    for name, rel, old, new, named in PRODUCTION:
        path = args.worktree / rel
        original = path.read_bytes()
        text = original.decode()
        assert text.count(old) == 1, name
        try:
            path.write_text(text.replace(old, new))
            build = subprocess.run(shlex.split(args.build_command), cwd=args.worktree, capture_output=True, text=True)
            built = build.returncode == 0
            failed = []
            if built:
                pin.write_text(json.dumps(dict(source_commit=head, mutant=name, file=rel, old=old, new=new)) + '\n')
                binary = args.binary.read_bytes()
                status = runner.run(argparse.Namespace(binary=args.binary, binary_sha256=sha(binary),
                                                       source_pin=pin, source_pin_sha256=sha(pin.read_bytes()),
                                                       run_dir=args.work / name))
                receipt = json.loads((args.work / name / 'receipt.json').read_text())
                failed = sorted(c['name'] for c in receipt['cases'] if not c['passed'])
        finally:
            path.write_bytes(original)
        assert sha(path.read_bytes()) == sha(original)
        rows.append(dict(mutant=name, file=rel, old=old, new=new, namedCase=named, built=built,
                         buildStderrTail=build.stderr[-2000:] if not built else '', failedCases=failed,
                         pass_=built and named in failed))
        print(json.dumps(dict(mutant=name, built=built, named=named, passed=rows[-1]['pass_'], failed=len(failed))), flush=True)
    report = dict(kind='production', sourceCommit=head, sourceSha256=before, sourceRestored=snapshot() == before,
                  mutants=rows,
                  passes=sum(r['pass_'] for r in rows), failures=sum(not r['pass_'] for r in rows))
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    return 0 if report['failures'] == 0 and report['sourceRestored'] else 1


if __name__ == '__main__':
    p = argparse.ArgumentParser(); sub = p.add_subparsers(dest='kind', required=True)
    o = sub.add_parser('oracle'); o.add_argument('--report', type=Path, required=True)
    q = sub.add_parser('production'); q.add_argument('--worktree', type=Path, required=True)
    q.add_argument('--build-command', required=True); q.add_argument('--binary', type=Path, required=True)
    q.add_argument('--work', type=Path, required=True); q.add_argument('--report', type=Path, required=True)
    a = p.parse_args()
    sys.exit(oracle_mutants(a) if a.kind == 'oracle' else production_mutants(a))
