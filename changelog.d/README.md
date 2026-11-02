# Changelog fragments

Each PR adds one file here, `<short-slug>.md` (lowercase letters, digits, `.`,
`_`, `-`; a PR number in the name is optional, for example `vector-records.md`
or `201-fix-gpkg.md`). Do not edit `CHANGELOG.md` in a PR: parallel edits to it
conflict. Fragments are merged into it at release.

The content is the entry bullets under an H3 section line. Known sections:
`Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`. A file may
hold several sections, and each needs at least one `- ` bullet. A bullet may
continue on indented lines.

```markdown
### Changed

- Breaking: `--maxPoints` now defaults to 8192 and must be at most 8192.

### Fixed

- GeoPackage epochs are respected.
```

Commands (Python 3 standard library only):

- `python3 scripts/changelog.py check` validates every fragment and exits
  non-zero on errors. CI runs it.
- `python3 scripts/changelog.py assemble` appends the entries to the matching
  sections of the top `## ... Unreleased` section of `CHANGELOG.md` (creating a
  section in Keep a Changelog order if it is missing), in file name order, then
  deletes the fragments. Running it again with no fragments changes nothing.

## Internal-only exemptions

Changes confined to development docs, tests, evidence, benchmark tooling or
behavior-preserving private refactors may
record an exemption here instead of a release bullet. Post-policy merges reviewed
for 0.4.0 (no shipped API or output change):

- #205: annotation/preset roadmap decisions.
- #207: H8 mutant corpus/test controls and extraction of the unchanged private
  writer error-classification helper.
- #210: evidence relocation and repository tree budget tooling.
- #211: tiler roadmap decisions.
- #212: benchmark flythrough lane and reporting.
- #220: roadmap renumbering.
- #267: foundation audit/rescope proposal.
- #282: deduplicate C1 specification copies.
- #305: deduplicate fixtures with shared references.
- Tree headroom: lossless C2 witness report storage and reader updates (internal only).
