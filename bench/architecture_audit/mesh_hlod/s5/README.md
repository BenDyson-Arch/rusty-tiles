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

D11 coarse drops (2026-10-28): `drops.py` recounts each coarse payload's dropped
child components from published keys, against its `droppedComponents` declaration
and its geometricError (exact distances); `drop_controls.py` pins an honest drop and
its undeclared, understated and border-touching refusals for both validators.

D11 border relief (A1, 2026-10-30): `relief.py` re-derives every declared relief step
from source incidence alone, in IEEE double like the producer:
- the shared border graph per depth;
- live junctions (degree, owner changes, terminal leaves, source-open ends, face retention);
- the walk order, cycle anchors and open-chain witnesses;
- isolated ears on relieved chains;
- the corridor greedy;
- each coarse cell's item count and feasibility trigger and request at `--triangle-cap`
  (85% to 75%, steps ≤ 8),
  the owner maximum and the finer-level monotone minimum.

A certificate whose `step` differs is `borderReliefStep`. `borders.py` also requires:
- certified ε within E_d/16 x 2^step;
- G at least 8 x each relieved certificate (`borderReliefFloor`), or 16 x on a collapsed
  single-child chain: the chain's next member holds the same chord as its finer
  displacement;
- each mixed-depth gap within min(max(E, floor), G)/8 of the coarser tile, where floor is
  its `reliefFloor`. An unrelieved coarser tile keeps E/8 beside a relieved deeper chord.

The rebuild maps each delivered tile to the frozen cells of its chain:
- a coarse record covers its contract depth through its `encodedDepth`;
- an omitted cell covers its omitted records' depths over its leaf interval;
- a leaf lies one level below its parent's chain.

A link placeholder, a record without `encodedDepth`, or a chain that does not start one
level below its parent's leaves the steps in `uninspected`. `border_controls.py`
pins an honest relieved arc at cap 4 and its wrong-step, understated-step and unfloored
mutants. It also pins an honest relieved collapsed chain (re-derived at both levels) and
its 8 x ε mutant, and a relieved deeper chord beside an exact coarser tile at a gap
between E/8 and G/8. The 3e880cb3 oracle accepted both of those mutants.

Feasibility-only R gate and bake receipts (2026-11-03) are external at
`~/.cache/rusty-tiles-evidence/hlod-border-relief-20261030/r5-astra/feasibility-cf43/execution-manifest.json`,
SHA-256 `c7522a15bb643025e7875b083e4236dd91b9d77c8dbb51686f7beba3a7a729f5`.
They bind executed source `6c3ac3a9291af1f3852f29edeedf318dc4b5b7c2` against
develop `cf43aef8b3036a8ac7f5caee47db4ab3b756aca6`, including #317 and #318.
The evidence was generated externally and never held in git. The adjacent
`r5-astra/FINAL.md` records the finite gates, bake and CPU comparisons, remaining
quality limits, both raw floor columns and the final A1 text.

