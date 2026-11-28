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

    def run_check(self, *passes, update=False, allow_growth=False):
        argv = ["--root", str(self.root), "--baseline", str(self.baseline)]
        for n, lines in enumerate(passes or ([],)):
            path = self.root / f"clippy-{n}.json"
            path.write_text("\n".join(lines) + "\n")
            argv += ["--clippy-json", str(path)]
        if update:
            argv.append("--update")
        if allow_growth:
            argv.append("--allow-growth")
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = lint_debt.main(argv)
        return code, out.getvalue() + err.getvalue()

    def record(self, *passes):
        """Write a baseline holding exactly these findings plus the tree's exemptions."""
        self.assertEqual(self.run_check(*passes, update=True, allow_growth=True)[0], 0)

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

    def test_both_passes_are_counted_and_a_shared_finding_once(self):
        portable = [message("clippy::unwrap_used", "src/a.rs", 3)]
        native = [message("clippy::unwrap_used", "src/a.rs", 3),
                  message("clippy::unwrap_used", "src/b.rs", 1)]
        self.record(portable, native)
        baseline = json.loads(self.baseline.read_text())
        self.assertEqual(baseline["clippy::unwrap_used"], {"src/a.rs": 1, "src/b.rs": 1})
        self.assertEqual(self.run_check(portable, native)[0], 0)

    def test_unchanged_counts_pass(self):
        lines = [message("clippy::unwrap_used", "src/a.rs", 3)]
        self.record(lines)
        code, output = self.run_check(lines)
        self.assertEqual(code, 0, output)
        self.assertIn("within the baseline", output)

    def test_growth_fails_and_prints_the_new_findings(self):
        self.record([message("clippy::unwrap_used", "src/a.rs", 3)])
        code, output = self.run_check([
            message("clippy::unwrap_used", "src/a.rs", 3),
            message("clippy::unwrap_used", "src/a.rs", 9, "new unwrap"),
            message("clippy::disallowed_methods", "src/b.rs", 1,
                    "use of a disallowed method `std::fs::remove_file`"),
        ])
        self.assertEqual(code, 1)
        self.assertIn("clippy::unwrap_used grew from 1 to 2", output)
        self.assertIn("src/a.rs: 1 -> 2", output)
        self.assertIn("new unwrap", output)
        self.assertIn("clippy::disallowed_methods(std::fs::remove_file) grew from 0 to 1", output)

    def test_a_new_exemption_is_growth(self):
        self.record()
        with open(self.root / "src/b.rs", "a") as f:
            f.write("#[expect(clippy::too_many_arguments, reason = \"wide\")]\nfn c() {}\n")
        code, output = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("expect(clippy::too_many_arguments) grew from 1 to 2", output)
        self.assertIn("src/b.rs: 0 -> 1", output)

    def test_moved_code_warns_without_failing(self):
        self.record([message("clippy::unwrap_used", "src/a.rs", 3)])
        code, output = self.run_check([message("clippy::unwrap_used", "src/b.rs", 7)])
        self.assertEqual(code, 0, output)
        self.assertIn("warning", output)
        self.assertIn("src/b.rs: clippy::unwrap_used 0 -> 1", output)

    def test_a_fall_passes_and_update_keeps_it(self):
        self.record([message("clippy::unwrap_used", "src/a.rs", 3)])
        code, output = self.run_check()
        self.assertEqual(code, 0, output)
        self.assertIn("clippy::unwrap_used 1 -> 0", output)
        self.assertEqual(self.run_check(update=True)[0], 0)
        self.assertNotIn("clippy::unwrap_used", json.loads(self.baseline.read_text()))
        code, output = self.run_check([message("clippy::unwrap_used", "src/a.rs", 3)])
        self.assertEqual(code, 1, output)

    def test_update_never_records_growth_without_allow_growth(self):
        self.record([message("clippy::unwrap_used", "src/a.rs", 3)])
        grown = [message("clippy::unwrap_used", "src/a.rs", 3),
                 message("clippy::unwrap_used", "src/b.rs", 1),
                 message("clippy::expect_used", "src/b.rs", 2)]
        code, output = self.run_check(grown, update=True)
        self.assertEqual(code, 0)
        self.assertIn("not recorded: clippy::expect_used", output)
        self.assertIn("not recorded: clippy::unwrap_used", output)
        baseline = json.loads(self.baseline.read_text())
        self.assertEqual(baseline["clippy::unwrap_used"], {"src/a.rs": 1})
        self.assertNotIn("clippy::expect_used", baseline)
        self.assertEqual(self.run_check(grown)[0], 1)
        self.assertEqual(self.run_check(grown, update=True, allow_growth=True)[0], 0)
        self.assertEqual(self.run_check(grown)[0], 0)

    def test_allow_growth_needs_update(self):
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            lint_debt.main(["--root", str(self.root), "--allow-growth"])

    def test_clippy_errors_stop_the_check(self):
        code, output = self.run_check([message(None, "src/a.rs", 1, "mismatched types", level="error")])
        self.assertEqual(code, 1)
        self.assertIn("mismatched types", output)

    def test_tree_exemptions_are_within_the_committed_baseline(self):
        # Exemptions need no build, so the committed baseline is checked against the source here.
        baseline = json.loads((ROOT / "scripts/lint_debt.json").read_text())
        recorded = lint_debt.totals({k: v for k, v in baseline.items() if k.startswith("expect(")})
        for key, total in lint_debt.totals(lint_debt.counts(lint_debt.exemptions(ROOT))).items():
            self.assertLessEqual(total, recorded.get(key, 0), key)


if __name__ == "__main__":
    unittest.main()
