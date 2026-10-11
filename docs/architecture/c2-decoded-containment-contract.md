# C2 decoded-content containment contract (slice 1)

Status: implemented on `feat/c2-decoded-containment`. Acceptance is the oracle parity and
nonauthor review recorded in [the C2 evidence](../../bench/architecture_audit/c2_decoded_containment/README.md).
This document is the frozen v3.3 contract text, plus the coordinator amendments listed at
the end. Its preparation, reviews and decision log are retained
[losslessly](../../bench/architecture_audit/c2_decoded_containment/preparation/). It claims
nothing about implicit tiling, RTC, regions, rotated boxes, deformation, rendered extent,
memory or release readiness.


## Claim

When `checks` contains `decodedContentContainment`, the archive satisfies all of the following:

- It is entirely inside the slice-1 domain (see the domain predicate).
- For every content reference to a tile payload (a non-`.json` URI), the payload was decoded
  from the actual archive bytes.
- Every vertex that the selected static scene draws was tested. This covers every core node
  instance and every POSITION accessor the instance uses.
  - "Draws" means the vertex is referenced by a core primitive's index values, with restart
    values excluded, or belongs to a non-indexed primitive.
  - `EXT_mesh_polygon` `loopIndices` are not draws (decided under Work bound).
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
| `EXT_mesh_polygon` | Polygon loop and offset accessors, typed by C1 (`payload.rs:797-835`). They change no POSITION value. C2 tests only vertices referenced by the core TRIANGLES indices; loops are not draws (decision under Work bound). O reports an informative all-vertex observation |
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

## Exact evaluation (frozen at interface freeze; R1, B1–B3)

**Representation.** Arbitrary-precision dyadic integers, with a statically checked bit-span cap.
Binary64 expansions are rejected:

- the reviewer's `span_probe.py` showed binary64 underflow at 4–8 rotated levels;
- the delta review's independent bound implementation found 0 violations in 251,516 checks
  (`delta/bound-adversary-observed.json`).

### Normative span definition (B1, frozen)

This definition is normative for both M and O. Both must compute **identical** spans, including
for the controls at the cap and one over. The reference implementation
[verify/frozen_bound_probe.py](verify/frozen_bound_probe.py) follows this text literally. O
re-implements it from this text.

**Pairs.** A pair `(lo, hi)` bounds the exponents of the set bits of a value. **ZERO** is a
separate marker that means the value is exactly 0. `span(ZERO) = 0` and
`span((lo, hi)) = hi − lo + 1`.

**Input pairs.** For an exact input `x`:

- `x = ±0`, including JSON `-0.0`: ZERO;
- otherwise: `(lowbit(x), ⌊log2|x|⌋)`, where `lowbit` is the exponent of the lowest set bit.

The fixed worst-case pairs for raw POSITION words apply even when the word is zero:

| Raw POSITION type | Pair |
| --- | --- |
| FLOAT | `(−149, 127)` |
| i8, u8 | `(0, 7)` |
| i16, u16 | `(0, 15)` |

Normalization constants use their exact pairs:

| Constant `k` | Pair |
| --- | --- |
| 1 (unnormalized) | `(0, 0)` |
| 127 | `(0, 6)` |
| 255 | `(0, 7)` |
| 32767 | `(0, 14)` |
| 65535 | `(0, 15)` |

The literal 2 in H has pair `(1, 1)`.

**Product `a·b`.**

- If either factor is ZERO, the result is ZERO.
- Otherwise, if either factor's pair is exactly `(0, 0)`, the result is the other factor's pair.
  A `(0, 0)` pair means a value of ±1 or 0, so this is sound.
- Otherwise the result is `(lo_a + lo_b, hi_a + hi_b + 1)`.

**Flat sum over the written terms.**

- ZERO terms are dropped.
- With no terms left, the result is ZERO.
- With exactly one term, the result is that term's pair.
- With `m ≥ 2` terms, the result is `(min lo, max hi + ⌈log2 m⌉)`.
- Negation leaves a pair unchanged.
- Sums are **never** re-associated. The term lists below are exhaustive.

