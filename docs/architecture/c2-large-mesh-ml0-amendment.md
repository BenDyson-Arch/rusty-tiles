# C2 decoded containment: large-mesh tiler amendment (ML0, A-C2-1 and A-C2-2)

Status: **DRAFT for the full (C2-style) treatment.** Written by the ML0 author; not frozen. The
slices require this text to be agreed with the C2-WB and C2 slice-3 owners. Before code it needs
an independent oracle author (O), hashed mutant controls and a fresh nonauthor precode review.
Nothing here records acceptance.

This note amends [c2-decoded-containment-contract.md](c2-decoded-containment-contract.md) (v3.3,
the post-freeze amendments and the C2-WB amendment; sha256 `a0a43e4f…575c`) **as further amended
by C2 slice 3** (frozen v3.1, `/tmp/rusty-tiles-c2s3-contract-20261012/contract-draft.md`
`8bc53429…31d6`, branch `feat/c2-implicit-containment` at `ac1bf94`). Neither text is edited.

**Base:** `origin/develop` `1b1ff9d99fe5e66b844d2c0453abdbe6ed20dd26`, which contains the C2-WB
merge (PR #164) and its WB-6 calibration. **Design:** large-mesh v5 (`design.md` `70a1eeec…afd1`,
§10.4, decision E44; ruling 4); review `698b3b61…c4ef`.

## Composition with C2-WB and slice 3

- **Order of landing.** C2-WB (merged) → slice 3 → ML7a → ML7b → **ML7c implements A-C2-1**.
  Slice 3 consumes "C2-WB's formula and default cap unchanged" (its §5.2). A-C2-1 changes only how
  the default of three caps is computed and what `limits` serializes. It changes no formula, unit,
  fact, enum name, chain, predicate or precedence of slice 1, C2-WB or slice 3.
- **Slice 3's at-cap controls** (§9.D) use explicit `ValidationLimits` through `inspect_selected`.
  Under A-C2-1 an explicit value *replaces* the default, so those pairs keep their meaning.
- **Slice 3's coverage statement and performance obligation** (§5.2, §10) are measured at the
  fixed 2^29 default. A-C2-1 makes archives above `S₀ = 8 MiB` (below) stream where they reported
  `workLimit`, so validation time grows on them by design. ML7c re-derives the coverage statement
  and records every perf-lane `validate` case before and after; growth on a case that moved from
  `workLimit` to streaming is an expected change, not a regression (O-C2-3).

## Unchanged

The claim (slice 1 and slice 3); the domain predicate and the 15-name reason enum (with slice 3's
`derivedImplicitVolume`); spans and `containmentSpanBits`; `containmentNodeDepth`; C2-WB's unit
formula, facts and floors; the precheck order and "first violation stops"; streaming order;
outcome precedence (R8); the Outside message; **no ResourceLimit from any C2 counter**; the
public field names `containmentWorkUnits`, `containmentBytesRead`, `containmentDecodedBytes`.

---

## A-C2-1. Archive-proportional defaults for three caps

**Why.** Rulings (b) and (c): validation never refuses for size and absolute volume caps are
opt-in. A fixed 2^29 units makes every large archive `workLimit`, so C2 would never inspect
large-mesh output.

### Rule

