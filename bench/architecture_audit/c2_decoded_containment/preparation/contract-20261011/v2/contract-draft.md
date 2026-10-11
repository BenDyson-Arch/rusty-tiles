# C2 decoded-content containment contract (draft v2)

## Status

This is decision-ready preparation, revised after a fresh nonauthor precode review that returned
GO-WITH-CHANGES. The review is at `/tmp/rusty-tiles-c2-precode-review-20261011/review.md`,
sha256 `7bb397dd…`. v1 is preserved unchanged under `v1/`. The map from each required change
R1–R9 to the text that answers it is in [change-log.md](change-log.md).

This lane did not do any of the following:

- edit production source;
- run Cargo, rustc, a native build or the CLI;
- run a producer.

The source pin is commit `e83e880` (the final head of PR #155). Every file cited here has the
same content at `e83e880` and at the earlier pin `70d184f`, with one exception: `docs/CLI.md`
changed only in a raster progress row, which nothing here cites. The pins are in
[verify/source-pins-e83e880.txt](verify/source-pins-e83e880.txt). They were computed from git
blobs, not from the moving worktree. Line numbers refer to `e83e880`. Implementation starts
from `develop` after PR #155 merges.

## Claim

When `checks` contains `decodedContentContainment`, the archive satisfies all of the following:

- It is entirely inside the slice-1 domain (see the domain predicate).
- For every content reference to a tile payload (a non-`.json` URI), the payload was decoded
  from the actual archive bytes.
- Every vertex that the selected static scene draws was tested. This covers every core node
  instance and every POSITION accessor the instance uses.
- Each such vertex, as an exact real value of its represented inputs, lies in the closed set of
  each of these volumes, evaluated in that volume's own frame:
  1. its own tile's `boundingVolume`;
  2. the `content.boundingVolume` of that reference, if present;
  3. the `content.boundingVolume` of every external-tileset reference through which the
     payload is reached (decision I2);
  4. every ancestor tile's `boundingVolume`, including ancestors reached through
     external-tileset roots (decision I1).

The claim does not cover:

- source fidelity or LOD error;
- rendered extent (point size, line width, shading, draping);
- GPU f32 dequantization;
- skins, morph targets or animation;
- polygon topology.

Boxes and spheres are convex, so containing the vertices also contains the core segments and
triangles between them.

Normative basis (`3d-tiles-core.adoc`):

- line 86 and line 752: "content for child tiles are completely inside the parent's bounding
  volume. This does not imply that a child's bounding volume is completely inside its parent's";
- line 167: a content volume encloses its content;
- line 790: "a parent tile completely encloses all of its children";
- line 835: hierarchical culling.

`hierarchyBounds` (`validate.rs:643-650`, `:679-685`) is a different and stronger declared-volume
check. It is unchanged in slice 1 (D4).

### Recorded interpretations (R5)

- **I1, ancestors are transitive.** "Inside the parent's bounding volume" is applied to every
  ancestor, not only the direct parent, so an Outside against a grandparent fails the archive.
  - Basis: line 790 (the parent encloses all of its children) and line 835 (culling a parent
    culls its subtree). Correct culling requires every ancestor to enclose the content.
  - Evidence it rests on: these quoted passages. The oracle applies the same reading.
- **I2, content volumes of external-tileset references participate.** A
  `content.boundingVolume` on a tile whose content is a `.json` external tileset encloses that
  content, which is the whole external tileset (`:167`). Every payload reached through the
  external tileset is therefore also tested against it, in the referencing tile's frame.
  - The slice-1 control is in
    [verify/slice1-witness-expectations.json](verify/slice1-witness-expectations.json).
  - The oracle corpus records whether any producer writes such volumes. The static search found
    none.
- **I3, coverage is per archive, all or nothing (replaces v1's partial streaming).** The archive
  either receives the claim for every reference or receives no C2 streaming at all. Under v1,
  some references in an excluded archive would still have been streamed.
  - Why: this removes per-(reference, volume) coverage semantics. Examples of the ambiguity it
    removes: a region ancestor above an in-domain box; explicit ancestors above an implicit
    root.
  - Cost: an excluded archive is no longer searched for Outside.
  - Partial streaming is a later option and needs its own contract delta.

## Slice-1 domain predicate

An archive is **in domain** when C1 succeeds, no exclusion reason below applies anywhere in it,
and the static precheck passes. Everything in the predicate is evaluated from tileset JSON and
admitted glTF/b3dm JSON, before any C2 position streaming.

| Exclusion reason (frozen enum) | Applies when |
| --- | --- |
| `implicitTiling` | Any tile, in any tileset document, has `implicitTiling` |
| `tilesetExtension` | Any extension name is used or present in tileset `extensionsUsed`/`extensionsRequired` or on a tileset, tile, content or boundingVolume `extensions` object and is not on the tileset allowlist |
| `tileMetadata` | Any tile has `metadata`, or any content has `metadata` |
| `upAxisOverride` | Tileset `asset.gltfUpAxis` is present and is not `"Y"` |
| `regionVolume` | Any tile or content volume tested for any reference is a `region` |
| `rotatedBox` | Any tested box has half-axes that are not a signed permutation (see Volumes) |
| `rtcCenter` | A b3dm feature-table JSON contains the key `RTC_CENTER`, in any form or with any value |
| `gltfExtension` | A glTF `extensionsUsed` entry is not on the glTF allowlist. This includes `CESIUM_RTC` and `EXT_mesh_gpu_instancing` |
| `sceneSelection` | `scene` is absent and `scenes` is absent, empty, or has more than one entry |
| `skin` | A selected node has `skin` |
| `morphTargets` | A selected mesh primitive has `targets`, or a selected node or mesh has `weights` |
| `animation` | A nonempty `animations` array exists |
| `quaternionWindow` | A selected node rotation has `|‖q‖² − 1| > 2^-18`, evaluated exactly (D2) |
| `numericSpan` | The static bound for any quantity of any (reference, instance) chain exceeds `containmentSpanBits` (Exact evaluation) |
| `workLimit` | A precheck total exceeds its cap (Work bound), or the selected node depth exceeds `containmentNodeDepth` |

Retained source payloads (`rustyTilesSourceContents`, `validate.rs:560-571`, and
`retainedContentUris`, `:908-919`) are not tile content. C2 neither tests them nor excludes the
archive because of them.

### Allowlists (R7)

The glTF allowlist names extensions that C2 interprets or that have no effect on drawn POSITION
or node transforms:

| Extension | Rationale |
| --- | --- |
| `KHR_mesh_quantization` | Changes POSITION component types. C2 consumes the exact dequantized meaning (normalized rationals, node T/S) |
| `EXT_meshopt_compression` | Compresses buffer views. C1 decodes NONE-filter views (`payload.rs:249-290`; other filters C1-Unsupported at `:271-272`). C2 reads the decoded bytes |
| `KHR_mesh_primitive_restart` | Index-stream semantics, admitted by C1's pinned profile. Restart values are not vertices |
| `EXT_mesh_features` | Feature-ID attributes and textures; no geometry |
| `EXT_structural_metadata` | Property tables and attributes; no geometry |
| `EXT_mesh_polygon` | Polygon loop indices into the same POSITION accessor. Because loops can name vertices that no triangle draws, C2 tests **every** POSITION vertex of a primitive that carries it. That is a superset |
| `KHR_materials_unlit` | Shading only |
| `KHR_texture_basisu`, `EXT_texture_webp` | Texture sources only. Used by legacy routes (`lossless.rs:60-67`, [A]) |

The tileset allowlist has a single entry: `3DTILES_content_gltf_vector`, written by vector at
`store.rs:970` and `encoding.rs:960`.

- The entry is provisional. O must pin the README at revision `c48ebdc` (cited at
  `docs/VECTOR.md:307`) and confirm that it defines no transform of decoded positions.
- If O finds positional semantics, the entry is dropped. Default explicit vector archives would
  then become `tilesetExtension`.
- Everything else is excluded, including `3DTILES_bounding_volume_S2`, other
  `3DTILES_bounding_volume_*` extensions, `3DTILES_implicit_tiling` and
  `3DTILES_multiple_contents`.

### Volumes

- **Box.** Allowed when each half-axis column has at most one nonzero entry and the nonzero
  entries lie in distinct rows (diagonal or signed permutation).
  - Row `j` has half-length `|entry|` from the column that covers it, or 0 if no column covers
    it.
  - Membership is `|p_j − c_j| ≤ h_j` for each row.
  - Zero half-lengths are exact planes, lines or points.
  - Every producer writes boxes of this shape.
  - General oriented boxes are `rotatedBox`. They move out of slice 1, together with D5 (review
    note 7).
- **Sphere.** Membership is `Σ(p_j − c_j)² ≤ r²`, with `r ≥ 0` (C1).
- **Region.** Excluded.
- C1's existing box-orthogonality admission (`validate.rs:240-249`) is unchanged.

### glTF records (InvalidInput when malformed; checked for every payload, R2d)

**Scene and node graph.**

- `scene` must index into `scenes`.
- `scenes[*].nodes` and `nodes[*].children` must be valid node indices.
- The graph must be a strict tree:
  - no node has two parents;
  - there are no cycles;
  - scene roots have no parent.
- `mesh` must be a valid index.

**Node fields.** Each field must have the correct type and length when present:

- `matrix` must have 16 finite entries whose last row is exactly `0,0,0,1`. It must not coexist
  with `translation`, `rotation` or `scale`.
- `rotation` must have finite components in [−1, 1] and be nonzero.
- `translation` and `scale` must be finite.

**Numbers.** Every number is the nearest binary64 value of the JSON text. A literal that
overflows binary64 is InvalidInput, as in the existing tileset number handling (`validate.rs:62-82`).

**Not checked.** Matrix TRS-decomposability and local no-shear are not policed (D2(iii)).

## Frame chain

For each instance, take the selected node path `n_1 … n_m`, with `n_1` the scene root, and a
drawn vertex with raw value `a`:

1. **Decode.**
   - FLOAT values become exact f32. Integer types stay exact integers.
   - Normalized values become `a/k` with `k ∈ {127, 255, 32767, 65535}`.
   - For signed normalized types, raw values at or below `−k` become exactly `−1`.
2. **Node frame.** `y = L_1 · … · L_m · x`, where `L = T·R(q)·S` or the column-major matrix, and
   `R(q) = H(q)/‖q‖²` (D2).
3. **Up axis.** `t = U(y)`, with `U(x,y,z) = (x, −z, y)`, applied exactly once.
4. **Test own and content volumes.** Test `t` against the own volume and the reference's
   content volume. Then apply the tile's own `transform`, if it has one, to reach the parent
   frame. Test any external-reference content volume at that level (I2), then the parent volume,
   and so on upward.
   - An ancestor's own transform is never applied before testing its volume.
   - External-tileset root transforms are steps in the chain.
   - Nothing is inverted, so singular transforms are fine.
   - The root placement transform enters no predicate unless an external tileset places that
     root.

## Exact evaluation (R1)

**Representation decision.** C2 uses arbitrary-precision dyadic integers with a statically
checked bit-span cap, not binary64 expansions.

Why:

- The review's `span_probe.py` showed binary64 expansions underflow at 4–8 rotated levels.
- This lane's [verify/span_bound_probe.py](verify/span_bound_probe.py), with output in
  `span-bound-observed.json`, confirms the exact span grows by about 160–320 bits per rotated
  level.
- The prior claims of "≤ about 40 components" and "exact at depth 128" are withdrawn.

### Canonical formulas

Every intermediate value is the exact result of these formulas. M and O both implement them, and
they also define the static bound:

- **Node step**, building the homogeneous 4×4 composite `G` root-first:
  - Matrix node: `G ← G·[[M, t], [0, 1]]`.
  - TRS node: `G ← G·[[H(q)·diag(s), n·t], [0, n]]`, where `n = Σq²` and
    `H(q) = [[w²+x²−y²−z², 2(xy−wz), 2(xz+wy)], [2(xy+wz), w²−x²+y²−z², 2(yz−wx)], [2(xz−wy), 2(yz+wx), w²−x²−y²+z²]]`.
- **Position:** `P = G₃·a + G_t·k`, `W = g·k`, where `g = G₄₄ > 0` and `k = 1` for unnormalized
  values.
- **U:** `P ← (P₀, −P₂, P₁)`.
- **Tile step:** `P ← T₃·P + T_t·W`. `W` is unchanged.
- **Box test:** `|P_j − c_j·W| ≤ h_j·W`.
- **Sphere test:** `Σ(P_j − c_j·W)² ≤ r²·W²`.

Every value here is a dyadic rational, because each input is a binary64, f32 or integer value and
only `+`, `−` and `×` appear. `W > 0` holds, so no comparison flips sign.

### Static bound

Each scalar carries a pair `(lo, hi)` bounding the exponents of its set bits, or the marker ZERO.
The rules:

| Operand | Pair |
| --- | --- |
| Exact input `x ≠ 0` | `(lowbit(x), ⌊log2|x|⌋)` |
| Exact input `0` | ZERO |
| FLOAT position | `(−149, 127)` |
| Raw i8/u8 | `(0, 7)` |
| Raw i16/u16 | `(0, 15)` |
| `k` | Its exact pair |
| Product | `(lo_a + lo_b, hi_a + hi_b + 1)`; ZERO absorbs |
| Sum of `m` nonzero terms | `(min lo, max hi + ⌈log2 m⌉)`; ZERO terms are dropped |
| Negation | Unchanged |

The **span** is `hi − lo + 1`.

**Domain.** A (reference, instance) chain is in domain iff every quantity in the canonical
formulas, evaluated with these rules over that chain's actual non-position inputs, has span at
most `containmentSpanBits`. That includes the composite entries, `P`, `W`, every tile step, and
the box/sphere operands including their squares. The proposed cap is **4096**.

O can compute this predicate from the contract text alone. It depends only on the JSON inputs,
never on position values.

What the probe found for this cap:

- Producer chains reach a bound span of 281–327 bits for point and vector chains (identity node,
  translation tile chains up to depth 24), and 83–112 bits for `--quantize`.
- Rotated TRS chains are admitted up to **16 generic or 12 near-axis levels**.
- Each exceeding chain is `numericSpan` (not inspected). It is never a failure.

**Expected W1 coverage.** W1 user models with at most 12 rotated nesting levels are covered.
Deeper rotated hierarchies get `numericSpan`. The exact count depends on the model, and the
oracle corpus records it.

**Rust representation (M).** A hand-rolled, fixed-capacity, sign-magnitude dyadic type:

- The capacity is one 4096-bit mantissa (64 × u64 limbs), plus a binary exponent.
- There is no heap allocation and no new dependency. `num-bigint` is only transitive today.
- Because the static bound holds, every intermediate fits. Overflowing capacity is an internal
  invariant violation, not an input outcome.
- An implementation may use a narrower fast path (for example i128 when the bound span is at most
  120 bits). Both paths decide the same exact predicate, so this is not a filter or fallback.

**D2 window.** The comparison `|‖q‖² − 1| ≤ 2^-18` is evaluated with the same exact arithmetic.
Subnormal squares cannot occur, which addresses the review's note under R1.

## Work bound (R2)

**Distinct vertices.**

- Each (instance, POSITION accessor) pair tests each **distinct referenced vertex** once.
- A bitmap of `accessor.count` bits records which vertices have been tested. That is at most C1's
  `accessorElements`, so 4M bits.
- Indexed draws mark only referenced indices. Restart values are skipped. Primitives with
  `EXT_mesh_polygon`, and primitives with no indices, mark every vertex.
- Repeated indices therefore never multiply the work, so the 9M-index case cannot turn a passing
  mesh into a failure.
- Undrawn vertices are still never tested, so the "visit undrawn vertices" control still bites.

**One upward walk per vertex.** For each tested vertex, the instance composite `G` is applied
once, then U, then the walk climbs the levels. At each level it tests the volumes, then applies
one tile step. Per-vertex cost is O(levels), not O(levels²).

**Units.** For each (reference, instance, level), the static per-test cost is
`u = ⌈s/64⌉²`, where `s` is the maximum static span at that level. The precheck total is

  `Σ_references Σ_instances Σ_accessors accessor.count × Σ_levels u`.

This is an upper bound, because deduplicated distinct vertices never exceed `accessor.count`. Each
unit stands for at most about 16 limb multiplications: one 3×3 matrix-vector product, the
translation and the predicate. Total C2 arithmetic is therefore at most about
`16 × containmentWorkUnits` limb multiplications. The per-unit time must be measured by R; it is
not claimed here.

**Caps.** All caps are checked in the precheck after C1 traversal and before any C2 streaming,
using phase-1 facts retained per unique URI. Exceeding any of them gives `workLimit` (not
inspected), never ResourceLimit.

| Cap | Proposed value |
| --- | --- |
| `containmentSpanBits` | 4096 (static; drives `numericSpan`) |
| `containmentWorkUnits` | 2^32 |
| `containmentBytesRead` | 2 GiB. Covers phase-2 re-reads (Σ member size per reference). It is **separate from** C1's `totalBytesRead`, which is not charged again |
| `containmentNodeDepth` | 64. Bounds the composite stack: ≤ 64 × 16 fixed dyadics |

Two C1 counters are untouched:

- C1's `total_payload_elements` is **not** charged again by phase-2 re-streaming.
- C1's per-URI cache keeps integrity facts only.

Each reference re-streams its payload. This is serial, with one decode alive at a time, under
C1's existing per-document decoded-byte limits.

**Producer-scale check.** A point cloud with 5.3M positions (C1's 16M-component ceiling), depth
20, and bound span ≤ 330 (6 limbs, u = 36) gives ≈ 3.8e9 units. That is within 2^32. The
oracle corpus must confirm that no realistic producer output reaches `workLimit`, or record each
one that does.

## Outcome precedence (R8)

The order is deterministic:

1. **C1 outcomes.** Existing order and meaning, including `hierarchyBounds`.
   - Within each payload inspection, C1 integrity runs first. The C2 record admission for that
     payload follows (malformed scene/node → **InvalidInput**).
   - Across payloads, the first failure in traversal order wins.
   - Exclusion facts are collected but do not stop traversal.
2. **Exclusions.** After traversal succeeds, any exclusion reason gives the result **C1 success
   with `notInspected` reasons**. All applicable reasons are listed, sorted and deduplicated.
   There is no streaming.
3. **Precheck.** `numericSpan` / `workLimit` give not-inspected.
4. **Streaming.** Phase 2 runs serially per reference in traversal order, then per instance,
   accessor and vertex index. Within a vertex, volumes are tested own → content → upward. The
   first proven Outside is **InvalidInput**. Host allocation refusal for the bitmap or chain is
   **ResourceLimit**; these are bounded by C1-admitted counts.
5. Otherwise the result is **Contained**.

## Report and failure schema (frozen before code; R7.4)

- **`checks`** gains `"decodedContentContainment"` only for outcome 5.
- **`notInspected`:**
  - Keeps `"decodedContentBounds"` unless the outcome is 5.
  - For outcomes 2 and 3, adds one entry `"decodedContentBounds.<reason>"` per reason, using
    the frozen 15-name enum above.
- **C1 payload label (R7.1).** The string `"scene transforms, skins, animations, morph targets
  and rendered bounds"` (`payload.rs:1102-1103`) splits into two entries:
  - `"skins, animations, morph targets and rendered extent"`, always inserted;
  - `"scene transforms"`, inserted per payload and removed from the aggregated set by
    `validate.rs` only for outcome 5.
- **Outside message**, a fixed template:
  `decoded content outside bounding volume: content=<archive member name> depth=<u> volume=<own|content|externalContent:<k>|ancestor:<k>> node=<u> mesh=<u> primitive=<u> vertex=<u>`.
  - The member name has already been admitted by C1 (C1 errors already carry member names).
  - No other source strings appear, and no float values.
- **Node-admission messages.** Use fixed text plus indices only.
- **`ValidationLimits`.** Gains the four `containment*` fields. This is a **public Rust API
  change**: it is a `pub` struct with `pub` fields and no `#[non_exhaustive]` (`types.rs:50-66`).
  - It breaks external struct literals.
  - No public function takes `ValidationLimits` as input.
  - CLI and Python serialize it unchanged (`bindings/python/src/lib.rs:976-981`).

## Exit-status changes versus today (complete list, R2d)

| Change | Archives affected |
| --- | --- |
| New **InvalidInput: Outside** | In-domain archives with a proven escape. Static predictions: point/vector edge cases with zero rounding padding; legacy mesh routes. The oracle corpus confirms or refutes each |
| New **InvalidInput: malformed scene/node records** | **Any** archive, including excluded ones, because admission runs in phase 1. C1 does not read nodes today |
| New **ResourceLimit** | Host allocation refusal only. No new counter produces ResourceLimit |
| Other behaviour | No change to Unsupported, Io, `totalBytesRead`, `total_payload_elements` or `hierarchyBounds` |
| Report schema | `checks` / `notInspected` entries, the payload-label split, and the `ValidationLimits` fields |

## Ownership of representation

| Owner | Responsibilities | Excludes |
| --- | --- | --- |
| **Format** (`content_integrity/payload*`, F) | Records; phase-1 admission and exclusion facts; RTC_CENTER key presence; per-URI static facts (instance node bounds, accessor counts and kinds); phase-2 event stream over live C1 views | Positions `Vec`, second decoder, math, policy |
| **Math** (`validate/containment*`, M) | Dyadic type, static bound, composite, walk, predicates | `serde`, I/O, `Value` |
| **Operation** (`validate.rs`, F) | Traversal level chain with volumes, content and external-content volumes, and transforms; precheck; streaming per reference; bitmap; label and reason assembly | — |

The interface is in [ownership-plan.md](ownership-plan.md).

## Disposition of existing code

| Component | Decision | Basis |
| --- | --- | --- |
| `StoredArchive`, `read_member`, reference budget, JSON admission, exact numbers | Retain | Accepted owners. C2 re-reads are charged to the new `containmentBytesRead` |
| `payload::inspect` (`payload.rs:371-1112`) | Rework | Phase-1 facts, exclusions and the event stream. `normalized()` (`:134-140`) returns a rounded f64, so C2 takes the raw integer plus a tag instead |
| `payload/records.rs:37` `Root`, `Mesh`, `Primitive`, `:54` `FeatureTable` | Rework | Add `scene`/`scenes`/`nodes`/`skins`/`animations`, node fields, `weights`, `targets`; add `RTC_CENTER` key presence (R3) |
| `unwrap_b3dm` (`payload.rs:292-337`) | Rework (F) | Return feature-table RTC_CENTER presence alongside the GLB slice. It is detection only; the range is not consumed in slice 1 |
| `check_extensions` and the `supported` list (`payload.rs:340-369`, `:396-401`) | Retain | C1 guarantees that any extension object is declared (`:355-360`), so the C2 allowlist over `extensionsUsed` is complete |
| C1 label at `payload.rs:1102-1103` | Rework | Split as specified above |
| `Check::payload` URI cache (`validate.rs:402-404`) | Retain for integrity facts only | Adds cached phase-1 static facts. Never caches a verdict |
| `ValidationTask` parent volume (`validate.rs:385-400`) | Rework into a level chain | Every level and transform is needed |
| `transform`, `point`, `scale_bound`, `Volume`, `inside`, `contains` (`validate.rs:197-338`) | Not used by C2 | Slack, inflation and rounded points. Retained for `hierarchyBounds` until D4 |
| Implicit expander (`validate.rs:575-608`; `implicit/tileset.rs:619-791`, `:1133-1135`) | Not used by C2 | D3: implicit is excluded |
| Producer bound code (`original_bounds.rs`, `partition.rs`, `placement.rs`, `terrain.rs:322-331`, `tileset_node.rs`, `bbox.rs`, `raster/grid.rs`) | Not a dependency, and M must not read it | The checker must stay independent of the writers it checks (#125) |
| `docs/architecture/public-surface-inventory.md:40` | Update (R) | It says "decoded world-content bounds remain unclaimed". New wording: "claimed only when `decodedContentContainment` is listed (explicit static Cartesian profile); otherwise unclaimed, with reasons" |

## Corrections carried from v1 (still valid)

- The interval-only prior policy is unusable at tight boundaries.
  - [verify/tight_bounds_probe.py](verify/tight_bounds_probe.py) widens every subtraction
    unconditionally. Q2's 40,000/40,000 Unresolved result therefore shows the cost of
    unconditional widening, not a property of every interval design.
  - Its Q3 omits the producers' `rounding` padding. The 2,546 Outside cases are a rounding
    *pattern*, not a P1/vector prediction (review note 5).
- Singular tile transforms need no exclusion.
- `ValidationLimits` has 15 fields today.
- W13 is meaning-dependent. Its v2 expectation is in `verify/w13-normalized.json`.
- Producer impact: implicit is the default for point and vector; no producer writes RTC, regions
  or rotated boxes.
- The prior 512 MiB whole-validation memory claim is not adopted.

## Independent acceptance plan

1. **Oracle O, split in two phases (review note 8).**
   - **O-explicit** gates slice 1. It is pure Python 3 with `Fraction`/`int` and no repository
     imports. It covers:
     - GLB, glTF and b3dm reading;
     - accessor decoding;
     - **its own NONE-filter EXT_meshopt_compression decoder for ATTRIBUTES, TRIANGLES and
       INDICES modes**, implemented from the pinned spec
       `primary/EXT_meshopt_compression.md` (sha256 `602608cb…`, 540 lines), with the upstream
       golden vectors as self-test only (R9);
     - scene/node walking;
     - the canonical formulas and static bound;
     - box/sphere predicates;
     - the domain predicate and the precheck.
   - **O-implicit** (subtree availability and metadata rows) gates D3 and A2. It is not needed
     for slice 1.
2. **Witnesses.** The slice-1 expectations for the 13 literal and 17 context witnesses, plus the
   new controls, are in `verify/slice1-witness-expectations.json`. Every Contained witness is
   paired with a one-ulp-outside mutant.
3. **Sensitive controls, specified as pairs (R9).**
   - **Data controls:** a base fixture and a mutated fixture, both hashed, each with expected
     verdicts. Both production and oracle must report both verdicts. Examples:
     - one-ulp outside;
     - undrawn outlier made drawn;
     - same URI under a different transform;
     - RTC added;
     - quaternion past the window;
     - span or work at cap and one over;
     - external-content volume shrunk.
   - **Implementation mutants:** a hashed patch against the final production source, and a
     separate hashed patch against the oracle, together with the data fixture whose verdict the
     mutant must flip in that implementation. Mutants:
     - omit U;
     - double U;
     - reverse tile order;
     - test undrawn vertices;
     - use the rounded `normalized()` quotient;
     - add 1e-6 slack;
     - treat a rounded f64 point as exact;
     - skip ancestors;
     - skip the external root transform;
     - reuse the URI cache for verdicts;
     - default to scene 0;
     - use the raw quaternion polynomial.
   - A mutant that flips nothing is a failed control.
4. **Producer corpus.** R generates the outputs listed in the ownership plan and hands O the
   **artifacts plus their hashes**. O reports one of Contained, Outside(facts) or
   NotInspected(reasons) per output. Production must match on every row. Each Outside row needs a
   recorded disposition.
5. **Limits.** Controls at each cap and one over. Show that refusal happens before streaming and
   never yields ResourceLimit.
6. **Adapters and final review.**
   - Rust/CLI/Python report parity.
   - Installed-wheel and official Blender checks as applicable.
   - Fresh nonauthor final-source review.
   - Exact-head CI.

## Proof limits

The following are not provided:

- implicit coverage (A2/D3), regions (#120), RTC (D6), rotated boxes (D5);
- deformation;
- rendered or GPU extent;
- whole-process memory or RSS;
- per-unit timings.

Parents #113, #120, #125 and #126 stay open. There is no main merge, tag or publication.