**Node step.** Nodes are visited in the selected scene, root-first.

- A node with **none** of `matrix`, `translation`, `rotation` or `scale` is **skipped**. This
  covers the `{"mesh":0}` node in every producer GLB. Under the product rule an explicit identity
  TRS or matrix node yields the **same** level spans; the probe confirms this (`producer…`: 297
  versus 297).
- Absent TRS fields take their defaults: `t = (0,0,0)`, `s = (1,1,1)`, `q = (0,0,0,1)`.
- **TRS node, with `q = (x, y, z, w)`:**
  - `n = Σ[x·x, y·y, z·z, w·w]`.
  - `H00 = Σ[w·w, x·x, −y·y, −z·z]`, `H11 = Σ[w·w, −x·x, y·y, −z·z]`,
    `H22 = Σ[w·w, −x·x, −y·y, z·z]`.
  - `H01 = 2·Σ[x·y, −w·z]`, `H02 = 2·Σ[x·z, w·y]`, `H10 = 2·Σ[x·y, w·z]`,
    `H12 = 2·Σ[y·z, −w·x]`, `H20 = 2·Σ[x·z, −w·y]`, `H21 = 2·Σ[y·z, w·x]`.
  - `L[r][c] = H[r][c]·s_c`, `L[r][3] = n·t_r`, `L[3] = (ZERO, ZERO, ZERO, n)`.
- **Matrix node:** `L[r][c] = pair(matrix[4c + r])` for `r < 3`, and
  `L[3] = (ZERO, ZERO, ZERO, (0, 0))`.

**Composite.**

- `G` starts as the identity: diagonal `(0, 0)`, ZERO elsewhere.
- For each non-skipped node: `G[i][j] ← Σ[G[i][0]·L[0][j], …, G[i][3]·L[3][j]]`.

**Level 0.** `a` is the raw-word pair and `k` the normalization constant.

- `P_r = Σ[G[r][0]·a, G[r][1]·a, G[r][2]·a, G[r][3]·k]` and `W = G[3][3]·k`.
- For signed normalized words the clamp happens first. The clamped value stays within the same
  pair.
- Then `U` is applied: `P ← (P₀, −P₂, P₁)`, which permutes the pairs.

**Tile step into level ℓ ≥ 1.**

- If the child tile has no `transform`, there is no step and the pairs are unchanged.
- Otherwise, with `T` column-major and an affine last row:
  `P_r ← Σ[T[r][0]·P₀, T[r][1]·P₁, T[r][2]·P₂, T[r][3]·W]`, and `W` is unchanged.

**Volume quantities at a level**, per row `j`:

- **Box.** `CW_j = c_j·W`, `D_j = Σ[P_j, −CW_j]`, `HW_j = h_j·W`. Here `h_j` is the pair of the
  covering signed-permutation entry, or ZERO.
- **Sphere.** `CW_j` and `D_j` as for a box, then `Q_j = D_j·D_j`, `LHS = Σ[Q₀, Q₁, Q₂]`,
  `R2 = r·r`, `W2 = W·W` and `RHS = R2·W2`.

**Quantities that count toward spans.** Every product and every flat-sum result above.

**Per-level spans:**

- `sInst` is the maximum over all node-step and composite quantities.
- `s₀` is the maximum over the level-0 products and sums, `W`, and the level-0 volume quantities.
- `s_ℓ` for `ℓ ≥ 1` is the maximum over that tile step's products and sums, `W`, and the level-ℓ
  volume quantities.

**Domain.** A reference `r` to URI `u` is in domain iff
`max(sInst(u), s₀(r), …, s_L(r)) ≤ containmentSpanBits`, computed from the per-URI hull
`Pair0(u)` as in "Per reference" below. The hull covers every instance of `u`, so a URI whose
instances are each within the cap can still be `numericSpan` when their hull is not. Otherwise
the archive is `numericSpan`.

