"""scripts/ci_scope.py: which pull requests run the full CI tier."""
import json
from pathlib import Path
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


if __name__ == "__main__":
    unittest.main()
