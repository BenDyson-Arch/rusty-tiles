# S1b: scalable shared borders

Governing contract: [D11 and S1b](hlod-040-decision.md#d11--scalable-borders-owner-decision-2026-10-24).
Implementation and acceptance are in progress; this note is not a passed gate.

Retain exact leaves, frozen ownership addresses, exact decoded-position incidence,
D4 representative identities, S2 source measurements and upward omission. Replace
permanent leaf-resolution coarse locks with shared level contracts. Remove any
superseded implementation rather than retaining compatibility paths.

Every contract must identify its source arc, every incident owner, common decoded
junctions, ordered subarcs and a displacement certificate against the leaf-decoded
arc. With depth increasing toward leaves, the predetermined scale ladder is
nonincreasing: epsilon_d <= E_d/16 and published G >= max(E_d, measured error,
child G). Certificates include codec displacement and all parameter breakpoints.
Same-depth owners use identical segments. Exact terminal-leaf contacts are pinned.
Unwelded source defects and T-junctions are counted, never tolerance-matched.

Parent inputs remain at most eight capped children. Border reconstruction precedes
interior simplification, S2 measurement and S3 charting. Permitted interior topology
loss is measured and reported; anything touching a border stays protected. Stalled
interiors must be remeshed before reporting the capped-frontier last resort. Omitted
cells hold no geometry and have only omitted ancestors. Delivery must reconcile
actual omitted cells with the producer's total and per-depth counts.

Graph extraction, ordered source-arc storage, shortcut construction and remeshing
need explicit bounded-memory and total-work arguments. Scratch admission precedes
writes, including random-access state and spill. Oversized incidence owner groups
must stream; no owner-pair enumeration or source-sized parent merge is acceptable.
These remain implementation proof obligations, not assumptions supplied by a scale
benchmark. Arithmetic fixtures cover 500M/billion inputs without baking them.

Acceptance uses exact-source receipts, adversarial geometry and texture fixtures,
decoded complete seam censuses, leaf-byte/ID equality and the existing resource
ledgers. S5's independent validator and oracle remain separately owned. The final
PR must list the contract and frontier changes its checks need to follow.

Local evidence: `~/.cache/rusty-tiles-evidence/hlod-s1b-20261026/`. Re-measure
`74580977` (the S5/S6 integration, byte-identical to `652f1800`) and the final
implementation on the same lane and cameras. Per-view byte
non-regression uses each dataset's restacked S3 baseline (tex3: 0.378/0.770/3.599 MB;
crop3: 0.418/0.849/3.881 MB), exact bytes and complete dependency accounting.
The mesh-1e7 sanity bake requires coordinator clearance; its command and receipts
will be prepared, while all local bakes remain tex3/crop3 only.

The cap-fit proof must distinguish general bounded work from a constructive surface
domain. A stalled, consistently oriented height-field patch with simple projected
boundary loops can be reconstructed by constrained Delaunay triangulation, using
only its boundary and mandatory interface vertices/edges. Flooding across boundary
constraints preserves all holes. With B boundary vertices, I mandatory interior
vertices and b loops, T = 2I + B + 2b - 4 per connected patch; source interior
sampling does not appear. Projection collisions, folds, nonmanifold edges and
crossing constraints are outside this constructive domain. They still enter bounded
cleanup/clustering and the explicitly reported last-resort frontier. This replaces
an unproved claim that vertex clustering alone guarantees cap fit. No parent input
or output cap is relaxed. S2 measures the reconstructed surface before S3 runs.

## Representation and bounded work

