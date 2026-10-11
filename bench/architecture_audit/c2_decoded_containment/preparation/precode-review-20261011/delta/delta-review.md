# C2 v2 delta review (R1–R9 follow-up)

Reviewer: the same fresh nonauthor who wrote `../review.md` (sha256 `7bb397dd…`). I did not write v2.

- **Reviewed:** `/tmp/rusty-tiles-c2-contract-20261011/`, read against `v1/` and `change-log.md`.
- **Source:** git blobs at `e83e880fc878ac27a5e9452c3ade60752d56fc44`.
- **Rules followed:** no cargo or rustc, no edits outside `delta/`, single-threaded `python3 -I`
  under `nice -n 10`.

## Verdicts

| Author | Verdict | Condition |
| --- | --- | --- |
| **F** (format, scene, operation) | **GO-WITH-CHANGES** | D1–D4 below are folded into the contract before the interface freeze. They are text and formula additions, not research |
| **M** (math) | **GO-WITH-CHANGES** | B1–B3 below are folded into the contract before the interface freeze. The R1 representation and bound are **sound**; what remains is bit-exact definition work so that O and M agree at the cap |

Nothing found here is NO-GO. R3, R4, R5, R6, R7, R8 and R9 are adequately answered. Spot checks:

- the RTC detection disposition;
- the reason enum;
- the precedence rule in the witness file;
- the meshopt oracle scope;
- the pairing of controls with mutants.

## What was verified

- **Pins.** All 29 git-blob pins in `verify/source-pins-e83e880.txt` match `e83e880`.
- **Manifest.** `verify/manifest-v2.txt` checks clean.
- **Author's probe.** I re-ran `verify/span_bound_probe.py` from a copy (`delta/rerun/`). Its output
  is byte-identical: `04b60bc1…`.
- **Extension declarations.** `payload.rs:353-356` does reject undeclared extension objects. So a
  glTF allowlist over `extensionsUsed` is complete for glTF.
- **POSITION types.** C1 admits only f32 or i8/u8/i16/u16 POSITION (`payload.rs:911-914`), which
  matches the bound's input pairs.
- **D2 basis.** The glTF schema does bound rotation components to [−1, 1]
  (`primary/node.schema.json`), so D2's InvalidInput basis holds.

## Focus 1: is the R1 span bound sound?

### Yes. I could not break it

**My probe.** I wrote an independent implementation of the static bound from the contract text
alone (`contract-draft.md:216-260`), without reading the author's probe code: `delta/bound_adversary.py`,
with output in `delta/bound-adversary-observed.json`.

It checks every canonical quantity, and **every left-to-right prefix of every sum**, against exact
`Fraction` values. The tested condition is `lo ≤ lowbit` and `hi ≥ ⌊log2|v|⌋`.

**Inputs exercised:** 400 adversarial chains. They mix:

- matrix and TRS nodes;
- structural zeros and singular matrices;
- zero and negative scales;
- f64 subnormals (5e-324), 2^500, −2^1000, and values near DBL_MAX/2^40;
- near-zero quaternion components (5e-324, 1e-300, 1e-17);
- general and singular tile transforms;
- box and sphere volumes;
- f32 subnormal and maximum positions;
- the i8 −128 clamp;
- u16/65535 and i16/32767 normalization.

**Result: 251,516 quantity checks, 0 violations.**

**Analytic confirmation:**

- Product: `lowbit(ab) = lowbit(a) + lowbit(b)`, and `|ab| < 2^(hi_a + hi_b + 2)`.
- Sum of m terms: `|Σ| < m·2^(H+1) ≤ 2^(H + 1 + ⌈log2 m⌉)`.
- Prefix sums lie inside the full-sum pair.
- In-domain products never need more than `span_a + span_b ≤ 4096` bits.

So the 4096-bit fixed-capacity mantissa cannot overflow on any in-domain input.

### What is still needed before M's interface freeze

**B1. Make the expression trees normative, bit for bit.** The bound depends on how sums are
associated.

- In `bound-adversary-observed.json`, flat versus nested association of `P = G₃·a + G_t·k` and of
  the tile step differs by **1 bit**.
- An absent-transform node treated as an identity TRS step adds **3 bits** (318 versus 321 at tile
  depth 20).
- The author's probe models producers with **no** node step (`span_bound_probe.py:179`), even
  though every producer GLB has a `{"mesh":0}` node (`glb.rs:312-321`).
