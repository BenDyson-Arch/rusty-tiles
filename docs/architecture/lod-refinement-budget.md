# Mesh LOD refinement budget

Design proposal, 2026-10-24; **no contract change or new output accepted here**.
Baseline source: `f97cd930fa36df0ca4213131f2e9d1026a5bcda0`.

**Recommend (b): separate the refinement estimate from the certified bound,
initially retaining today's geometry, leaf sizing and texture bake.** Test a
bounded, percentile-guarded fourfold reduction in published refinement errors.
If that calibration passes, the existing m2048 payloads predict **2.6 / 7.1 /
68.3 MB**, versus **35.0 / 63.4 / 230.8 MB**, for the three views below. These
are conditional forecasts, not measured percentile errors or accepted visual
quality. Smaller proxies are a later experiment: they can increase downloads
when their errors grow. The owner must explicitly change the roadmap requirement
before (b) can ship.

## Measurements and meaning

Read-only inputs are `proxy-density/{m2048,tex3}/view` under
`/home/bend/scratch/rusty-tiles-scratch/`. [Audit instructions and identities](../../bench/architecture_audit/lod_refinement/README.md)
record decoded measurements, reproducible scenarios and proof limits. m2048 has
2,806 content tiles: 847 proxies and 1,959 leaves carrying exactly 9,999,999
source triangles. Every measured tile has three stored vertices per triangle.
387 proxies have at least 7,676 triangles; maximum is 8,190. The usual 7,680
target is 8,192 minus H17's fill headroom.

Let `E` be the tile error, `H` screen height, `f = H/(2 tan(fovY/2))`,
`A` decoded surface area and `N` triangles. At SSE 16, distance to the tile box
is `d_switch = E f/16`. An equivalent front-facing surface gives
`pixels²/triangle = (A/N)(16/E)²`; its reciprocal is triangles/pixel.
For these landscape cameras, the simulator's 60° **horizontal** FOV gives
`f = width/(2 tan(30°))`: switch distances are `86.60 E` metres at 1600×1000
and `138.56 E` metres at 2560×1600. Camera range does not enter this ratio.

The final column substitutes the **parent's** error, approximating density when
the tile first appears. Both are diagnostics: surface area includes hidden,
back-facing and overlapping faces; foreshortening, perspective variation and
the offset from nearest-box distance to surface distance are omitted. They are
not measured raster coverage. Medians below are calculated per tile; parentheses
are p10–p90, not confidence intervals. Extent is the longest published tile-box edge.

| Proxy depth | Tiles | Triangles/tile | Error, m | Extent, m | Area, m² | px²/tri at own switch | px²/tri at parent switch |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 1 | 7,680 | 11.67 | 104 | 15,707 | 3.8 | — |
| 1 | 4 | 7,679 (7,676–7,680) | 6.147 (4.76–7.03) | 52.5 | 4,444 | 4.3 | 1.09 |
| 2 | 16 | 7,679 (5,186–7,680) | 0.943 (0.592–3.05) | 26.9 | 1,051 | 38.3 | 1.26 |
| 3 | 34 | 7,679 (5,280–7,680) | 0.621 (0.139–1.83) | 13.6 | 317 | 20.2 | 1.56 |
| 4 | 88 | 7,679 (4,799–7,680) | 0.338 (0.133–0.903) | 6.93 | 79.4 | 27.5 | 1.43 |
| 5 | 237 | 7,679 (4,800–7,680) | 0.148 (0.0647–0.349) | 3.46 | 19.0 | 35.8 | 1.67 |
| 6 | 321 | 6,261 (4,425–7,680) | 0.0624 (0.0215–0.165) | 1.74 | 4.66 | 51.0 | 5.49 |
| 7 | 131 | 5,962 (4,306–7,679) | 0.00538 (0.00364–0.0210) | 0.861 | 0.968 | 1,456 | 17.76 |
| 8 | 15 | 4,373 (4,164–6,802) | 0.00392 (0.00314–0.00779) | 0.421 | 0.216 | 655 | 93.95 |

Against a four-pixel-area target, depth 1–5 content arrives roughly **2.4–3.7×
too dense** under this approximation. At its *own* switch it is mostly already
sparser than that target; the root is close to it. Thus `N_target =
A(16/E)²/4`, clipped to 256–8,192, barely reduces this output. Using parent
error gives a useful *entry* budget, but parent errors themselves change when
children are resimplified. A viewport-independent target is possible because
resolution cancels at the switch; it must be recalibrated after refinement policy.
The deep, nearly full-detail proxies are not the first-view bottleneck.