**Soundness evidence (this lane).**
[verify/frozen-bound-observed.json](verify/frozen-bound-observed.json) records 304,455 quantity
and prefix checks with **0 violations**. The adversarial inputs include:

- ±0, ±1, subnormals, 2^500, −2^1000 and near-DBL_MAX values;
- singular matrices;
- f32 subnormal and f32 maximum positions;
- the signed clamp.

Every left-to-right prefix of every flat sum lies inside the full-sum pair. So **any** runtime
association M uses has every intermediate covered by the bound, and so does multiplying by an
exact ±1 at runtime.

**Rules outside the domain.**

- **The D2 window.** `Σq² − 1` is evaluated exactly but is *outside* `numericSpan`. For any f64
  `q` its span is at most 2151 bits, so it always fits (delta review).
- **Per-URI hull monotonicity.** The precheck aggregates pairs per URI as an entrywise hull
  (D4). The rules are monotone under hull: a hull pair is never `(0, 0)` unless every member is,
  and the general product rule contains the pass-through result. So a hull-derived bound contains
  every actual instance's bound, and soundness and capacity still hold.

### Worked example (B1; the at-cap / one-over control)

The setup:

- One TRS node with `translation = (10.5, 0, −3)`, `scale = (2, 4, 0.25)` and no rotation.
- u16 normalized POSITION, so `k = 65535`.
- An own box with `c = (1.5, 0, 2)` and half-lengths `(100, 0, 50)`.
- A child `transform` that translates by `(1000.25, −20, 0)` into a parent box with `c = 0` and
  half-lengths `(2048, 2048, 2048)`.

The input pairs:

| Input | Values and pairs |
| --- | --- |
| `t` | 10.5 → `(−1, 3)`, 0 → ZERO, −3 → `(0, 1)` |
| `s` | 2 → `(1, 1)`, 4 → `(2, 2)`, 0.25 → `(−2, −2)` |
| `q` | Default: `w` → `(0, 0)`; `x`, `y`, `z` → ZERO |
| Translation `(1000.25, −20, 0)` | `(−2, 9)`, `(2, 4)`, ZERO |
| Box values | 1.5 → `(−1, 0)`, 100 → `(2, 6)`, 50 → `(1, 5)`, 2048 → `(11, 11)` |

The trace:

1. **Node step.**
   - `w·w` is `(0, 0)` by pass-through, so `n = (0, 0)` and `H` is diagonal `(0, 0)`.
   - `L₀₀ = (1, 1)`, `L₁₁ = (2, 2)`, `L₂₂ = (−2, −2)`.
   - `L₀₃ = n·t₀ = (−1, 3)`, `L₁₃ = ZERO`, `L₂₃ = (0, 1)`, `L₃₃ = (0, 0)`.
   - `G = I·L` reproduces `L` exactly.
   - `sInst = 5`.
2. **Level 0, positions.**
   - `P₀ = Σ[(1,1)·(0,15) = (1,17), (−1,3)·(0,15) = (−1,19)] = (−1, 20)`.
   - `P₁ = (2,2)·(0,15) = (2, 18)`.
   - `P₂ = Σ[(−2,14), (0,17)] = (−2, 18)`.
   - `W = (0,0)·(0,15) = (0, 15)`.
   - After `U`: `((−1, 20), (−2, 18), (2, 18))`.
3. **Level 0, own box.**
   - Row 0: `CW = (−1,0)·(0,15) = (−1, 16)`, `D = Σ[(−1,20), (−1,16)] = (−1, 21)` (span 23),
     `HW = (2, 22)`.
   - Row 1: `CW = ZERO`, `D = (−2, 18)`, `HW = ZERO`.
   - Row 2: `CW = (1, 17)`, `D = (1, 19)`, `HW = (1, 21)`.
   - **`s₀ = 23`.**
4. **Level 1.**
   - `P₀ = Σ[(0,0)·(−1,20) = (−1,20), (−2,9)·(0,15) = (−2,25)] = (−2, 26)` (span 29).
   - `P₁ = Σ[(−2,18), (2,20)] = (−2, 21)`.
   - `P₂ = (2, 18)`.
   - Parent box: `CW = ZERO`, `D_j = P_j`, `HW = (11,11)·(0,15) = (11, 27)`.
   - **`s₁ = 29`.**
