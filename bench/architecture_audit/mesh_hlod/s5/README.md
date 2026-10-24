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
valid, as are S1's repeated coarse-content cells. An empty ADD override cannot
clear a barrier owned by a renderable REPLACE ancestor.

The native census externally groups canonical point keys (12 bytes) and
unordered endpoint-pair edge keys (24 bytes), retaining actual per-leaf face
incidences. Canonical keys are decoded common-frame f32 bits, with negative zero
normalized. A triangle with fewer than three distinct keys contributes no
interface items. No triangle geometry soup, chord inference, distance matching
or T-junction obligations are used. Shared points and edges must be retained
exactly by every ancestor with an outside owner; required edge directions are
checked independently. Closed, locally balanced and nonmanifold items remain in
the incidence relation. Three distinct collinear keys still contribute items.

The separate original-ID check retains conservative full-width vertex ranges
and bounded per-payload alias replay, including disjoint boxes and repeated
references. Every shared original key must retain its exact placed f64 position;
f32 seam canonicalization cannot weaken that identity relation. Selected rereads,
decoding, key/map work, group members, owner walks and SQLite VM work keep their
shared charges; dense work refuses rather than silently truncating.

`interfaceCensus` reports decoded-leaf `sharedPoints`, `sharedSegments`,
`sourceOpenEdges` and `sourceOpenEdgesWithSharedEndpoints`. A source-open edge has
exactly one incident surface face globally; its two endpoints are shared when
each occurs in a different leaf. Incidence counts faces, never net orientation
or owner count. The report names its decoded-leaf basis; authenticating source
canonical positions and measuring exempt crack widths require the source oracle.
It also reports nonleaf content references above 8,192 triangles and their maximum
triangle count, without rejecting them. The coarse primitive metadata row limit
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

`oracle.py` checks source leaf multiplicity/IDs/geometry/normals, materials and
nearest coarse representatives, with D4 ties in source order. Analytic manifests
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
all generated members against `controls-sha256.json`; positives assert that
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

The earlier S5 receipts at `33ce60c5` are historical and superseded by this
review revision. Their cache remains
`/home/bend/.cache/rusty-tiles-evidence/hlod-s5-20261024/`, bound by execution
manifest `afd9e6fa265fc9c2e1659186304a9f22bce41d6a6436b77196435a9f77a3cb72`
and review manifest `6f94c1c7f748799bcc273f8fddc1a178f2b6e0670ee3596882e0c19247034b13`.
The old retained tex3's eight continuity failures remain reproducible; fresh
latest-S1 output is measured separately. No browser or 2048 bake runs in S5.

### Reviewed checkpoint at `9bc1760d`

The R9 closed-leaf identity correction is committed at `0f5b85fe`: conservative
full-width original-ID intervals select exact alias replay independently of
decoded bounds. R10 separately approved that identity/resource correction at
`9bc1760de300bbdfd04de627f67696d917220086`; it did not approve producer output,
final cost or the pending D1 revision.

The validator suite passes 174 tests with two helper tests intentionally ignored.
The permanent acceptance test passes 89 cases and 613 fixture hashes, including
closed/nonmanifold retained and lost edges, separated shared originals and
disjoint boxes. Those identity mutants fail native `sourceIdentity` and the
source oracle's `leafGeometry`. Scoped all-target lint passes with the inherited
producer `too_many_arguments` exception; S2 owns its removal during rebase.

S1 `644fc407` was merged without changing producer logic beyond conflict
integration. The fresh tex3 bake refused before publication: 314,783,887 scratch
bytes exceeded the declared 230,338,145-byte stage bound (1.60 GiB RSS, 60 CPU-s).
The coordinator stopped this superseded campaign; crop3 never started and no
new archive/oracle verdict exists. S1 owns separate component declarations in
the rework. The 10 GB cap is unchanged. Historical dc9085f8 tex3/crop3 oracles
both found the independently confirmed tile-28 exact-coincident reversal;
their predicate remains strict. Final bake/oracle/cost follow the stack freeze.

Review, commands, test logs, pinned artifacts and refusal receipts are retained
in `/home/bend/.cache/rusty-tiles-evidence/hlod-s5-review-20261024/`.
Immutable checkpoint manifest `sha256-9bc1760d.json` has SHA256
`0617b2f4cec6f3c44b03275a4379a82b95de0d42a657c265499055da60720d51`.

### Exact-seam implementation checkpoint

The permanent integration passes 107 archive/profile cases and 679 fixture
hashes: 56 base, nine lattice, nine winding, 15 native scene/frame, ten
nonmanifold/original-alias and eight exact-key cases. The separate width suite
passes 12 controls and 112 exact rational comparisons. The 8,193-face positive
checks above-cap reporting without rejection. The welded detour has opposite
source incidences so its missing exact edge reaches `interfaceSegments` in both
validators; the former T-only detours are counted source-open positives.

The broader validator run passes 173 tests with two helpers ignored; the final
23 native tests include three additional direction-mask cases. C1/C2/I1's 20
external tests and scoped all-target lint pass, retaining only the inherited
producer argument-count exception. These executions establish the recorded
control domain, not real-output acceptance. A fresh full R11 review precedes
the S1 `055be853` integration and own tex3/crop3 campaign; final cost waits for
the stack freeze. Source-original census and width measurements require that
source-aware campaign, not producer reports.

Receipts are retained under the same evidence cache's `exact-d1/` directory;
its `sha256.json` has SHA256
`43aae428cd257339d266fb27b0a4ed4d61d29de9ba31ac262ee2ceeef6705350`.

R11 reviewed `284c6391` and reproduced one P2: the surviving H17 profile could
bypass representative identity. The relaxation is removed; HLOD checks are
unconditional and unsupported API/CLI profiles refuse. The 52 applicable base
cases pass, including the wrong representative, plus H17/unknown API and CLI
refusals. All 503 base fixture hashes remain unchanged. The exact R11 witness
now fails `representativeIdentity` in HLOD mode and cannot return acceptance
under either unsupported profile. This correction has no fresh nonauthor review
yet; the owner requested handoff without another Codex review.

R11's source/resource/width findings and correction receipts are cached under
`r11-profile-fix/`, with immutable manifest `sha256-r11-profile-fix.json`, SHA256
`ae5e81bcd67fcf4a1314f89ef1f14e3e1bcec32157943decee5ab7ad6fcd6b2f`.
The hard 8,192 limit and upward-omission/frontier checks remain pending. The
above-cap positive described above is superseded, not current-contract evidence.
Neither `055be853` nor the withdrawn `8074b7db` clone-frontier head was merged.
The next S1 head must publish the top-connected empty cap before integration.
D11 border certificates, E/G inequalities, terminal locks, appearance continuity
and topology-loss audits remain deferred to S1b's implementation.