The simulator reproduces 70 / 144 / 510 requested GLBs. Its close view totals
3.006M downloaded triangles and 9.017M stored vertices, matching the reported
3M/9M load. Its ideal final visible frontier is smaller: 317 tiles / 1.744M
triangles. Ancestors and offscreen siblings account for the difference; this
audit does not establish what a live frame actually draws. Close-view bytes
are 51.6% embedded images, 46.9% geometry/provenance BIN data and 1.4%
GLB JSON/chunk plus hierarchy overhead. Keeping triangle IDs is fully compatible
with the proposed reduction in the number of tiles requested.

## Options on identical cameras

Cells are **decimal MB / millions of downloaded triangles**, including requested
ancestor/offscreen content; hierarchy bytes include `tileset.json`. Views use
heading −0.8, pitch −0.65, SSE 16, and range in bounding-sphere radii. The original
simulator models base traversal; it does not model cache eviction, occlusion,
dynamic SSE, scheduling/cancellation or interaction after this cold view.

| Policy / assumption | 1600×1000, 3r | 2560×1600, 3r | 2560×1600, 0.75r |
| --- | ---: | ---: | ---: |
| Existing output | 35.01 / 0.463 | 63.37 / 0.949 | 230.81 / 3.006 |
| (a) 2,048 proxy triangles; unchanged errors | 24.86 / 0.172 | 42.39 / 0.346 | 176.01 / 1.435 |
| (a) Same cap; error grows with `sqrt(N_old/N_new)` | 46.87 / 0.397 | 68.62 / 0.639 | 282.05 / 2.317 |
| (a) Four px²/tri at own switch; unchanged errors | 34.79 / 0.457 | 62.95 / 0.937 | 229.82 / 2.977 |
| (b) Publish `E/2`; existing GLBs | 11.82 / 0.132 | 24.44 / 0.304 | 121.40 / 1.992 |
| **(b) Publish `E/4`; existing GLBs** | **2.58 / 0.038** | **7.10 / 0.097** | **68.27 / 1.042** |
| (b) Publish `E/8`; existing GLBs | 0.43 / 0.008 | 2.58 / 0.038 | 40.45 / 0.545 |
| (c) Bypass odd proxy depths; all leaves retained¹ | 61.69 / 0.880 | 113.50 / 1.587 | 469.91 / 4.929 |
| (d) `E/4` + 2,048 cap; unchanged error after simplification | 1.56 / 0.010 | 4.59 / 0.027 | 45.08 / 0.376 |
| (d) Same combination; square-root error growth | 8.62 / 0.043 | 14.84 / 0.086 | 75.54 / 0.748 |
| (d) `E/4` + bypass odd proxy depths¹ | 13.45 / 0.131 | 13.51 / 0.131 | 113.47 / 1.627 |

**(a) Triangle budgets:** Geometry/provenance BIN bytes scale with retained
triangle count; existing image and GLB JSON bytes stay fixed. Leaves stay exact.
This is an accounting estimate, not a re-encode: meshopt rates, charting, locked
seams and image pages may change. The square-root law is an illustrative
surface-spacing stress case, **not a prediction or upper bound** on geometric
error. Newly certified errors could be better or worse. Lowering the cap does
not guarantee fewer bytes because it can expose many more descendants.

**(b) Refinement estimate:** These runs change proxy errors only, preserve leaf
snap errors, and enforce parent ≥ child. GLB costs are measured exactly for each
resulting request set; proposed certificate metadata is not included (budget it
separately, initially ≤0.2 MB added per tested view). Half/quarter/eighth are
sensitivity factors, not measured RMS/p99-to-certificate ratios. A percentile
suppresses small extreme deviations; RMS suppresses them further and depends on
area weighting. Prefer a high percentile plus explicit worst-case floor over RMS
alone. Neither protects a narrow silhouette or an important tiny component.
Certificate slack from whole-patch witnesses and finite work budgets also
contributes to C; these outputs do not separate slack from actual worst deviation.
Tighter certification could reduce C without a contract change, but its savings
and computation cost have not been measured here.

**(c) Wider tiles / larger leaves:** Raising the leaf cap to 32k is already
rejected by the roadmap's picking evidence: four provenance tables picked
correctly only 8/56 times. Multiple ≤8,192-row contents could preserve identity,
but need fresh consumer proof and do not remove their total bytes. Coalescing
spatial cells while keeping full-detail ≤8,192-triangle leaves cannot enlarge
dense leaves without splitting elsewhere. Existing parents already provide
wider-area proxies; delaying refinement makes use of them.

