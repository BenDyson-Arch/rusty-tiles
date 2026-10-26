# S5 validator and independent HLOD acceptance

This slice implements the owner’s D1–D4 exact-seam amendment published with S1
`055be853`. The source-aware oracle and archive-only census have separate truth
boundaries. Final stack acceptance and cost require the final integrated campaign.

Sampled G is finite, nonnegative and ordered; it is not a certified maximum.
C1 still owns resources, hashes, codecs and accessor admission, C2 owns decoded
bounds, and I1 owns implicit availability. No H17 certificate is required for
HLOD. `geometricErrorAccuracy` remains explicitly uninspected. Source-distance
sampling is the separately invoked `../s2/audit.py`, never an archive certificate.

Retain the bounded validator/facts/replay owners. Add source-identity admission
and a disk-backed decoded delivery relation; replace ad-hoc seam probes with
permanent independent controls. The census harvests E2 `816d0670`'s decoded leaf
incidence/ancestor cross-product with exact shared corner and edge keys. No
producer facts, lock files, reports or producer imports supply independent truth.

## Archive validation

`src/validate/identity.rs` projects only owned C1-admitted raw JSON, preserving
opaque overflowing extras and exact integer lexemes. It validates every present
identity column, unsigned scalar representation, counts/ranges, null IDs and
triangle-constant feature IDs. Unsupported metadata modifiers and declared local
`.gltf` identity fail closed. Views are cached within the document decode limit;
repeated scans are preflighted through the shared payload-element budget.

`src/validate/hlod_delivery.rs` stores hierarchy, references, original vertex
correspondence and decoded boundary incidences in scratch SQLite. A partial
identity strip is rejected regardless of traversal order once source identity is
expected. Original triangle IDs must address the matching primitive identity and
use rows one-to-one. Coarse `source_triangle` is refused under D4; same-payload
repeated cells do not change the payload's role. Semantic NORMAL/TEXCOORD accessor
shapes are checked. Archive correspondence does not establish external source truth.

The native hierarchy includes the explicit walk, external-document placements,
I1's expanded availability walk and memo replays. `replaceContentContinuity`
rejects ordinary contentless cells between a REPLACE renderable ancestor and its
renderable descendants. Leading empty roots and external-link placeholders are
valid. An empty ADD override cannot clear a barrier owned by a renderable
REPLACE ancestor. All delivery checks, continuity included, apply only once HLOD
identity is expected: other tilesets may keep empty intermediates, and the disk
store (with its scratch) is created only then, from a bounded pending buffer.
A coarse edge with two equal-direction faces is refused unless descendant leaves
hold that multiplicity (a nonmanifold spine). That admits the count only: native
validation does not prove coarse face orientation away from required interface
directions; the oracle's coincident-face guard does (`book-reversed-face`).
Expectation is owned outside payload labels: `--expect-profile hlod`
(`ValidationRequest::expect_profile`) or the archive's own declaration
(`asset.extras.rustyTiles.lod` or `conversion.json` `lod` equal to `hlod`) makes
identity and every delivery check mandatory, so stripping all labels refuses.
An undeclared, unflagged tileset validates as generic; HLOD is never inferred.

Upward omission (S1 `756283e2` amendment) is checked once identity is expected,
from the delivered hierarchy alone. `triangleCap` refuses any payload, leaf or
coarse, above 8,192 triangles over all its scene instances. `omissionPropagation`
refuses a content-less cell with a content ancestor in explicit, implicit or
external subtrees; link and implicit-root placeholders are transparent and add
no cell depth. `frontierCoverage` refuses a branch that ends content-less, ADD
content left drawn over renderable descendants, and the same payload bytes
(by SHA-256, so copies under new member names count) twice in one tile or
anywhere but down a single-child chain. Together these make the first
renderable cover complete and single: every renderable leaf lies below exactly
one cover tile. `emptyCellRefinement` (R9) requires every content-less cell's
geometricError to be at least 16 x its box diagonal (sphere: diameter). Under
CesiumJS base traversal an empty tile refines only while its SSE exceeds the
maximum (default 16); at that floor it refines until the whole box projects
below about one pixel, while children's error alone (S1 `756283e2`) stops at
once and, with no content ancestor by design, draws nothing.

