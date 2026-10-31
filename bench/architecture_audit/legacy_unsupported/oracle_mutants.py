#!/usr/bin/env python3
"""Oracle sensitivity for the H7 rule: each mutant must make `check.py --oracle` fail.

    python3 -I oracle_mutants.py

Copies this lane and the two oracles (C2 `c2o`, O-implicit) into a temporary tree,
applies one exact text substitution per mutant, and runs the lane's oracle check and the
two oracle self-tests that do not check pins on the copy. Exits 0 only when the unmutated copy passes and every mutant is killed. Never
writes into the repository.
"""
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

AUDIT = Path(__file__).resolve().parent.parent
GLTF = 'c2_decoded_containment/oracle/c2o/gltf.py'
CHECK = 'c2_decoded_containment/oracle/c2o/check.py'
I1 = 'implicit_read/oracle/i1_oracle.py'

MUTANTS = [
    ('no-suffix-rule', GLTF,
     '        if fmt is not None:\n            raise LegacyContent(fmt, member, "suffix")',
     '        if False:\n            raise LegacyContent(fmt, member, "suffix")'),
    ('no-magic-rule', GLTF,
     '        if fmt is not None:\n            raise LegacyContent(fmt, member, "magic")',
     '        if False:\n            raise LegacyContent(fmt, member, "magic")'),
    ('magic-b3dm-only', GLTF,
     'return magic.decode("ascii") if magic in tuple(f.encode() for f in LEGACY_FORMATS) else None',
     'return magic.decode("ascii") if magic == b"b3dm" else None'),
    ('magic-anywhere', GLTF,
     '    magic = bytes(head[:4])\n',
     '    magic = next((f.encode() for f in LEGACY_FORMATS if f.encode() in bytes(head)), b"")\n'),
    ('case-sensitive-suffix', GLTF,
     '    suffix = "".join(chr(ord(c) + 32) if "A" <= c <= "Z" else c for c in tail[1])\n    return suffix if',
     '    suffix = tail[1]\n    return suffix if'),
    ('substring-suffix', GLTF,
     '    tail = member.rsplit(".", 1)\n    if len(tail) != 2 or "/" in tail[1]:\n        return None\n',
     '    for f in LEGACY_FORMATS:\n        if f in member.lower():\n            return f\n'
     '    tail = member.rsplit(".", 1)\n    if len(tail) != 2 or "/" in tail[1]:\n        return None\n'),
    ('no-cmpt', GLTF,
     'LEGACY_FORMATS = ("b3dm", "i3dm", "pnts", "cmpt")',
     'LEGACY_FORMATS = ("b3dm", "i3dm", "pnts")'),
    ('legacy-as-invalid-input', CHECK,
     'outcome = {"outcome": "Unsupported", "phase": "C1",',
     'outcome = {"outcome": "InvalidInput", "phase": "C1",'),
    ('classify-before-existence', GLTF,
     '        if member not in archive.infos:\n            # L0: existence precedes classification (C1 reports a missing member).\n'
     '            raise PayloadError("archive member %r is missing (C1 is expected to reject)" % member)\n'
     '        fmt = legacy_suffix(member)\n        if fmt is not None:\n            raise LegacyContent(fmt, member, "suffix")\n',
     '        fmt = legacy_suffix(member)\n        if fmt is not None:\n            raise LegacyContent(fmt, member, "suffix")\n'
     '        if member not in archive.infos:\n'
     '            raise PayloadError("archive member %r is missing (C1 is expected to reject)" % member)\n'),
    ('no-template-rule', I1,
     '        if legacy is not None:\n            unsupported(f"legacy {legacy} content template", stage)',
     '        if False:\n            unsupported(f"legacy {legacy} content template", stage)'),
    ('template-rule-on-subtree', I1,
     '    if is_content and out and out[-1][-1][0] == "lit":\n        legacy = _ascii_legacy_suffix',
     '    if out and out[-1][-1][0] == "lit":\n        legacy = _ascii_legacy_suffix'),
]


def copy_tree(target):
    for rel in ('legacy_unsupported', 'c2_decoded_containment/oracle', 'implicit_read/oracle'):
        shutil.copytree(AUDIT / rel, target / rel, ignore=shutil.ignore_patterns('__pycache__'))


# The two oracle self-tests that do not check pins (slice3.py does: every c2o mutant
# drifts, so it would kill them all for the wrong reason).
SELF_TESTS = ('c2_decoded_containment/oracle/self_test/run.py', 'implicit_read/oracle/self_test/test_oracle.py')


def oracle_check(root):
    result = subprocess.run([sys.executable, '-I', str(root / 'legacy_unsupported/check.py'), '--oracle'],
                            capture_output=True, text=True, timeout=1200)
    failed = [json.loads(line)['name'] for line in result.stdout.splitlines()
              if line.startswith('{') and not json.loads(line).get('passed', True)]
    code = result.returncode
    for rel in SELF_TESTS:
        test = subprocess.run([sys.executable, '-I', str(root / rel)], capture_output=True, text=True, timeout=1200)
        if test.returncode != 0:
            code = code or test.returncode
            failed.append('self-test:' + rel.split('/')[0])
    return code, failed


def main():
    ok = True
    with tempfile.TemporaryDirectory(prefix='h7-oracle-mutants-') as tmp:
        base = Path(tmp) / 'base'
        copy_tree(base)
        code, failed = oracle_check(base)
        print(json.dumps(dict(mutant='unmutated', exit=code, failed=failed)), flush=True)
        ok &= code == 0
        for name, rel, old, new in MUTANTS:
            root = Path(tmp) / name
            copy_tree(root)
            path = root / rel
            text = path.read_text()
            assert text.count(old) == 1, (name, 'anchor must match exactly once')
            path.write_text(text.replace(old, new))
            code, failed = oracle_check(root)
            killed = code != 0 and bool(failed)
            ok &= killed
            print(json.dumps(dict(mutant=name, killed=killed, flippedBy=failed)), flush=True)
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
