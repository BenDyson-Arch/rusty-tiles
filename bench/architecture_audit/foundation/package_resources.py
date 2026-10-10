#!/usr/bin/env python3
"""Linux wait4/resource measurements of F0 packaging, not a general resource bound."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import sys
import tempfile


METER = """
import json, resource, subprocess, sys, time
start = time.monotonic()
completed = subprocess.run(sys.argv[1:], check=True, capture_output=True, text=True)
print(json.dumps({"stdout": completed.stdout,
                  "max_rss_kib": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,
                  "elapsed_seconds": round(time.monotonic() - start, 4)}))
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--work-parent", type=Path, required=True)
    parser.add_argument("--results", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    measurements = []
    with tempfile.TemporaryDirectory(prefix="package-resources-", dir=args.work_parent) as work:
        root = Path(work)
        for label, count, payload_bytes in [
            ("payload_8MiB", 1, 8 << 20),
            ("payload_128MiB", 1, 128 << 20),
            ("payload_512MiB", 1, 512 << 20),
            ("inventory_128", 128, 64),
            ("inventory_4096", 4096, 64),
        ]:
            source = root / label
            source.mkdir()
            (source / "tileset.json").write_bytes(b"opaque manifest\n")
            for number in range(count):
                with (source / f"payload-{number:06d}.bin").open("wb") as file:
                    file.truncate(payload_bytes)
            output = root / "output.3tz"
            completed = subprocess.run(
                [sys.executable, "-c", METER, str(binary), "--json", "convert",
                 "-i", str(source), "-o", str(output)],
                check=True, capture_output=True, text=True,
            )
            measured = json.loads(completed.stdout)
            receipt = json.loads(measured.pop("stdout"))["packageReceipt"]
            assert receipt["memberCount"] == count + 1
            assert receipt["sourceBytes"] == count * payload_bytes + 16
            assert receipt["archiveBytes"] == output.stat().st_size
            measurements.append({"case": label, "receipt": receipt,
                                 **measured})
            output.unlink()
    args.results.parent.mkdir(parents=True, exist_ok=True)
    args.results.write_text(json.dumps({
        "platform": platform.platform(),
        "binary_sha256": hashlib.file_digest(binary.open("rb"), "sha256").hexdigest(),
        "checkout_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "measurements": measurements,
        "limits": ["One Linux process per case; resource RUSAGE_CHILDREN maximum RSS and monotonic elapsed time.",
                   "Sparse zero-filled source payloads; no claim about other filesystems or concurrency.",
                   "Inventory/index memory grows with member count; payload buffer is 64 KiB.",
                   "Does not establish an upper memory bound or measure OS page cache."]
    }, indent=2) + "\n")
    print(json.dumps(measurements, indent=2))


if __name__ == "__main__":
    main()