`interfaces` externally groups exact decoded keys. Its resident owner prefix is
fixed; a larger owner run spills, preserving every owner. `borders::graph` stores
vertices, edges, full sorted owner runs, two adjacency links per edge, and a
fixed-height leaf ancestor table in charged random-access files. Source-open edge
endpoints pin chain contacts without inventing T-junction edges. A vertex is
removable only with two active edges and equal coarsened owner sets. High valence
walks each adjacency once; full owner comparisons happen only at degree two.
Consequently the total compared owner rows per level are bounded by incidence,
not an owner cross-product. Chord emission binary-skips each owner's DFS leaf
interval, so it does not rescan a large leaf-owner run for every chord; counted
reads test the adversarial fan-plus-zigzag family. Binary source-key lookup and sorting give O(n log n)
work; fixed-height level walks add O(nh). Fixed page caches bound resident I/O
state independently of source size.
Each graph store has 128 fixed 16 KiB pages; the memory plan charges all
overlapping stores through the same cache constant.
S2's per-job query charge is 16 MiB, allowing 384 cached fine blocks when its
upper hierarchy is fully resident (64 on the paged path). This fixed cache
budget changes neither search order nor the B=1,024 nearest-search limit.

Ordered chains use the original edge ordinal as common parameter. Coordinate
corridors select shortcuts in linear work, retaining a nested subset of finer
endpoints. A second direct visit to each source breakpoint computes outward
error including floating-point evaluation. Cycles retain three anchors; open
chains retain an interior witness when one exists, so retriangulation cannot
replace an entire boundary path by a single closing edge. Junctions and terminal
leaf contacts are exact. E_d is the captured root's maximum axis extent divided
by 1024 and by 2^d. Every endpoint uses the common leaf-canonical f32 bits; the
encoder verifies those bits. No nearest-curve sampling supplies a certificate.

`geometry` merges at most eight immediate capped children. It reconstructs
contracted borders, simplifies, and checks actual topology. Border mapping only
queries the union of immediate child interface keys: coarsening merges
owners, and nested canonical endpoints ensure every live parent border was a child
border. Interior vertices cannot acquire a new external owner. This avoids disk
searches for interior vertices without changing the contract. Dense local remaps
preserve first-use vertex order; required-edge checking removes found witnesses
from the required set instead of materializing all interior edges.

A protected boundary
component is compared as a multigraph with degree-two vertices suppressed;
border contacts and branch vertices remain anchors. A bounded retry pins protected boundary vertices and component supports while
allowing their adjacent faces to retriangulate. Retaining whole incident rings
wasted interior triangles and increased measured G in the tex3/crop3 probe.
Interior duplicate removal/component cleanup, constructive projected remeshing
and a fixed six-round clustering pass precede a reported frontier. These bounded
jobs cost P*F(8C); the constructive domain above has a cap-fit argument, while
`remesh_stalled` outside that domain records failure of the bounded remesher,
not a mathematical proof that no alternative triangulation exists. That proof
limit remains subject to independent acceptance.
Topology eligibility computes diameter/contact bounds only for disappeared
components. Its boundary graph drops interior edges once before comparing paths
and cycles; closure face sets are built only when a closure is detected. These
reductions preserve the same topology decisions and canonical output.

The ready window counts resident/spilled siblings, pending jobs and in-flight
outputs; its key bound is 8(h+1)+32. A spill is admitted before file creation and
reloads at most eight children inside the job charge. Completed-file accounting
uses atomic addition because worker loads can release older files concurrently.
Spill statistics are evidence; geometry and identity cannot depend on spilling.
The forced-spill test compares actual published bytes across these paths.

## Durable audit format (v1)

The internal `RT_HLOD_AUDIT_DIR` enables an exclusive sidecar outside the archive.
`borders.jsonl` is durable beyond stage deletion. The report names its path,
SHA-256 and bytes; each `proxies.jsonl` row has a byte offset and length. Writes
are admitted through ScratchLedger and the output volume before creation; a
tmpfs sidecar also enters the memory plan. Nothing is requested by the client.

