#!/usr/bin/env python3
"""Find the full-matrix CI run of a commit and whether its build artifacts can be reused.

release.yml (for a tag or a dry run) and dev-build.yml publish only a commit that a non-PR
`ci.yml` run passed with both `Full matrix` and `Rust` successful. Pull-request runs never
count, even when they ran the full tier. Such a run's portable and wheel jobs upload the
archives and wheels they tested (`binary-<target>`, `wheel-<target>`), and its `Build
version` job uploads `ci-build-<version>`, naming the version they carry. A run whose 9
artifacts are all unexpired, whose version is the one being published, and whose
ci-build.json names that version, this commit and that run, is reused, so the release
publishes what CI tested instead of rebuilding it.

    ci_run.py select --repo OWNER/REPO --commit SHA --version VERSION [--in-progress]

prints GitHub output lines: `run-id=` the newest qualifying run (empty if none), and
`artifact-run=` the newest qualifying run with reusable artifacts (empty: build from
source). Without --in-progress a run must have completed successfully as a whole, which is
the release rule; with it, a run still in progress qualifies once its two verdict jobs
passed, which the dev build needs because the nightly run dispatches it. Uses `gh api`
with GH_TOKEN; stdlib only.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile

# The four portable targets (Intel macOS, x86_64-apple-darwin, is not one from 0.4.1).
TARGETS = (
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
)
VERDICT_JOBS = ("Full matrix", "Rust")


def required_artifacts(version):
    names = [f"binary-{target}" for target in TARGETS] + [f"wheel-{target}" for target in TARGETS]
    return names + [f"ci-build-{version}"]


def gh_api(path, jq, paginate=False):
    command = ["gh", "api", "-X", "GET", path, "--jq", jq]
    if paginate:
        command.insert(2, "--paginate")
    completed = subprocess.run(command, check=True, capture_output=True, text=True)
    return [line for line in completed.stdout.splitlines() if line]


def gh_manifest(repo, run, version):
    """The ci-build.json of a run, or None if it cannot be downloaded or parsed."""
    with tempfile.TemporaryDirectory(prefix="rusty-tiles-ci-build-") as temp:
        completed = subprocess.run(["gh", "run", "download", str(run), "--repo", repo,
                                    "--name", f"ci-build-{version}", "--dir", temp],
                                   capture_output=True, text=True)
        if completed.returncode != 0:
            return None
        try:
            return json.loads((Path(temp) / "ci-build.json").read_text())
        except (OSError, ValueError):
            return None


def manifest_problem(manifest, commit, run, version):
    """Why a run's ci-build.json does not describe this build (None: it does)."""
    if not isinstance(manifest, dict):
        return "ci-build.json could not be read"
    expected = {"version": version, "commit": commit, "run": str(run)}
    wrong = [f"{key}={manifest.get(key)!r} (expected {value!r})"
             for key, value in expected.items() if str(manifest.get(key)) != value]
    return "ci-build.json has " + ", ".join(wrong) if wrong else None


def candidate_runs(api, repo, commit, in_progress):
    """Non-PR ci.yml runs of the commit, newest first."""
    query = f"repos/{repo}/actions/workflows/ci.yml/runs?head_sha={commit}&per_page=100"
    if not in_progress:
        query += "&status=success"
    rows = api(query, '.workflow_runs[] | [.id, .event, .head_sha] | @json')
    runs = []
    for row in rows:
        run, event, head = json.loads(row)
        # head_sha filtering is the API's; check it rather than trust it.
        if event != "pull_request" and head == commit:
            runs.append(int(run))
    return sorted(set(runs), reverse=True)


def passed_verdicts(api, repo, run):
    passed = set(api(f"repos/{repo}/actions/runs/{run}/jobs?per_page=100",
                     '.jobs[] | select(.conclusion == "success") | .name', paginate=True))
    return all(job in passed for job in VERDICT_JOBS)


def live_artifacts(api, repo, run):
    return set(api(f"repos/{repo}/actions/runs/{run}/artifacts?per_page=100",
                   '.artifacts[] | select(.expired | not) | .name', paginate=True))


def select(api, repo, commit, version, in_progress, manifest=None):
    manifest = (lambda run: gh_manifest(repo, run, version)) if manifest is None else manifest
    gate, reusable = None, None
    required = required_artifacts(version)
    for run in candidate_runs(api, repo, commit, in_progress):
        if not passed_verdicts(api, repo, run):
            continue
        if gate is None:
            gate = run
        missing = [name for name in required if name not in live_artifacts(api, repo, run)]
        if missing:
            print(f"Run {run} passed but cannot be reused; missing or expired: {', '.join(missing)}",
                  file=sys.stderr)
            continue
        problem = manifest_problem(manifest(run), commit, run, version)
        if problem:
            print(f"Run {run} passed but cannot be reused: {problem}", file=sys.stderr)
            continue
        reusable = run
        break
    return gate, reusable


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    chooser = commands.add_parser("select")
    chooser.add_argument("--repo", required=True)
    chooser.add_argument("--commit", required=True)
    chooser.add_argument("--version", required=True)
    chooser.add_argument("--in-progress", action="store_true")
    args = parser.parse_args()
    gate, reusable = select(gh_api, args.repo, args.commit, args.version, args.in_progress)
    print(f"run-id={gate or ''}")
    print(f"artifact-run={reusable or ''}")
    if gate is None:
        print(f"No full-matrix CI pass for {args.commit}", file=sys.stderr)
    elif reusable is None:
        print(f"Full CI matrix passed for {args.commit} in run {gate}; no run has reusable "
              f"{args.version} artifacts, so they will be built from source", file=sys.stderr)
    else:
        print(f"Full CI matrix passed for {args.commit} in run {gate}; reusing the "
              f"{args.version} archives and wheels of run {reusable}", file=sys.stderr)


if __name__ == "__main__":
    main()
