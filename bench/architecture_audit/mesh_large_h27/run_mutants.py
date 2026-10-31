#!/usr/bin/env python3
"""Execute isolated bake mutations, restoring each source before continuing.

The coordinator must hold a Cargo lane lock for this whole command. No other
agent may edit the frozen Rust source or build this target while it runs.
Receipts distinguish compilation failure from a caught runtime mutation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    root = Path(subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"], text=True).strip())
    out = args.output_dir.expanduser().resolve()
    out.mkdir(parents=True, exist_ok=True)
    spec_bytes = args.cases.read_bytes()
    spec = json.loads(spec_bytes)
    original_sources = {
        str(p.relative_to(root)): digest(p.read_bytes())
        for p in sorted((root / "src/mesh_large/bake").glob("*.rs"))
    }
    receipt = {
        "head": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
        "cases_sha256": digest(spec_bytes),
        "original_sources": original_sources,
        "scope": "Compiled component mutations; no H17 sink/GLB assembly proof.",
        "cases": [],
    }
    env = os.environ.copy()
    env.update(CARGO_BUILD_JOBS="4", RAYON_NUM_THREADS="2", RUST_TEST_THREADS="2")
    for case in spec["cases"]:
        path = (root / case["path"]).resolve()
        if not path.is_relative_to((root / "src/mesh_large/bake").resolve()):
            raise RuntimeError(f"mutation outside bake ownership: {case['name']}")
        if not case["test"].startswith("mesh_large::bake::"):
            raise RuntimeError(f"test must be fully qualified: {case['name']}")
        original = path.read_bytes()
        text = original.decode()
        if text.count(case["old"]) != 1:
            raise RuntimeError(f"nonunique mutation anchor: {case['name']}")
        if case.get("restored_source_sha256", digest(original)) != digest(original):
            raise RuntimeError(f"source pin differs: {case['name']}")
        mutated = text.replace(case["old"], case["new"]).encode()
        if case.get("mutated_source_sha256", digest(mutated)) != digest(mutated):
            raise RuntimeError(f"mutated pin differs: {case['name']}")
        record = {
            "name": case["name"], "path": case["path"], "test": case["test"],
            "original_sha256": digest(original), "mutated_sha256": digest(mutated),
        }
        try:
            path.write_bytes(mutated)
            command = [
                "cargo", "test", "--locked", "--no-default-features", "--lib",
                "--no-run", "--message-format=json", "--target-dir",
                str(args.target_dir.expanduser().resolve()), "-j", "4",
            ]
            build = subprocess.run(command, cwd=root, env=env, capture_output=True)
            build_json = out / f"{case['name']}-build.jsonl"
            build_log = out / f"{case['name']}-build.stderr"
            build_json.write_bytes(build.stdout)
            build_log.write_bytes(build.stderr)
            record.update(build_exit=build.returncode,
                          build_json_sha256=digest(build.stdout),
                          build_stderr_sha256=digest(build.stderr))
            if build.returncode:
                record["verdict"] = "compile_failure_not_caught"
            else:
                artifacts = [json.loads(line) for line in build.stdout.splitlines()]
                binary = next(Path(a["executable"]) for a in artifacts
                              if a.get("reason") == "compiler-artifact"
                              and a.get("target", {}).get("name") == "rusty_tiles"
                              and a.get("profile", {}).get("test")
                              and a.get("executable"))
                record["binary_sha256"] = digest(binary.read_bytes())
                run = subprocess.run([str(binary), case["test"], "--exact", "--test-threads=2",
                                      "--nocapture"], cwd=root, env=env,
                                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                (out / f"{case['name']}-runtime.log").write_bytes(run.stdout)
                record.update(runtime_exit=run.returncode,
                              runtime_sha256=digest(run.stdout))
                record["verdict"] = (
                    "caught_runtime" if run.returncode == 101
                    and b"running 1 test" in run.stdout
                    and b"1 failed" in run.stdout else "survived_or_invalid_runtime")
        finally:
            path.write_bytes(original)
            record["restored_sha256"] = digest(path.read_bytes())
            assert record["restored_sha256"] == record["original_sha256"]
            receipt["cases"].append(record)
            (out / "matrix.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(f"{case['name']}: {record['verdict']}", flush=True)
        if record["verdict"] != "caught_runtime":
            return 1
    assert original_sources == {
        str(p.relative_to(root)): digest(p.read_bytes())
        for p in sorted((root / "src/mesh_large/bake").glob("*.rs"))
    }
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
