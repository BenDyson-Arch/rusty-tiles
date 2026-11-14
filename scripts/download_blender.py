"""Download a checksum-pinned official Blender bundle for wheel acceptance.

Requires host Python 3.12+ for tar extraction's data filter. Blender supplies
the interpreter that actually installs and tests the candidate wheel.

The archive comes from download.blender.org, or from the mirrors listed in
blender_official.json when that fails; every copy must have the pinned SHA-256.
With --cache-dir, a verified archive already in that directory is used without
downloading, and a downloaded one is kept there (CI caches it by its SHA-256).
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import platform
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile


MANIFEST = Path(__file__).with_name("blender_official.json")
CANONICAL_ROOT = "https://download.blender.org/release/"


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def download_verified(url, destination, expected_sha256):
    # Verify the checked-in digest, rather than trusting a new checksum fetched
    # from the same server at execution time. Nothing is extracted on mismatch.
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "rusty-tiles-wheel-acceptance/1.0"})
        with urllib.request.urlopen(request, timeout=90) as source, destination.open("wb") as output:
            shutil.copyfileobj(source, output)
        if sha256_file(destination) != expected_sha256:
            raise RuntimeError(f"Blender archive SHA-256 mismatch: {url}")
    except BaseException:
        destination.unlink(missing_ok=True)
        raise


def archive_urls(manifest, release, spec):
    """The official archive URL, then the same path on each listed mirror."""
    url = release["base_url"] + spec["archive"]
    urls = [url]
    if url.startswith(CANONICAL_ROOT):
        urls += [mirror + url[len(CANONICAL_ROOT):] for mirror in manifest.get("mirrors", [])]
    return urls


def fetch_archive(urls, destination, expected_sha256):
    """Use a verified archive already at destination, else download the first good copy.

    Returns where the bytes came from: "cache" or the URL.
    """
    if destination.is_file():
        if sha256_file(destination) == expected_sha256:
            return "cache"
        destination.unlink()
    failures = []
    for url in urls:
        try:
            download_verified(url, destination, expected_sha256)
            return url
        except Exception as error:  # noqa: BLE001 - try the next mirror, report them all
            failures.append(f"{url}: {error}")
            print(f"warning: {url}: {error}", flush=True)
    raise RuntimeError("No source had the pinned Blender archive:\n" + "\n".join(failures))


def extract_bundle(archive, destination):
    if archive.name.endswith(".tar.xz"):
        with tarfile.open(archive) as source:
            source.extractall(destination, filter="data")
    elif archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as source:
            for member in source.infolist():
                path = PurePosixPath(member.filename.replace("\\", "/"))
                if path.is_absolute() or ".." in path.parts or ":" in member.filename:
                    raise ValueError(f"Unsafe Blender archive path: {member.filename}")
            source.extractall(destination)
    elif archive.suffix == ".dmg":
        # Copy the intact application bundle, preserving symlinks and signing
        # metadata, and detach even if copying fails. No system installation.
        with tempfile.TemporaryDirectory(prefix="rusty-tiles-blender-mount-") as mount:
            subprocess.run(["hdiutil", "attach", str(archive), "-readonly", "-nobrowse",
                            "-mountpoint", mount], check=True, timeout=120)
            try:
                subprocess.run(["ditto", str(Path(mount) / "Blender.app"),
                                str(destination / "Blender.app")], check=True, timeout=180)
            finally:
                subprocess.run(["hdiutil", "detach", mount], check=True, timeout=120)
    else:
        raise ValueError(f"Unsupported Blender archive: {archive.name}")


def validate_distribution(record, executable):
    """Bind acceptance to the pinned bundle and the extracted executable."""
    manifest = json.loads(MANIFEST.read_text())
    release = manifest["releases"].get(record.get("release"))
    spec = None if release is None else release["platforms"].get(record.get("distribution_platform"))
    if (spec is None or record.get("version") != release["version"]
            or record.get("archive_url") != release["base_url"] + spec["archive"]
            or record.get("archive_sha256") != spec["sha256"]):
        raise RuntimeError("Blender distribution evidence does not match the pinned official bundle")
    if (Path(record["executable"]).resolve() != executable.resolve()
            or record.get("executable_sha256") != sha256_file(executable)):
        raise RuntimeError("Blender executable does not match the verified distribution evidence")
    return release, spec


def main():
    manifest = json.loads(MANIFEST.read_text())
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", choices=manifest["releases"], required=True,
                        help="Pinned Blender release series")
    parser.add_argument("--platform", required=True,
                        help="Official bundle platform of that release, e.g. linux-x64")
    parser.add_argument("--directory", type=Path, required=True, help="New temporary bundle directory")
    parser.add_argument("--report-json", type=Path, required=True)
    parser.add_argument("--cache-dir", type=Path,
                        help="Keep the verified archive here, and reuse a verified one found here")
    args = parser.parse_args()
    directory = args.directory.resolve()
    report = args.report_json.resolve()
    # Stale success cannot survive a failed download or an incorrect host.
    report.unlink(missing_ok=True)
    release = manifest["releases"][args.release]
    if args.platform not in release["platforms"]:
        parser.error(f"Blender {args.release} has no pinned official bundle for {args.platform}; "
                     f"choose from {', '.join(release['platforms'])}")
    spec = release["platforms"][args.platform]
    if platform.system() != spec["system"] or platform.machine().lower() not in spec["machines"]:
        parser.error(f"{args.platform} does not match this host: {platform.system()} {platform.machine()}")
    directory.mkdir(parents=True, exist_ok=False)
    completed = False
    try:
        url = release["base_url"] + spec["archive"]
        if args.cache_dir:
            args.cache_dir.mkdir(parents=True, exist_ok=True)
            archive = args.cache_dir.resolve() / spec["archive"]
        else:
            archive = directory / spec["archive"]
        source = fetch_archive(archive_urls(manifest, release, spec), archive, spec["sha256"])
        print(f"Blender archive from {source}", flush=True)
        extract_bundle(archive, directory)
        executable = directory / spec["executable"]
        if not executable.is_file():
            raise RuntimeError(f"Verified Blender bundle lacks its executable: {executable}")
        record = {
            "distribution": "official-blender.org",
            "release": args.release,
            "distribution_platform": args.platform,
            "version": release["version"],
            "archive_url": url,
            "archive_sha256": spec["sha256"],
            "archive_source": source,
            "checksum_url": release["checksum_url"],
            "executable": str(executable),
            "executable_sha256": sha256_file(executable),
        }
        report.parent.mkdir(parents=True, exist_ok=True)
        report.write_text(json.dumps(record, indent=2) + "\n")
        if not args.cache_dir:
            archive.unlink()  # The extracted bundle and checksum evidence suffice.
        completed = True
        print(executable)
    finally:
        if not completed:
            shutil.rmtree(directory)


if __name__ == "__main__":
    main()
