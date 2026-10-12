#!/usr/bin/env python3
"""Linux allocation witness for H1's supported tiny-sort-run configuration.

The producer is stopped while measuring owned files' st_blocks, so the witness
is one stable allocation snapshot rather than a sum across changing lifetimes.
No filesystem is filled and no ENOSPC is injected. It is a lower-bound witness
for live allocation, not a measurement of the complete physical peak.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import time


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as source:
        while raw := source.read(1 << 20):
            result.update(raw)
    return result.hexdigest()


def allocation(parent):
    physical = logical = files = directories = 0
    for directory, _, names in os.walk(parent):
        paths = ([Path(directory)] if Path(directory) != parent else []) + [Path(directory) / name for name in names]
        for path in paths:
            try:
                info = path.stat()
            except FileNotFoundError:
                continue
            physical += info.st_blocks * 512
            if stat.S_ISREG(info.st_mode):
                logical += info.st_size
                files += 1
            elif stat.S_ISDIR(info.st_mode):
                directories += 1
    return {"physical_allocated_bytes": physical, "logical_file_bytes": logical,
            "live_files": files, "owned_directories": directories}


def verify_snapshot(snapshot, need, file_cap):
    assert snapshot["physical_allocated_bytes"] <= need, "physical allocation exceeds full preflight allowance"
    assert snapshot["live_files"] <= file_cap, "live files exceed declared bound"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--label", required=True)
    args = parser.parse_args()
    assert sys.platform == "linux", "stopped-process st_blocks acceptance is Linux only"
    assert not args.work.exists(), "probe work directory must be fresh"
    args.work.mkdir(parents=True)
    args.binary = args.binary.resolve()
    oracle_path = Path(__file__).resolve().parents[3] / "tests/h1_tree_oracle.py"
    spec = importlib.util.spec_from_file_location("h1_reference", oracle_path)
    oracle = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(oracle)
    source = oracle.generate_scale(args.work / "source", 4096)
    scratch = args.work / "scratch"
    scratch.mkdir()
    output = args.work / "dump"
    env = dict(os.environ, H1_INPUT=str(source), H1_OUTPUT=str(output), H1_SCRATCH=str(scratch),
               H1_JOBS="1", H1_RUN_BYTES="256", H1_FAN_IN="64", H1_BUFFER_BYTES="17", H1_PROFILE="quality")
    cpus = sorted(os.sched_getaffinity(0))[:2]
    command = ["nice", "-n", "10", "taskset", "-c", ",".join(map(str, cpus)), str(args.binary),
               "--exact", "mesh_large::tests::external_driver", "--ignored", "--nocapture"]
    with (args.work / "driver.log").open("wb") as log:
        producer = subprocess.Popen(command, env=env, stdout=log, stderr=log)
        snapshot = None
        stopped = False
        started = time.monotonic()
        try:
            while producer.poll() is None:
                assert time.monotonic() - started < 90, "allocation witness timed out"
                if snapshot is None and allocation(scratch)["live_files"] >= 1024:
                    os.kill(producer.pid, signal.SIGSTOP)
                    pid, status = os.waitpid(producer.pid, os.WUNTRACED)
                    assert pid == producer.pid and os.WIFSTOPPED(status), "producer did not stop"
                    stopped = True
                    snapshot = allocation(scratch)
                    os.kill(producer.pid, signal.SIGCONT)
                    stopped = False
                time.sleep(.0025)
            code = producer.wait()
        finally:
            if stopped:
                os.kill(producer.pid, signal.SIGCONT)
            if producer.poll() is None:
                producer.terminate()
                producer.wait()
    assert code == 0, "production driver failed; inspect driver.log"
    assert snapshot is not None, "tiny-file control did not reach 1024 live files"
    report = json.loads((output / "report.json").read_text())["capture"]
    need = report["scratch"]["need_bytes"]
    file_cap = report["scratch"]["max_live_files"]
    verify_snapshot(snapshot, need, file_cap)
    try:
        verify_snapshot(snapshot, snapshot["physical_allocated_bytes"] - 1, file_cap)
    except AssertionError:
        sensitivity = True
    else:
        raise AssertionError("insufficient physical allowance sensitive control did not fail")
    assert list(scratch.iterdir()) == [], "private scratch was not cleaned up"
    # Full exact source replay validates this workload after the sampling pause.
    audited = oracle.verify_scale(source.parent / "scale-truth.json", output, 8192, 96 << 20)
    receipt = {"schema": 1, "label": args.label, "binary_sha256": digest(args.binary),
               "probe_sha256": digest(__file__), "oracle_sha256": digest(oracle_path),
               "source_sha256": {p.name: digest(p) for p in source.parent.iterdir()},
               "config": {key: value for key, value in env.items() if key.startswith("H1_")},
               "measurement": "st_blocks*512 with SIGSTOP/waitpid confirmed producer stop; one stable live allocation snapshot",
               "snapshot": snapshot, "scratch_report": report["scratch"],
               "logical_peak_bytes": report["scratch_peak_bytes"],
               "insufficient_allowance_sensitive_control": sensitivity,
               "source_replay": audited, "cleanup_entries": 0}
    (args.work / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"receipt": str(args.work / "receipt.json"), "snapshot": snapshot,
                      "preflight_need_bytes": need, "file_cap": file_cap}, indent=2))


if __name__ == "__main__":
    main()