The native census externally groups canonical point keys (12 bytes) and
unordered endpoint-pair edge keys (24 bytes), retaining actual per-leaf face
incidences. Canonical keys are decoded common-frame f32 bits, with negative zero
normalized. A triangle with fewer than three distinct keys contributes no
interface items. No triangle geometry soup, chord inference, distance matching
or T-junction obligations are used. Shared points and edges must be retained
exactly by every ancestor with an outside owner; required edge directions are
checked independently. Closed, locally balanced and nonmanifold items remain in
the incidence relation. Three distinct collinear keys still contribute items.

The separate original-ID check records, during ingest's single decode, one disk
row per original vertex (full-width source and vertex IDs, payload-frame point)
and scans them in key order, applying each leaf placement as a replay would. Every
shared original key, including disjoint geometry and repeated references, must
keep its exact placed f64 position; f32 seam canonicalization cannot weaken that
identity relation. Each payload is decoded at ingest and once per placement for
its surface; decoding, key/map work, group members, owner walks and SQLite VM
work keep their shared charges; dense work refuses rather than truncating.

`interfaceCensus` reports decoded-leaf `sharedPoints`, `sharedSegments`,
`sourceOpenEdges` and `sourceOpenEdgesWithSharedEndpoints`. A source-open edge has
exactly one incident surface face globally; its two endpoints are shared when
each occurs in a different leaf. Incidence counts faces, never net orientation
or owner count. The report names its decoded-leaf basis; authenticating source
canonical positions and measuring exempt crack widths require the source oracle.
It reports the largest payload's triangles and the derived omission census:
`omittedCells`, `omissionSources` (omitted cells whose cell children all
render), `omittedByDepth`, `frontierTiles` and `frontierLeaves`. These are
never read from producer reports. The coarse primitive metadata row limit
remains 8,192. Other coarse interior/exterior orientation is uninspected by native
validation; the oracle retains the stronger coincident-face guard everywhere.

One shared invertible root placement is removed before coordinate arithmetic.
Relative tile transforms are conjugated by the mandatory glTF Y-up conversion;
original-vertex joins use actual leaf references after placement. Raw scene
admission refuses uncovered deformation, scene selection and moving extensions
before advertising interface coverage. Geometry-neutral material extensions are
admitted explicitly. Continuity still runs before a traversal-owned scene refusal.

## Independent real-archive oracle

`archive.py` reads directories or `.3tz` directly and expands explicit, external
and implicit trees, including subtree availability/metadata. `continuity.py`
uses that same independent expansion and produces a named verdict for implicit
and legacy archives. `decode.mjs` accepts paths or GLB bytes on stdin, decodes
POSITION/NORMAL/TEXCOORD_0 and selected-scene matrix/TRS transforms, and derives
position/normal/UV tolerances independently from scalar storage, codec bits and
transforms. UINT64 columns cross JSON as decimal strings and become Python exact
integers. URI-shadow buffers and unknown BIN replacements are refused; required
meshopt virtual fallback buffers are supported.

`meshopt.py` uses the retained independent Khronos-spec codec in
`tests/ext_meshopt_spec.py`, plus specification-based OCTAHEDRAL, QUATERNION,
EXPONENTIAL and index decoding. No npm package or producer decoder is required.
Unsupported scene extensions, animation, skin, morph or UV1 source expectations
fail closed.

