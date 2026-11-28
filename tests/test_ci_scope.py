"""scripts/ci_scope.py: which pull requests run the full CI tier."""
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import ci_scope


def changed(path, patch="@@ -1 +1 @@\n-a\n+b", status="modified", previous=None):
    entry = {"filename": path, "status": status}
    if patch is not None:
        entry["patch"] = patch
    if previous:
        entry["previous_filename"] = previous
    return entry


class ScopeTests(unittest.TestCase):
    def setUp(self):
        # A stand-in checkout: one Rust file with a platform block, one without.
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / "src").mkdir()
        (self.root / "src" / "platform.rs").write_text(
            "fn a() {}\n#[cfg(windows)]\nfn b() {\n    let x = 1;\n}\n")
        (self.root / "src" / "plain.rs").write_text("fn a() {}\n")

    def full(self, files, labels=(), base="develop"):
        return bool(ci_scope.reasons({"files": files}, set(labels), base, root=self.root))

    def test_build_platform_and_ci_paths_run_the_full_tier(self):
        for path in ("Cargo.toml", "Cargo.lock", "build.rs", ".cargo/config.toml",
                     "vendor/basis_universal/encoder/basisu_enc.cpp", "src/basisu_ktx2/ffi.rs",
                     "src/spill/volume.rs", "src/dir_sync.rs", "src/runtime/directory_platform.rs",
                     ".github/workflows/ci.yml", "ci/versions.env", "Dockerfile",
                     "scripts/package_cli.sh", "scripts/blender_official.json",
                     "bindings/python/Cargo.toml", "bench/demodata/Cargo.toml",
                     "tools/x/Cargo.lock", "bindings/python/build.rs"):
            with self.subTest(path=path):
                self.assertTrue(self.full([changed(path)]))
        self.assertTrue(self.full([changed("src/new.rs", status="renamed", previous="src/spill/old.rs")]))

    def test_ordinary_changes_run_the_fast_tier(self):
        for path in ("src/plain.rs", "src/missing.rs", "docs/CLI.md", "README.md",
                     "scripts/changelog.py", "Cargo.toml.orig", "src/spillover.rs", "src/build.rs.md"):
            with self.subTest(path=path):
                self.assertFalse(self.full([changed(path)]))
        self.assertFalse(self.full([changed("src/plain.rs", patch=None, status="renamed", previous="src/a.rs")]))

    def test_changed_platform_conditional_lines_run_the_full_tier(self):
        cases = {
            "@@ -1 +1,2 @@\n+#[cfg(windows)]\n fn a() {}": True,
            "@@ -1 +1 @@\n-#[cfg(all(unix, not(target_os = \"macos\")))]\n+fn a() {}": True,
            "@@ -1 +1 @@\n+    if cfg!(target_os = \"windows\") {}": True,
            "@@ -1,2 +1,2 @@\n #[cfg(windows)]\n-fn a() {}\n+fn b() {}": False,
            "@@ -1 +1 @@\n+#[cfg(test)]\n": False,
            "@@ -1 +1 @@\n+// windows are nice\n": False,
        }
        for patch, expected in cases.items():
            with self.subTest(patch=patch):
                self.assertEqual(self.full([changed("src/plain.rs", patch=patch)]), expected)
        # GitHub omits the diff of very large files: run the full tier.
        self.assertTrue(self.full([changed("src/plain.rs", patch=None)]))
        # A removed file is judged by its diff, which lists every removed line.
        self.assertTrue(self.full([changed("src/gone.rs", patch="@@ -1 +0,0 @@\n-#[cfg(unix)]", status="removed")]))
        self.assertFalse(self.full([changed("src/gone.rs", patch="@@ -1 +0,0 @@\n-fn a() {}", status="removed")]))

    def test_any_change_to_a_file_with_platform_code_runs_the_full_tier(self):
        # An edit inside an existing cfg(windows) block shows no cfg line in its diff.
        inside = "@@ -3,3 +3,3 @@\n fn b() {\n-    let x = 1;\n+    let x = 2;\n }"
        self.assertTrue(self.full([changed("src/platform.rs", patch=inside)]))
        self.assertTrue(self.full([changed("src/platform.rs", patch=None, status="renamed", previous="src/old.rs")]))
        self.assertFalse(self.full([changed("src/plain.rs", patch=inside)]))

    def test_label_main_base_and_doubt_run_the_full_tier(self):
        docs = [changed("docs/CLI.md")]
        self.assertTrue(self.full(docs, labels=["full-ci"]))
        self.assertFalse(self.full(docs, labels=["bug"]))
        self.assertTrue(self.full(docs, base="main"))
        self.assertTrue(self.full([]))
        self.assertTrue(self.full([changed(f"docs/{n}.md") for n in range(300)]))
        self.assertTrue(ci_scope.reasons(None, set(), "develop"))

    def test_command_line(self):
        with tempfile.TemporaryDirectory() as temp:
            compare = Path(temp) / "compare.json"
            compare.write_text(json.dumps({"files": [changed("docs/CLI.md")]}))
            run = lambda *extra: subprocess.run(
                [sys.executable, "-I", str(ROOT / "scripts" / "ci_scope.py"), "full", str(compare), *extra],
                capture_output=True, text=True, check=True).stdout
            self.assertEqual(run(), "full=false\n")
            (Path(temp) / "x.rs").write_text("#[cfg(target_os = \"macos\")]\nfn a() {}\n")
            compare.write_text(json.dumps({"files": [changed("x.rs")]}))
            self.assertEqual(run("--root", temp), "full=true\n")
            compare.write_text(json.dumps({"files": [changed("docs/CLI.md")]}))
            self.assertEqual(run("--label", "full-ci"), "full=true\n")
            compare.write_text("not json")
            self.assertEqual(run(), "full=true\n")

    def test_patterns_compile(self):
        self.assertGreater(len(ci_scope.load_patterns()), 10)