- The planned "span at cap and one over" controls (`slice1-witness-expectations.json`
  newSlice1Controls; ownership-plan M tests) therefore need O and M to agree to the bit.

State the following in the contract:

- (a) Every sum is flat over its written terms:
  - `P_r` is a 4-term sum;
  - the tile-step row is a 4-term sum;
  - a `G·L` entry is a 4-term sum;
  - `n` is a 4-term sum;
  - an H diagonal is a 4-term sum;
  - an H off-diagonal is `2·(2-term sum)`;
  - the sphere left-hand side is a 3-term sum of squares.
- (b) A node with no `matrix` and no TRS fields is a TRS step with default values. Alternatively,
  state that it is skipped; either way, say which.
- (c) Exact-zero JSON inputs, including `-0.0`, are ZERO.
- (d) The D2 window evaluation `Σq² − 1` is outside `numericSpan`. Its span is at most 2151 bits
  for any f64 q, so it always fits.

Making `span_bound_probe.py`'s bound functions normative pseudo-code would also satisfy B1.

**B2. Spheres halve the admitted rotated depth.** The contract's coverage statement (16 generic
or 12 near-axis levels, `contract-draft.md:266-269`) holds for boxes only.

- The author's probe measures only box operands (`span_bound_probe.py:159-162`).
- The sphere test squares `P − cW`, which doubles the span.
- In `bound-adversary-observed.json`, generic depth 10 is 1974 bits for a box against 3952 bits
  for a sphere, and depth 12 is 2290 against 4582. So with a sphere anywhere in the chain, about
  10 generic levels are admitted.

Separately, a single admissible quaternion with a subnormal-size component (for example 1e-300)
puts even a depth-4 chain at about 9000 bits, which gives `numericSpan`.

Both cases are sound (not inspected), but state them in the Expected W1 coverage paragraph.

**B3. Keep large dyadic containers off the stack.** At 4096 bits, every dyadic occupies at least
512 B. Sizes:

- the composite stack (64 × 16 dyadics) is about 0.5 MiB;
- a `Chain` holding dyadics for up to 128 levels would be several MiB.

`validate` runs on Python threads through `py.detach` (`bindings/python/src/lib.rs:976-981`) and
inside Blender. On a 2 MiB thread stack, a fixed-size array of this size aborts the process
instead of returning a typed outcome.

The contract already maps "host allocation refusal for the bitmap or chain" to ResourceLimit
(`contract-draft.md:352-353`). Say explicitly that the composite stack and chain are fallible heap
allocations, and that only O(1) scratch dyadics live on the stack. This is cheap to fix now and
expensive later.

**O can compute the predicate independently** from the text once B1 is in. It uses only JSON
inputs, and the FLOAT pair is a fixed worst case.

## Focus 2: do the R2 caps bound worst-case time and memory per archive?

### What R2 got right

- **Per-vertex arithmetic is bounded.** The walk is single-pass. Units
  `Σ_refs Σ_instances Σ_accessors count × Σ_levels ⌈s/64⌉²` bound it up to a constant. The
  "≈16 limb multiplications per unit" figure is approximate. From the canonical formulas I count:
  - about 18·L per box level;
  - about 22·L² per sphere level.

  State the constant as "c ≤ 24", or have R measure it. This does not block.
- **The bitmap is bounded.** It needs at most 4M bits, about 0.5 MiB. In fact C1's per-payload
  `element_cap` (`payload.rs:415`) means `accessor.count ≤ 4M/3` for VEC3. Say that **one**
  reusable bitmap is alive at a time.
- **Cap values and ownership are explicit.** The values are 4096, 2^32, 2 GiB and depth 64. Each is
  explicitly deferred to R for measurement (`open-decisions.md:153-158`), and is retunable without
  changing semantics. That is acceptable.

### Not yet bounded: three work sources outside the units (required)

**D1. Node visits and composite products are not charged.** Phase 2 re-walks the node tree per
reference, and each node step is a 4×4 product on dyadics. Mesh-less nodes contribute zero units.

An adversarial archive shows the gap:

- a payload of about 32k mesh-less or 1-vertex nodes (C1 allows `value_nodes` = 65,536 per
  document, `content_integrity.rs:37`);
- referenced up to 262,144 times (C1 `references`);
- that gives roughly 8.6e9 node steps × 64 dyadic products, almost none of it in units.