Review 3 correction: those library/integration runs used `--release`, which hid
overflow in the no-relief cap sentinel's percentage multiplication. Both comparisons
now widen to `u128`; the unchanged controls and full default-profile library suite
pass with overflow checks (1,250 passed, 24 ignored). New evidence is external at
`~/.cache/rusty-tiles-evidence/hlod-border-relief-20261030/r5-astra/review3-fix/manifest.json`,
SHA-256 `fe60c91dc69b557475ebba1af91b1b7bf2ac9acf3fd2cc2bb4fd1cfc073a25f1`.
It binds the source patch to parent `c7cbad68`; it was never held in git. The production
cap's comparisons are equivalent, so the city sweep and bake receipts remain unchanged.

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
A coarse edge with two equal-direction faces is refused unless each matches a
descendant source face of that direction on its own side (a nonmanifold spine; see
D11 below), so a page reversed onto the spine (`book-reversed-face`) is refused
natively and by the oracle's coincident-face guard. Native validation still does
not prove coarse face orientation away from shared edges.
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
nearest coarse representatives with exact rational point-triangle distances
(integer region tests, no float Gram determinant; r12 slivers stay controls). A
named primitive other than the source-order nearest is accepted only within
tau = 16 * 2^-52 * (|centroid| + descendant source diagonal) of the exact
minimum, an allowance for the producer's f64 arithmetic only (D4 ties); the
largest excess is reported and exact-tie, within/beyond-tau controls pin it. Analytic manifests
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
A leaf's incident depth is its frozen depth, not its delivered one. A leaf below a
collapsed single-child chain is quantized on the grid of its depth in the frozen tree.
`relief.leaf_depths` derives that depth from the audit sidecar before the D11 audit
(each coarse record's depth through its `encodedDepth`). Delivered depths stand when no
sidecar is given or the labels do not fit; the D11 audit then refuses or leaves relief
uninspected. `border_controls.py` `chain-grid` pins it: a chain leaf and a genuine leaf
share a changed point at one delivered depth. The 20107b1d oracle refused that at
`sourceCanonical`. The unlabelled mutant still refuses.

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
Appearance continuity remains deferred; D11 borders follow below.

## D11 scalable borders (S1b)

D11 lets an ancestor contract its shared leaf border: a delivered chord joins
two retained exact source vertices and stands for the source arc between them,
parameterized by original leaf-edge ordinal. Both validators derive each coarse
tile's border from decoded leaf incidence (edges/points with an owner leaf inside
and one outside), split it at retained vertices and require: junctions and
point-only contacts exact, every arc delivered as exactly one chord, no chord
shared by two arcs, every arc edge covered, and chord directions witnessed on
every edge of the arc. Directed incidences are counted, never reduced to a mask
(S1b review 1): a coarse edge used n >= 2 times in one direction must have each of
those faces matched, one to one, to a descendant source face running the same way
on every source edge it stands for, with its third corner on the same side of the
edge (exact acute wedge): a count of unrelated pages licenses nothing (review 2).
Without that correspondence it keeps the other direction and its same-direction
faces lie on distinct sides: the exact sign of |d|^2 (u.w) - (u.d)(w.d) (d the
edge, u and w towards the third corners) is never positive, so no acute wedge, and
no face is collinear with the edge. A simplified book spine's perpendicular pages
(0) and a collapsed plate's opposite sheets (negative) pass; a lone reversed face
(2/0), a fold re-flipped onto the border, a duplicate or a leaning flap is refused,
balanced or not. The 90 degree boundary is a policy: an acute nonmanifold junction
keeps its source correspondence or is refused. Coincident coarse faces (n on one
corner set, either orientation) need, at every corner, a descendant coincident
source set of at least n faces (native: an index of every leaf face; oracle:
verified source faces, all corners in one connected twin region). Exact (epsilon = 0) archives reduce to the D1
exact-seam census, except that collinear vertices may contract at zero
displacement (an exact image).

Native (`src/validate/hlod_delivery/borders.rs`, archive only) bounds each
contracted vertex's displacement from its chord by the tile's G/16
(`borderDisplacement`); the residual (v - a) - (b - a) i/m is rounded outward in
proportion to those differences, so a rigid translation changes nothing. The archive carries no logical depth (chains
are spliced, placeholders transparent), so pairwise rules use only what the tree
proves: siblings share one depth and must deliver identical images
(`sameDepthBorderGap`); a tile whose cell parent is an ancestor of a disjoint tile
is no deeper than it, so a leaf there locks it exactly (`terminalBorderLock`) and a
coarse tile needs d_X + d_Y <= G_X/8 (`mixedDepthBorderGap`). G/16 per tile bounds
every other co-rendered pair by (G_X + G_Y)/16. The census reports
`contractedBorderArcs`/`contractedBorderVertices`. Native cannot verify source
arcs, certificates, E_d, same-depth cousins, G_coarse/8 where the finer tile has
the larger G, terminal contacts the tree does not order, or topology receipts.