class WorkflowTierTests(unittest.TestCase):
    """What ci.yml runs for each tier, read from the workflow text (stdlib only, no YAML)."""

    @staticmethod
    def job(workflow, name):
        text = (ROOT / ".github" / "workflows" / workflow).read_text()
        match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  \S|\Z)", text, re.M | re.S)
        assert match, f"{workflow} has no job {name}"
        return match.group(1)

    def condition(self, workflow, name):
        block = self.job(workflow, name)
        match = re.search(r"^    if: (>-\n(?:      .*\n)+|.*\n)", block, re.M)
        return " ".join(match.group(1).replace(">-", "").split()) if match else ""

    def portable_rows(self, full):
        """The portable matrix for a `full` output, as GitHub evaluates `a && 'X' || 'Y'`."""
        block = self.job("ci.yml", "portable")
        expression = " ".join(re.search(r"^        include: >-\n(.*?\}\})", block, re.M | re.S).group(1).split())
        match = re.fullmatch(r"\$\{\{ fromJSON\(needs\.changes\.outputs\.full == 'false' && '(\[.*?\])'"
                             r" \|\| '(\[.*?\])'\) \}\}", expression)
        self.assertIsNotNone(match, expression)
        fast, every = (json.loads(group) for group in match.groups())
        return fast if full == "false" else every

    def test_fast_tier_runs_the_windows_portable_job_only(self):
        self.assertEqual(self.portable_rows("false"),
                         [{"os": "windows-2022", "target": "x86_64-pc-windows-msvc", "slow": True}])

    def test_full_tier_and_a_failed_classifier_run_every_portable_target(self):
        for full in ("true", ""):
            with self.subTest(full=full):
                rows = self.portable_rows(full)
                self.assertEqual({row["target"]: row["slow"] for row in rows}, {
                    "x86_64-unknown-linux-gnu": False, "aarch64-unknown-linux-gnu": False,
                    "aarch64-apple-darwin": False, "x86_64-pc-windows-msvc": True})
                self.assertIn({"os": "windows-2022", "target": "x86_64-pc-windows-msvc", "slow": True}, rows)

    def test_docs_only_changes_skip_the_portable_job_in_both_tiers(self):
        # The portable job packages with the build version, so both run in either tier.
        for name in ("portable", "version"):
            with self.subTest(job=name):
                condition = self.condition("ci.yml", name)
                self.assertIn("needs.changes.outputs.docs_only != 'true'", condition)
                self.assertNotIn("needs.changes.outputs.full", condition)
        # The other full-tier matrices still skip the fast tier.
        self.assertIn("needs.changes.outputs.full != 'false'", self.condition("ci.yml", "encoder-link"))

    def test_only_develop_starts_the_dev_build(self):
        condition = self.condition("ci.yml", "dev-build")
        self.assertIn("github.ref == 'refs/heads/develop'", condition)
        self.assertIn("inputs.nightly", condition)
        self.assertIn("--ref develop", self.job("ci.yml", "dev-build"))

    def test_nightly_branches_report_to_their_own_issue(self):
        dispatch = self.job("nightly.yml", "dispatch")
        branches = re.search(r"^        branch: \[(.*)\]$", dispatch, re.M).group(1).split(", ")
        self.assertEqual(branches[0], "develop")
        self.assertEqual(len(branches), len(set(branches)))
        self.assertIn('--ref "$BRANCH" -f nightly=true', dispatch)
        self.assertIn('title="Full CI matrix failing on $BRANCH"', dispatch)
        tracking = self.job("ci.yml", "tracking-issue")
        self.assertIn('title="Full CI matrix failing on $BRANCH"', tracking)
        self.assertIn("BRANCH: ${{ github.ref_name }}", tracking)
        # Pushes report on develop only; any nightly run reports to its branch.
        condition = self.condition("ci.yml", "tracking-issue")
        self.assertIn("(github.event_name == 'push' && github.ref == 'refs/heads/develop')", condition)
        self.assertIn("(github.event_name == 'workflow_dispatch' && inputs.nightly)", condition)


if __name__ == "__main__":
    unittest.main()
