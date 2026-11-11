"""scripts/ci_run.py: which CI run gates a release, and whether its artifacts are reused."""
import json
from pathlib import Path
import sys
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import ci_run

REPO = "owner/repo"
SHA = "a" * 40
VERSION = "0.4.1"
ALL = set(ci_run.required_artifacts(VERSION))


class FakeApi:
    """Answers the three queries ci_run makes from a table of runs."""

    def __init__(self, runs):
        # runs: id -> dict(event, sha, success, jobs, artifacts)
        self.runs = runs
        self.queries = []

    def __call__(self, path, jq, paginate=False):
        self.queries.append(path)
        if "/workflows/ci.yml/runs?" in path:
            only_success = "&status=success" in path
            return [json.dumps([run, spec["event"], spec.get("sha", SHA)])
                    for run, spec in sorted(self.runs.items(), reverse=True)
                    if not only_success or spec.get("success", True)]
        run = int(path.split("/runs/")[1].split("/")[0])
        if "/jobs" in path:
            return list(self.runs[run].get("jobs", ci_run.VERDICT_JOBS))
        if "/artifacts" in path:
            return list(self.runs[run].get("artifacts", ALL))
        raise AssertionError(path)


def manifest_of(runs):
    def manifest(run):
        spec = runs[run]
        if "manifest" in spec:
            return spec["manifest"]
        return {"version": VERSION, "python-version": VERSION, "kind": "release", "commit": SHA, "run": str(run)}
    return manifest


def select(runs, in_progress=False, version=VERSION):
    return ci_run.select(FakeApi(runs), REPO, SHA, version, in_progress, manifest=manifest_of(runs))


class SelectTests(unittest.TestCase):
    def test_reuses_the_newest_passing_run(self):
        self.assertEqual(select({10: {"event": "push"}, 11: {"event": "workflow_dispatch"}}), (11, 11))

    def test_pull_request_runs_never_count(self):
        self.assertEqual(select({10: {"event": "pull_request"}}), (None, None))

    def test_both_verdict_jobs_must_pass(self):
        for jobs in (("Rust",), ("Full matrix",), ()):
            with self.subTest(jobs=jobs):
                self.assertEqual(select({10: {"event": "push", "jobs": jobs}}), (None, None))

    def test_release_rule_requires_a_completed_successful_run(self):
        runs = {10: {"event": "push", "success": False}}
        self.assertEqual(select(runs), (None, None))
        self.assertEqual(select(runs, in_progress=True), (10, 10))

    def test_a_run_for_another_commit_is_ignored(self):
        self.assertEqual(select({10: {"event": "push", "sha": "b" * 40}}), (None, None))

    def test_missing_or_expired_artifacts_fall_back_to_an_older_run_or_a_build(self):
        for missing in sorted(ALL):
            with self.subTest(missing=missing):
                runs = {10: {"event": "push", "artifacts": ALL - {missing}}}
                self.assertEqual(select(runs), (10, None))
                runs[9] = {"event": "push"}
                self.assertEqual(select(runs), (10, 9))

    def test_artifacts_of_another_version_are_not_reused(self):
        self.assertEqual(select({10: {"event": "push"}}, version="0.4.1-dev.202611121734"), (10, None))

    def test_the_manifest_must_name_this_version_commit_and_run(self):
        good = {"version": VERSION, "commit": SHA, "run": "10"}
        for wrong in ({"commit": "b" * 40}, {"run": "9"}, {"version": "0.4.0"}):
            with self.subTest(wrong=wrong):
                runs = {10: {"event": "push", "manifest": {**good, **wrong}}}
                self.assertEqual(select(runs), (10, None))
        self.assertEqual(select({10: {"event": "push", "manifest": None}}), (10, None))
        self.assertEqual(select({10: {"event": "push", "manifest": good}}), (10, 10))
        # An older run with a good manifest is reused instead.
        runs = {10: {"event": "push", "manifest": {**good, "run": "3"}}, 9: {"event": "push"}}
        self.assertEqual(select(runs), (10, 9))

    def test_required_artifacts(self):
        self.assertEqual(len(ALL), 11)
        self.assertIn("binary-x86_64-pc-windows-msvc", ALL)
        self.assertIn("wheel-aarch64-apple-darwin", ALL)
        self.assertIn("ci-build-0.4.1", ALL)


if __name__ == "__main__":
    unittest.main()