**Fix:** charge `Σ_refs Σ_visited_nodes 64·⌈s_G/64⌉·⌈s_L/64⌉` (or a simpler conservative per-node
unit) in the precheck. Alternatively, build composites once per URI and charge them once. Either
way, state the memory this requires.

**D2. Index scanning is not charged.** Deduplication bounds the *tests*, but marking the bitmap
still scans every index occurrence per (reference, instance, primitive). Units count only
POSITION `accessor.count`.

An adversarial archive: a 1-vertex POSITION with a near-4M-element index accessor, drawn by
thousands of node instances, referenced many times. The scan work is unbounded by
`containmentWorkUnits`.

**Fix:** add `Σ_refs Σ_instances Σ_primitives (index count)` to the precheck. Alternatively,
compute the referenced-vertex set once per (reference, mesh primitive) and reuse it across
instances.

**D3. Re-decode and C1 re-validation per reference are bounded only indirectly.** Phase 2 re-runs
the payload decode for each reference. `containmentBytesRead` (2 GiB) bounds the raw bytes. The
meshopt expansion ratio and C1's whole-document value scan per re-decode can still multiply the
work.

Required: either

- charge Σ per-URI decoded bytes and C1 elements per reference in the precheck (phase-1 facts
  already exist); or
- state and justify the maximum decode work per re-read byte.

**D4. The precheck and its retained facts need their own bound.**

- "Per-URI static facts (instance node bounds, accessor counts)" (`contract-draft.md:393`), if
  kept per instance for every unique URI, has no stated memory bound. The worst case is about
  65,536 URIs × about 32k instances × 16 pairs.
- Evaluating the precheck per (reference, instance, level) has no stated time bound.

Required:

- aggregate the static facts per URI (for example, an entrywise max pair over instances plus
  Σ counts, which is still sound and only more conservative); or
- state a cap, and make the precheck loop exit early once the running total passes
  `containmentWorkUnits`.

### Definitions to freeze together with B1

So that the "work units exactly at cap / one over" control is oracle-checkable:

- define `s` per level as the maximum over an enumerated set of quantities (P after the step, cW,
  hW, the differences, the squares);
- define the level-0 contents (`G` entries, the G·a terms);
- define whether two primitives sharing one POSITION accessor in an instance count once.

### Non-blocking notes on R2

- The control "indexed mesh with 9M index occurrences" is not constructible in one payload. C1's
  per-payload component cap is 4M (`payload.rs:415`, `accessor_elements`). Specify it as at least
  3 payloads, or as near-4M per payload.
- Positions count at the worst-case FLOAT pair (277 bits). That makes producer spans about 300 bits
  (6 limbs, u = 36), against an exact span of about 61 bits (one limb). The contract's own estimate
  is 3.8e9 units against the 4.29e9 cap for a 5.3M-point depth-20 cloud (`contract-draft.md:332-335`),
  which is a thin margin. Explicit point-cloud depth may reach 64 (`point_cloud/tiles.rs:151`).
  - Option: tighten the position pair per accessor using exponents C1 already scans
    (`payload.rs:707-735`). That would be deterministic and oracle-computable, and would give about
    9× headroom.
  - Not required for the code to start: it only affects how many archives are covered.

## Focus 3: does I3 or the new reasons hollow out the check?

**No, for ordinary explicit producer output.** I checked each reason against the producer source
at `e83e880`. In the table, "Inventory" refers to `producer-inventory.md` "v2 slice-1 coverage
prediction".