5. **Result.** The maximum span is **29**.
   - The control with `containmentSpanBits = 29` is in domain (Contained if the vertex is inside).
   - The control with `containmentSpanBits = 28` is `numericSpan`.
   - Run both through `inspect_selected` with explicit limits (`validate.rs:805`).
6. **Units** (Work bound, next section). With one POSITION accessor of count 4, the vertex units
   are `4 × (⌈23/64⌉² + ⌈29/64⌉²) = 4 × 2 = 8`. The node units are `128 × ⌈5/64⌉² = 128`. The
   index units are 4 (non-indexed), and `E(u)` is C1's `elements_checked` for that payload.

The probe output `workedExample` matches: `sInst = 5`, `levelSpans = [23, 29]`, and 8 vertex
units.

### Expected coverage (B2)

Measured with the normative rules at `containmentSpanBits = 4096`
([verify/frozen-bound-observed.json](verify/frozen-bound-observed.json)), the maximum admitted
rotated TRS node depth is:

| Quaternion style | Box-only chain | Sphere anywhere in chain |
| --- | --- | --- |
| Generic | 16 | **10** |
| Near-axis (components ≈ 1e-17) | 14 | **6** |
| One subnormal-scale component (≈ 1e-300) | **1** | **0** |

The sphere squares `D_j`, which doubles the span. A single admissible quaternion with a tiny
component adds thousands of bits; the delta review measured about 9000 bits at depth 4.

All of these cases are sound: they give `numericSpan` (not inspected), never a failure. Producer
chains have large margins:

- 297 bits for point/vector translation chains of depth 20, with the `{"mesh":0}` node skipped;
- 90 bits for `--quantize` at depth 16.

**W1:** user models are covered up to the depths above. Deeper models, or exporter output with
denormal-scale quaternion noise, are not inspected. The oracle corpus records the actual counts.

### Representation and memory (B3)

**Values.** Every dyadic value is held in a fallibly allocated **heap** arena:
`Vec::try_reserve_exact`, sized once per phase-2 payload stream from the selected limits. Only
O(1) index handles and fixed-size f64 facts live on the stack. No fixed-size dyadic array lives on
the stack: `validate` runs on Python threads (`bindings/python/src/lib.rs:976-981`) and inside
Blender.

**One dyadic** has capacity `⌈(containmentSpanBits + 64)/64⌉` u64 limbs. The extra 64 bits are
headroom for runtime ±1 products and carries, plus an i64 exponent and sign and length fields. At
the proposed cap of 4096 that is 65 limbs, so **544 B** per dyadic (including header and
alignment).

**Arena per stream:** `(16 × (containmentNodeDepth + 1) + 32)` dyadics.

- The 16-entry composite stack is one frame per node depth level.
- The 32 dyadics are walk scratch: `P`, `W`, the next `P`, the `D`/`CW`/`HW`/`Q`/`LHS`/`R2`/`W2`/`RHS`
  terms and product temporaries.
- At depth 64 that is 1,072 dyadics, **583,168 B ≈ 0.56 MiB**.

**Bitmap.** One reusable bitmap, sized to the payload's largest POSITION count. C1's per-document
`element_cap` limits a VEC3 count to at most 4,000,000/3 (`payload.rs:415`), so the bitmap is at
most **166,667 B**.

**Level chain.** f64 facts only: transform, up to two volumes and roles, per level. That is at
most 129 levels × 344 B, so **≤ 44 KiB**. Dyadics are created from the facts in the scratch area.

**Phase-1 and precheck facts** (D4):

- Per URI, **120 B**: 4 hull pairs, `sInst`, `sPos`, and `T`, `Nodes`, `Idx`, `E`, `Dec`, `B` and
  `maxNodeDepth` as u64. The maximum is 65,536 URIs (C1 `archive_entries`), **≤ 7.5 MiB**.
