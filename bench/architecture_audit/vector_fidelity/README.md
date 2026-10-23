# Vector finite 0.4.0 acceptance

This is the owner-approved 2026-10-24 non-mesh bar from
`docs/architecture/040-foundation-rescope.md` (#267) and the non-mesh section of
`docs/architecture/hlod-040-decision.md` (#268). The documents were read from
`origin/docs/foundation-rescope-040` and `origin/docs/hlod-direction-040` because
they were not yet present on this branch's develop base `cda7ca6110c7`.

Retain the transactional vector implementation, source-chart/repair policies,
serializer, codecs and publication owner within their existing domains. This
slice changes acceptance only. No geometry kernel, source reader, aggregation
policy, memory cap or public API is replaced. The independent fixture is the
missing source-to-published-output evidence; existing lifecycle, seams, LOD,
reuse and admission controls remain substantive evidence.

## Invariants and independent checks

`tests/vector_fidelity_oracle.py` reads only the actual published tileset,
external tilesets, subtree availability/metadata and GLBs. It never reads
`vector-build.json`, reconstructs producer hierarchy H, imports the producer,
or calls the product validator. Its small reader supports the producer's bounded
quadtree forest, box bounds, scalar/string/boolean metadata and TS/matrix node
frames. Other profiles are outside this witness. It reuses the existing
specification-written `ext_meshopt_spec.py` decoder and independent WGS84/box
math from the terrain oracle; neither uses meshoptimizer or product transforms.

`tests/test_vector_fidelity.py` establishes:

- Raw and quantized/meshopt point coordinates, exact disconnected-line segment
  multiset and no cross-feature joins. Metadata includes booleans, strings, Float64 values,
  nullable integers, integers above 2^53 and JSON lists with nulls.
- A square minus a literal rectangular hole and two disjoint polygon parts:
  every triangle has its explicitly expected upward normal, lies in the accepted
  regions, triangle interiors do not overlap, and signed area equals the accepted area. Convex clipping, area and
  separating axes provide an analytic union check for these finite fixtures.
- Strict refusal, explicit skip without repair, and opt-in bow-tie repair into
  two independently specified triangles with bijective vertex matching; the
  surviving point is checked too, and rejected identities remain absent.
  Collapsed-repair and displaced-survivor controls fail the corresponding checks.
- Full GLB/tile frame composition, own/ancestor containment and finite monotone
  errors, across more than one forest subtree. Local, geographic and projected
  inputs use asymmetric XYZ and explicit +7 m height where applicable. UTM32
  references are frozen pyproj 3.8.0 / PROJ 9.8.1 coordinates of known WGS84
  positions; expected ECEF uses the independent ellipsoid equations. Position
  tolerance is derived independently from the physical POSITION cells and complete
  node/tile linear map, plus 1e-5 m fixture allowance. Normalized uint16 uses
  half a 1/65535 cell; raw Float32 uses half its IEEE-754 spacing. The maximum
  leaf cell radius is capped at 1e-3 m for these finite fixtures. Producer
  rounding/quantization error reports do not determine this tolerance. Unrecorded
  coarser shared-grid snapping in Float32 fallback is outside this small witness;
  existing seam tests retain their separate coverage.
- Unchanged and edited fresh/reuse world geometry and exact fields/identities.
  Existing `vector_acceptance_oracle.py::run_forest` separately refuses corrupt
  reused published content and proves exact published-byte reuse.
- Well-framed archive controls omit a full-detail feature, change its source ID,
  fill its hole, reverse a triangle, drop a polygon triangle, shift placement or
  shrink an ancestor box. An inflated-error-report control cannot relax precision. A decoded metadata
  control also proves that integer 1 cannot substitute for boolean true. The named semantic
  checks reject these in both encodings; they do not rely on checksums failing.

`bindings/python/tests/test_api.py::test_vector_cli_python_published_geometry_metadata_height_and_defaults`
compares independently decoded GeoJSON/GeoPackage output from the installed
portable wheel and CLI, including exact large integers/list/null values,
resolved budgets/defaults, source identities and +7 m height. Mixed valid/invalid
input also compares strict refusal without publication and explicit skip outcomes
with exact surviving metadata/identity in both formats. The paired test
requires `RUSTY_TILES_BIN`; ordinary wheel jobs without a CLI explicitly skip
that one test. **The paired CLI/Python result is local evidence only; it is
not executed by current CI wheel jobs.** Existing vector API tests retain strict/refusal, unchanged reuse
and callback-failure preservation coverage.

## Resource evidence and execution

Raw receipts, logs, wheel, source hashes and artifact hashes are outside git in
`/home/bend/.cache/rusty-tiles-evidence/040-bar-vector/README.md`. The small
`evidence.sha256` manifest pins those receipts. The compiled producer is pinned
by binary hash and the unchanged production source at the develop base; final
acceptance source changes only tests and documentation.

`resources.py prepare` authors the exact 100,000-feature analytic stress profile
outside measurement: points, disconnected lines, holed polygons, one 4,096-vertex
line and scalar/list metadata. It is explicitly synthetic, not relabelled as
real local data. A separate realistic run uses the existing local
`tinyowl-demodata/geojson/full_walem.geojson` without source preparation.

Both resource runs used a **dev-profile (debug) binary**, built with
`CARGO_PROFILE_DEV_DEBUG=0` (debug symbols disabled, no release optimization).
No release-profile resource measurement is claimed. Both run through the requested slot-2 `measure.py`, `chrt -i 0 nice -n 19`,
with two requested vector jobs and three LOD levels, quantization and meshopt.
Child stdout goes to a file. The monitor samples total owned run-directory
bytes (including output), descriptors, threads and process swap at 50 ms;
RSS is fresh child-process high-water from `measure.py`, not fixture generation.
Sampled disk peaks are measurements, not rigorous allocation upper bounds.
The receipt preserves failed attempts, source exclusions and cleanup evidence.

RAM remains input-dependent: individual geometry/properties, field/layer
catalogues, hierarchy/reuse inventory and concurrent encoding allocations have
no universal byte bound. The 1 GiB RSS / 4 GiB scratch gates apply to the finite
100k profile. Arbitrary giant properties, exhaustive topology/repair/CRS audits,
per-algorithm ledgers and universal hostile-input resource proofs remain 0.4.x.

## Retained policy and proof limits

See [VECTOR.md](../../../docs/VECTOR.md) for source/property selection,
source-chart and best-fit repair/refusal semantics, list JSON encoding,
experimental consumer revisions and aggregation. CesiumJS 1.146.0 and the
pinned vector extension drafts remain the audited consumer profile. Coarse
point aggregation deliberately replaces ordinary source identity with aggregate
rows; this fixture leaves aggregation off and credits the existing aggregation
suite separately. Analytic polygon topology here does not certify every source,
repair, triangulation, CRS, LOD approximation or consumer. Existing seam and
subtree-boundary fixtures extend this finite check; no browser, mesh bake or
new consumer campaign was part of this slice. Final release/platform promotion
is separate from this per-tiler acceptance.

## Executed resource results

The two-job / three-LOD / quantized-meshopt native binary completed the exact
100k stress witness with 100,000 accepted features, 0 skipped, 2,048 leaves and
4,096 total tiles: **0.20 GiB high-water process RSS** and **270,512,108 B
(257.98 MiB) sampled owned disk**, including output. Both finite budgets pass.
Largest feature: 4,096 vertices / 191,015 B canonical feature JSON; largest
properties: 112 B; five fields / one layer. Wall 392.9 s at idle priority;
sampled descriptors 10, process threads 4 (including the two workers), swap 0.
No staging remained.

The unmodified realistic Walem input completed with explicit repair/skip policy:
1,371 accepted / 3 rejected of 1,374 features, **0.12 GiB RSS**, **28,227,047 B
(26.92 MiB) sampled owned disk**, 24.3 s idle wall; descriptors 9, threads 4,
swap 0, no staging. Source geometry maximum 758 vertices; 37 fields / one layer;
feature/property byte maxima and unchanged source hash are in provenance.
The initial strict attempt refused publication: IDs 109, 426 and 841 exceed the
existing 2 cm intersection-repair limit. This is retained refusal evidence,
not a successful measurement or a newly established producer defect.

RSS is rounded by the supplied wrapper to 0.01 GiB. No existing 100k mixed
local source matched the requested profile, so the exact stress witness and
realistic smaller corpus are explicitly separate receipts. Neither establishes
a universal resource bound. See the external README for commands, versions,
worker/descriptor/cleanup observations, exact artifact hashes and review results.

## Original acceptance checks (8a4f83e9)

Native/default `cargo clippy --all-targets -- -D warnings` pass. Native vector
units: 163 pass; portable: 156 pass. The process-CWD compression test that is
ignored in concurrent units also passes alone in both configurations. Vector
integration suites: 22 native / 35 portable pass, plus each CWD child process.
Python vector suites: 62 tests per backend; native-backend tests: six per
backend. Both admission/forest oracle configurations pass. The final vector API
suite has five passing tests, including CLI/installed-wheel exact readback and
strict/skip parity. The official installed-wheel runner previously passed all
37 tests with empty PATH; the expanded pairing was rerun at final source.
Changelog and tree-budget checks pass. Independent round-2 review resolves its
three acceptance-test findings; no production defect was established. Final
receipts/review details are pinned in the external evidence and hash manifest.

## PR #270 independent-review follow-up

Fill expectations now require an explicit normal: upward for the horizontal
fixtures, with a separate oracle-only +X wall-normal probe. Repaired triangles
accept cyclic rotations, not reversed permutations. Published raw/packed GLB
controls reverse one triangle and remove one polygon triangle; the winding and
coverage-area assertions respectively reject them. Inflating conversion.json's
quantization error to 1e100 leaves the independent allowance unchanged and still
rejects a displaced point.

Position tolerance now comes from physical POSITION cells, the complete node /
tile linear map, and a fixed analytic frame allowance. It does not consume
producer rounding or quantization error estimates. Coarser unrecorded Float32
snapping is outside this finite reader's certification; existing seam controls
remain separate. Both resource receipts are dev-profile measurements, and the
paired CLI/Python comparison is explicitly local evidence only.

Follow-up verification: 64 Python vector tests per retained backend; all five
focused oracle tests and all five installed-wheel vector API tests per backend
passed at the final follow-up source. Independent nonauthor review found no
remaining defect in the five requested changes. Rust production code, retained
binary/wheel hashes and the original resource runs are unchanged; Rust suites
and clippy results above remain the original receipts, without a redundant
rebuild. Follow-up logs, source hashes and review live under `review-nits/` in
the external evidence directory and are pinned by the refreshed manifest.
