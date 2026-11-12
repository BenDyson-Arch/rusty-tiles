"""scripts/release_notes.py: the GitHub release body from the tagged CHANGELOG section."""
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import release_notes

CHANGELOG = textwrap.dedent("""\
    # Changelog

    Intro text.

    ## 0.4.1 - 2026-12-01

    ### Removed

    - Intel macOS.

    ### Fixed

    - A fix.

    ## 0.4.0 - 2026-11-11

    ### Added

    - Everything.

    ## 0.3.1

    ### Changed

    ## 0.2.0

    - Old.
    """)


class ReleaseNotesTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="rusty-tiles-notes-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / "CHANGELOG.md").write_text(CHANGELOG)

    def notes(self, version="0.4.1", tag="v0.4.1", previous="v0.4.0", dry_run=False):
        return release_notes.notes(self.root, version, tag, "owner/repo", previous, dry_run)

    def test_section_runs_to_the_next_heading_and_links_the_tag(self):
        text = self.notes()
        self.assertTrue(text.startswith("### Removed\n\n- Intel macOS.\n\n### Fixed\n\n- A fix.\n\n"))
        self.assertNotIn("Everything", text)
        self.assertNotIn("## 0.4.1", text)
        self.assertIn("(https://github.com/owner/repo/blob/v0.4.1/CHANGELOG.md#041---2026-12-01)", text)
        self.assertIn("[v0.4.0...v0.4.1](https://github.com/owner/repo/compare/v0.4.0...v0.4.1)", text)

    def test_section_without_a_date_and_without_a_previous_tag(self):
        text = self.notes("0.2.0", "v0.2.0", previous=None)
        self.assertIn("- Old.", text)
        self.assertIn("CHANGELOG.md#020)", text)
        self.assertNotIn("compare", text)

    def test_last_section_runs_to_the_end(self):
        self.assertTrue(self.notes("0.2.0", "v0.2.0").startswith("- Old.\n"))

    def test_missing_or_empty_section_fails(self):
        for version, message in (("0.5.0", "no '## 0.5.0' section"), ("0.3.1", "has no entries"),
                                 ("0.4", "no '## 0.4' section")):
            with self.subTest(version=version), self.assertRaisesRegex(release_notes.NotesError, message):
                self.notes(version, f"v{version}")

    def test_duplicate_section_fails(self):
        (self.root / "CHANGELOG.md").write_text(CHANGELOG + "\n## 0.4.0\n\n- Again.\n")
        with self.assertRaisesRegex(release_notes.NotesError, "2 '## 0.4.0' sections"):
            self.notes("0.4.0", "v0.4.0")

    def test_hand_written_summary_goes_first(self):
        (self.root / "release-notes").mkdir()
        (self.root / "release-notes" / "v0.4.1.md").write_text("Highlights of 0.4.1.\n\n")
        text = self.notes()
        self.assertTrue(text.startswith("Highlights of 0.4.1.\n\n### Removed\n"))
        # Another tag's summary is not used.
        self.assertFalse(self.notes("0.4.0", "v0.4.0").startswith("Highlights"))
        (self.root / "release-notes" / "v0.4.1.md").write_text(" \n")
        with self.assertRaisesRegex(release_notes.NotesError, "is empty"):
            self.notes()

    def test_dry_run_tolerates_a_missing_section(self):
        text = self.notes("0.4.2-dev.202612011200", "dry-run", previous=None, dry_run=True)
        self.assertIn("a release tag needs a `## 0.4.2-dev.202612011200 - DATE` section", text)
        with self.assertRaises(release_notes.NotesError):
            self.notes("0.3.1", "v0.3.1", dry_run=True)  # empty is still an error

    def test_overlong_section_is_cut_at_a_line_and_keeps_the_links(self):
        entries = "".join(f"- Entry {n} {'x' * 200}\n" for n in range(1000))
        (self.root / "CHANGELOG.md").write_text(f"## 0.5.0 - 2027-01-01\n\n### Added\n\n{entries}\n## 0.4.0\n\n- Old.\n")
        text = release_notes.notes(self.root, "0.5.0", "v0.5.0", "owner/repo", "v0.4.0")
        self.assertLessEqual(len(text), release_notes.BODY_LIMIT)
        self.assertIn("The changelog is shortened here", text)
        self.assertTrue(text.rstrip().endswith("(https://github.com/owner/repo/compare/v0.4.0...v0.5.0)"))
        kept = [line for line in text.split("\n") if line.startswith("- Entry ")]
        self.assertTrue(all(line.endswith("x" * 200) for line in kept))  # whole lines only
        self.assertGreater(len(kept), 400)

    def test_anchor_matches_github(self):
        # Anchors GitHub renders for this repository's CHANGELOG.md at v0.4.0.
        for heading, expected in (("0.4.0 - 2026-11-11", "040---2026-11-11"), ("0.2.0", "020"),
                                  ("0.4.1-rc.1 - 2026-12-01", "041-rc1---2026-12-01")):
            with self.subTest(heading=heading):
                self.assertEqual(release_notes.anchor(heading), expected)

    def test_command_line_and_the_real_changelog(self):
        run = lambda *args: subprocess.run([sys.executable, "-I", str(ROOT / "scripts" / "release_notes.py"), *args],
                                           capture_output=True, text=True)
        result = run("0.4.0", "--tag", "v0.4.0", "--repo", "BenDyson-Arch/rusty-tiles", "--previous", "v0.3.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("blob/v0.4.0/CHANGELOG.md#040---2026-11-11", result.stdout)
        result = run("9.9.9", "--tag", "v9.9.9", "--repo", "o/r")
        self.assertEqual(result.returncode, 1)
        self.assertIn("no '## 9.9.9' section", result.stderr)


if __name__ == "__main__":
    unittest.main()
