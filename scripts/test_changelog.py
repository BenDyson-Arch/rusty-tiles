#!/usr/bin/env python3
"""Tests for scripts/changelog.py. Run: python3 -I scripts/test_changelog.py"""
import contextlib
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("changelog", Path(__file__).resolve().with_name("changelog.py"))
changelog = importlib.util.module_from_spec(spec)
spec.loader.exec_module(changelog)

BASE = """# Changelog

Intro.

## 0.4.0 - Unreleased

### Changed

- old changed

### Added

- old added

### Fixed

- old fixed

## 0.3.0 - 2026-10-07

### Added

- released
"""


def run(root, mode):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = changelog.main([mode, "--root", str(root)])
    return code, err.getvalue()


class Fragments(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        (self.root / "changelog.d").mkdir()
        (self.root / "changelog.d" / "README.md").write_text("# readme\n")
        self.log = self.root / "CHANGELOG.md"
        self.log.write_text(BASE)

    def frag(self, name, text):
        (self.root / "changelog.d" / name).write_text(text)

    def names(self):
        return sorted(p.name for p in (self.root / "changelog.d").iterdir())

    def test_check_accepts_valid(self):
        self.frag("a.md", "### Changed\n\n- one\n  continued\n\n### Fixed\n- two\n")
        self.frag("201-b.md", "### Added\n- three\n")
        self.assertEqual(run(self.root, "check"), (0, ""))

    def test_check_accepts_no_fragments(self):
        self.assertEqual(run(self.root, "check")[0], 0)

    def test_check_rejects_bad_fragments(self):
        cases = {
            "empty.md": "",
            "blank.md": "\n\n",
            "nohead.md": "- orphan\n",
            "unknown.md": "### Misc\n- x\n",
            "nobullet.md": "### Added\n\n",
            "emptybullet.md": "### Added\n- \n",
            "prose.md": "### Added\nplain text\n",
            "h2.md": "## 1.0\n### Added\n- x\n",
            "h1.md": "# Title\n### Added\n- x\n",
            "Bad Name.md": "### Added\n- x\n",
            "notes.txt": "### Added\n- x\n",
        }
        for name, text in cases.items():
            with self.subTest(name):
                self.frag(name, text)
                code, err = run(self.root, "check")
                self.assertEqual(code, 1)
                self.assertIn(name, err)
                (self.root / "changelog.d" / name).unlink()

    def test_assemble_merges_groups_and_deletes(self):
        self.frag("b.md", "### Fixed\n\n- fix b\n\n### Removed\n\n- removed b\n")
        self.frag("a.md", "### Changed\n\n- changed a\n  more\n\n### Security\n- sec a\n")
        self.frag("c.md", "### Added\n- added c\n")
        self.assertEqual(run(self.root, "assemble")[0], 0)
        self.assertEqual(self.names(), ["README.md"])
        expected = BASE.replace("- old changed\n", "- old changed\n- changed a\n  more\n")
        expected = expected.replace("- old added\n", "- old added\n- added c\n")
        expected = expected.replace("### Fixed\n\n- old fixed\n\n", "### Removed\n\n- removed b\n\n### Fixed\n\n- old fixed\n- fix b\n\n### Security\n\n- sec a\n\n")
        self.assertEqual(self.log.read_text(), expected)

    def test_removed_inserted_before_later_section_and_released_untouched(self):
        self.frag("r.md", "### Removed\n- gone\n")
        run(self.root, "assemble")
        text = self.log.read_text()
        self.assertIn("- old added\n\n### Removed\n\n- gone\n\n### Fixed", text)
        self.assertTrue(text.endswith(BASE[BASE.index("## 0.3.0"):]))

    def test_assemble_is_idempotent(self):
        self.frag("a.md", "### Added\n- x\n")
        run(self.root, "assemble")
        once = self.log.read_text()
        self.assertEqual(run(self.root, "assemble")[0], 0)
        self.assertEqual(self.log.read_text(), once)

    def test_assemble_invalid_changes_nothing(self):
        self.frag("good.md", "### Added\n- x\n")
        self.frag("bad.md", "### Nope\n- x\n")
        self.assertEqual(run(self.root, "assemble")[0], 1)
        self.assertEqual(self.log.read_text(), BASE)
        self.assertEqual(self.names(), ["README.md", "bad.md", "good.md"])

    def test_assemble_requires_unreleased_top_section(self):
        self.log.write_text(BASE.replace("0.4.0 - Unreleased", "0.4.0 - 2026-11-01"))
        self.frag("a.md", "### Added\n- x\n")
        code, err = run(self.root, "assemble")
        self.assertEqual(code, 1)
        self.assertIn("Unreleased", err)
        self.assertEqual(self.names(), ["README.md", "a.md"])

    def test_empty_unreleased_section_gets_new_sections(self):
        self.log.write_text("# Changelog\n\n## 1.0 - Unreleased\n\n## 0.9\n\n### Added\n\n- old\n")
        self.frag("a.md", "### Added\n- x\n")
        run(self.root, "assemble")
        self.assertEqual(self.log.read_text(), "# Changelog\n\n## 1.0 - Unreleased\n\n### Added\n\n- x\n\n## 0.9\n\n### Added\n\n- old\n")

    def test_repository_fragments_are_valid(self):
        root = Path(__file__).resolve().parent.parent
        self.assertEqual(changelog.main(["check", "--root", str(root)]), 0)


if __name__ == "__main__":
    unittest.main()
