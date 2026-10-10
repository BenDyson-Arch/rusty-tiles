# Separate corrected-head remote review — 2026-10-10

Accepted: the bounded stored-archive owner/current C1 slice at exact corrected
PR head `7cbc0884e79d7057d58e9a1fce79dd4d6e1c9440` has passed the applicable
remote, installed-wheel and official Blender gates. This separate review supports
the authorized merge of that exact head into develop. It grants no broader A2 or
release acceptance.

[Initial remote pins](initial-remote-pins.json) record this first read-only review
phase; [final artifact pins](final-artifact-pins.json) bind completed acceptance.
The reviewer performed source/receipt/hash inspection and small independent
fixture checks only; no Cargo, producer, installed-package or heavy execution.

The reviewed PR head is `7cbc0884e79d7057d58e9a1fce79dd4d6e1c9440`.
Its entire tree `58aadfdc1bb78c507d020ab91494dc83a3d646a3` equals the fetched
CI merge preview `2f0df910c18af4da938a5e83cc674a9e96ae6f8c`, whose parents are
develop `9d2973db06967f87d759c87c34d8fdd0921c86ed` and that PR head. All 93
production and seven acceptance input hashes remain identical to the separate
correction2 review. The merge preview therefore runs the same complete source
tree, with its distinct commit identity retained explicitly.

The main archive receipt pins the executed debug CLI SHA-256
`c50c76c19dfbde1f0c33832807ec886fb9b5339205091ed3a2ffa2f715046844`,
merge-preview source commit, unchanged fixture driver and unchanged runner. All
45 fixture hashes match independently reconstructed tiny archive bytes, and all
45 recorded categories, exit codes and JSON reports agree with the independent
framing oracle: 14 admitted, seven Unsupported and 24 InvalidInput. This includes
central-order identity, ZIP64 and descriptors, coherent physical overlap,
malformed TLVs/ranges, index cardinality/hash/offset/CRC association, duplicate
names and Unicode-path overrides. Each receipt asserts that its source bytes
remained unchanged. These executed categories do not replace the earlier native
injected-I/O and allocation-order controls with a Python model.

The same binary hash appears in all 20 actual resource runs, the C1 135-case
regression receipt and the actual staircase producer control. The resource runs
cover ten named cases twice. Triangle and legal shared indices succeed; all
eight over-limit cases return ResourceLimit with exit 3, including the implicit
capacity-before-read control. Recorded whole-process Linux RSS ranges from
78,092 to 99,796 KiB and wall time from 0.125 to 0.292 seconds. Six descriptors
were observed at peak; 32 permission-denied sampling events limit descriptor
visibility. These are observed measurements, not universal RAM or descriptor
limits. The resource corpus read-only snapshot check is true.

The staircase source hash matches the committed fixture. Its independently
checked ring has 50 vertices and area 300 square metres. The producer receipt
records eight b3dm controls, eight fragments, maximum 32 tile vertices and 4,412
tile bytes, within its 32-vertex and 16,384-byte options. The reviewed checker
source performs actual b3dm/embedded-GLB framing and padding checks. The remote
producer archive and CLI bytes themselves are not present in this initial
download, so their hashes remain executed receipt assertions rather than a
second reviewer hash of those artifacts. The ancillary 135-case receipt has
consistent JSON/exit outcomes and read-only assertion; this does not grant new
contract acceptance for every historical semantic case.

The initial `main-run.json` and `wheel-run.json` snapshots identify the correct
head but still say `in_progress` with null conclusion. The partial wheel artifact
metadata also cannot establish completed installed-wheel or Blender acceptance.
Those initial snapshots are retained as initial observations only; completed
acceptance uses the separately downloaded final records below. Historical local
source/artifact phases remain separately bound and unchanged.

## Completed remote and installed-artifact gates

Final main run `38040463690` and wheel run `38040463666` both report completed
success at the exact reviewed PR head. Final individual job records and the PR
rollup agree: 12 main jobs, five installed-wheel jobs and four official Blender
jobs passed, for 21 applicable successes. Release acceptance and the optional
distribution-Blender job were correctly skipped under this pull-request workflow.
The final metadata therefore resolves the initial in-progress observations.

The reviewer hashed the actual five downloaded wheels and matched each hash to
both CPython 3.10 and 3.14 installed receipts. All ten receipts report 44 tests,
zero failures/errors/skips and empty PATH. Package locations identify temporary
installed site-packages. Targets cover Linux x86_64/aarch64, macOS x86_64/arm64
and Windows x86_64. This includes the formerly exhausted macOS x86 adaptive proof
with its explicitly increased cooperative checker capacity.

All four official Blender receipts likewise report 44 tests, zero
failures/errors/skips and empty PATH, bound to the same applicable downloaded
wheel hashes. Their runtime identities are Blender 4.5.14 LTS with embedded
Python 3.11.15. The standalone distribution records exactly match their nested
acceptance records and the committed platform-specific official archive digests,
version, URL and runtime architecture. The reviewed runner verifies the extracted
executable hash before execution and checks runtime version/architecture after
completion. These bundle/executable identities are executed runner checks; this
review did not download official bundles for an additional local hash check.

The acceptance harness scripts and official manifest were checked against the
reviewed Git head. Final GitHub artifact metadata binds the downloaded receipt
artifacts to the correct workflows/head. The actual five wheel hash-verification
record also agrees with the reviewer-computed hashes and paired test receipts.
The main remote debug CLI hash remains an executed receipt hash; the remote CLI
and producer archive bytes were not independently downloaded in this lane.

The durable remote index at
`archive_candidate_evidence/remote-7cbc088/index.json` has SHA-256
`f6ba29667b4f08036f130479b43ebb3e02f5b7e2d914b1634e53b04938cb3aa0`.
All 30 compressed records match their indexed compressed/expanded digests and
lengths and the actual downloaded source records byte for byte. This preserves
the final main/wheel/job/artifact/rollup records, four main C1 receipts, ten
CPython reports, eight Blender distribution/acceptance records, and actual wheel
hash verification.

This new remote evidence and review are untracked at the accepted CI head. The
coordinator will retain them in the next foundation checkpoint after merging the
accepted exact head, preserving that tested source identity. No earlier local
executable is rebound to this corrected head, and no source modification is
required by this final review. Remaining broader roadmap, A2, #113/#121 and
release gates remain outside this finite acceptance.
