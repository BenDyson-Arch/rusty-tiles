# C2 open decisions

These are the choices that are still unresolved. Each comes with a recommendation and the
evidence that would settle it. Settled items are recorded in `contract-draft.md`:

- exact zero-tolerance predicates;
- forward evaluation in each volume's frame;
- singular tile transforms supported;
- RTC and regions deferred;
- per-reference re-streaming;
- no reuse of `validate.rs` slack helpers;
- an oracle that is independent of `src/`.

D1–D3 block slice 1. D4–D7 must be fixed before the slice is accepted, but each has a safe
interim position.

---

## D1. Public selection and report meaning *(blocks slice 1)*

**Question.** How does C2 appear in `inspect`? `ValidationRequest` currently carries no
options (`src/validate/types.rs:5-17`), and `checks` is a fixed list
(`validate.rs:965-979`).

**Options.**

- **(a) Default strict.** Any C2 domain exclusion fails the operation as Unsupported.
  This turns today's successful inspections of every default point/vector output
  (implicit) into failures, and the same for any third-party b3dm with `RTC_CENTER`.
- **(b) Default with honest per-archive coverage.**
  - C2 always runs on in-domain content.
  - A proven Outside is InvalidInput.
  - `decodedContentContainment` enters `checks` only when every content reference was
    covered. Otherwise `decodedContentBounds` stays in `notInspected`, with a reason from a
    finite enum.
- **(c) Opt-in.** A separate request selection, such as `--check decodedContent`. Old
  default success stays unchanged, but this adds a new public request field across Rust,
  CLI and Python.

**Recommendation: (b).**

- It needs no options bag and no new request API.
- Labels stay truthful: the success label means full coverage.
- Real defects in covered content always fail.
- It matches how C1 already reports uninspected classes.

If CI users later need "fail unless proven", add a single `require` selection as a separate
reviewed API. Do not add it in slice 1.

**Evidence to settle.**

- Run the producer corpus (`contract-draft.md`, acceptance step 4) through the independent
  oracle.
- List every output whose exit status would change under (b). The expected changes are
  proven-Outside cases only: the 1-ulp producer cases and the legacy routes.
- For each change, choose between a producer fix and an accepted documented capability
  change.
- Settle the finite not-inspected reason enum and the Outside fact schema. The candidate
  enum is `implicitTiling`, `rtcCenter`, `cesiumRtc`, `sceneSelection`, `upAxisOverride`,
  `skin`, `morphTargets`, `animation`, `gpuInstancing`, `positionalExtension`,
  `regionVolume`, `degenerateBox`, `metadataVolumeOverride` and `numericRange`.
- Coordinator sign-off that (b) is not a permissive fallback. It narrows the claim; it does
  not weaken a check.

---

## D2. Quaternion meaning, and who owns node validity windows *(blocks slice 1; W1 blocker)*

**Question.** There are two parts:

1. For a represented node `rotation` q, which matrix is "the" decoded transform?
   - **Raw polynomial** `1 − 2(y²+z²)…`, which the prior witness W13 assumes.
   - **Normalized rotation** `H(q)/‖q‖²`, which is exact and rational with no square root.
2. Which component owns the unit-norm and local-matrix validity windows, and with what
   constants?

**Facts.**

- The prior proposal's windows are `|‖q‖²−1| ≤ 2^-20` and `|cos| ≤ 2^-20`. Both are
  narrower than W1's own source admission:
  - quaternion `|‖q‖−1| ≤ 1e-6`, roughly `|‖q‖²−1| ≤ 2e-6`, at
    `mesh_archive/source.rs:714-717`;
  - matrix `|cos| ≤ 1e-6` at `source.rs:684-692`.
- W1 emits source nodes unchanged, so a strict C2 using those windows would refuse W1
  outputs that W1 accepted.
- Containment soundness needs only a finite, nonzero q under either meaning.
- W1's bounds enclose the hull of both meanings (`original_bounds.rs:128-139`), so W1 passes
  either way.
- An f32-rounded exact unit quaternion has `|‖q‖²−1| ≤ 2^-22 + 2^-46`. This lane re-checked
  the bound analytically: the absolute rounding error is at most 2^-25 for |q_i| < 1.

