# PR B evidence

Draft: https://github.com/BenDyson-Arch/rusty-tiles/pull/312.
Tested implementation: `ca963b780688b4c00660a7ce595e9ed470188eb2`, on
PR A `5c319c9b8b9bbf4b3073dc365bcc7a1daee9587d`.

Bulk evidence was never tracked. It is retained at
`/home/bend/.cache/rusty-tiles-evidence/pr-b-20261102/`:

- `FINAL.md`: SHA-256 `90926e85bbb97b8a598212d8c0608f04b3e02e56f5f5a26c46fc8eb3ade7d39d`.
- `MANIFEST.sha256`: SHA-256 `96bcd681bc661b91066cc4ebafc111808eacd383f5348438e67683d4a0e54f01`.

The report covers guarded D10 support admission, per-tile triangles/G,
source-floor/achieved appearance, exact PR A leaf comparison, 200-city
comparison, crop3 S5, native archives, three slot-1 CPU pairs, and resource
limits. The manifest binds final logs, scripts, artifacts and references.
No full Walem bake or floor result is claimed. A/R integration and separate
nonauthor review remain open; this evidence does not authorize promotion.

## PR A merge follow-up

Merged source: `6d224ccd5934da26aa40afbe40dc35045b811b0a` (parents `9b848fed` and `b5ef178f`).
Current validation is in the evidence directory's `merge-a-b5ef178f/`:

- `REPORT.md`: SHA-256 `6489e7c88db5508bbf3e662140346def31b8543e1e8c3f65a48e7c118bfe788f`.
- `MANIFEST.sha256`: SHA-256 `5f67cb65c279cf3255a0fb99cdb32761d3b446c0c30b99d0c73056bc989b5d63`.

The requested digest/library/integration/clippy/fmt checks pass; the digest
fixture was not regenerated. Prior crop/S5/CPU results above retain their
original source identity. The Sol review of `9b848fed` predates this merge.
