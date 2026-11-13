#!/usr/bin/env python3
"""Record and verify the SHA-256s of release files (stdlib only, any platform).

    sha256s.py write SUMS FILE...       write "<sha256>  <name>" lines (sha256sum format)
    sha256s.py check --dir DIR SUMS...  verify DIR against one or more SUMS files

CI's portable and wheel jobs `write` the hashes of the archive and wheels they built, right
after building them, as the sha256-binary-<target> and sha256-wheel-<target> artifacts. A
release or dev build that reuses those files runs `check` on what it downloaded before it
smoke-tests or publishes anything. `check` fails unless the regular files in DIR are exactly
the names listed and every hash matches; a name listed twice with different hashes also
fails.
"""
import argparse
import hashlib
from pathlib import Path
import re
import sys

LINE = re.compile(r"([0-9a-f]{64}) [ *]([^/\\\n]+)")


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def write(sums, files):
    lines = sorted(f"{sha256_file(path)}  {Path(path).name}\n" for path in files)
    Path(sums).parent.mkdir(parents=True, exist_ok=True)
    Path(sums).write_text("".join(lines), encoding="utf-8")
    sys.stdout.write("".join(lines))


def read_sums(paths):
    expected, errors = {}, []
    for path in paths:
        for number, line in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), 1):
            if not line.strip():
                continue
            match = LINE.fullmatch(line)
            if match is None:
                errors.append(f"{path}:{number}: not a '<sha256>  <name>' line")
                continue
            digest, name = match.groups()
            if expected.get(name, digest) != digest:
                errors.append(f"{name} is listed with two different hashes")
            expected[name] = digest
    return expected, errors


def check(directory, sums):
    expected, errors = read_sums(sums)
    present = {path.name: path for path in Path(directory).iterdir() if path.is_file()}
    for name in sorted(set(expected) - set(present)):
        errors.append(f"{name}: listed but not downloaded")
    for name in sorted(set(present) - set(expected)):
        errors.append(f"{name}: downloaded but not recorded by CI")
    for name in sorted(set(expected) & set(present)):
        actual = sha256_file(present[name])
        if actual != expected[name]:
            errors.append(f"{name}: SHA-256 {actual} differs from CI's {expected[name]}")
        else:
            print(f"{name}: OK {actual}")
    if not expected and not errors:
        errors.append("no hashes to check")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    writer = commands.add_parser("write")
    writer.add_argument("sums")
    writer.add_argument("files", nargs="+")
    checker = commands.add_parser("check")
    checker.add_argument("--dir", required=True)
    checker.add_argument("sums", nargs="+")
    args = parser.parse_args()
    if args.command == "write":
        write(args.sums, args.files)
        return 0
    errors = check(args.dir, args.sums)
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
