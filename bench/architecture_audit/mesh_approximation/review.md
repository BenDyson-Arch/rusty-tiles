# F1d1 separate nonauthor review

Reviewed production source `0eeb906f7f043eb608a5223e2817703c6836ea4b`
against `AGENTS.md`, the architecture gate, and
`docs/architecture/mesh-approximation-contract.md`. The reviewer authored no
production changes, ran no Cargo builds or Git commands, and treated both source
and existing acceptance drivers as untrusted. Findings below distinguish source
observations, executed defects, corrective execution, and proof limits.

**Conclusion:** no unresolved production finding for the declared F1d1 root
proxy slice after the bounds correction. The inspected representation and pinned
portable/native artifacts meet the bounded contract supported by this evidence.
Coordinator-owned resource measurements, package checks and native lint remain
separate evidence categories; this review does not close #121/#113 or authorize
a release.

## Exact source and artifact binding

The final external `source-artifacts.json` manifest under
`/home/bend/.cache/rusty-tiles-f1d-evidence` was independently checked: all 92
listed production source hashes match the workspace, and all four listed
artifacts match their actual bytes:

| Artifact | SHA256 |
| --- | --- |
| Portable CLI | `2098adf353e3d0c4eddde66b977e7a38514f7de1e3abd379051c35c2fb3d86e6` |
| Native CLI | `81fca6a80c1e64cfe6cec953838eba8cce6b39cadcd45a193e4a1096c8e44965` |
| Release CLI | `930142ab6abc1fa1b8c416a8b07a6051bc62e05064a22bf545d8a3054e8e1bcf` |
| Installed wheel | `48b7a1529acc6e27dbb8a8f13f5fc2600feb7333b0e07a9da4c5ff3a114a7e51` |

The relevant final source hashes include approximation
`42d9b6ca892ad31ba45d0b9911d95464181f7e55bf9294389857ee5a9256fc98`
and partition
`4f2a5f2cd02efc1071e262dd3ba377dc5dcfa428c726ca11a5458e52b50cc594`.
All five compressed/raw receipt hashes and lengths in `storage-index.json`
were checked. Current oracle and browser driver hashes match their receipts;
the five portable and five native retained source/archive hashes match actual
files. Browser archive `31bc76d3b99d18339fcee713ef5ae00229e944ce05a3d7336cc0aa237615304a`
is the exact independently checked viewer archive.

## Executed findings and disposition

1. **Production root bounds defect, corrected.** The pre-correction production
   `8fe9bf6c9382bfbe6aaa32e30cf6e79822d8643e` grid archive
   `fec032e6282688e181e1d451796caae9bce675c4ed8ea6152cba9d64ac78c9a6`
   enclosed stored positions but failed to enclose two complete child boxes.
   Independent exact Fraction endpoint comparison found axis 1 low-side excess
   of `1/4503599627370496` metres for each child. The root union rounded
   `center ± half` inward before building its enclosing box. Final source rounds
   subtraction downward and addition upward before union; zero half axes use
   exact centers. The correction is justified because these intervals contain
   the exact real child endpoints before the existing outward root-box step.
   The reviewer executed the corrective frozen CLI on the same source and
   leaf16/root-limit16/error8 request: archive
   `88b696f00cb5d7ebef77d1c1381df20fec54881f3e5efa0ea81c8511f48d3c56`
   has zero exact child containment failures and passes the complete artifact
   oracle. `review_probes/child-box-before.json` and `child-box-after.json`
   retain both observations. A scalar [-4,0] negative control reproduces the
   old failure; 10,000 independent randomized child-box cases found one old
   failure and zero with the outward correction.
2. **Checker report/component gaps, corrected.** The initial checker accepted
   a region count of 999, an invented textured appearance profile, and a root
   with one face for two disjoint coincident authored components. Independent
   controls reproduced these gaps. The corrected checker rejects all three;
   an unchanged positive remains accepted. A count floor cannot identify which
   component authored an output face when component supports coincide exactly.
   Final evidence states this limit explicitly; source inspection and original
   key component controls establish the implementation invariant.