**Recommendation.**

- **(i) Meaning.** C2 uses the normalized meaning `H(q)/‖q‖²`. It is the rotation that the
  glTF unit-quaternion field denotes, and it is exact with rational arithmetic. Update W13
  so that "normalize" is no longer listed as a fault. The difference is about 1.04e-15
  (`verify/witness-check.json`).
- **(ii) Windows.** The unit window is a *format-validity* rule, not containment numerics.
  - It belongs to one constant owned by the payload format owner (node records).
  - Outside the window is InvalidInput, because glTF requires a unit quaternion.
  - Use a dyadic bound on `‖q‖²` so that the comparison is exact.
  - Choose **`|‖q‖²−1| ≤ 2^-18`** (≈ 3.8e-6). It admits everything W1 admits today, so W1
    behavior is unchanged. W1 may later be pointed at the same constant.
  - The tighter alternative is 2^-20 together with narrowing W1 at `source.rs:715`. That is
    a W1 capability reduction and needs its own justification.
- **(iii) Local matrices.** C2 does not police local-matrix orthogonality or TRS
  decomposability in slice 1. Soundness does not need it, and adding it would create a new
  W1 conflict at `source.rs:689`. It remains a separately scoped node-validity item, owned by
  the format layer when it is introduced.

**Evidence to settle.**

- Pin the actual quaternion-to-matrix formulas of the intended consumers: CesiumJS
  `Matrix3.fromQuaternion` and the glTF loaders used in Blender acceptance. This lane did
  not pin them, so it records no claim about which form they use.
- Pin Khronos glTF-Validator's unit-quaternion threshold from source.
- Scan the retained W1/F1 fixture corpus and the exporter samples for maximum `|‖q‖²−1|`
  and maximum `|cos|`.
- Executed controls must cover:
  - the window edges, exactly at 2^-18 and one ulp beyond;
  - a zero quaternion;
  - components outside [−1, 1].

---

## D3. Implicit coverage path *(blocks the default claim for P1/vector)*

**Question.** Point-cloud and vector outputs default to implicit tiling:

- point cloud: `point_cloud.rs:601-608`, with `--explicit` at `main.rs:299-301`;
- vector: `vector/pipeline/store.rs:976-982`, with `--explicit` at `main.rs:335-337`.

Their effective volumes are `TILE_BOUNDING_BOX` metadata rows. The derived cells do not
contain the content. Vector places children by array slot (`implicit/tileset.rs:117-121`),
and the derived-cell check is applied only at subtree roots, with 1e-6 slack (`:284-297`).

How should C2 cover them?

**Options.**

- **(a) Slice 1 explicit-only.** Implicit content is not inspected (`implicitTiling`).
  Coverage arrives when #126 A2's accepted effective-volume owner exists.
- **(b) Build on the current expander.** Run C2 over `implicit::expand_tileset_bounded`
  output (`validate.rs:575-608`). This old owner honours only the private `rustyTile` schema
  (`implicit/tileset.rs:718-733`, `:1133-1135`) and is being replaced by A2.
- **(c) Check derived cells.** This gives false Outside against spec-legal overrides
  (metadata semantics `adoc:145,161`; A2 semantics audit `audit.md:74-88`).

**Recommendation: (a).** Keep the C2 predicate interface as `(effective volume, frame
chain, drawn position)`, so that A2 plugs in without a C2 rework. Meanwhile, producer
acceptance for P1/vector runs the **independent oracle** on both implicit and `--explicit`
outputs. The oracle decodes subtree availability and metadata rows itself, as the A2 plan
already requires (`implicit-rewrite-contract.md:265-275`).

Option (b) would make a public success label depend on an owner that is due for replacement.
That is churn, and an unproven dependency. Option (c) is wrong.

**Evidence to settle.**

- A2's accepted effective-volume/ordinal owner.
- Oracle runs on default point/vector outputs showing content inside the effective metadata
  boxes, including ancestors.
- An explicit record of how many default outputs remain "not inspected" until A2 lands, so
  that release notes do not over-claim.

---

## D4. Fate of `hierarchyBounds`

