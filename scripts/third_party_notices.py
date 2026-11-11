#!/usr/bin/env python3
"""Stage and check the third-party licence notices that ship with rusty-tiles.

Binaries, wheels, the container image and the crate built from this tree may
contain vendored third-party code (vendor/README.md). Its upstream licence and
NOTICE files must travel with every artefact:

  third_party_notices.py stage DIR
      copy the notices to DIR/third_party/<component>/<path>
  third_party_notices.py check [--max-bytes N] [--rust-licenses KIND] ARTEFACT...
      require every notice, byte-identical, in each .tgz/.tar.gz/.crate/.zip/.whl
      (members named .../<component>/<path>); optionally bound the artefact size.
      --rust-licenses cli|native-geospatial|python additionally requires the generated
      Rust crate licence file of that kind (licenses/, scripts/third_party_licenses.py) as
      .../THIRD_PARTY_LICENSES.md or .../THIRD_PARTY_LICENSES-KIND.md; "all" is for the
      crate, which carries all three under licenses/

Both commands first check that the list below covers every licence-like file in
the vendored trees, so an upstream update cannot add one unnoticed.
"""
import argparse
import pathlib
import sys
import tarfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
# component directory under vendor/ -> its notice files, relative to it
NOTICES = {
    "basis_universal": [
        "LICENSE",
        "NOTICE",
        "LICENSES/Apache-2.0.txt",
        "LICENSES/BSD-3-clause.txt",
        "LICENSES/MIT.txt",
        "zstd/LICENSE",
    ],
}
LICENCE_LIKE = ("LICENSE", "LICENCE", "NOTICE", "COPYING")


def notice_files():
    files = []
    for component, paths in NOTICES.items():
        base = ROOT / "vendor" / component
        listed = set(paths)
        found = {
            path.relative_to(base).as_posix()
            for path in base.rglob("*")
            if path.is_file() and (path.name.upper().startswith(LICENCE_LIKE) or "LICENSES" in path.parts)
        }
        if found != listed:
            sys.exit(f"error: vendor/{component} notices {sorted(found)} differ from the list {sorted(listed)}")
        files += [(component, path, (base / path).read_bytes()) for path in paths]
    return files


RUST_LICENCE_KINDS = ("cli", "native-geospatial", "python")


def rust_licence_files(kind):
    """(display name, accepted member-name suffixes, bytes) for the generated Rust licence files."""
    kinds = RUST_LICENCE_KINDS if kind == "all" else (kind,)
    files = []
    for each in kinds:
        path = ROOT / "licenses" / f"THIRD_PARTY_LICENSES-{each}.md"
        suffixes = [f"/THIRD_PARTY_LICENSES-{each}.md"]
        if kind != "all":
            suffixes.append("/THIRD_PARTY_LICENSES.md")
        files.append((path.name, suffixes, path.read_bytes()))
    return files


def members(artefact):
    name = artefact.name
    if name.endswith((".tgz", ".tar.gz", ".crate")):
        with tarfile.open(artefact, "r:gz") as archive:
            for member in archive.getmembers():
                if member.isfile():
                    yield member.name, archive.extractfile(member).read()
    elif name.endswith((".zip", ".whl")):
        with zipfile.ZipFile(artefact) as archive:
            for member in archive.infolist():
                if not member.is_dir():
                    yield member.filename, archive.read(member)
    else:
        sys.exit(f"error: {artefact}: unknown artefact type")


def check(artefacts, max_bytes, rust_licenses=None):
    expected = notice_files()
    rust = rust_licence_files(rust_licenses) if rust_licenses else []
    failed = False
    for artefact in artefacts:
        artefact = pathlib.Path(artefact)
        size = artefact.stat().st_size
        contents = {}
        for member, data in members(artefact):
            contents.setdefault(member.replace("\\", "/"), data)
        for component, path, data in expected:
            suffix = f"/{component}/{path}"
            matches = [m for m in contents if ("/" + m).endswith(suffix)]
            if not matches:
                print(f"error: {artefact.name}: no {component}/{path}", file=sys.stderr)
                failed = True
            elif not any(contents[m] == data for m in matches):
                print(f"error: {artefact.name}: {matches[0]} differs from vendor/{component}/{path}",
                      file=sys.stderr)
                failed = True
        for display, suffixes, data in rust:
            matches = [m for m in contents if any(("/" + m).endswith(suffix) for suffix in suffixes)]
            if not matches:
                print(f"error: {artefact.name}: no {display}", file=sys.stderr)
                failed = True
            elif not any(contents[m] == data for m in matches):
                print(f"error: {artefact.name}: {matches[0]} differs from licenses/{display}", file=sys.stderr)
                failed = True
        if max_bytes is not None and size > max_bytes:
            print(f"error: {artefact.name} is {size:,} bytes, over {max_bytes:,}", file=sys.stderr)
            failed = True
        print(f"{artefact.name}: {size:,} bytes, {len(expected)} third-party notices"
              f"{f' and {len(rust)} Rust licence file(s)' if rust else ''} checked")
    return 1 if failed else 0


def stage(destination):
    for component, path, data in notice_files():
        target = pathlib.Path(destination) / "third_party" / component / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("stage").add_argument("destination")
    checker = commands.add_parser("check")
    checker.add_argument("--max-bytes", type=int)
    checker.add_argument("--rust-licenses", choices=(*RUST_LICENCE_KINDS, "all"))
    checker.add_argument("artefacts", nargs="+")
    args = parser.parse_args()
    if args.command == "stage":
        return stage(args.destination)
    return check(args.artefacts, args.max_bytes, args.rust_licenses)


if __name__ == "__main__":
    sys.exit(main())