3. **Checker material omission defect, corrected.** A valid materialless source
   converted successfully on the pinned earlier CLI (32 to 7 faces), but the
   checker crashed with `KeyError: 'materials'`. The checker now handles exact
   omission, includes a materialless positive and rejects an invented explicit
   default material. The reviewer reran the materialless source against final
   production: seven faces, exact ideal face-certificate squared bound 4, and
   complete oracle acceptance. Before/after receipts and final reproduction
   are retained under `review_probes`.
4. **Documentation and reference-domain gaps, corrected.** Initial Python
   documentation claimed schema 5 and no coarse approximation, and the sole
   mixed-magnitude rational example exceeded the admitted ±1e6 position domain.
   Final documentation describes schema 6 and paired proxy arguments; the
   rational controls now include an admitted magnitude-boundary case.

## Source reasoning and independent execution

Witness weights are nonnegative integers with exact sum 2^24. Their scaled
values are exact binary numbers, so an ordinary projection is only a proposal.
The final interval reconstruction encloses every coordinate of an explicit
point in the target simplex. Outward residual endpoints, square, sum and final
square root bound its exact distance. Taking the maximum of the three explicit
corner distances proves coverage of the whole convex source face; exhaustive
minima over target faces and maxima over source faces, in both directions and
per region, then prove the claimed bound. Equality of complete corner supports
can safely certify zero, including degenerate supports. No sampling or optimizer
score supplies acceptance truth.

An independent Python replay compared this arithmetic with exact Fraction
distances for 10,001 deterministic in-domain finite f32 controls: zero
understatements. Its non-outward negative control does understate the exact
distance. This is arithmetic replay and source reasoning, not direct execution
of private Rust functions or a universal floating-point proof from random tests.

Original authored keys survive source decode, including reflected winding.
Connectivity is scoped to each selected primitive instance, and each component
has a separate original-position meshopt input. Candidate indices are checked;
empty proposals and invalid triples fail; no proposal can join two input
components. Final total reduction/face count and checked face-pair admission are
validated before workspace creation. Comparisons stream and checkpoint every
64 pairs. The pair ceiling does not establish total RSS or bound an individual
meshopt call's cancellation latency.

Independent final-CLI reviewer probes demonstrate materialless 32-to-7
reduction, coincident authored-key 16-to-8 reduction, and refusal of a one-face
request for two components before output-parent creation. A separate reviewer
execution on the earlier pinned source retained an independent degenerate point
component while reducing 33 faces to 8; that evidence is supplementary rather
than claimed as a corrective-source execution. The final independent owner
receipts cover five successful fixtures, twelve pre-workspace refusals, and
fourteen sensitive corrupted controls for each artifact. Exact rational
references demonstrate spikes, interior holes, degenerate supports and the
4 m whole-face subdivision limitation at true distance zero.

Root and leaf serialization consumes the same certified decoded f32 positions.
Root region tuples partition the selected originals, including removed faces,
without assigning synthetic source-triangle identities. Material factors and
omissions and optional node label presence retain their separate owners. Root
geometric error is the requested positive budget after certificate admission;
leaf errors are zero; top-level omission covers both the conservative diagonal
with the explicit 1 m routing minimum and root budget. F0 retains one Attempt,
workspace cleanup, chunked writes, archive sealing and publication. Inspected
proxy failure seams and full coordinator test execution support reuse of that
lifecycle; this reviewer did not run new Cargo failure-injection builds.