The oracle (`borders.py`) re-derives the same arcs from source canonical keys with
exact Fraction displacement. With `--border-audit borders.jsonl` (an index only)
it joins records to payloads by SHA-256 and leaf interval and validates every depth
label (coarse and omitted records) against the tree: cell siblings equal, strictly
deeper below a labelled cell (`borderDepth`). It recomputes E_d from the captured
source extent (`borderScale`, G >= E_d), then checks, in this order: same-depth
equality and |Q_d - Q_(d+k)| <= min(E_d, G)/8 at every shared source breakpoint;
terminal contacts, where a leaf not proven deeper than the tile (exact depth from
a labelled sibling, else the lower bound below its labelled cell ancestor) forces
an exact contact and a proven one must also be claimed; each certificate (hinted
chain re-walked, ordinals, endpoints, exact displacement <= epsilon <= E_d/16, one
certificate per arc, interface items); then topology against the parent job's
decoded input. Dropped components must be interior and below max(E, child G); the
receipt may not under-report their measured loss to the delivered proxy, and G
must cover 1.05 times it. Input and output boundary edges (output vertices mapped
to input keys within codec tolerance) are clustered by shared vertices; removed
and added cycles are counted separately on the common boundary graph after
suppressing unshared degree-two vertices (S1b's own definition), and the receipt
may exceed but never undercount them. Every cluster whose boundary edges changed
is measured on its conservative changed-face domain (`hlod-s1b-borders.md`):
faces present on one side only, reachable from the cluster's boundary vertices
through boundary, new or lost vertices, never through an unchanged interior (one
vertex-to-face index per tile; each vertex expanded once per flood, charged). Every
grid control on the changed coarse faces is measured exactly against source
(closures) and every control on the lost input faces against the proxy (openings),
plus each removed or added cycle's centroid projected across the change. No feature
test filters the grid (review 3). A control is skipped only when one target face,
proposed by a float search, is at an exact squared distance no greater than the
running exact maximum, so it cannot raise it; floats never decide (review 4).
Losses must fit E where cycles changed and G where a boundary was only simplified;
a change without a witness is refused. Distances come from a complete
box-hierarchy nearest search with certified lower bounds and exact
Fractions. All topology losses are sampled witnesses under D2, like G itself: S1b's
quarter-grid controls plus this checker's own deterministic barycentric level-8
grid on every affected face. They bound a continuous maximum from below only.
`contractedVertices` (input vertices on contracted arcs) and the complete omission
frontier (with multiplicity) are compared exactly; logical depths must be integers
0..63; history-dependent counters are listed as uninspected. Without a sidecar, any contraction is refused
(`borderAudit`). `--producer-report` compares the run report's sidecar hash and
bytes.

`border_controls.py` builds a literal two-cell fixture with an honest sidecar: 13
positives and 46 named mutants, including every review-1, -2 and -3 reproducer (geometry: both validators where the archive shows
it; sidecar lies and topology receipts: oracle, while native passes). Review 4's
kernel witnesses run first: the two-triangle grid whose float early-out
under-reported the exact maximum, and 1,024- and 8,192-face fans whose domain
flood must stay linear and charged. Also run is a contracted bump sheet, baked by
the real producer, which must pass both validators.


The N3 static-filter probes are `filter_controls.py` (exact rational enclosures,
including 32 subdivisions) and `certificate_probe.py` (published-byte corpus,
source-exact Grid/certificate timing, and `micro` mode for box/filter/interval
paths). Run timing through the shared measure.py slot locks. The harness replaces
only hash-map/error/effort plumbing; it does not time decode or grid construction.
Bulk evidence is external at
`~/.cache/rusty-tiles-evidence/n3-certificate-20261102/` (generated, never tracked).
The merged-dup-2000-40 corpus SHA-256 is
`d7807142a7f5a4d37c9bc673bc60b6905f264c9c6313778a20ef0162f51bb34a`, derived from
archive SHA-256 `cad47aa33c007f6437a5b2b59bd22b044673963026e96b0f8174ee2532b824a8`.
Baseline certificate source is develop `ef7db6f372ca60c8c02afc91ce3dd320c73dbfa9`.
See the external FINAL.md for exact executed source/artifact identities and limits.
The native `cached_faces_preserve_verdicts_at_the_effort_limit` control compares
cold/hit/miss caches at nine remaining budgets for both sampled and continuous
checks. It includes out-of-cell and in-cell misses, and a later certifier requiring
more point tests than the first certifier. Cache reuse preserves the original
cell-list order; every outcome and effort/certificate counter must match.
`cache_controls.py OUT` replays that native control against the reviewed priority
order and an in-cell-only priority mutant; both must fail. Like the timing
harness, it substitutes only hash-map/error/effort plumbing.
Review-fix runs are in `~/.cache/rusty-tiles-evidence/n3-cert-317-fix1-20261102/`.

N3 centroid-box pruning (2026-11-03) retains both existing certificate predicates.
`filter_controls.py OUT` also checks 8,496 box-separation decisions against exact
rational distances. Native controls check strict outward box boundaries, interval
boxes that overlap despite a far midpoint, and the three-unit completion boundary
with cold/hit/miss coordinate caches. `cache_controls.py` retains the priority
mutants. Before/after weighted #309 measurements, per-height receipts and exact
input/source hashes are in `~/.cache/rusty-tiles-evidence/n3-certificate-2-20261103/`;
these are measurements on an isolated accounting overlay, not production weights
or approval of R. The existing synthetic archives are read without rebaking.
The generated evidence was never tracked (there is no former holding commit).
The source baseline is develop `cf43aef8b3036a8ac7f5caee47db4ab3b756aca6`;
measurement receipts are bound by SHA-256:

- `measure-10.json`: `b88548220a4f132eee4ee56cc2a1f64e2b202b4fa04703b0164b247b14bde27f`
- `measure-17.json`: `8e6fd6ab675997ffa6cd8e9be61af3bb0c1220b81ce716e5589b33bceffd83b8`

The 2026-11-03 owner decision permits deterministic pruning to lower the budget
needed for acceptance; it never permits cache-dependent work or verdicts.
`cache_controls.py OUT` also reads `pruning_control.rs` and reconstructs the
unpruned scan by removing only the pruning branch from the extracted source.
The differential test sweeps 19 geometries, budgets through each old completion
threshold, and cold/hit/miss caches. Every old accept must remain an accept,
all charged work must be nonincreasing, and strict budget improvements must
occur. A mutant that refuses on separation must fail this test. The native
three-unit control retains actual Budget/Error wiring; neither the old scan
nor a pruning mode switch is added to production.

Review-fix CPU evidence is in
`~/.cache/rusty-tiles-evidence/n3-cert2-319-fix1-20261103/`. The scoped
`dupscale.sh` regeneration produced byte-identical ci-32000 and dup-16000 archives,
retained there read-only with producer/generator provenance. Four interleaved
slot-1 P-core pairs compare actual develop `cf43aef8` with implementation
`b5de7154`, using identical phase instrumentation and checking full report
identity. Mean whole CPU is 83.768 → 81.113 s and 36.577 → 34.787 s, respectively.
ci-32000's remainder varies in both directions; its mean change is +0.898 s,
not a demonstrated zero-cost difference. Three additional pairs replay the exact
historical #309-overlay binaries: remainder CPU is 35.236 → 35.222 s. The prior
single-pair +8.081 s remainder increase was not reproduced. These are bounded
shared-host observations, not a universal speed guarantee or a new choice of R.
The external evidence was never tracked; SHA-256 bindings are:

- `paired-summary.json`: `d7686983f146bc9037f158af50d9820746c349bb80df1e2a080586c8a864b501`
- `overlay-summary.json`: `5149887db0803c08d2ae51db128017bee26b07451efb4e11ed069cafdd2ccb7c`

The #309 work-model integration updates the retained source-extracted harness to
read the selected source's cost constants and supply its error variants. The
current native control is
`cached_faces_keep_certificate_work_and_honest_point_queries`: certificate
coordinate-cache state changes neither verdicts nor charges; point-query caching
may change work, but every executed test is charged and repeated queries with the
same cache state agree. The priority mutants therefore alter only the certificate
scan. Each invocation asserts that one named test actually ran, preventing a
renamed test from silently running zero controls. The pruning differential uses
the certificate cache and sweeps through the weighted completion thresholds;
all old accepts remain accepts and charged work is nonincreasing. Its 19-geometry
corpus and all three rejecting mutants are retained. This is harness maintenance,
not a production policy change; the native unit tests own actual Budget/Error
wiring. The passing current-source runs and earlier harness compilation failure
are retained under `~/.cache/rusty-tiles-evidence/bypass-recovery-20261103/s5/`.
- `inputs.json`: `2dcef14b9487bf0f45fb764278600058f2a9c9bc12038bcd97bd4667ba985b02`

The current harness receipt is `s5/cache309-receipt.json`, SHA-256
`b53db78638c91685fa870e1374edc1a771d52625a986e5c6adca6ba57b7efa67`. It records source, generated-kernel, binary and log hashes.
The evidence was created externally and never held in git; this commit first
records its pointer.
