# C2 v3 interface-freeze confirmation (delta2)

- **Reviewer:** the same nonauthor who wrote `../review.md` (`7bb397dd…`) and
  `../delta/delta-review.md` (`5f4b6fe8…`).
- **Reviewed:** `/tmp/rusty-tiles-c2-contract-20261011/` v3. Every file passes
  `sha256sum -c SHA256SUMS`. `contract-draft.md` is `f840ff0b…` and `ownership-plan.md` is
  `99deab3c…`.
- **Rules followed:** read-only, no cargo, `python3 -I` under `nice -n 10`.

## Verdicts

| Author | Verdict |
| --- | --- |
| **F** | **GO-WITH-CHANGES.** One sentence, H1 below. No further review is needed once it is applied |
| **M** | **GO-WITH-CHANGES.** The same sentence, H1 |

Every condition from the delta review is met:

- B1–B3;
- D1–D4;
- the EXT_mesh_polygon inconsistency;
- O-b.

## Re-run against the v3 normative text

I re-implemented the v3 rules from `contract-draft.md:224-350` without using
`frozen_bound_probe.py`. The code is `delta2/bound_adversary_v3.py`, and its output is
`delta2/bound-adversary-v3-observed.json`. The re-implementation covers:

- ZERO for ±0;
- the (0,0) pass-through;
- the transform-less node skip;
- no tile step when there is no `transform`;
- the exhaustive flat term lists;
- the sphere `R2·W2` form;
- the per-level spans `sInst`, `s₀` and `s_ℓ`.

**Soundness: 137,342 quantity and prefix checks, 0 violations.** The inputs mixed:

- skipped, matrix and TRS nodes;
- ±1 and 0 entries;
- f64 subnormals, 2^500 and −2^1000;
- identity, axis and tiny quaternions;
- missing and general tile transforms;
- boxes and spheres, including zero radius and half-lengths;
- f32 extreme positions;
- the i8, u8, i16 and u16 clamp and normalization.

The pass-through rule is sound for any (0,0) pair, not just for exact inputs. A value with
`lowbit ≥ 0` and `|v| < 2` must be −1, 0 or 1.

**The worked example reproduces exactly:**

- `sInst = 5`;
- level spans `[23, 29]`, so the maximum is 29 (in domain at cap 29, `numericSpan` at cap 28);
- vertex units `4 × 2 = 8`;
- node units `128`.

All of these match `contract-draft.md:352-407` and the author's probe.

## Conditions from the delta review

| Condition | Status | Where in v3 |
| --- | --- | --- |
| B1 bit-exact trees | Met | `contract-draft.md:224-350`: exhaustive term lists, skip rule, ZERO, D2 window outside the domain |
| B2 sphere and subnormal coverage | Met | `contract-draft.md:409-431`: generic 16 box / 10 sphere, near-axis 14 / 6, subnormal 1 / 0. The generic box and sphere figures match my delta measurements |
| B3 heap placement | Met | `contract-draft.md:433-474`: fallible `try_reserve_exact` arena, no stack dyadics, refusal is ResourceLimit before the first test |
| D1 node work | Met | `Nodes(u)` per node visit, skipped nodes included (`:491-500`), charged per reference (`:509`) |
| D2 index scanning | Met | `Idx(u)` (`:491-500`); stream step 3 (`:542-555`) |
| D3 re-decode | Met | `E(u)`, `B(u)`, `Dec(u)`, with separate byte caps |
| D4 memory and precheck time | Met | 120 B per URI, early exit, O(references × levels); memory bounds stated |
| The 12M-index control | Met | 3 payloads of 3,999,999 elements each |
| EXT_mesh_polygon | Met | Only core indices count as drawn, which is consistent with the Claim and the bitmap |
| O-b | Met | Five fields; struct-literal break recorded |

## Residual required change

**H1. Make the per-URI hull the normative domain definition.**

The text currently says two different things:

- `contract-draft.md:327` defines the domain per **(reference, instance)**.
- The precheck (`:491`, `:504-507`) decides `numericSpan`, and the units, from the per-URI
  **`Pair0` entrywise hull** over instances.

The hull is sound, because it is more conservative. But for a URI with more than one instance the
two definitions give different answers.

**Measured case:** two instances with scales 2^-200 and 2^200. Each has a level-0 span of 278 bits,
but the hull span is 678 bits (`hullVsInstance` in the probe output). An "at cap / one over"
control on a multi-instance URI would therefore get different verdicts depending on whether O
followed `:327` or `:504`.

**Fix:**

1. Reword `:327` along these lines: "A reference `r` is in domain iff
   `max(sInst(u), s₀(r), …, s_L(r)) ≤ containmentSpanBits`, computed from `Pair0(u)` as in *Per
   reference*."
2. Add one data control: a two-instance URI where each instance alone is within the cap but the
   hull is over it, with expected outcome `numericSpan`.

No interface change is needed: `UriBounds` and `hull` already exist (`ownership-plan.md:123-126`).

## Correctly still open (none of these blocks F or M)

- D4, D5 and D6.
- O-a, which gates the vector corpus row only.
- The cap values, which R measures.
- The viewer quaternion formulas and the Khronos threshold.
- The optional per-accessor tightening of the position pair, which affects coverage only.