`source_manifest.py` independently reads the input GLB with mmap and streams
original geometry, vertex IDs, normals, UVs, material classes/constants and source
order to SQLite. Source keys and ordinals remain exact. It imports no producer.
The root-relative glTF frame removes common archive placement and applies
relative tile transforms. Exact source canonicalization currently requires that
frame and an identity postcapture source transform; world and other postcapture
frames are explicitly refused as `oracleDomain`/`sourceCanonical`. Source
manifests can also pin a separately accepted baseline, but the
real campaign uses the original input GLBs.

`omission.py` independently re-derives the cap, upward omission, single cover,
empty-cell refinement and omission census from `archive.expand` and payload
bytes (its own GLB accessor count, not `decode.mjs` or native code), and only
then compares with an S1 stage `manifest.json` (`--producer-manifest`):
`omittedProxies`, `omissionSources` and `omittedByDepth` must equal the derived
values, else `omissionReport`. `oracle.py` runs the same owner after its source
checks and also refuses a decoded payload above 8,192 faces (`triangleCap`).

`oracle.py` checks source leaf multiplicity/IDs/geometry/normals, materials and
nearest coarse representatives. A named primitive other than the source-order
nearest is accepted only within tau = 16 * 2^-52 * (|centroid| + descendant
source diagonal) of the minimum distance (D4 ties), and the largest excess is
reported; the exact-tie, within-tau and beyond-tau controls pin it. Analytic manifests
check exact UVs/materials. Real textured leaves may legitimately crop/repack UVs
and texture bindings: their material class/PBR constants/slot presence and
rendered UV structure are checked, while UV/source-pixel correspondence is
explicitly counted as uninspected. No large epsilon substitutes for image proof.

Source position fidelity and exact canonicalization are separate. The oracle
keeps original source XYZ, identities, attributes and source order. It derives
canonical f32 keys from captured source positions and independently decoded
integer lattice spacings, using ties-to-even f64 rounding and the coarsest
required incident-depth grid. FLOAT and non-axis-representable integer storage
are distinguished explicitly; conservative codec tolerance never supplies seam
equality or invents a weld. Source-equal FLOAT delivery needs no inferred grid.

Exact source-canonical point/edge ownership produces requirements. The oracle
counts original surface-face incidence after canonical welding, excluding only
triangles with fewer than three keys. A crack or directed border-winding witness
is exempt only when all its supports have incidence one; incidence two or more,
nonmanifold supports, shared-point requirements and exact coincident orientation
remain strict. Open boundaries, candidate open-edge changes, and exempt
crack/winding events are deduplicated and reported separately. A crack event
needs a positive certified sampled width witness; source-open edges or missing
open-edge keys alone are not counted as cracks. An unmeasured candidate is not
evidence of a zero-width gap.

Width diagnostics sample complete corresponding boundary traces with bounded
work and memory. A truncated spatial traversal reports a certified sampled
Hausdorff lower bound, never a nearest-distance overestimate. Ambiguous trace
correspondence and unresolved queries are explicit counts, never silent zero.
Width precision does not change exemption eligibility or exact seam acceptance.

`spatial.py` keeps archive-sized source, canonical incidence, requirements and
geometry indexes on disk. Queries, group visits and representative searches have
charges and caps. Mixed-depth pairs are enumerated without an archive-sized
cross-product. Full 2048 CPU/RSS and pair counts remain S8 measurements.

## Reproduction and evidence

All builds, tests and bakes use the required scope:

```sh
systemd-run --user --scope -q -p MemoryMax=10G -p MemorySwapMax=0 -- chrt -i 0 nice -n 19 COMMAND
```

Set `CARGO_TARGET_DIR=/home/bend/.cache/rt-wt/target-hlod-s5`,
`CARGO_BUILD_JOBS=3`, and both dev/test debug settings to zero for Cargo.
`cargo test --test hlod_acceptance` regenerates the permanent corpus and checks
every generated family against its digest in `controls-digest.json`; positives assert that
`hlodFeatureIds` and continuity actually ran. Rust/CLI verdicts must agree.
Python 3 and Node are needed. `python3 controls.py SCRATCH` runs the independent
named controls without Rust. The final native and real-archive receipts follow
in the execution record below.