- Tile arena: at most 65,536 tiles (C1 `hierarchy_visits`) × **≤ 336 B** (transform, own and
  content volume, parent index, role), so **≤ 21 MiB**.
- Reference list: at most 262,144 × 16 B, so **≤ 4 MiB**.

**Peak C2-added memory:** at most **about 33.5 MiB** of facts plus **about 0.8 MiB** of phase-2
working storage at the proposed caps. This excludes C1's own live decode owners and process
baseline; it is not an RSS claim.

**Allocation failure.** Every one of these reservations is fallible. A refusal is
**ResourceLimit**, which is outcome 4 in the precedence order. All reservations happen before the
first vertex test of the stream.

## Work bound (frozen formulas; R2, D1–D4)

The cap **values** are proposals that R measures and may retune before merge. Retuning changes
coverage only, never semantics. The **formulas** below are frozen.

All counts use checked u64 arithmetic. Overflow counts as exceeding the cap.

### Per-URI facts (D4)

These are computed during the first C1 inspection of each unique payload URI `u`, stored
**once per URI** (O(1) each, aggregated over instances), and cached alongside C1's per-URI
integrity facts:

| Fact | Definition |
| --- | --- |
| `Pair0(u)` | Entrywise hull (`min lo`, `max hi`; ZERO ∪ p = p) over every (instance, distinct POSITION accessor) of the level-0 `P` pairs after U and the `W` pair |
| `sInst(u)` | Maximum span over node-step and composite quantities over all instances |
| `sPos(u)` | Maximum span over the level-0 position products and sums, and `W`, over all (instance, accessor) pairs |
| `T(u)` | `Σ_instances Σ_{distinct POSITION accessors a referenced by that instance's mesh primitives} a.count`. Two primitives that share an accessor in one instance count it **once** |
| `Nodes(u)` | `Σ` over node visits in the selected-scene tree of: 1 for a skipped node, otherwise `128·⌈s_step/64⌉²`, where `s_step` is the maximum span of that node's `L` quantities, its `G·L` products and the resulting `G` entries (D1) |
| `Idx(u)` | `Σ_instances Σ_primitives` (`indices.count` if indexed, else `POSITION.count`) (D2) |
| `E(u)` | C1 `elements_checked` for the payload (`PayloadInspection.elements_checked`) (D3) |
| `Dec(u)` | Bytes that C1 materializes while inspecting `u`: owned external and data-URI buffers plus meshopt-decoded views. F adds this C1 fact (D3) |
| `B(u)` | Archive bytes read during C1 inspection of `u`: the member plus resolved buffers and images (D3) |
| `maxNodeDepth(u)` | Deepest selected node path, counting all nodes, including skipped ones |

### Per reference `r` to URI `u`, levels `0..L_r`

1. Start from `Pair0(u)`. Apply the normative tile step and volume quantities level by level.
2. Set `s₀(r) = max(sPos(u), level-0 volume quantities)` and `s_ℓ(r)` as defined in the
   normative span definition.
3. If `max(sInst(u), s₀(r), …, s_L(r)) > containmentSpanBits`, the result is `numericSpan`.
4. Otherwise:
   - `units(r) = T(u)·Σ_ℓ ⌈s_ℓ(r)/64⌉² + Nodes(u) + Idx(u) + E(u)`.
   - `bytes(r) = B(u)`.
   - `decoded(r) = Dec(u)`.
   - If `maxNodeDepth(u) > containmentNodeDepth`, the result is `workLimit`.

### Precheck (D4)

The precheck accumulates `Σ units`, `Σ bytes` and `Σ decoded` **during** the C1 traversal, in
traversal order (reference by reference). It **stops at the first violation**: either a
`numericSpan` from step 3, or the first running total that exceeds `containmentWorkUnits`,
`containmentBytesRead` or `containmentDecodedBytes`. It records exactly one precheck reason.
Precedence still applies: any exclusion reason anywhere in the archive overrides the precheck
reason.

**Time:**

- O(1) per reference per level, so at most `references × (hierarchy_depth + 1)`, about 33.8M pair
  operations at C1's limits.
