"""scripts/check_ci_config.py: workflow literals must match ci/versions.env."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import check_ci_config


VERSIONS = """\
# comment
MATURIN_VERSION=1.15.0
TIMEOUT_SHORT=5
TIMEOUT_LONG=90
GDAL_VERSION=3.12.4
GDAL_SHA256=68844ae29557b7efae4292c3b4cb3a3b8a79d14b765b89c5a7b17cbae7fa715a
PROJ_VERSION=9.8.1
GEOBASE_REPOSITORY=ghcr.io/example/geobase
UNSUPPORTED_TARGETS=x86_64-apple-darwin
UNSUPPORTED_RUNNERS=macos-13,macos-15-intel
"""

WORKFLOW = """\
name: Example
on: push
jobs:
  short:
    runs-on: ubuntu-24.04
    timeout-minutes: 5 # versions: TIMEOUT_SHORT
    steps:
      - run: python -m pip install 'maturin==1.15.0' # versions: MATURIN_VERSION
  choose:
    runs-on: ubuntu-24.04
    timeout-minutes: ${{ inputs.reuse && 5 || 90 }} # versions: TIMEOUT_SHORT TIMEOUT_LONG
    steps:
      - run: true
  call:
    uses: ./.github/workflows/other.yml
"""

DOCKERFILE = """\
ARG GDAL_VERSION=3.12.4
ADD --checksum=sha256:68844ae29557b7efae4292c3b4cb3a3b8a79d14b765b89c5a7b17cbae7fa715a https://example/gdal.tar.gz /gdal.tar.gz
"""


class CheckCiConfigTests(unittest.TestCase):
    def tree(self, workflow=WORKFLOW, dockerfile=DOCKERFILE, versions=VERSIONS):
        temp = tempfile.TemporaryDirectory(prefix="rusty-tiles-ci-config-")
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        (root / "ci").mkdir()
        (root / "ci" / "versions.env").write_text(versions)
        (root / ".github" / "workflows").mkdir(parents=True)
        (root / ".github" / "workflows" / "example.yml").write_text(workflow)
        (root / "Dockerfile").write_text(dockerfile)
        return root

    def errors(self, **files):
        return check_ci_config.check(self.tree(**files))

    def test_consistent_tree_passes(self):
        self.assertEqual(self.errors(), [])

    def test_changed_value_fails(self):
        errors = self.errors(workflow=WORKFLOW.replace("maturin==1.15.0", "maturin==1.14.0"))
        self.assertEqual(len(errors), 1)
        self.assertIn("expected MATURIN_VERSION=1.15.0", errors[0])

    def test_value_must_be_a_whole_token(self):
        errors = self.errors(workflow=WORKFLOW.replace("timeout-minutes: 5 #", "timeout-minutes: 15 #"))
        self.assertEqual(len(errors), 1)
        self.assertIn("TIMEOUT_SHORT=5", errors[0])

    def test_both_keys_of_a_choice_are_checked(self):
        errors = self.errors(workflow=WORKFLOW.replace("|| 90 }}", "|| 45 }}"))
        self.assertEqual(len(errors), 1)
        self.assertIn("TIMEOUT_LONG=90", errors[0])

    def test_unknown_key_fails(self):
        errors = self.errors(workflow=WORKFLOW.replace("# versions: MATURIN_VERSION", "# versions: MATURIN"))
        self.assertIn("unknown key MATURIN", errors[0])

    def test_unmarked_time_limit_and_pin_fail(self):
        workflow = WORKFLOW.replace(" # versions: TIMEOUT_SHORT\n", "\n", 1)
        workflow = workflow.replace(" # versions: MATURIN_VERSION", "")
        errors = self.errors(workflow=workflow)
        self.assertEqual(len(errors), 2)
        self.assertIn("timeout-minutes needs", errors[0])
        self.assertIn("pinned version needs", errors[1])

    def test_job_without_time_limit_fails(self):
        workflow = WORKFLOW + "  late:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: true\n"
        errors = self.errors(workflow=workflow)
        self.assertEqual(errors, [".github/workflows/example.yml: job late has no timeout-minutes"])

    def test_dockerfile_defaults_and_checksums_are_checked(self):
        errors = self.errors(dockerfile=DOCKERFILE.replace("3.12.4", "3.12.3").replace("68844a", "000000"))
        self.assertEqual(len(errors), 2)
        self.assertIn("ARG GDAL_VERSION should default to 3.12.4", errors[0])
        self.assertIn("is not in ci/versions.env", errors[1])

    def test_geobase_is_the_source_stage_or_the_pinned_image_of_these_versions(self):
        digest = "@sha256:" + "0" * 64
        cases = {
            "geospatial-source": True,
            "ghcr.io/example/geobase:3.12.4-9.8.1" + digest: True,
            "ghcr.io/example/geobase:3.12.4-9.8.1": False,
            "ghcr.io/example/geobase:3.12.3-9.8.1" + digest: False,
            "ghcr.io/other/geobase:3.12.4-9.8.1" + digest: False,
        }
        for value, valid in cases.items():
            with self.subTest(value=value):
                errors = self.errors(dockerfile=f"ARG GEOBASE={value}\n" + DOCKERFILE)
                self.assertEqual(errors == [], valid, errors)

    def test_unsupported_targets_and_runners_fail_outside_comments(self):
        row = "          - os: macos-15-intel\n            target: x86_64-apple-darwin\n"
        workflow = WORKFLOW.replace("    steps:\n      - run: true\n", "    steps:\n      - run: true\n" + row, 1)
        errors = self.errors(workflow=workflow)
        self.assertEqual(len(errors), 2, errors)
        self.assertIn("macos-15-intel is not supported", errors[0])
        self.assertIn("x86_64-apple-darwin is not supported", errors[1])
        commented = WORKFLOW + "# Intel macOS (x86_64-apple-darwin, macos-15-intel) is not a target.\n"
        self.assertEqual(self.errors(workflow=commented), [])
        # Other runners are fine, and a label is matched whole.
        self.assertEqual(self.errors(workflow=WORKFLOW.replace("ubuntu-24.04", "macos-15", 1)), [])
        self.assertEqual(len(self.errors(workflow=WORKFLOW.replace("ubuntu-24.04", "macos-13-xlarge", 1))), 1)

    def test_artifact_reuse_targets_are_checked(self):
        root = self.tree()
        (root / "scripts").mkdir()
        (root / "scripts" / "ci_run.py").write_text('TARGETS = (\n    "x86_64-apple-darwin",\n)\n')
        errors = check_ci_config.check(root)
        self.assertEqual(len(errors), 1)
        self.assertIn("scripts/ci_run.py:2", errors[0])

    def test_malformed_versions_file_is_refused(self):
        with self.assertRaises(SystemExit):
            self.errors(versions=VERSIONS + "BROKEN = 1\n")
        with self.assertRaises(SystemExit):
            self.errors(versions=VERSIONS + "TIMEOUT_SHORT=6\n")

    def test_repository_is_consistent(self):
        completed = subprocess.run([sys.executable, str(ROOT / "scripts" / "check_ci_config.py")],
                                   capture_output=True, text=True)
        self.assertEqual(completed.returncode, 0, completed.stderr)


if __name__ == "__main__":
    unittest.main()
