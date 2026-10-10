# F1d2 final CI and merge evidence

PR #150 merged into `develop` at
`34a76152d18b02553691f472207225f116307a67`. The merge tree is identical to
reviewed head `43d14b1708c92d8bba9d2567da695558fd5320ac`.

All 21 active final-head checks passed in CI run `38031595353` and wheel run
`38031595401`. The separate nonauthor reviewer inspected the actual portable,
native, browser and Khronos receipts, five wheel platforms and four official
Blender runs. All installed runs passed 44 tests. Two intentional skips are
release acceptance and optional distribution Blender; neither is reported as
a pass. The existing bounded F1d2 proof limits remain unchanged.

The remote reviewer verified all 16 downloaded artifact ZIP identities and 23
receipt bytes, actual wheel hashes against Python and Blender receipts, and
the PR test-merge parents/tree against the reviewed production and driver
bytes. The original expanded records remain in the external final evidence
directory. [storage-index.json](storage-index.json) records lossless compressed
identities, original paths and sizes. No wheels or binaries are committed.

This accepts the bounded local-surface slice and its merge. It does not close
#113/#121 or authorize tagging, publication or release.
