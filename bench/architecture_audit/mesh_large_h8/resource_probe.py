#!/usr/bin/env python3
"""Root-launched H8 Linux RSS/scratch receipt for the actual production CLI.

No fixture generation/copying. Input and binary hashing is streamed. wait4 gives
child high-water RSS (including the small inherited launcher floor); periodic
scratch st_blocks observations are non-atomic, not a physical peak proof. Native
workspace declarations remain provisional, not universal worst-case bounds.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import stat
import sys
import tempfile
import time


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as source:
        while block := source.read(1 << 20):
            result.update(block)
    return result.hexdigest()


def inventory(root):
    return {str(path.relative_to(root)): {"bytes": path.stat().st_size, "sha256": digest(path)}
            for path in sorted(root.rglob("*")) if path.is_file()}


def allocation(root):
    physical = logical = files = 0
    for directory, _, names in os.walk(root):
        for path in [Path(directory)] + [Path(directory) / name for name in names]:
            try:
                info = path.stat()
            except FileNotFoundError:
                continue
            physical += info.st_blocks * 512
            if stat.S_ISREG(info.st_mode):
                logical += info.st_size
                files += 1
    return {"physical_bytes": physical, "logical_bytes": logical, "files": files}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def inside(path, parent):
    return path == parent or parent in path.parents


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("binary", "input", "output", "scratch", "receipt"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--input-root", type=Path, help="Source-only tree to hash; default input parent")
    parser.add_argument("--source-identity", required=True, help="Explicit fixture/generator revision or manifest SHA-256")
    parser.add_argument("--code-identity", required=True, help="Explicit exact source revision/manifest identity")
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--profile", choices=("fast", "balanced", "quality", "lossless"), default="balanced")
    parser.add_argument("--expected-triangles", type=int)
    parser.add_argument("--sample-seconds", type=float, default=.1)
    parser.add_argument("--timeout-seconds", type=float, default=6*60*60)
    args = parser.parse_args()
    args.receipt = args.receipt.resolve()
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    result = {"schema": 1, "passed": False, "source_identity": args.source_identity,
              "code_identity": args.code_identity, "runner_sha256": digest(__file__),
              "errors": [], "measurement_limits": [
                  "Linux wait4 high-water RSS includes inherited small Python launcher floor",
                  "periodic physical scratch stat observations are non-atomic; not complete peak or worst-case proof",
                  "native workspace charge is producer provisional policy, not a universal measured worst case",
                  "empty PATH proves no PATH-resolved external tools required; absolute executable spawning is not ruled out by PATH alone",
                  "exact output/source hashes establish this run only; semantic/image oracle is separate"]}
    private = None
    pid = None
    start = time.monotonic()
    try:
        require(sys.platform == "linux" and hasattr(os, "posix_spawn"), "Linux posix_spawn/wait4 required")
        require(args.jobs > 0 and args.sample_seconds > 0 and args.timeout_seconds > 0, "positive jobs/timing required")
        args.binary = args.binary.resolve(strict=True)
        args.input = args.input.resolve(strict=True)
        args.output = args.output.resolve()
        args.scratch = args.scratch.resolve()
        source_root = (args.input_root or args.input.parent).resolve(strict=True)
        require(source_root.is_dir() and inside(args.input, source_root), "input must belong to source-only inventory root")
        require(not args.output.exists(), "output must be fresh, CreateNew producer policy")
        require(not inside(args.output, source_root) and not inside(args.scratch, source_root)
                and not inside(args.receipt, source_root), "output/scratch/receipt must be outside immutable input tree")
        require(set(range(4)).issubset(os.sched_getaffinity(0)), "CPU affinity0-3 unavailable; root must choose compatible host")
        require(Path("/usr/bin/taskset").is_file() and Path("/usr/bin/nice").is_file(), "absolute nice/taskset required")
        args.scratch.mkdir(parents=True, exist_ok=True)
        private = Path(tempfile.mkdtemp(prefix="h8-resource-", dir=args.scratch))
        before_inputs = inventory(source_root)
        before_binary = digest(args.binary)
        result.update({"binary": str(args.binary), "binary_sha256_before": before_binary,
                       "input": str(args.input), "input_root": str(source_root), "inputs_before": before_inputs,
                       "profile": args.profile, "jobs": args.jobs, "output": str(args.output),
                       "private_scratch": str(private), "scratch_parent": str(args.scratch)})
        # Track only new producer-owned siblings; existing unrelated user stages
        # are never deleted or treated as this invocation's cleanup failures.
        args.output.parent.mkdir(parents=True, exist_ok=True)
        siblings_before = {p.name for p in args.output.parent.iterdir()}
        command = ["/usr/bin/taskset", "-c", "0-3", "/usr/bin/nice", "-n", "10", str(args.binary),
                   "--json", "mesh-leaf-payloads", "--input", str(args.input), "--output", str(args.output),
                   "--profile", args.profile, "--jobs", str(args.jobs), "--scratch-dir", str(private)]
        env = dict(os.environ, PATH="", RAYON_NUM_THREADS="2", RUST_TEST_THREADS="2")
        stdout_path = args.receipt.with_suffix(".stdout.log")
        stderr_path = args.receipt.with_suffix(".stderr.log")
        result.update({"command": command, "environment": {"PATH": "", "RAYON_NUM_THREADS": "2", "RUST_TEST_THREADS": "2"},
                       "stdout": str(stdout_path), "stderr": str(stderr_path), "affinity": "0-3", "nice": 10})
        peak = {"physical_bytes": 0, "logical_bytes": 0, "files": 0}
        samples = 0
        run_started = time.monotonic()
        with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
            pid = os.posix_spawn(command[0], command, env, setsid=True, file_actions=[
                (os.POSIX_SPAWN_DUP2, stdout.fileno(), 1), (os.POSIX_SPAWN_DUP2, stderr.fileno(), 2)])
            while True:
                snapshot = allocation(private)
                samples += 1
                peak = {key: max(peak[key], snapshot[key]) for key in peak}
                reaped, status, usage = os.wait4(pid, os.WNOHANG)
                if reaped:
                    pid = None
                    break
                if time.monotonic()-run_started > args.timeout_seconds:
                    os.killpg(pid, signal.SIGKILL)
                    _, status, usage = os.wait4(pid, 0)
                    pid = None
                    result["errors"].append("production CLI timed out")
                    break
                time.sleep(args.sample_seconds)
        code = os.waitstatus_to_exitcode(status)
        result.update({"exit_code": code, "wall_seconds": time.monotonic()-run_started,
                       "cpu_seconds": usage.ru_utime+usage.ru_stime,
                       "user_seconds": usage.ru_utime, "system_seconds": usage.ru_stime,
                       "peak_rss_bytes_wait4": usage.ru_maxrss*1024,
                       "sampled_scratch_peak": peak, "physical_samples": samples})
        result["binary_sha256_after"] = digest(args.binary)
        result["inputs_after"] = inventory(source_root)
        result["binary_unchanged"] = result["binary_sha256_after"] == before_binary
        result["inputs_unchanged"] = result["inputs_after"] == before_inputs
        result["scratch_remaining_before_runner_cleanup"] = sorted(p.name for p in private.iterdir())
        added_siblings = sorted(p.name for p in args.output.parent.iterdir()
                                if p.name not in siblings_before and p != args.output
                                and p not in (stdout_path, stderr_path, args.receipt))
        # D1 CreateNew uses this staging namespace. Other evidence workers can
        # write unrelated reports into the same cache while this run executes.
        new_siblings = [name for name in added_siblings
                        if name.startswith((".tiles-dir-", ".tiles-previous-"))]
        result["concurrent_unrelated_siblings"] = [name for name in added_siblings
                                                   if name not in new_siblings]
        result["new_unpublished_siblings"] = new_siblings
        result["published_output_present"] = args.output.exists()
        result["failed_run_published_output"] = code != 0 and args.output.exists()
        require(code == 0, f"production CLI failed with exit{code}; retained logs/receipt")
        require(result["binary_unchanged"] and result["inputs_unchanged"], "binary/source changed during measurement")
        require(not result["scratch_remaining_before_runner_cleanup"], "producer left owned scratch entries")
        require(not new_siblings, "producer left unpublished sibling stage/lock entries")
        require(args.output.is_dir(), "successful CLI did not publish output directory")
        cli_report = json.loads(stdout_path.read_text())
        require(cli_report.get("ok") is True, "exit0 CLI result does not claim success")
        resources = json.loads((args.output/"resources.json").read_text())
        manifest = json.loads((args.output/"manifest.json").read_text())
        result.update({"cli_result": cli_report, "producer_manifest": manifest, "resources": resources})
        capture = resources["capture"]
        declared_rss = max(capture["memory"]["bound_bytes"], resources["declaredStage2MemoryBytes"])
        tmpfs = capture["scratch"]["need_bytes"] if capture["scratch_memory_backed"] else 0
        result.update({"declared_serial_rss_bound_bytes": declared_rss,
                       "memory_backed_scratch_separate_bytes": tmpfs,
                       "rss_within_declared_bound": result["peak_rss_bytes_wait4"] <= declared_rss,
                       "logical_scratch_within_bound": capture["scratch_logical_peak_bytes"] <= capture["scratch"]["logical_need_bytes"],
                       "sampled_physical_scratch_within_bound": peak["physical_bytes"] <= capture["scratch"]["need_bytes"]})
        require(result["rss_within_declared_bound"], "RSS exceeds max(serial capture,stage2) declared bound")
        require(result["logical_scratch_within_bound"] and result["sampled_physical_scratch_within_bound"], "scratch exceeds declared allowance")
        artifacts = inventory(args.output)
        volatile = {name: value for name, value in artifacts.items() if name == "resources.json"}
        stable = {name: value for name, value in artifacts.items() if name != "resources.json"}
        members = {name: value for name, value in stable.items() if name.startswith("leaves/") and name.endswith(".glb")}
        report = manifest["report"]
        result.update({"stable_artifacts": stable, "volatile_artifacts": volatile,
                       "stable_artifact_inventory_sha256": hashlib.sha256(json.dumps(stable, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
                       "members": {"count": len(members), "total_bytes": sum(x["bytes"] for x in members.values()),
                                   "maximum_bytes": max((x["bytes"] for x in members.values()), default=0)},
                       "reported_triangles": report["triangles"]})
        require(len(members) == report["leaves"], "published membercount differs from report")
        require(result["members"]["total_bytes"] == report["glb_bytes"], "published memberbytes differ from report")
        require(result["members"]["maximum_bytes"] <= 96 << 20, "member exceeds96MiB")
        manifest_members = 0
        with (args.output/"leaves.jsonl").open() as lines:
            for line in lines:
                entry = json.loads(line)
                actual = members.get(entry["uri"])
                require(actual is not None and actual["bytes"] == entry["bytes"]
                        and actual["sha256"] == entry["sha256"], "leafmanifest byte/hash differs from actual member")
                manifest_members += 1
        require(manifest_members == len(members), "leafmanifest rowcount differs from actual members")
        if args.expected_triangles is not None:
            require(report["triangles"] == args.expected_triangles, "reported trianglecount differs from explicit expected input")
        result["passed"] = True
    except BaseException as error:
        result["errors"].append(f"{type(error).__name__}: {error}")
    finally:
        if pid is not None:
            try:
                os.killpg(pid, signal.SIGKILL)
                os.wait4(pid, 0)
            except (ProcessLookupError, ChildProcessError):
                pass
        if private is not None:
            try:
                if private.exists():
                    if "scratch_remaining_before_runner_cleanup" not in result:
                        result["scratch_remaining_before_runner_cleanup"] = sorted(p.name for p in private.iterdir())
                    shutil.rmtree(private)  # only the fresh runner-owned private directory
                result["runner_private_scratch_removed"] = True
            except OSError as error:
                result["passed"] = False
                result["errors"].append(f"owned scratch cleanup: {error}")
                result["retained_private_scratch"] = str(private)
        result["runner_total_wall_seconds"] = time.monotonic()-start
        args.receipt.write_text(json.dumps(result, indent=2)+"\n")
    print(json.dumps({key: result.get(key) for key in ("passed", "exit_code", "wall_seconds", "peak_rss_bytes_wait4", "members", "errors")}, indent=2))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
