"""Resolve and apply the version of a rolling development build (Python 3.10+, stdlib only).

`resolve` prints the development version of the checked-out commit as GitHub output lines.
The base is the next patch after the workspace version when that version is already tagged
`vX.Y.Z`, and otherwise the workspace version itself (develop often bumps it before the
release). The development number is the commit's UTC committer time, YYYYMMDDHHMM, so later
commits sort later and a rebuild of the same commit keeps its version. Cargo spells it
`0.4.1-dev.202611121734`; maturin writes `0.4.1.dev202611121734` into the wheel metadata,
which pip orders after 0.4.0 and before 0.4.1.

`stamp VERSION` writes a Cargo version into the checkout without committing it: the
`[workspace.package]` version in Cargo.toml and the Cargo.lock entries of the workspace
crates (the lock packages without a `source`), so `--locked` builds still pass. It refuses a
checkout whose workspace crates do not all have the current workspace version.
"""
import argparse
from datetime import datetime, timezone
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SEMVER = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z.-]+))?")
LOCK_PACKAGE = re.compile(r'^\[\[package\]\]\nname = "([^"\n]+)"\nversion = "([^"\n]+)"\n((?:.+\n)*)', re.M)


def workspace_version(manifest):
    """Return the [workspace.package] version and its (start, end) span in the text."""
    section = re.search(r"^\[workspace\.package\]\n((?:(?!\[).*\n)*)", manifest, re.M)
    if section is None:
        raise SystemExit("error: Cargo.toml has no [workspace.package] section")
    matches = list(re.finditer(r'^version = "([^"\n]+)"$', section.group(1), re.M))
    if len(matches) != 1:
        raise SystemExit("error: [workspace.package] must set exactly one version")
    match = matches[0]
    offset = section.start(1)
    return match.group(1), (offset + match.start(1), offset + match.end(1))


def release_core(version):
    """MAJOR, MINOR and PATCH of a SemVer version; a pre-release suffix is dropped."""
    match = SEMVER.fullmatch(version)
    if match is None:
        raise SystemExit(f"error: workspace version {version!r} is not SemVer")
    return tuple(int(part) for part in match.groups()[:3])


def development_version(core, tagged, committed):
    """Cargo and PEP 440 spellings of the development version after release `core`."""
    major, minor, patch = core
    if tagged:
        patch += 1
    base = f"{major}.{minor}.{patch}"
    number = committed.astimezone(timezone.utc).strftime("%Y%m%d%H%M")
    return f"{base}-dev.{number}", f"{base}.dev{number}"


def git(*arguments):
    return subprocess.run(["git", "-C", str(ROOT), *arguments], check=True,
                          capture_output=True, text=True).stdout.strip()


def read(path):
    """Text with universal newlines, and the newline to write it back with."""
    raw = path.read_bytes()
    return raw.decode("utf-8").replace("\r\n", "\n"), "\r\n" if b"\r\n" in raw else "\n"


def write(path, text, newline):
    with path.open("w", encoding="utf-8", newline=newline) as output:
        output.write(text)


def resolve():
    version, _ = workspace_version(read(ROOT / "Cargo.toml")[0])
    core = release_core(version)
    tagged = bool(git("tag", "--list", "v{}.{}.{}".format(*core)))
    committed = datetime.fromtimestamp(int(git("show", "-s", "--format=%ct", "HEAD")), timezone.utc)
    cargo, python = development_version(core, tagged, committed)
    print(f"version={cargo}")
    print(f"python-version={python}")


def stamp(version):
    if SEMVER.fullmatch(version) is None:
        raise SystemExit(f"error: {version!r} is not a SemVer version")
    manifest_path, lock_path = ROOT / "Cargo.toml", ROOT / "Cargo.lock"
    manifest, manifest_newline = read(manifest_path)
    lock, lock_newline = read(lock_path)
    current, (start, end) = workspace_version(manifest)
    local = [match for match in LOCK_PACKAGE.finditer(lock) if "\nsource = " not in "\n" + match.group(3)]
    if not local:
        raise SystemExit("error: Cargo.lock has no workspace crates")
    stale = [f"{match.group(1)} {match.group(2)}" for match in local if match.group(2) != current]
    if stale:
        raise SystemExit(f"error: Cargo.lock workspace crates differ from version {current}: {', '.join(stale)}")
    if version == current:
        print(f"Workspace version is already {version}")
        return
    for match in reversed(local):
        lock = lock[:match.start(2)] + version + lock[match.end(2):]
    write(manifest_path, manifest[:start] + version + manifest[end:], manifest_newline)
    write(lock_path, lock, lock_newline)
    names = ", ".join(match.group(1) for match in local)
    print(f"Stamped {version} (was {current}) into Cargo.toml and Cargo.lock ({names})")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("resolve", help="Print version= and python-version= for this commit")
    stamp_parser = commands.add_parser("stamp", help="Write a Cargo version into the checkout")
    stamp_parser.add_argument("version")
    args = parser.parse_args()
    if args.command == "resolve":
        resolve()
    else:
        stamp(args.version)


if __name__ == "__main__":
    main()