¹ The executable surrogate collapses alternate internal levels, retaining root,
even-depth proxies and every leaf with their existing boxes/errors. A retained
parent spans roughly twice the linear extent of the omitted level. This tests
the tempting "fewer, wider LOD stages" approach, and shows its larger jumps can
load more fine content. It is **not** a rebinned octree or a valid implicit
address rewrite. Metadata cost is held at the corresponding uncollapsed
scenario's cost. Actual cell/leaf redesign requires new simplification,
certificates, addressing and seam incidence; no credible numeric forecast for
that redesign follows from these outputs. Forest consolidation alone changes
request dependencies, not the GLB set or these geometry costs.

**(d) Combinations:** Budget cuts offer further savings only if they do not
undo the refinement gain. Adding the four-pixel *own-switch* budget to `E/4`
makes no difference on these views. Do (b) first, then evaluate a soft
2,048-triangle/entry-density target on the measured new hierarchy. Do not combine
tree redesign, calibration and aggressive simplification in one acceptance slice.
On tex3, baseline is 4.47 / 4.47 / 12.30 MB; `E/4` gives 0.32 / 0.32 / 4.47 MB.
Its plateaus confirm that a global multiplier does not promise smooth savings.

## Proposed contract and recommendation

Keep two independent quantities, in metres, after decoded-position certification:

- `C`: certified worst-case bound, with both directed terms and the existing
  child maximum, source/lattice identity and decoded-geometry binding.
- `R`: renderer refinement estimate. Proposed first calibration:
  `q = max(area-weighted sampled p99(source→proxy), p99(proxy→source))`;
  `R_raw = min(C, max(C/4, 2q))`; `R = max(R_raw, children's R)`.
  Leaves keep `R = C`. Missing/incomplete metric work selects `R = C`, recorded
  explicitly. Consequently `C/4 ≤ R ≤ C`; no uncertified decrease of `C` occurs.

Measure both distributions directly against captured source and final decoded
proxy, including vanished source regions and filled proxy faces. Do not compose
percentiles or substitute child proxies for the source. Use deterministic,
area-stratified bounded sampling, record sample coverage/work exhaustion, and
report RMS/max as diagnostics. Separate component, boundary and silhouette
controls are required because p99 can miss them. This task measured no source
deviation distribution, so the percentile guard's hit rate is unknown.

Publish `R` as core `geometricError` and implicit `TILE_GEOMETRIC_ERROR`; preserve
`C` and directed bounds in a versioned tile metadata class (explicit tile
metadata and implicit property-table rows). Declare the policy at tileset level;
carry certificate/metric bindings to the same geometry and source identity.
Routing/external/implicit-root errors must remain consistent and monotone.
Keep H27 bake spacing and fill-facing checks on **C**, not the newly named R:
otherwise changing refinement silently rebakes textures and invalidates these
forecasts. No codec, texture density or feature-ID changes are proposed.

This **contradicts**, rather than satisfies by relocation, milestone 3's
"Published geometric error bounds both directions against source geometry."
An approved amendment must say that the published *certificate metadata* bounds
both directions, while `geometricError` is a declared refinement estimate. Amend
H17's published-error equation/terminology, H16's handoff, the mesh guide and
milestone-4 acceptance together. Keeping C in an extra property does not make
R a worst-case bound. Until that amendment is accepted, publish C.

The present `validate` checks `geometricErrorOrder`, resource integrity and
decoded containment; it does **not** recompute source-distance certification
from an archive without its source. Those checks remain unchanged for R.
Add authored-policy checks for finite nonnegative C/directed terms, policy
version, `max(directed) ≤ C`, monotone C and R, `C/4 ≤ R ≤ C`, metadata
consistency across explicit/implicit boundaries, and binding integrity. Report
these as certificate-*record* checks, not independent proof of the distances.
An independent source-aware H17 auditor must still test the sound two-sided
bound, rounding/budget exhaustion, vanished/fill accounting and provenance;
samples can falsify a bound but cannot prove a global supremum. Preserve the
interval witness proof and sensitive negative controls as well as sampled tests.

**Expected outcome if the quarter-error calibration is accepted:**

| View | MB | Downloaded M triangles | Ideal frontier M triangles | GLBs requested |
| --- | ---: | ---: | ---: | ---: |
| 1600×1000, 3r | 2.6 | 0.038 | 0.031 | 5 |
| 2560×1600, 3r | 7.1 | 0.097 | 0.074 | 13 |
| 2560×1600, 0.75r | 68.3 | 1.042 | 0.593 | 158 |

