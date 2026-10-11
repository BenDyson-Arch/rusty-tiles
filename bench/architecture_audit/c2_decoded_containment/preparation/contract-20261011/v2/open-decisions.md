# C2 decisions (v2)

## Status

v2 responds to the precode review (`/tmp/rusty-tiles-c2-precode-review-20261011/review.md`).
v1 is in `v1/open-decisions.md`.

- D1, D2, D3 and D7 are now **decided for slice 1**. Each is listed with the residual evidence
  that could later revise it.
- D4, D5 and D6 remain open. None of them blocks slice 1.
- Two smaller items are open as well: O-a and O-b.

Per the review, M may start only after a short delta review of R1 and R2 (D7 and the Work bound
in `contract-draft.md`). F may start record and stream work once R3, R6, R7 and R8 are accepted.

---

## Decided for slice 1

### D1. Public selection and report meaning: decided as (b′)

The check runs inside the default `inspect`. Coverage is per archive and all-or-nothing (I3).

- `decodedContentContainment` is listed only when every reference in the archive is covered.
- If any exclusion applies, or the precheck fails, the operation stays a C1 success with
  `notInspected` reasons, and nothing is streamed.
- A proven Outside in an in-domain archive is InvalidInput.

**Change from v1's (b).** v1 still streamed the in-domain references of an excluded archive.
That needed a per-(reference, volume) coverage definition (R5c), so it is dropped. Partial
streaming becomes a future contract delta, if it is ever wanted.

**Frozen in the contract:**

- the 15-name reason enum;
- the `notInspected` entry format;
- the Outside message template;
- the payload-label split (R7.1).

These are in `contract-draft.md`, "Report and failure schema".

**Complete list of exit-status changes** (the R2d evidence list):

1. New InvalidInput for a proven Outside in an in-domain archive.
2. New InvalidInput for malformed scene/node records, in **any** archive, because admission runs
   in phase 1.
3. New ResourceLimit only for host allocation refusal. No new counter produces ResourceLimit.
4. No change to Unsupported, Io, `totalBytesRead`, `total_payload_elements` or `hierarchyBounds`.
5. Report-schema additions, including new `ValidationLimits` fields. That is a public Rust
   struct-literal break.

**Residual evidence.** The oracle corpus must report, per producer output:

- which rows change exit status under items 1 and 2;
- which rows hit `workLimit`.

Each such row needs a disposition: a producer fix, a documented capability change, or a
retune of a cap.

**Coordinator.** Confirm that (b′) narrows the claim rather than weakening a check.

### D2. Quaternion meaning and windows: decided

1. **Meaning.** A node rotation means `R(q) = H(q)/‖q‖²`, the rotation denoted by q. It is exact
   with dyadic arithmetic.
   - W13 is updated: `verify/w13-normalized.json`; the difference from the raw reading is about
     1.04e-15.
2. **Window.** C2 uses a validator-side constant: `|‖q‖² − 1| ≤ 2^-18`, evaluated exactly.
   - Beyond the window, the archive is **not inspected** (`quaternionWindow`). This is not
     InvalidInput (R4).
   - Zero, nonfinite, or any component outside [−1, 1] remains InvalidInput, because the glTF
     schema requires all of these.
   - Arithmetic, as verified by the reviewer: W1's `|‖q‖ − 1| ≤ 1e-6` implies
     `|‖q‖² − 1| ≤ 2.0000010018e-6`, which is below 2^-18 (3.8147e-6). So every W1 input is
     admitted.
   - An f32-rounded exact unit quaternion satisfies `|‖q‖² − 1| ≤ 2^-23 + 2^-48`. This is the
     tight bound, from review note 6. v1 stated 2^-22 + 2^-46.
   - **No constant is shared with W1** (R4). A shared constant conflicts with #125's
     no-writer/validator-shared-helper rule and with the ownership plan.
3. **Local matrices.** C2 does not police orthogonality or TRS decomposability in slice 1.

**Residual evidence:**

- Pin the consumer formulas: CesiumJS `Matrix3.fromQuaternion`, and the glTF loaders used in
  Blender acceptance.
- The reviewer recalls, unverified, that CesiumJS uses `H(q)` *without* dividing by `‖q‖²`. That
  would be a third reading, a uniform scale by `‖q‖²`.
- Inside the window, all readings differ by at most `|‖q‖² − 1|` times the extent, relative.
  For W1 extents of at most 1e6 m, that is at most about 3.8 m in the worst case.
- If pinned consumers scale non-unit quaternions, decide whether to tighten the window. A
  tighter window narrows coverage; it never adds failures.
- Also pin the Khronos glTF-Validator threshold.

### D3. Implicit coverage: decided (a) for slice 1 (R6)

Slice 1 is explicit-only. Implicit archives are not inspected (`implicitTiling`).

**Basis, settled at source level:**

