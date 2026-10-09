# C1 payload inspection evidence and review method

Final functional source commit:
`64122fd3085347ea95c2d2458bc5c109bfd06a59`, on
`feat/c1-payload-validation`, based on develop
`00a9d1c34c3017a9475e6099c2805d6d2366d70a`.
Frozen native binary SHA-256
`775267c80696e0df575437ee5e1ef5c634d579ea2aa2b637d50746d8cc4d5beb`
passed 77 independent CLI cases, 20 fresh-process resource probes and 147
actual producer b3dm framing controls. Fresh corpus bytes exactly reproduce
the checked fixtures. Installed portable release wheel passed all 77 cases with complete success-report
and failure-class parity against the CLI under `python -I` and empty `PATH`.
The installed extension hash matches its wheel ZIP entry.
The source commit alone is not an execution result or release authorization.

[Independent implementation review](independent-review.md) records nonauthor
review of archive authority, raw ZIP admission, payload inspection and bounded
implicit presentation. It identifies the reviewer's authored public types,
adapters/docs so that authorship is not misrepresented as independent proof.
Source-derived findings, separately executed counterexamples and attributed
coordinator tests are distinguished.

The independent corpus writes its own ZIP/index/GLB/accessor bytes and records
analytic facts rather than asking the producer to establish expected validity.
It exercises typed Rust/CLI outcomes and preservation of input bytes. Primary
specification pins and finite capability/inspection gaps are documented in
[VALIDATION.md](../../../docs/VALIDATION.md). Optional metadata/material semantics,
implicit availability/addressing, decoded content containment and source/LOD
fidelity remain unclaimed.

Historical audit and intermediate candidate artifacts are retained:

- `baseline-historical.json` and `baseline-hand-fixtures.json`: legacy defects.
- `candidate-role-defects.json`: early mixed-role/semantic-set acceptance.
- `candidate-work-amplification.json`: early repeated-index work timeout.
- `audit.json`: audit results with their own provenance.

These counterexamples explain redesign/fixes; they do not imply that a revised
candidate still has those defects. Conversely a source fix is not an executed
acceptance pass. Final evidence must name exact source/binary/fixture hashes,
features/toolchain, actual passed checks and unverified limitations. The shared
JSON callback, archive-wide expansion counters and actual buffer/index decoding
are inspected ownership boundaries, not a universal RSS/time guarantee.

Final native records: `cli.json`, `resources.json`,
`producer-alignment.json`, and `historical-final.json`; `audit.json` pins their
hashes. Resource measurements are observations, not universal RSS or time bounds.

`wheel-parity.json` records installed-wheel/extension hashes and input preservation.
The portable wheel excludes GDAL/native JPEG; these checks do not imply those features.