Each `kind: tile` line contains `tile` (the final publication row) and `contract`.
The contract identifies source logical `depth`, `scale`, required `interface`
items and `certificates`. Keys are three canonical f32 bit-pattern integers.
Each certificate gives an oriented chain hint (`chain` first source edge,
`origin`), inclusive source endpoint ordinals `first`/`end`, delivered
`endpoints`, outward `epsilon`, and `terminalLock`. One parameter unit is one
original leaf-decoded edge. `terminalVertices` also lists exact terminal point
contacts, including those with no shared edge. Summary `terminalLocks` counts
terminal edges plus distinct terminal vertices per tile; these are typed locks,
not a count of disjoint geometric objects. `interface` includes all live
point-only junctions. These records are producer claims, never independent proof.

A `kind: omitted` line records the logical source/leaf interval, depth, reason,
frontier keys and attempted topology receipt. Reasons are
`contract_lower_bound`, `contraction_stalled`, `remesh_stalled` and
`descendant_omission`. `contraction_stalled` means the original-source-checked
contraction and its bounded recovery proposals all failed; a missing nested
endpoint is an invalid-state error, never this ordinary omission. Summaries per
depth include E, maximum epsilon, topology changes by kind, attempted versus
published changes, terminal locks, omission sources/cells and frontier uses,
including zeros. Topology records include contracted vertices, reconstructed
faces, duplicate faces, dropped components, closed/opened holes, remeshed
vertices/rounds, rejected rounds and constrained remeshes. S2 runs after all
accepted changes. A diameter never floors G. Component diameter is only an
eligibility bound against the existing max(E, child G).

## Independent acceptance boundary

S5 must reconstruct source incidence and source-subarc correspondence from its
source DB and independently decoded delivered geometry. The ordinal hints let
it find a chain cheaply; it must re-derive the chain, complete coverage, actual
edges/junctions, every parameter breakpoint's displacement, terminal contacts,
E/G inequalities, source defects and topology/frontier counts. It must compare
those results with receipts. Same-depth equality is exact; mixed-depth testing
includes every depth difference, not just adjacent levels. Original leaf bytes
and IDs remain exact and separately checked. Single-child payload reuse requires
equal complete decoded contracts; encoded sample depth may differ from published
logical depth, whose E still floors G.

The independent r1 review found real producer folds hidden by orientation flips.
These were not validator semantics. Contraction now rejects inverted snaps,
new area from degenerate faces, and unsupported directed edge fans. Constrained
reconstruction changes only faces incident to contracted vertices, fixes the first
interior ring and preserves the rest of the surface. Projected charts separate
nonmanifold sheets and sharp folds. New zero-area faces are discarded; no arbitrary
component witness invents a repair triangle. Topology comparison starts before
contraction. Interior clustering likewise rejects inversions instead of flipping.

Contraction failure enters sheet reduction, constructive remeshing and clustering
before the frontier. Every recovery contraction uses the original input face and
edge licenses. Constrained contraction has a finite input-sized reconstruction
cap, including frozen faces; publication still applies the payload cap. Required
rows join to their source edges/points in three edge probes per source face,
without a patch-by-row cross-product. Receipts count new oriented faces with
multiplicity, excluding retained faces.

For a simple projected sheet that CDT cannot reduce safely, a final fan proposal
uses an existing local interior source vertex and preserves every boundary edge.
Two oppositely oriented coincident sheets use distinct centers. At most two
nearest eligible centers are checked per panel; positive projected wedges and
exactly one winding establish a nonoverlapping fan. Holes, mandatory interior
edges, incompatible mandatory points and non-star-shaped panels refuse this
proposal. The unchanged original-source, interface and topology checks still
govern publication, followed by S2. Fan construction is O(m log m) for a capped
parent input of m faces. The local Spade CDT kernel has no established worst-case
O(m log m) bound here; total source-size work retains the fixed-cap P*F(8C) term
above, separately from global sorting. Controls cover production-nearest omission
breadth and actual contraction/recovery allocation, not just no-op mappings.

