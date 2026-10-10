#!/usr/bin/env python3
"""Store exact receipt bytes and verify both sides of deterministic compression.

This records evidence; it never runs or accepts a converter. Classification and
scope are explicit coordinator decisions. Existing entries survive incremental
additions. Raw source paths remain in the index even after checkout relocation.
"""

import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
INDEX = ROOT / "storage-index.json"
PRODUCTION = "4553533e64c1494e706888a2c8e3a38a6867ce56"
CLASSES = {"accepted-bounded", "prior-check-log", "nonacceptance", "source-provenance", "independent-control"}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def compress(data):
    output = io.BytesIO()
    with gzip.GzipFile(fileobj=output, mode="wb", filename="", compresslevel=9, mtime=0) as stream:
        stream.write(data)
    return output.getvalue()


def provenance(data):
    """Preserve available receipt-authored pins without inferring absent pins."""
    try:
        value = json.loads(data)
    except (ValueError, UnicodeDecodeError):
        return {"kind": "raw-text", "execution_pins": "See associated receipt/coordinator-checks; not inferred from text storage."}
    if not isinstance(value, dict):
        return {"kind": "json-list", "execution_pins": "Original bytes preserve individual command/log records; no new driver identity inferred."}
    keys = {
        "production_source_commit", "production_commit", "source_commit", "base_source_commit",
        "binary_path", "binary", "binary_sha256", "native_binary", "native_binary_sha256",
        "release_binary", "release_binary_sha256", "production_sha256", "source_sha256",
        "driver_sha256", "driver_hashes", "authored_driver_sha256", "checker_sha256",
        "oracle_sha256", "probe_sha256", "generator_sha256", "png_generator_sha256",
        "invocation", "scope", "platform", "python", "status", "passed",
        "files", "final_driver_sha256", "wheel", "wheel_sha256", "package_version",
        "empty_path", "tests_run", "ok",
        "package_path", "package_sha256", "verified_production_sha256",
        "required_docs_and_oracles_sha256", "normalized_metadata",
        "package_verification_scope", "command", "exit_code",
    }
    pins = {key: value[key] for key in sorted(keys & value.keys())}
    if isinstance(value.get("execution"), dict):
        pins["execution"] = {
            key: value["execution"][key]
            for key in ("binary_path", "binary_sha256", "scope")
            if key in value["execution"]
        }
    return pins


def verify(index):
    for entry in index["entries"]:
        stored = (ROOT / entry["path"]).read_bytes()
        if entry["encoding"] not in {"gzip", "raw"}:
            raise ValueError(f"Unknown encoding: {entry['path']}")
        if len(stored) != entry["stored_bytes"] or sha(stored) != entry["stored_sha256"]:
            raise ValueError(f"Stored receipt identity mismatch: {entry['path']}")
        raw = gzip.decompress(stored) if entry["encoding"] == "gzip" else stored
        if len(raw) != entry["original_bytes"] or sha(raw) != entry["original_sha256"]:
            raise ValueError(f"Original receipt identity mismatch: {entry['path']}")
        if entry["encoding"] == "gzip" and stored != compress(raw):
            raise ValueError(f"Non-deterministic gzip receipt: {entry['path']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--add", nargs=4, action="append", default=[], metavar=("CLASSIFICATION", "RAW_PATH", "NAME", "SCOPE"))
    parser.add_argument("--slot", nargs=3, action="append", default=[], metavar=("NAME", "STATUS", "REFERENCE"))
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    index = json.loads(INDEX.read_text()) if INDEX.exists() else {
        "schema_version": 1,
        "production_source_commit": PRODUCTION,
        "storage": "Exact raw bytes; files >=16384 bytes use gzip level9 with mtime0 and no filename. Smaller files remain raw. Compression creates no new acceptance.",
        "entries": [],
        "final_slots": {},
    }
    entries = {entry["original_name"]: entry for entry in index["entries"]}
    for classification, raw_path, name, scope in args.add:
        if classification not in CLASSES:
            parser.error(f"unknown classification: {classification}")
        if Path(name).name != name or Path(name).suffix not in {".json", ".log", ".md", ".py"}:
            parser.error("NAME must be a receipt/log/document basename; binaries and generated archives are excluded")
        source = Path(raw_path).resolve()
        raw = source.read_bytes()
        compressed = len(raw) >= 16384
        stored = compress(raw) if compressed else raw
        relative = Path("receipts") / (name + (".gz" if compressed else ""))
        previous = entries.get(name)
        if previous and previous["original_sha256"] != sha(raw):
            parser.error(f"immutable receipt name already records different bytes: {name}; choose a new name")
        (ROOT / relative).parent.mkdir(exist_ok=True)
        (ROOT / relative).write_bytes(stored)
        entries[name] = {
            "original_name": name, "original_path": str(source),
            "path": str(relative), "encoding": "gzip" if compressed else "raw",
            "original_bytes": len(raw), "original_sha256": sha(raw),
            "stored_bytes": len(stored), "stored_sha256": sha(stored),
            "classification": classification, "scope": scope,
            "receipt_provenance": provenance(raw),
        }
    for name, status, reference in args.slot:
        if status not in {"pending", "complete"}:
            parser.error("slot status must be pending or complete")
        index["final_slots"][name] = {"status": status, "reference": reference}
    index["entries"] = [entries[name] for name in sorted(entries)]
    verify(index)
    if args.add or args.slot or not INDEX.exists():
        INDEX.write_text(json.dumps(index, indent=2, sort_keys=True) + "\n")
    print(f"Verified {len(index['entries'])} stored receipts with exact original-byte recovery.")


if __name__ == "__main__":
    main()
