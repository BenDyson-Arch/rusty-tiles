# C2 decisions (v3, interface freeze)

**v3 changes.**

- D7 is frozen bit-exactly (B1–B3, D1–D4).
- O-b is decided by the coordinator.
- D1's list of public changes now records the struct-literal break.
- The delta review is at `/tmp/rusty-tiles-c2-precode-review-20261011/delta/delta-review.md`.
- v2 is preserved in `v2/`.

## Status

v2 responds to the precode review (`/tmp/rusty-tiles-c2-precode-review-20261011/review.md`).
v1 is in `v1/open-decisions.md`.

- D1, D2, D3 and D7 are now **decided for slice 1**, and D7's definitions are frozen in v3. Each
  is listed with the residual evidence that could later revise it.
- O-b is decided by the coordinator.
- D4, D5 and D6 remain open. None of them blocks slice 1.
- O-a is still open.
- The cap values are R-measured proposals.

The delta review gave GO-WITH-CHANGES for F and M, conditional on B1–B3 and D1–D4. v3 folds those
in, and the coordinator confirms the freeze before coding.

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
5. Report-schema additions: `checks`/`notInspected` entries and the payload-label split.
6. **Public Rust API (O-b):** five new `pub` fields on `ValidationLimits`
   (`containment_span_bits`, `containment_work_units`, `containment_bytes_read`,
   `containment_decoded_bytes`, `containment_node_depth`). This **breaks external struct
   literals**. It is recorded in the migration notes. No public function consumes the struct.

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

### D7. Exact arithmetic and work: decided and frozen (R1, R2, B1–B3, D1–D4)

**Arithmetic.** Arbitrary-precision dyadic integers, held in a fallible heap arena.

- One dyadic is 544 B at a span cap of 4096.
- The arena holds `16·(depth+1) + 32` dyadics, which is about 0.56 MiB at depth 64.
- Refusal is ResourceLimit.
- Nothing is on the stack. This is required because `validate` runs on Python and Blender
  threads.

**Domain.** The normative span definition in the contract fixes all of the following:

- flat sums over exhaustive term lists;
- the (0,0) pass-through;
- skipped transform-less nodes;
- ZERO for ±0;
- fixed worst-case position pairs;
- per-level spans.

O and M compute identical spans. The worked example has a maximum span of 29, so the control is
in domain at a cap of 29 and `numericSpan` at 28.

**Soundness.** Two independent checks found no violations:

- delta review: 0 violations in 251,516 checks;
- this lane's normative probe: 0 violations in 304,455 checks.

**Coverage at 4096 bits (B2):**

| Quaternion style | Box-only rotated levels | With a sphere |
| --- | --- | --- |
| Generic | 16 | 10 |
| Near-axis | 14 | 6 |
| One subnormal-scale component | 1 | 0 |

Producer chains are at most 297 bits.

**Work (frozen formulas).** Facts are aggregated per URI (hull of `Pair0`, plus `sInst`, `sPos`,
`T`, `Nodes`, `Idx`, `E`, `Dec`, `B` and `maxNodeDepth`). For each reference:

  `units = T·Σ_ℓ⌈s_ℓ/64⌉² + Nodes + Idx + E`

The bytes and decoded-bytes totals are accumulated alongside. The precheck accumulates during
traversal and exits at the first violation. Every cap gives `workLimit` or `numericSpan`, never
ResourceLimit.

**Peak C2-added memory:** at most about 33.5 MiB of facts plus about 0.8 MiB of working storage.
This is not an RSS claim.

**Residual evidence (cap values only; R measures):**

- the per-unit time `c` for each unit class;
- the corpus totals against the caps;
- whether the worst-case FLOAT position pair should later be tightened per accessor. That is
  the delta review's coverage option: it narrows nothing and would need a contract delta.

### O-b. Placement of the C2 caps: decided by the coordinator

The caps live in the existing public `ValidationLimits`, so validation limits keep one owner.
They are five named fields with defaults (listed in D1 item 6). F owns the `types.rs` edit. The
values are proposals that R may retune before merge.

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