The independent census counts directed incidences, including multiplicity. A
contracted nonmanifold book spine can legitimately carry two equally directed
pages: support must be re-derived on every original source subedge in the tile's
leaf interval. No certificate alone supplies that support. Unsupported same-side
pairs remain failures. S5's separately owned follow-up must retain this check and
adapt original-point requirements to the common D11 contract.

Audit v1 has additive location records. Each `topologyAttempt` has a tile key,
local attempt ordinal, acceptance flag, measured receipt and `hintOnly: true`
locations. Final tile/omission records carry `topologyAttempts`; a tile's
`publishedTopologyAttempt` is its accepted record's byte index. A final omission
may also reference `constructionTopology` for cleanup drops that never reached an
assessed proposal. Each operation has a kind, ordinal and location-bank index.
Drop banks identify individual components; closure/opening banks conservatively
identify affected regions, with full bounds/counts and at most eight actual face
and edge anchors per bank. `truncated` states when anchors are a subset. Coordinates
are candidate/source controls before final output quantization, not proof of
final decoded geometry. `sourceHint` supplies the exact descendant interval.
S5 must re-derive operations from the source and delivered surfaces.

Record serialization is counted before any record bytes are written. A tmpfs
sidecar has a fixed internal 64 MiB budget included in the memory plan and
ScratchLedger; exceeding it refuses with ResourceLimit and never drops evidence.
Ordinary disk uses the conservative logical maximum with actual-growth free-space
checks. The archive and client request graph never contain these records. Attempt records
can arrive in worker completion order. Determinism tests compare every audit fact
and location after removing physical byte-address links, and compare complete
client archives byte-for-byte across job counts, budgets and forced spills.
Public `diagnostics.hlod` uses `omitted_proxies`, `omission_sources`,
`omitted_by_depth` and `borders_by_depth`; shared nested report records retain
camelCase, consistent with existing `appearance_by_depth`/`error_measurement`.

## Topology eligibility revision (2026-10-26)

The owner rejected the whole-boundary diameter floor: a branching boundary's
extent says nothing about a filled hole's displacement. Remove it. Retain exact
border protection and S2's final decoded two-sided measurement. Reject deletion
of any border-connected component or interior component whose bounding diameter
is not strictly below max(E, child G), including implicit meshopt deletions.

A proposed closure is checked before publication against the captured source.
Its measurement domain conservatively includes every changed output face in an
affected surface component and the original affected boundary edges. This avoids
assuming a planar hole or treating planarity as loss. Decoded patch vertices,
edge/interior controls and source-projected boundary controls measure both
directions. For the boundary-to-surface direction, retain unchanged faces of the
same affected component as witnesses too: a branching boundary can contain a
retained loop far from the changed patch. Omitting those witnesses creates an
artificial hole in the query index and can falsely reject a flat-sheet closure.
The added carrier indices are bounded by the parent output; changed patch faces
still receive every patch-to-source control. Use E/1.05 as the admission limit
so S2's margin cannot make an
admitted closure set G. Unsupported/over-budget proposals retain their boundary
through a bounded retry. Interior remeshing may not hide an unchecked closure.

All measurements are bounded by the eight-child input and capped output; source
queries use S2's fixed search budget. Receipts distinguish measured closure and
component-loss controls from diameter eligibility, and report their contribution
above the pre-existing G floor. Final S2 samples every delivered face and includes
these additional controls. The conservative changed-face domain can reject a
closure because another change in that component exceeds E; that is a documented
cap-fit cost, not permission to omit silently or increase E based on the result.

Independent controls: a planar courtyard whose inradius exceeds E stays open;
a small interior closure has nonzero measured loss; identical sub-G components
with/without border contact differ in eligibility; a large interior component
survives. Final tex3/crop3 gates remain open until re-baked with source identities,
per-depth G/operation attribution, complete first-view accounting and CPU receipts.