Artifact-bound Cesium 1.146 observations use unchanged SSE 16 and identity model
matrix: natural coarse traversal shows 44 proxy faces and six exact region-array
queries; fine traversal shows 192 source faces and six exact triangle tuples.
Missing-label controls do not return matching properties. Browser receipts
contain no page errors, failed requests or external requests. The installed
wheel receipt reports 43 API tests, zero failures/errors/skips and empty PATH.
Official glTF validation reports zero errors/warnings for the 50 positive
source/root/leaf entries; the 51st entry is the intentionally corrupted
zero-position-view control with two errors. That validator does not understand
the metadata extensions; independent decoding and public Cesium queries supply
their semantic evidence.

The separately owned release resource receipt is bound to the final release
binary and retained archive. It decodes an 8,192-to-1,023 face reduction and
16,760,832 comparisons (99.90% of admission), and refuses an above-ceiling
request before creating the output parent. Its current run records 14.81 seconds,
25,408 KiB raw wait4 peak RSS and 16,080 KiB sampled procfs peak RSS. Raw wait4
peak can include inherited Python memory before exec; sampled descriptor,
thread and scratch peaks can miss short spikes. At this size the driver verifies
membership/work and independently proves the requested 100 m budget using
the common box diameter; it does not replay the detailed reported 7 m witness
certificate. These measurements establish this invocation, not total-memory,
performance or cancellation-latency guarantees. Native, installed wheel and
full-detail replay receipts were reviewed separately from this resource run.

## Supplemental placed consumer and final integration review

Production remains frozen at `0eeb906`. The additional
`placed_proxy_browser.py` and `placed-browser.json.gz` were separately inspected
and the reviewer reran the complete probe against the frozen portable binary
in a unique temporary directory. The replay produced the identical source
`533a7bd8d306521e026bbb51c1e3a4a7e953f68e59fae9fc7695c426817d7739`
and placed archive
`95ecbbd227ccb52ff7e64d8831b5fbf4e82f730696394dfe7dcb34a9312bd4fc`.
It reproduced six complete coarse membership queries and six exact fine
triangle queries, with natural 44-to-192 face refinement, SSE 16, identity viewer
model matrix and no page errors, failed requests or external requests.

The placement reference derives its frame from the explicitly requested
Brisbane anchor `(153.02, -27.47, 25)`, using independently authored Decimal80
trigonometry, WGS84 ellipsoid constants and ENU basis columns. Its expected
values never come from an emitted matrix. Camera destinations and target points
receive the independently calculated affine transform; camera directions and
up vectors receive only its linear part. The local glTF-to-tile axis conversion
is already applied before these operations. The reference includes every
matrix entry and strict placement report expectations. The executed maximum
basis component error is `1.1863174139341632e-16`; the maximum ECEF origin
component error is `8.845845404077495e-10` metres, under the declared `2e-15`
and `1e-8` limits respectively. An additional reviewer crosscheck using the
alternate two-semiaxis radius formula and ordinary double-precision trigonometry
matches the emitted basis/origin and the independently placed cameras/targets
within `9.313225746154785e-10` metres. This algebraic crosscheck supplements the
stricter Decimal80 reference; it is not a new placement-domain proof.

The missing-placement, wrong-height and swapped-east/north controls modify both
the manifest matrix and report matrix, preserving their self-consistency; all
three fail the independent reference check. The viewer plan override is local
to the Python test process, restored after execution and bound by the placed
driver digest. It changes camera/target inputs only. The unchanged JavaScript
driver still uses public `tileVisible`, `SceneTransforms` and picking APIs; it
sets no private traversal state or viewer placement correction. Missing-label
combinations return zero matching queries. The scope remains one anchor with
default identity orientation and zero offset; no general placed-proxy or exact
world Hausdorff acceptance follows.

All placed receipt driver, binary, source, archive, browser executable, Cesium
JavaScript and Cesium/Playwright package-manifest hashes match actual retained
bytes. The current 17 compressed evidence entries match their raw/stored
identities and lengths; current receipt driver hashes and all 92 production
source hashes match. Coordinator log hashes also match their recorded files.
Small supplemental reviewer replay and algebra receipts are retained under
`review_probes`; the complete primary arrays/tuples remain in the placed owner
receipt. The placed probe digest is
`7c531064477335c6b9a8df93e2b0062ec7ea3c686ac1d450f38721c5f60017b2`.