Let `S` be the sum of stored data lengths over the selected central-directory catalog (the same
`S` as A-C1-4's read budget; the stored-3TZ profile has no compression). `S` is known before
traversal, so the caps are fixed before the precheck starts.

| Field | Default after A-C2-1 | Today |
| --- | --- | --- |
| `containmentWorkUnits` | `max(B0, κ_u·S)`, `B0` = 2^29 (WB-6) | 2^29 |
| `containmentBytesRead` | `max(2^31, κ_r·S)` | 2^31 |
| `containmentDecodedBytes` | `max(2^31, κ_d·S)` | 2^31 |

- Products saturate at `u64::MAX` (C2 already treats overflow of a running total as exceeding).
- **Explicit values replace the default.** `--max-volume containment-units=N` (and
  `containment-bytes`, `containment-decoded`; A-C1-4's option) sets that cap to `N`. It is not
  combined with `κ·S`. Over the cap is still `workLimit`, never ResourceLimit.
- **Serialized `limits` carry the effective value** applied to this archive, as numbers. The
  report shape does not change.
- Private controls (`inspect_selected`) may also set `B0` and each `κ` explicitly.

### Constants (WB-6 numbers appended)

- `B0` = 2^29 = 536,870,912, from C2-WB WB-6: the largest power of two with `cap × τ_E ≤ 8 s`,
  with `τ_E` = 14.65 ns/unit (E-core) and `τ_P` = 8.10 (P-core) on the measurement host.
- **`κ_u` = 64 units per stored byte (provisional).** Rule: `κ_u ≥ 2 ×` the largest measured
  producer rate `units / S`, including ML6 outputs. Measured so far: mf2-lod3 5.36 and W5 14.25
  units per stored byte [E-rev, C2-WB probes]; the mesh tiler is projected at about 24 at depth 6
  (non-indexed leaves, D1). Frozen in ML9 (design §13, ML9 "κ_u frozen").
- **`κ_r` = 2 and `κ_d` = 16 (provisional, this draft's proposal; the design gives no value).**
  Same rule: at least twice the largest measured `Σ bytes(r) / S` and `Σ decoded(r) / S` over the
  producer corpus. A payload referenced once is read once, so `Σ bytes / S ≤ 1` for forest output;
  `Dec` counts meshopt-decoded views, whose ratio to stored bytes is not yet measured on
  large-mesh output (O-C2-1).
- **Threshold.** `κ_u·S > B0` exactly when `S > S₀ = B0/κ_u` = 8,388,608 bytes (8 MiB). Archives
  of at most 8 MiB keep the default 2^29 and its serialized value. For bytes and decoded the
  thresholds are 1 GiB and 128 MiB.

### Time bound (replaces the "about 7.9 s" sentence in VALIDATION.md for large archives)

At the cap, phase-2 time ≤ `τ × cap`, as C2-WB states: on the measurement host's E-core,
`max(7.87 s, κ_u·S·14.65 ns)`. With `κ_u` = 64 that is 937.6 ns per stored byte, about **938 s
per 10^9 stored bytes** (P-core 518 s). The design's 929/502 s/GB used the pre-WB-6 τ of
14.51/7.85 and is corrected here. This is a calibration ceiling at the cap, not a typical time:
at the measured producer rates (5–24 units per byte) the projected cost is 73–352 s per 10^9
bytes on an E-core. The per-archive bound now grows with `S` instead of staying at 8 s.

### Exit-status and report changes (complete)

| # | Change | Archives |
| --- | --- | --- |
| XC-1 | `notInspected` `decodedContentBounds.workLimit` → `checks` `decodedContentContainment` | `S` above the threshold, precheck totals between today's cap and the new one, no Outside |
| XC-2 | `workLimit` → **InvalidInput: Outside** | The same, with a proven escape |
| XC-3 | `workLimit` → ResourceLimit (host allocation refusal only) | The same, when the fallible reservations of streaming fail |
| XC-4 | Report: `limits.containmentWorkUnits`, `containmentBytesRead`, `containmentDecodedBytes` | Every archive above the respective threshold |

### Acceptance criteria (O computes `S`, the units and the expected caps from the archive bytes)

- **AC-C2.1 Effective value.** For archives with `S` = 2^23 − 1, 2^23, 2^23 + 1 and one large `S`,
  the serialized `containmentWorkUnits` equals O's `max(2^29, 64·S)` (that is 2^29, 2^29,
  2^29 + 64, and O's value). The same sweep for the other two fields around 1 GiB and 128 MiB.
- **AC-C2.2 Large-archive inspection.** An archive with `S` > 8 MiB whose O units exceed 2^29 but
  are ≤ `64·S` reports `decodedContentContainment` (or the Outside that O predicts). A mutant
  keeping the fixed 2^29 default reports `workLimit`.
- **AC-C2.3 Default function at-cap pairs** (private controls): with `κ_u` = 0 and `B0` = O's
  units: Contained; `B0` = units − 1: `workLimit`. With `B0` = 0 and `κ_u` chosen by O so that
  `κ_u·S ≥ units > (κ_u − 1)·S`: Contained at `κ_u`, `workLimit` at `κ_u − 1`.
- **AC-C2.4 Explicit replaces.** `containment-units=N` with `N` = units − 1 on a large-`S`
  archive whose `64·S` exceeds units: `workLimit`, exit 0. With `N` = units: Contained.
- **AC-C2.5 No ResourceLimit.** Every cap violation yields `notInspected`, never ResourceLimit,
  for each of the three fields.
- **AC-C2.6 Parity.** Every slice-1, C2-WB and slice-3 corpus archive with `S` ≤ 8 MiB gives an
  identical report. The amended C1 limits pins (`c2wb_cap_amendment/2026-10-12`, which expect
  536,870,912) stay valid; O lists each pinned fixture's `S`. For larger `S`, the verdict changes
  only from `workLimit`, and only where O's totals are within the new caps.
- **AC-C2.7 Producer headroom (ML7c, ML9).** On every producer witness, including ML6 outputs,
  units, bytes and decoded are each ≤ ½ of the default caps.
- **Mutants:** fixed default kept; `κ_u` applied to entries or to file length instead of `S`;
  `min` instead of `max`; explicit value combined by `max` with the default; serialized value
  shows `B0` instead of the effective cap; product wrapping instead of saturating.

---

## A-C2-2. KTX2 textures and the shared quantization lattice: no rule change

**Findings.**

- **KTX2 textures.** `KHR_texture_basisu` is already on the C2 glTF allowlist (texture sources
  only). Textures have no effect on drawn POSITION or node transforms. Its C1 admission
  (A-C1-1) is what turns large-mesh archives from Unsupported into C2 candidates. No C2 change.
- **Quantized geometry on the shared per-depth lattice.** C2 decodes `KHR_mesh_quantization`
  integer POSITION words exactly, with normalized words as `a/k` and node T/S as exact rationals
  (frame chain steps 1–2). Lattice positions `translation + scale·u` with a power-of-two `scale`
  are dyadic, so every span stays small (slice 3 §5.1 observed ≤ 290 bits against 4,096). No C2
  change.

**Producer obligations (for ML4, ML5 and ML6 tests; checked by C2 itself on producer output).**
Without these, a correct C2 would report Outside or an exclusion on large-mesh output.

1. **Boxes from exact decoded positions.** Each tile's row box (and the root JSON box) contains
   the exact decoded value of every drawn vertex, after node transforms and `U`. The box is
   stored as center `c` and half-axes `h` in f64, and C2 evaluates `c ± h` exactly. Naive
   `c = (min+max)/2`, `h = (max−min)/2` in f64 can round so that `c + h < max`, which is a proven
   Outside. The producer chooses `c`, `h` and verifies `c − h ≤ min` and `c + h ≥ max` in exact
   (or directed-rounding) arithmetic.
2. **Ancestors contain descendants.** C2 tests every ancestor (interpretation I1), so each
   tile's box contains the union of its own decoded content and every descendant's box.
3. **Boxes after the final geometry.** Computed after quantization, the mixed-depth snap and U1
   vanishing (design B1's required text: offset, u16 range and box after the snap).
4. **No rebase drift.** The P amendment pins δ ≡ 0 for the mesh producer (no child transforms),
   so payloads are byte copies. If δ were nonzero, the rebased translation and the row `B(t)+δ(t)`
   would be rounded separately in f64, and a vertex on the box face could escape by one ulp.
5. **Axis-aligned rows and rows everywhere** (A-I1-3), so neither `rotatedBox` nor
   `derivedImplicitVolume` applies.
6. **Allowlisted extensions only:** `KHR_mesh_quantization`, `EXT_meshopt_compression` (NONE
   filter), `KHR_texture_basisu`, `EXT_mesh_features`, `EXT_structural_metadata`,
   `KHR_materials_unlit`. Anything else (for example `KHR_texture_transform`) excludes the whole
   archive with `gltfExtension`.
7. **Scene records:** one selected scene, no skins, morph targets or animations; node rotations
   absent or within `quaternionWindow`.

**Acceptance (producer slices, light process).** `validate` reports `decodedContentContainment`
on every large-mesh perf-lane archive within caps (ML5, ML6; at scale after ML7c in ML9). Two
regression fixtures: a vertex exactly on a box face at the maximum coordinate, and a box whose
naive f64 center and half-axes would exclude its maximum vertex. Both must be Contained.

---

## Open questions

- **O-C2-1 (C2 owners, design author).** Values of `κ_r` and `κ_d`. The design sets only `κ_u`.
  This draft proposes 2 and 16, subject to measurement on ML6 output.
- **O-C2-2 (C2-WB owner).** Confirm `S` as the sum of stored data lengths over the selected
  catalog (not the archive file length, which includes headers and the central directory).
- **O-C2-3 (coordinator).** Accept that validation time grows by design for archives that move
  from `workLimit` to streaming, and say how the perf-lane gate treats them.
- **O-C2-4 (slice-3 owner).** Add a large-mesh forest witness to slice 3's witness table once ML4
  produces one. Slice 3's corpus has none today.