- Plus O(nodes + primitives) per unique URI inside the C1 inspection that already scans them.

### What the units bound

| Unit source | What each unit stands for |
| --- | --- |
| Vertex units | At most **24** limb multiplications per unit. The delta review counted about 18·L per box level and about 22·L² per sphere level, with `L = ⌈s/64⌉` limbs |
| Node units | One 4×4 composite product (64 dyadic products) plus `L` construction (about 26 products), each at most `⌈s/64⌉²` limb multiplications; 128 per `⌈s/64⌉²` covers both |
| `Idx` and `E` | O(1) primitive operations per index occurrence or C1 element: one read and one bitmap set or check |

So phase-2 time ≤ `c × containmentWorkUnits` primitive operations, plus the decode of at most
`containmentDecodedBytes` materialized bytes and `containmentBytesRead` archive bytes. R measures
`c` per unit class. The weights (1 for `Idx` and `E`, 128 for nodes, `⌈s/64⌉²` for vertices) are
frozen.

### Stream algorithm (normative for cost; M and F)

Per reference, serially:

1. Re-run `payload::inspect` with the phase-2 consumer. This revalidates C1, which is charged by
   `E`.
2. Walk the selected-scene tree, keeping a composite stack in the arena. That is one step per
   non-skipped node, charged by `Nodes`.
3. For each instance and each distinct POSITION accessor `a` its mesh references:
   - clear the bitmap;
   - for each primitive using `a`, mark its index values, skipping restart values. A
     non-indexed primitive marks all of `0..count`. This is charged by `Idx`;
   - then, for each set bit in ascending order, decode the raw word and run the upward walk. This
     is charged by `T·Σ_ℓ`.

**`EXT_mesh_polygon` (decided).** Only indices referenced by the core primitive are marked.
`loopIndices` are **not** draws: C1 types the loop accessors but does not range-check their
values against POSITION (`payload.rs:797-835`), and no `EXT_mesh_polygon` spec is pinned. This
matches the Claim ("drawn") and the bitmap. O additionally reports an informative, non-gating
"all POSITION vertices" observation for polygon primitives in the corpus.

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
- **`ValidationLimits` (O-b, coordinator decision).** The new C2 caps live in the existing
  public `ValidationLimits`, so validation limits keep a single owner. F edits
  `src/validate/types.rs` to add exactly five named `pub u64` fields with `Default` values. The
  values are proposals that R measures and may retune before merge:

  | Rust field | Serialized name | Proposed default |
  | --- | --- | --- |
  | `containment_span_bits` | `containmentSpanBits` | 4096 |
  | `containment_work_units` | `containmentWorkUnits` | 4,294,967,296 (2^32) |
  | `containment_bytes_read` | `containmentBytesRead` | 2,147,483,648 (2 GiB) |
  | `containment_decoded_bytes` | `containmentDecodedBytes` | 2,147,483,648 (2 GiB) |
  | `containment_node_depth` | `containmentNodeDepth` | 64 |

  This is a **public Rust API change**:
  - `ValidationLimits` is a `pub` struct with `pub` fields and no `#[non_exhaustive]`
    (`types.rs:50-66`), so the new fields **break external struct-literal construction**.
  - No public function takes `ValidationLimits` as input.
  - CLI and Python serialize it unchanged (`bindings/python/src/lib.rs:976-981`).
  - The change is listed in the exit-status and public-change table below and in the migration
    notes.
  - Internal controls set these fields through `inspect_selected` (`validate.rs:805`).

## Exit-status changes versus today (complete list, R2d)