Allow separately for the added certificate metadata. If calibration supports
only a twofold reduction, use **11.8 / 24.4 / 121.4 MB** and **0.132 / 0.304 /
1.992M** triangles instead. If guards force C, the baseline remains possible.
There is no defensible unconditional forecast for the proposed p99 policy yet.

**Visual risk:** With `R=C/4`, the certificate's usual SSE projection can reach
64 pixels at a 16-pixel refinement threshold; at `C/8` it could reach 128.
These are worst-bound projections, not measured popping. Coarse silhouettes,
filled holes, vanished islands and texture detail persist closer to the camera.
H17 guarantees bit-identical **same-depth frontiers including shallower leaves**;
it explicitly does not guarantee bit-watertight mixed proxy depths under REPLACE.
Those gaps remain bounded in world space by the two certified errors and can
become more visible when held closer. A refinement-only change preserves the
existing topology guarantee; it does not guarantee visually invisible seams.
Reject failing views or retain C on affected tiles (propagating R upward); then
rerun byte estimates. New stitching/skirts are outside this proposal.

## Implementation slices and acceptance gates

1. **Measure without changing output.** In `src/mesh_large/lod/{certify,mod}.rs`
   collect deterministic bidirectional area samples against source, paired with
   final C and candidate R, with bounded work and explicit exhaustion. Preserve
   rounding, removed-region/fill witnesses and the frozen lattice. Extend
   `lod/tests.rs` and the independent H17 audit with a large smooth surface plus
   tiny far island, a narrow silhouette, filled hole, two material labels and
   unequal mixed-depth neighbours. Test sample area weighting and determinism;
   don't present a sampled p99 as a certified percentile.
2. **Publish the approved dual-error contract.** Resolve one policy in
   `mesh_large/config.rs`; split `lod/handoff.rs` and
   `delivery/{source,plan}.rs` fields into certificate and refinement meanings.
   Extend `src/implicit.rs`, `src/metadata.rs` and mesh delivery metadata as
   required; preserve unrelated producers. Audit `bake_pass.rs` and bake handoff
   consumers so all appearance/certified-facing uses stay on C. Keep proxy and
   leaf GLBs identical in this slice. Add explicit, implicit, external-link and
   packed/synthetic-depth tests with deliberately different R/C values.
3. **Validate and independently accept.** Extend `src/validate.rs`/schema/facts
   and traversal controls for certificate records. Mutation controls must catch
   understated C, swapped R/C, missing implicit metadata, stale bindings and
   nonmonotone R; certificate tests must still reject omitted vanished regions,
   fills across seams and inward rounding. R<C must pass the *new* record policy,
   not the old published-bound claim. In a later authorized consumer run, test
   stock Cesium SSE 16 at all three views plus orbit/approach and mixed-depth
   seams; compare pixels/popping, actual requested URLs/bytes and picks against
   the baseline. Annotation-readiness checks retain every leaf triangle identity,
   proxy survivor/fill semantics and extensible feature-ID sets. Require separate
   nonauthor review before accepting the implementation; none is claimed here.
4. **Optional proxy-budget experiment.** Only after calibration, distinguish
   desired faces (e.g. 2,048) from the hard 8,192-row consumer limit. In
   `lod/{mod,mesh,incidence}.rs`, retain a depth-wide shared seam retention policy
   and consensus; per-tile QEM targets cannot independently remove seam vertices.
   Do not let a smaller target silently loosen island/fill policy. Respect locked
   geometry floors, recertify after encoding, report missed soft targets, and
   refuse hard-limit/picking failures. Measure new errors and bytes rather than
   assuming linear savings. Retain `tree.rs` and full-detail leaf sizing for now.

## Owner decisions

1. **Contract:** accept a versioned refinement-estimate profile with C in metadata
   (recommended), or retain the current certified `geometricError` requirement?
   The latter rules out (b); tighter certification and budget experiments remain,
   but these measurements establish no drastic safe reduction under that contract.
2. **Quality envelope:** trial `C/4` with the p99 guard and explicit visual/seam
   rejection (recommended), or start at `C/2`? Do not adopt `C/8` without stronger
   evidence. Approve particular appearance losses through camera comparisons,
   not merely a numeric worst-bound allowance.
3. **Rollout:** calibrate an opt-in profile before changing default `balanced`
   (recommended), or make the accepted policy the new default immediately?
   Keep the previous certified profile available with unambiguous semantics.
4. **Scope:** refinement first with 8,192 hard limits and existing proxy targets
   (recommended), or additionally fund the separately measured 2,048-target slice?
   No leaf enlargement, texture redesign or loss of per-triangle IDs is needed.