The topology receipt compares boundary paths after degree-two suppression and
counts cycles lost from, and added to, their common graph separately. A closure
and simultaneous opening cannot cancel in a net cycle count. This is a producer
correspondence rule; S5 must re-derive it. The final S2 proxy strata and centroid
controls also track every face in the conservative closure domain; any over-E
sample rejects the proposal before S3, even if the preliminary fixed controls
missed it. Rejected closure attempts are counted separately from delivered changes.

The first four logical build depths target 3/4 of the payload cap before trying
the full cap; deeper parents retain the full target. This internal budget tuning
leaves the 8,192 hard cap, E ladder, eligibility rules and frontier algorithm
unchanged. It trades measured coarse geometry detail for client payload room;
actual final G and all three first-view totals must still pass the same gates.
The 3c4923eb probe established spare coarse G but larger tex3 textures because
S3 spacing follows G; it did not authorize a byte-growth waiver.

Opened holes are a separate operation. The changed input faces of an affected
surviving component conservatively cover the lost region; fixed controls are
projected onto captured source and measured against final decoded coarse geometry.
The new rim is sampled back to source. Final S2 source strata mapped to changed
reference faces and proxy strata on rim-adjacent faces also enter eligibility.
A budget-exhausted reference correspondence conservatively includes the sample.
Both directions, including the 5% margin, must fit E. Any new boundary component
with an exact border-edge or junction contact is rejected before measurement.
Thus an admitted opening cannot set G; bounded retries keep the surface instead.
Intermediate remeshing cannot hide an unmeasured opening.

Each tile row now includes `attemptedTopology` even when it publishes: it sums
measured proposals including rejected ones and the accepted proposal. The separate
`topology` receipt describes published operations. Both aggregate by depth and
include zeros. `openedHoleToProxy` and `openedHoleBoundaryToSource` are local
measured maxima; `openedHoleGIncrease` is the margined contribution above the
incoming max(E, child G), using the same definition as closures/components.
`closedHoleFinalG`, `openedHoleFinalG`, and `droppedComponentFinalG` give affected
tiles' final published G (zero for absent kinds, max-reduced across tiles).
All regions/controls are bounded by the eight-child input and capped output.
As with closures, conservative region assignment is a cap-fit cost requiring
real byte/G/CPU acceptance, not a continuous source-surface correspondence proof.

A rejected opening first gets a bounded surface-preserving retry: pin every
vertex of original reference faces incident to its new rim and run ordinary
reduction again. This preserves those nondegenerate faces without inventing
a closure patch. Pins grow monotonically for at most three feedback rounds;
all rejected proposals stay in attempted receipts. The fixed cap and full-cap
retry remain unchanged. The 9a5f conservative-control probe reported omission
and failed first-view bytes; it is rejected evidence, not an accepted frontier.

The CPU gate includes full source measurement and appearance work. Smaller
coarse targets reduce bytes but can increase nearest-search work: the 1/2 trial
used 276.26 million search steps against 265.61 million at 3/4. Keep the measured
3/4 target and remove redundant geometry bookkeeping without changing output.
Exact-source paired CPU receipts, including failed trials, remain in the evidence
directory. Full-cap retry and all acceptance gates stay.

Source-support checks precede contraction in both direct and constrained paths.
A new shared chord receives only the directional minimum over a complete,
contiguous path of unique real previous-interface edges. Face licenses are never
expanded by that path certificate. A child cannot create a new edge between shared
contacts or a new face with three shared corners; canonical decode retains input
keys, so merging children cannot manufacture original-source multiplicity.
Changed interior repeated fans must still pass the exact nonacute-side check.
Local free neighbours of two shared contacts are pinned before reduction, avoiding
the broad contact-star protection that exhausted the cap on a real parent.

The independent census checks both repeated directions, including balanced 2/2,
using integer arithmetic independent of the producer's expansion predicate. A
coincident group needs a descendant source group of sufficient multiplicity at
every coarse corner; unrelated source twins and aggregate edge counts give no
license. These edge/face checks do not claim general triangle-interior intersection
detection. Final-source independent review and exact-artifact gates remain required.