| Change | Archives affected |
| --- | --- |
| New **InvalidInput: Outside** | In-domain archives with a proven escape. Static predictions: point/vector edge cases with zero rounding padding; legacy mesh routes. The oracle corpus confirms or refutes each |
| New **InvalidInput: malformed scene/node records** | **Any** archive, including excluded ones, because admission runs in phase 1. C1 does not read nodes today |
| New **ResourceLimit** | Host allocation refusal only, for the fallible reservations of the per-URI facts, tile arena, reference list, dyadic arena or bitmap (B3). No new counter produces ResourceLimit: every cap gives `numericSpan` / `workLimit` |
| Other behaviour | No change to Unsupported, Io, `totalBytesRead`, `total_payload_elements` or `hierarchyBounds` |
| Report schema | `checks` / `notInspected` entries and the payload-label split |
| **Public Rust API** | Five new `pub` fields on `ValidationLimits`. This breaks external struct literals (O-b) |

## Ownership of representation

| Owner | Responsibilities | Excludes |
| --- | --- | --- |
| **Format** (`content_integrity/payload*`, F) | Records; phase-1 admission and exclusion facts; RTC_CENTER key presence; the per-URI aggregated facts `T`, `Nodes` (via M's bound functions), `Idx`, `E`, `Dec`, `B`, `maxNodeDepth`; phase-2 event stream over live C1 views | Positions `Vec`, second decoder, math, policy |
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
5. **Limits.** Controls at each cap and one over, run through `inspect_selected` with explicit
   `ValidationLimits`. Show that refusal happens before streaming and never yields
   ResourceLimit. Required pairs:
   - **The worked example at `containmentSpanBits = 29`:** in domain. At 28: `numericSpan`. The literal
     fixture's verdict is Outside (own row-1 half-length 0 at centre 0); the Contained-at-cap control uses the
     same spans with own-box row-1 centre 3 and z = 0 (v3.3).
   - **A work-units pair:** cap set to exactly that archive's computed `Σ units`, giving
     Contained, and to one less, giving `workLimit`. Do the same for `containmentBytesRead`,
     `containmentDecodedBytes` and `containmentNodeDepth`.
   - **The repeated-index control, which spans at least 3 payloads.** C1 caps a payload at 4M
     components (`payload.rs:415`), so this needs three GLBs. Each has a 3-vertex FLOAT POSITION
     (9 components) and an indexed TRIANGLES accessor of 3,999,990 indices, so each payload has
     3,999,999 elements and the archive has 11,999,997, which is within the 16M archive cap.
     Each payload is referenced once, giving about 12.0M index occurrences, which is at least
     9M.
     - Expected: Contained at the default caps, with 9 vertex tests per level and about 24M
       `Idx + E` units. The control must never yield ResourceLimit.
     - With `containmentWorkUnits` set to one below the archive's `Σ units`: `workLimit`.
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

## Amendments adopted after the interface freeze

The full record is in `preparation/contract-20261011/change-log.md`, sections v3.2 and v3.3.
The amendments with semantic effect:

- **Raw word pairs.** The raw POSITION word selects its span pair for normalized and
  unnormalized words alike: FLOAT (−149, 127); i8/u8 (0, 7); i16/u16 (0, 15).
- **Frame order.** A child tile's transform takes frame l−1 into frame l. Each level's volumes
  are tested before that level's own tile transform is applied.
- **Test order within a level.** Own < Content < ExternalContent < Ancestor.
- **Traversal order.** Depth-first pre-order: a tile's contents in array order, then its
  children in array order. External tilesets are expanded in place. Each entry of a core
  `contents` array is a separate reference.
- **Vector clip.** A `3DTILES_content_gltf_vector` object with `clip: true` is a non-allowlisted
  use: `tilesetExtension`, or `gltfExtension` on the glTF side.
- **Outside message fields.**
  - `depth` is the tree depth of the referenced content's tile; an external root is its
    referencing tile's depth + 1.
  - `primitive` is the lowest primitive index whose draw references the vertex.
  - `vertex` is the POSITION element index.
- **Node charges.** `Nodes(u)` charges every selected node, using the static node step bound.
- **Duplicate references.** A duplicate child or scene root, or a node reachable twice, is
  InvalidInput.
- **C1 first.** C1 refusals precede C2.
- **Internal faults.** An internal capacity fault is the public
  `ValidationFailure::InvalidState` (`invalid_state`, exit 1). It is never a verdict.