The final CI addition follows the native artifact and local consumer steps,
exports the native library path, uses a fresh placed-artifact directory and
the provisioned pinned consumer paths, records checkout HEAD provenance and
the actual binary digest, and includes its JSON in the evidence upload. The
reviewer inspected this integration without executing Git or Cargo; a local
probe replay does not claim a completed hosted CI run. The implementation,
architecture index and user documentation describe the finite choice, emitted
budget, exact omissions and remaining approximation gates consistently with
the inspected source. No supplemental blocker or unresolved production finding
was identified.

## #149 active CI checker follow-up

Rust CI run `38027622273`, job `114141722779`, at head
`711e663b6bc388e59e7803043169c802cc5e42c6` reached the active F1b3 resource
driver after Rust tests passed. Its stale schema 5 / F1c2 profile guard rejected
three successful conversions. The corresponding F1c1 guard had the same stale
expectation, although that driver was not reached in the failed job. Earlier
local verification did not replay these two complete active CI resource drivers.
The retained before receipt reproduces the PBR checker failure with a successful
conversion from the frozen native binary; this is a checker defect, not a
production conversion failure.

Both inspectors now require schema 6, `f1d1-root-proxy-gltf-v1`, and exactly
`{"kind":"full_detail"}`. These resource workloads request the default
full-detail mode. Existing artifact, count, placement and resource checks remain
in place. Reversing only the PBR profile/schema change and removing its new mode
assertion reconstructs the prior probe bytes exactly, matching the before
receipt digest `7ec7d9bbfd1dea5e0c72da37f6fad39b5811ef721200cdfde929b25da56a5469`.
The final retained coordinator replays pass all four PBR cases and all ten
placement cases: nine successful artifact inspections, five typed refusals,
and three local/placed content byte-equality comparisons. Their driver,
generator, native binary, compressed/raw receipt and failure excerpt identities
match retained bytes; all 92 frozen production source hashes remain unchanged.

Independent guard controls use all nine successful retained report payloads.
Schema 5, schema 7, the old profile, root-proxy mode, missing approximation and
an extra approximation property each fail before archive access: 54 rejected
mutations. Each unchanged payload reaches the deliberately absent archive and
raises `FileNotFoundError`. These controls test report-gate sensitivity only;
they do not claim another complete artifact replay. The compact
[review receipt](ci-checker-review.json) records pins, mutations and reproduction
instructions. No blocker was found in this narrow correction. A new hosted CI
run remains required; the two concurrent coordinator replays do not establish
fresh timing or sampled-memory baselines. This addendum does not broaden the
production, approximation or release conclusions above.

## Reproduction and remaining limits

Run `review_probes/interval_controls.py` and
`review_probes/child_box_arithmetic.py` for exact arithmetic controls.
`review_probes/child_box_containment.py PATH_TO_ARCHIVE` checks exact emitted
child endpoints. `review_probes/artifact_controls.py --binary PATH_TO_FROZEN_CLI`
recreates the small final-CLI materialless and authored-key controls. They use
temporary fixture/output directories and require no build. Small JSON receipts
are retained; generated archives and binaries remain in the external evidence
location. The reviewer did not authorize release, publication or external
communications.

Acceptance remains local to stored payload coordinates and the finite selected
profile. It proves no topology, multiplicity, normal deviation, texture/appearance
or pixel equivalence, exact world Hausdorff distance under a finite placement
matrix, general CRS, recursive HLOD, total RSS ceiling or global legacy removal.
The certificate can refuse geometrically coincident subdivisions because its
complete-face bound is loose. These are declared profile limits and separate
migration gates, not omitted obligations silently promoted by green tests.