**Question.** `hierarchyBounds` rejects any child or content volume that escapes its parent
or tile (`validate.rs:643-650`, `:679-685`). The core text explicitly allows a child volume
to extend outside its parent when the content is enclosed
(`3d-tiles-core.adoc:752`; this is the loose-quadtree case at `:790`). The check also uses
`1e-6·(1+|axes|)` relative slack, so it admits small true violations (prior model controls).
It is therefore wrong in both directions as a normative claim.

**Options.**

- (a) Keep it unchanged as a stronger declared-volume check.
- (b) Remove its rejection once C2 covers the archive.
- (c) Make it report-only.

**Recommendation.**

- **Slice 1:** leave it unchanged. For implicit archives it is still the only spatial check.
- **After D3 (a) completes:** remove its default rejection. C2 is the normative replacement.
  Then delete `scale_bound`, `inside` and `contains` together with it (`validate.rs:211-338`).
  Do not keep a renamed stronger option unless a concrete consumer needs one.

**Evidence to settle.**

- Materialize the context control "child-volume-escapes-but-content-inside" (prior
  `witnesses.json`): today it gives InvalidInput, and under C2 it is Contained.
- Count current producer outputs whose child volumes escape. The A2 audit found that
  vector/point padding overhangs regular cells.
- Coordinator decision on the capability change, because it changes which archives fail.

---

## D5. Box represented-set interpretation for non-orthogonal half-axes

**Question.** A rotated box written in f64 is never exactly orthogonal. Which set does C2
test against?

- **Parallelepiped:** `c + B·[−1,1]³` with the represented B.
- **Orthogonal-only:** refuse any box that is not exactly orthogonal.

There is no producer impact: every producer writes diagonal boxes.

**Recommendation.**

- Use the parallelepiped reading for nonsingular B, decided exactly by Cramer comparisons.
  It is the only exact reading of rounded rotated boxes, and it coincides with the oriented
  box when B is orthogonal.
- Keep C1's existing `1e-8` orthogonality admission (`validate.rs:240-249`) as a format
  well-formedness rule.
- Rotated rank-deficient boxes are not inspected (`degenerateBox`).

**Evidence to settle.** Pin CesiumJS `OrientedBoundingBox` membership and culling semantics
for non-orthogonal `halfAxes`. Add an oracle control that uses Gaussian elimination, distinct
from the production Cramer path.

---

## D6. b3dm `RTC_CENTER` JSON precision *(deferred; not slice 1)*

**Question.** The feature-table JSON `RTC_CENTER` is typed `float32[3]`. Is the JSON number
rounded to f32 (round-to-nearest-even) before use, as the prior proposal suggests, or used as
binary64?

No producer writes `RTC_CENTER`.

**Recommendation.**

- Slice 1 marks it not inspected (`rtcCenter`).
- In slice 2, adopt f32 RNE then exact promotion for JSON, and little-endian f32 for the
  binary form. Finite values that overflow f32 are not inspected; nonfinite binary values
  are InvalidInput.

**Evidence to settle.**

- Pinned feature-table semantics (`feature-table.adoc:40-83`).
- The CesiumJS b3dm loader's handling of JSON `RTC_CENTER`.
- Third-party b3dm corpus prevalence.

---

## D7. Exact-arithmetic magnitude domain and expansion caps

**Question.** Expansion arithmetic is exact only if no TwoProduct error term underflows and
no partial sum overflows. Which input domain is "in-domain", and what happens beyond it?

**Recommendation.**

- **Math owner.** Proves a domain stated over the actual operations used:
  - point-chain application step by step (matrix·vector, never matrix·matrix), with
    compression after each step;
  - nonoverlapping expansions, whose length is bounded by the f64 exponent span (≤ about 40
    components);
  - Dekker split with no dependence on FMA.
- **Beyond the domain:** not inspected (`numericRange`), never Contained.
- **Work:** expansion work is bounded per test and counted by `containmentTests`.
  ResourceLimit applies only to the counters.

**Evidence to settle.**

- Unit proofs and controls in the math module:
  - deep nested rotations (node depth 128);
  - magnitudes near 2^±1000;
  - subnormal coordinates;
  - normalized denominators combined with `‖q‖²`.
- Oracle agreement on every control.
- A measured per-test cost for the producer corpus, to confirm that no interval filter is
  needed in slice 1.