From the repository root, inside that scope:

```sh
python3 bench/architecture_audit/mesh_hlod/s5/source_manifest.py INPUT.glb SOURCE.sqlite
python3 bench/architecture_audit/mesh_hlod/s5/oracle.py OUTPUT.3tz SOURCE.sqlite --scratch SCRATCH --profile hlod
python3 bench/architecture_audit/mesh_hlod/s5/continuity.py OUTPUT.3tz
```

H17 archives are outside S5 acceptance. S6 removes the H17/H27 producer without
backward compatibility. Historical H17 diagnostic results remain evidence of
legacy depth-mix gaps; no legacy admission path or interface exemption makes
them pass. Common C1/C2, identity and continuity checks still apply to archives.

Superseded checkpoint receipts (`33ce60c5`, `9bc1760d`, the exact-seam and R11
records) moved to `~/.cache/rusty-tiles-evidence/tree-trim-s5-20261025/README-history-f1e841c0.md`
with their immutable manifests unchanged.

### Upward-omission revision (S1 `98a1bec5`)

S1 `756283e2` then `98a1bec5` are merged; S2 sampling and S3 appearance are
kept, with two test-initializer seams. The above-cap positive is replaced by an
8,192 positive and 8,193 coarse/leaf `triangleCap` negatives. Analytic controls
whose root is content-less now give it the empty-cell floor (`refinable`);
only those `tileset.json` hashes changed. `omission_controls.py` adds 12
explicit/external/implicit cases with oracle/native parity, including R9's
children-error omitted cells, and producer-report agreement/mismatch.
Integration passes 121 cases and 772 hashes. Round 9's S1 `756283e2` cap12/cap30
archives fail `emptyCellRefinement`; their corrected variants pass, with derived
omission counts equal to S1's (21/16/[1,4,16], 17/12/[1,4,12]).

On develop `652f1800` (S1/S2/S3 merged) own tex3/crop3 bakes publish. Both
oracles pass every check except `representativeIdentity`: 628 of 499,554 coarse
faces; all sampled cases are distance ties between source primitives (equal or
1 ulp), broken differently from D4's source order. S1's omission report equals
the derived counts. Native validation exceeds the scaled `totalPayloadElements`
budget (about 4.2e9 needed vs 0.92e9 on tex3); with raised caps every native
check passes, agreeing with the oracle's shared/open-edge counts.

Superseded checkpoint receipts (`33ce60c5`, `9bc1760d`, the exact-seam and R11
records) moved to `~/.cache/rusty-tiles-evidence/tree-trim-s5-20261025/README-history-f1e841c0.md`
with their immutable manifests unchanged.

### Upward-omission revision (S1 `98a1bec5`)

S1 `756283e2` then `98a1bec5` are merged; S2 sampling and S3 appearance are
kept, with two test-initializer seams. The above-cap positive is replaced by an
8,192 positive and 8,193 coarse/leaf `triangleCap` negatives. Analytic controls
whose root is content-less now give it the empty-cell floor (`refinable`);
only those `tileset.json` hashes changed. `omission_controls.py` adds 12
explicit/external/implicit cases with oracle/native parity, including R9's
children-error omitted cells, and producer-report agreement/mismatch.
Integration passes 121 cases and 772 hashes. Round 9's S1 `756283e2` cap12/cap30
archives fail `emptyCellRefinement`; their corrected variants pass, with derived
omission counts equal to S1's (21/16/[1,4,16], 17/12/[1,4,12]).

Own tex3/crop3 bakes at this integration refuse before publication: S1's walk
ledger finds S2's unadmitted scratch (314,783,887 > 216,795,519 and
264,788,907 > 166,800,539 bytes). No archive, oracle or native verdict exists
for them; S2/S3 must admit their scratch.
D11 border certificates, E/G inequalities, terminal locks, appearance continuity
and topology-loss audits remain deferred to S1b's implementation.