| Output | Reasons that could fire | Source evidence | Prediction |
| --- | --- | --- | --- |
| Point cloud `--explicit` (Inventory row 2) | none | glTF uses `EXT_mesh_features`, `EXT_structural_metadata`, `KHR_materials_unlit` (`point_cloud/tiles.rs:401-405`), all allowlisted. One node, `scene:0` (`glb.rs:312-321`). No tile or content `metadata`, no tileset extension | Covered |
| Vector `--explicit`, plain, `--quantize` and `--meshopt` (row 4) | `tilesetExtension` only if O-a drops the entry | Tileset `extensionsUsed:["3DTILES_content_gltf_vector"]` (`store.rs:970`) and **content-level** `extensions.3DTILES_content_gltf_vector` (`encoding.rs:960`). The allowlist applies by name at every level (`contract-draft.md:96`), so both are fine. glTF extensions all allowlisted (`encoding.rs:451`). No RTC (`BATCH_LENGTH:0`) | Covered, subject to O-a |
| F1c2/F1d mesh (row 6) | none | `EXT_mesh_features` + `EXT_structural_metadata` only (`mesh_archive/encode.rs:288-297`, `:493-505`) | Covered |
| W1 local / Wgs84 (row 7) | `numericSpan` for deep rotated user nodes | W1 admits **no** root `extensionsUsed`, `animations` or `skins` (`mesh_archive/source.rs:1222-1237`), so `gltfExtension`, `skin` and `animation` cannot fire. `quaternionWindow` cannot fire because W1 ≤ 2.000001e-6 < 2^-18 | Covered up to the B2 depth |
| Legacy `mesh-to-3tz` explicit (row 8) | none expected | `EXT_meshopt_compression`, `KHR_texture_basisu` and `EXT_texture_webp` (`lossless.rs`, `glb_write.rs`) are all allowlisted | Covered, or Outside per the inventory |
| Point/vector **default** (implicit) (rows 1 and 3) | `implicitTiling` | D3(a), decided | **Not covered.** This is the real, documented gap: default point and vector output gets no claim until slice 3. Release notes must say so (D3 slice-3 gate) |
| Terrain (row 5) | — | A directory output, which `inspect` cannot reach (`validate.rs:757`) | Not reachable. This predates C2 and is not a C2 regression |

**I3's actual cost** is Outside detection inside *excluded* archives, which is disclosed at
`contract-draft.md:84` and `change-log.md:79-80`. For producer output the exclusions are
homogeneous across the archive, so I3 does not remove coverage that per-reference streaming would
have given.

**One inconsistency, non-blocking.** `EXT_mesh_polygon` primitives test **every** POSITION vertex
(`contract-draft.md:127`, `:293`). That is stricter than the Claim's "every vertex that the
selected static scene draws" (`:30`). An undrawn, out-of-volume vertex in a polygon primitive
would therefore be InvalidInput. Either:

- state the polygon superset in the Claim and in the exit-status list; or
- mark exactly the triangle and loop indices (C1 already reads the polygon records,
  `records.rs:45`).

## Focus 4: are the remaining open items correctly left open?

| Item | Assessment | Blocks F? | Blocks M? |
| --- | --- | --- | --- |
| D4 `hierarchyBounds` | Correctly open. It is unchanged in slice 1, and a capability decision for the coordinator | No | No |
| D5 rotated boxes | Correctly open. `rotatedBox` exclusion, and no producer writes them | No | No |
| D6 RTC precision | Correctly open. Slice 1 needs detection only, which is specified (F reworks `records.rs:54`, `unwrap_b3dm`) | No | No |
| O-a `3DTILES_content_gltf_vector` README | Correctly open. It gates the vector corpus row and coverage, not code | No | No |
| O-b cap placement | Correctly open, and R-owned. It must be settled before F edits `types.rs` (`open-decisions.md:229`) | Only for `types.rs` | No |
| Cap values | Correctly deferred to R measurement. The **formulas** are not deferrable (B1, D1–D4), because they decide `workLimit` / `numericSpan` at the boundary | No (formulas: yes, via D1–D4) | No (formulas: yes, via B1) |
| Viewer quaternion formulas / Khronos threshold | Correctly open. Tightening the window only narrows coverage and never adds a failure (`open-decisions.md:90-91`). The 3.8 m worst-case figure is in D2 | No | No |

None of the open items needs to be decided before coding starts. The blocking work is the
definitional text in B1–B3 and D1–D4, done at the interface freeze.

## Summary of required changes

**For M:**

- **B1.** Normative expression trees: flat sums, identity-node handling, ZERO rule, and D2 window
  scope.
- **B2.** Sphere and subnormal coverage statement.
- **B3.** Heap placement of the composite stack and chain.

**For F:**

- **D1.** Charge node and composite work.
- **D2.** Charge index scanning, or share the referenced set.
- **D3.** Bound per-reference re-decode work.
- **D4.** Aggregate the per-URI static facts and bound the precheck.

**Both (with R):** freeze the per-level `s` and the units definitions so that the at-cap controls
are bit-exact.

## Artifacts (under `/tmp/rusty-tiles-c2-precode-review-20261011/delta/`)

- `bound_adversary.py` and `bound-adversary-observed.json`: the independent bound check and
  sphere/association measurements.
- `rerun/span_bound_probe.py` and `rerun/span-bound-observed.json`: reproduction of the author's
  probe (byte-identical).
