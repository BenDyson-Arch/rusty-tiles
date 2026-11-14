"""scripts/lint_debt.py: lint findings and exemptions may fall, never grow."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import lint_debt


def message(code, file, line, text="finding", level="warning", kind="lib"):
    """One `cargo clippy --message-format=json` compiler message."""
    return json.dumps({
        "reason": "compiler-message",
        "target": {"kind": [kind], "name": "rusty-tiles"},
        "message": {
            "level": level,
            "message": text,
            "code": {"code": code} if code else None,
            "rendered": f"{level}: {text}\n  --> {file}:{line}:1\n",
            "spans": [{"file_name": file, "line_start": line, "column_start": 1,
                       "is_primary": True}],
        },
    })


class LintDebtTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / "src").mkdir()
        (self.root / "tests").mkdir()
        (self.root / "src/a.rs").write_text(
            "#[expect(clippy::too_many_arguments, reason = \"wide\")]\nfn f() {}\n"
            "#[cfg_attr(\n    not(any(test, feature = \"x\")),\n    expect(\n        dead_code,\n"
            "        unused_imports,\n        reason = \"tests only\"\n    )\n)]\nfn g() {}\n"
            "#![expect(dead_code, reason = \"module\")]\n"
            "fn h() { let _ = Some(1).expect(\"value\"); }\n"
        )
        (self.root / "src/b.rs").write_text("fn b() {}\n")
        self.baseline = self.root / "lint_debt.json"
        self.json = self.root / "clippy.json"

    def run_check(self, lines, update=False):
        self.json.write_text("\n".join(lines) + "\n")
        argv = ["--root", str(self.root), "--baseline", str(self.baseline),
                "--clippy-json", str(self.json)]
        if update:
            argv.append("--update")
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = lint_debt.main(argv)
        return code, out.getvalue() + err.getvalue()

    def test_exemptions_count_each_lint_of_every_attribute_form(self):
        found = lint_debt.counts(lint_debt.exemptions(self.root))
        self.assertEqual(found, {
            "expect(clippy::too_many_arguments)": {"src/a.rs": 1},
            "expect(dead_code)": {"src/a.rs": 2},
            "expect(unused_imports)": {"src/a.rs": 1},
        })

    def test_findings_keep_ratchet_lints_once_and_key_bans_by_path(self):
        lines = [
            message("clippy::unwrap_used", "src/a.rs", 3),
            message("clippy::unwrap_used", "src/a.rs", 3, kind="bin"),  # same finding, binary
            message("clippy::needless_return", "src/a.rs", 4),  # not ratcheted
            message("clippy::disallowed_methods", "src/b.rs", 1,
                    "use of a disallowed method `std::env::var`"),
            message("clippy::unwrap_used", "/home/x/.cargo/registry/dep.rs", 1),  # outside the tree
            "Compiling rusty-tiles",
        ]
        found, errors = lint_debt.findings(lines, self.root)
        self.assertEqual(errors, [])
        self.assertEqual(lint_debt.counts(found), {
            "clippy::disallowed_methods(std::env::var)": {"src/b.rs": 1},
            "clippy::unwrap_used": {"src/a.rs": 1},
        })

    def test_unchanged_counts_pass(self):
        lines = [message("clippy::unwrap_used", "src/a.rs", 3)]
        self.assertEqual(self.run_check(lines, update=True)[0], 0)
        code, output = self.run_check(lines)
        self.assertEqual(code, 0, output)
        self.assertIn("unchanged", output)

    def test_growth_fails_and_prints_the_findings(self):
        self.assertEqual(self.run_check([message("clippy::unwrap_used", "src/a.rs", 3)], update=True)[0], 0)
        code, output = self.run_check([
            message("clippy::unwrap_used", "src/a.rs", 3),
            message("clippy::unwrap_used", "src/a.rs", 9, "new unwrap"),
            message("clippy::disallowed_methods", "src/b.rs", 1,
                    "use of a disallowed method `std::fs::remove_file`"),
        ])
        self.assertEqual(code, 1)
        self.assertIn("src/a.rs: clippy::unwrap_used grew from 1 to 2", output)
        self.assertIn("new unwrap", output)
        self.assertIn("clippy::disallowed_methods(std::fs::remove_file) grew from 0 to 1", output)

    def test_a_new_exemption_is_growth(self):
        self.assertEqual(self.run_check([], update=True)[0], 0)
        with open(self.root / "src/b.rs", "a") as f:
            f.write("#[expect(clippy::too_many_arguments, reason = \"wide\")]\nfn c() {}\n")
        code, output = self.run_check([])
        self.assertEqual(code, 1)
        self.assertIn("src/b.rs: expect(clippy::too_many_arguments) grew from 0 to 1", output)

    def test_a_fall_fails_until_the_baseline_is_updated(self):
        lines = [message("clippy::unwrap_used", "src/a.rs", 3)]
        self.assertEqual(self.run_check(lines, update=True)[0], 0)
        code, output = self.run_check([])
        self.assertEqual(code, 1)
        self.assertIn("src/a.rs: clippy::unwrap_used 1 -> 0", output)
        self.assertEqual(self.run_check([], update=True)[0], 0)
        self.assertEqual(self.run_check([])[0], 0)

    def test_clippy_errors_stop_the_check(self):
        code, output = self.run_check([message(None, "src/a.rs", 1, "mismatched types", level="error")])
        self.assertEqual(code, 1)
        self.assertIn("mismatched types", output)

    def test_baseline_matches_the_tree_exemptions(self):
        # Exemptions need no build, so the committed baseline is checked against the source here.
        baseline = json.loads((ROOT / "scripts/lint_debt.json").read_text())
        recorded = {key: files for key, files in baseline.items() if key.startswith("expect(")}
        self.assertEqual(recorded, lint_debt.counts(lint_debt.exemptions(ROOT)))


if __name__ == "__main__":
    unittest.main()