- implicit volumes are `TILE_BOUNDING_BOX` metadata rows (`implicit/tileset.rs:1133-1135`);
- vector writes array-slot cells (`:117-121`);
- the current expander honours only the private `rustyTile` class (`:718-733`) and is due for
  replacement by A2.

The predicate interface `(level chain of volumes and transforms, instance frame, raw vertex)` is
independent of how the hierarchy was addressed. A2's effective-volume owner can therefore supply
levels without reworking C2.

**Slice-3 gate (moved here from v1's D3 "evidence to settle"):**

- A2's accepted effective-volume owner;
- O-implicit oracle runs on default point/vector outputs;
- a record of how many default outputs stay not inspected until then, so that release notes
  don't over-claim.

### D7. Exact arithmetic and work: decided (R1, R2)

**Arithmetic: arbitrary-precision dyadic integers.**

- The type is fixed-capacity, hand-rolled and heap-free, owned by M. Its capacity is one
  4096-bit mantissa.
- No new dependency is added. If M prefers `num-bigint` as a direct dependency, R must approve
  it separately; the semantics do not change.
- Withdrawn from v1:
  - binary64 expansions;
  - "≤ about 40 components";
  - "exact at node depth 128".

**Domain.** The static exponent-pair bound over the canonical formulas, with FLOAT positions
counted at worst case (−149, 127), must not exceed `containmentSpanBits = 4096`.

- O computes this predicate from the contract text alone.
- Beyond it, the archive is `numericSpan` (not inspected).
- Probe evidence (`verify/span-bound-observed.json`):
  - The bound stays within about 300 bits of the exact span.
  - Producer chains have a bound span of at most 327 bits.
  - The cap admits 16 generic or 12 near-axis rotated node levels.
- The reviewer's binary64 failure points, underflow at 4–8 levels, do not apply to this
  representation.

**Work** (full text in `contract-draft.md`, Work bound):

- Distinct referenced vertices per (instance, accessor), tracked with a bitmap of at most
  `accessorElements` bits.
- One upward walk per vertex.
- Static units `u = ⌈span/64⌉²` per (reference, instance, level).
- A precheck after traversal against these caps:
  - `containmentWorkUnits = 2^32`;
  - `containmentBytesRead = 2 GiB`, separate from C1's;
  - `containmentNodeDepth = 64`.
- Exceeding any cap gives `workLimit` (not inspected), never ResourceLimit.

**Residual evidence.** R measures:

- the per-unit time;
- the corpus totals against the caps.

The cap values are engineering proposals, retunable without changing semantics. M's unit tests
must cover:

- spans at the cap and one over;
- signed clamp;
- the `k` and `‖q‖²` denominators;
- deep translation-only chains (cheap);
- deep rotated chains (excluded).

---

## Still open (not blocking slice 1)

### D4. Fate of `hierarchyBounds`

**Unchanged in slice 1.** The recommendation stands: remove its default rejection, together with
the slack helpers at `validate.rs:197-338`, once C2 covers implicit archives (slice 3).

**Evidence:**

- the context control "child-volume-escapes-but-content-inside", which is C1 InvalidInput today;
- a count of producer outputs with escaping child volumes;
- a coordinator capability decision.

### D5. General (rotated) boxes, now out of slice 1

Slice 1 excludes non-signed-permutation boxes (`rotatedBox`), following review note 7. No
producer writes them.

**Recommendation for a later slice:** use the parallelepiped reading `c + B·[−1,1]³` for
nonsingular B, decided exactly by Cramer comparisons (degree-3 span growth stays within the cap).
Keep C1's `1e-8` orthogonality admission.

**Evidence:**

- pinned CesiumJS `OrientedBoundingBox` semantics;
- an oracle control based on Gaussian elimination.

### D6. b3dm `RTC_CENTER` JSON precision (slice 2)

Slice 1 detects the key and excludes the archive (`rtcCenter`, R3).

**Slice-2 recommendation:**

- JSON form: round to f32 (RNE), then promote exactly.
- Binary form: little-endian f32, with bounds and alignment consumed.
- Finite values that overflow f32: not inspected.
- Nonfinite binary values: InvalidInput.

**Evidence:**

- `feature-table.adoc:40-83`;
- the CesiumJS b3dm loader;
- third-party prevalence.

### O-a. `3DTILES_content_gltf_vector` on the tileset allowlist

The entry is provisional. O pins the README at revision `c48ebdc` (`docs/VECTOR.md:307`) and
confirms that it defines no positional transform.

If the README does define one, the entry is dropped. Default explicit vector output would then
be `tilesetExtension`, and that is recorded as a coverage gap. This gates the vector corpus row,
not the code.

### O-b. Placement of the C2 caps

The recommendation is to add the four `containment*` fields to `ValidationLimits`, which is a
documented struct-literal break. The alternative is a separate `pub` report field holding
C2-only limits; it avoids breaking `ValidationLimits` literals but still changes
`ValidationReport`.

R decides this before F starts on `types.rs`. It is a cosmetic API choice; the semantics are
identical.
