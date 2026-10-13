# C2 decoded-content containment contract (slice 1)

> Bench evidence paths below under `bench/architecture_audit/` that no longer exist were
> moved out of the tree at `e29cd19d2f23`; each audit directory README gives the recovery
> note ([index](../../bench/architecture_audit/README.md)).

Status: implemented on `feat/c2-decoded-containment` (PR #157). Acceptance evidence is indexed in
[the C2 acceptance record](../../bench/architecture_audit/c2_decoded_containment/acceptance/README.md);
the separate nonauthor review and its resolution are recorded on PR #157.
This document is the frozen v3.3 contract text, plus the coordinator amendments listed at
the end. Its preparation, reviews and decision log are retained
[losslessly](../../bench/architecture_audit/c2_decoded_containment/preparation/). It claims
nothing about RTC, regions, rotated boxes, deformation, rendered extent, memory or release
readiness. Implicit archives are covered since the
[slice-3 amendment](#c2-slice-3-amendment-implicit-archives-2026-10-12) (except derived cells).
The [H7 amendment](#h7-amendment-legacy-content-unsupported-proposed-2026-10-13) (PROPOSED) removes
b3dm from the domain: legacy content is a C1 Unsupported outcome and `rtcCenter` is never reported.


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
| `implicitTiling` (slice 1; replaced in the same position by `derivedImplicitVolume`, see the [slice-3 amendment](#c2-slice-3-amendment-implicit-archives-2026-10-12)) | Any tile, in any tileset document, has `implicitTiling` |
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

**Numbers.** Every number is the nearest binary64 value of the JSON text. Overflow
outcomes differ by document. Tileset JSON is parsed into a value by C1, which refuses a
literal that overflows binary64 as Unsupported ("JSON exceeds finite owned-document
representation"); the oracle's `tileset-number-overflow` fixture executed this. glTF JSON is
admitted without a value tree, so an overflowing node `matrix`/`translation`/`rotation`/`scale`
component or accessor `min`/`max` bound is InvalidInput at node or accessor admission. This
corrects the v3 text (InvalidInput everywhere) and the first v3.3 correction (Unsupported
everywhere).

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

**Level chain.** f64 facts only: transform, up to three volumes and roles, per level. That is at
most 129 levels × 472 B (`size_of::<LevelFacts>()`, C2-WB restatement), so **≤ 60 KiB**. Dyadics are created from the facts in the scratch area.

**Phase-1 and precheck facts** (D4):

- Per URI, **152 B** (`size_of::<UriFacts>()`, C2-WB restatement): hull facts, `sInst`, `sPos`,
  `T`, `Nodes`, `Idx`, `E`, the per-reference `Dec`, `B`, `J`, `Res`, `Prim`, `maxNodeDepth` and the
  exclusion bits. The maximum is 524,288 URIs (C1 `archive_entries`), **≤ 76 MiB**.
- Tile arena: at most 262,144 tiles (C1 `hierarchy_visits`) × **376 B** (`size_of::<TileFact>()`:
  transform, own and external-content volume, parent index, depth and the C2-WB `chain` bits),
  so **≤ 94 MiB**.
- Reference list: at most 1,048,576 (C1 `references`) × 32 B (`size_of::<Reference>()`), so
  **≤ 32 MiB**.

**Peak C2-added memory:** at most **about 202 MiB** of facts plus **about 0.8 MiB** of phase-2
working storage at the current caps (sizes measured with `size_of` on x86_64 Linux at the C2-WB
final source; they were estimated at about 160 MiB before C2-WB). This excludes C1's own live decode owners and process
baseline; it is not an RSS claim. (The C1 entry, visit and reference caps were raised on
2026-10-11 from 65,536, 65,536 and 262,144, when these figures were 7.5, 21 and 4 MiB, about
33.5 MiB in total; measured whole-process costs are in
`bench/architecture_audit/validator_caps`.)

**Allocation failure.** Every one of these reservations is fallible. A refusal is
**ResourceLimit**, which is outcome 4 in the precedence order. All reservations happen before the
first vertex test of the stream.

## Work bound (frozen formulas; R2, D1–D4)

The cap **values** are proposals that R measures and may retune before merge. Retuning changes
coverage only, never semantics. The **formulas** below are frozen; the C2-WB amendment
(2026-10-12, contract draft v2.1) replaced the per-reference units, the node units and the
default cap, and added the facts `J`, `Res` and `Prim` (see "C2-WB amendment" at the end).

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
| `Nodes(u)` | `Σ` over node visits in the selected-scene tree of: 1 for a skipped node, otherwise `max(256, 128·⌈s_step/64⌉²)` (C2-WB §3.2), where `s_step` is the maximum span of that node's `L` quantities, its `G·L` products and the resulting `G` entries (D1) |
| `Idx(u)` | `Σ_instances Σ_primitives` (`indices.count` if indexed, else `POSITION.count`) (D2) |
| `E(u)` | C1 `elements_checked` for the payload (`PayloadInspection.elements_checked`) (D3) |
| `Dec(u)` | Bytes that C1 materializes while inspecting `u`: owned external and data-URI buffers plus meshopt-decoded views. F adds this C1 fact (D3) |
| `B(u)` | Archive bytes read during C1 inspection of `u`: the member plus resolved buffers and images (D3). C1 resolves, and reads, a member once per buffer or image entry, so a member named by two entries counts twice |
| `J(u)` (C2-WB) | Σ `CheckedDocument::value_nodes()` over every JSON document C1 admits for `u`: the glTF JSON or GLB JSON chunk (also inside a b3dm) and the nonempty b3dm feature-table and batch-table JSON |
| `Res(u)` (C2-WB) | Buffer and image **entries** with a non-`data:` `uri`: one per resolver call. `data:` buffers are not resources; their bytes are in `Dec` |
| `Prim(u)` (C2-WB) | Σ over the instances of the selected scene of the instance mesh's primitive count |
| `maxNodeDepth(u)` | Deepest selected node path, counting all nodes, including skipped ones |

### Per reference `r` to URI `u`, levels `0..L_r`

1. Start from `Pair0(u)`. Apply the normative tile step and volume quantities level by level.
2. Set `s₀(r) = max(sPos(u), level-0 volume quantities)` and `s_ℓ(r)` as defined in the
   normative span definition.
3. If `max(sInst(u), s₀(r), …, s_L(r)) > containmentSpanBits`, the result is `numericSpan`.
4. Otherwise:
   - `units(r) = T(u)·V(r) + Nodes(u) + Idx(u) + E(u) + Dec(u) + B(u) + 16·J(u) + 160·Res(u)
     + 4·Prim(u) + 1152 + 8·(L_r + 1)` (C2-WB §3.1), with `q_ℓ = ⌈s_ℓ(r)/64⌉²` and
     `V(r) = Σ_{ℓ=0..L_r} ([ℓ = 0]·max(20, q_ℓ) + [ℓ ≥ 1 and the child tile has a transform]·max(30, q_ℓ)
     + Σ_{box volumes tested at ℓ} max(25, q_ℓ) + Σ_{sphere volumes tested at ℓ} max(32, q_ℓ))`.
     The per-reference terms apply to every reference, including one to a URI with no drawn
     instance (there `T·V = 0`).
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
`c` per unit class. The weights (1 for `Idx` and `E`, 128 for nodes, `⌈s/64⌉²` for vertices) were
frozen in slice 1. C2-WB replaced them with per-operation floors (20 level-0 base, 30 step, 25
box, 32 sphere, 256 node) and per-reference charges for bytes, JSON value nodes, resources and
primitives, because cheap-unit shapes cost up to 776 ns per slice-1 unit; R's measured `c` per
class is in the C2-WB amendment below.

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
| **Public Rust API** | Five new `pub` fields on `ValidationLimits`. This breaks external struct literals (O-b). Default `containmentWorkUnits` is 2^29 (C2-WB WB-6: the largest power of two with `cap × τ_E ≤ 8 s` on the measurement host; it was 2^28 before C2-WB) |
| New **InvalidState** | `ValidationFailure::InvalidState` (`invalid_state`, exit 1) for an internal capacity-invariant breach only; never a verdict about the archive |

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

- derived implicit cells (`derivedImplicitVolume`; slice 3 replaced "implicit coverage (A2/D3)"),
  regions (#120), RTC (D6), rotated boxes (D5);
- deformation;
- rendered or GPU extent;
- whole-process memory or RSS;
- per-unit timings outside the C2-WB class matrix, on other hosts, or for debug builds
  (the C2-WB calibration is a measurement on one host, not a bound).

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

## C2-WB amendment (work bound, 2026-10-12)

Source: C2-WB contract draft v2.1 (`/tmp/rusty-tiles-c2wb-contract-20261012/contract-draft.md`,
sha256 `ea7b3acf…`), after delta reviews 1 (NO-GO on v1) and 2 (GO-WITH-CHANGES on v2). The
claim, domain, span definition (B1), predicates and outcome precedence are unchanged.

**Algorithms (preconditions; verdicts and counts unchanged).**

- WB-A: C1 JSON admission is one O(bytes) pass in the legacy failure order (C1 note
  `c1-json-admission-wb-a-amendment.md`).
- WB-B: `InstanceView::mark` groups primitives by POSITION accessor, so it is
  O(accessors + primitives + index reads) per instance.

**Formula.** Per-reference units, node units and the facts `J`, `Res`, `Prim` are as stated in
"Per-URI facts" and "Per reference" above. Phase 1 carries one `chain: u16` per tile (its own and
its external-content volume's shape exclusions ORed with the parent's), so
`Containment::reference` is O(1) and `build_levels` runs only before `precheck_step`.

**Worked example (§3.6).** Spans `[23, 29]`, `q = [1, 1]`: `V = (20 + 25) + (30 + 25) = 100`,
vertex units 400, node units 256, so the slice-1 worked-example fixture totals
`400 + 256 + 4 + E 12 + Dec 0 + B 652 + 16·56 + 160·0 + 4·1 + 1152 + 16 = 3,392`. Oracle, hand
derivation and production agree.

**Calibration and default cap (WB-6).** R measured the final source (merge `706b082` plus the
acceptance-only commit `e8cf53a`; release build with only the default cap raised, so every
class streams) on an Intel i7-12700KF under contract §9.D: a run counts only with own-CPU
foreign busy ≤ 5% and contention neighbour ≤ 10% (P-core: SMT sibling; E-core: busiest other
E-core of 16–19); each class is the median of 7 clean A/B pairs (A against its
`gltfUpAxis` twin). 95 classes were decided on both core types: O-WB's matrix-size class
fixtures, R's probe matrix (1-limb chains, large spans, references), the b51f83d producer
twins and an images-by-URI class.

| | τ (max over classes, ns/unit) | Worst class | Default cap | Bound at the cap |
| --- | --- | --- | --- | --- |
| E-core (stated) | **14.65** | box + step, 8 levels, translations of magnitude 2^300 (`boxT-E300-L8`) | **2^29** | **≤ 7.87 s** |
| P-core | 8.10 | same | 2^29 | ≤ 4.35 s |

`τ_E ≤ 14.9`, so the cap is 2^29 (rule of §6: the largest power of two with
`cap × τ_E ≤ 8 s`). The margin is small (1.7%). The pre-fix super-linear rows fell to the
linear range (E-core): deep JSON numbers at depth 62 from 22.22 to 2.33 ns/unit; `quad` from
35.26 to 8.54; `quadB` from 77.23 to 8.04; the end-to-end case from 83.66 to 7.74. Full table,
run records and hashes: `bench/architecture_audit/c2_decoded_containment/acceptance/README.md`
("C2-WB gate").

**Producer coverage at 2^29 (oracle units).** W5 vec-perf twins 38–40%, W2 26%, W3 7%, every
other explicit twin ≤ 4%. `mf2-lod3-explicit` totals 438,590,913 units (81.7%), but C1 refuses
that archive (`JSON exceeds value-node limit`, also at b51f83d), so it was not timed.

**Acceptance amendments (final review, 2026-10-12).**

- **A3 restated as ns/unit flatness.** The draft's "time per (instance, accessor +
  primitive) within 1.25× of the shared class" is confounded by `T·V`: `quad` has
  `T` = 120,000 drawn vertices against 40 for the shared-accessor class, so the measure
  mostly counts vertex work. A3 is instead: after WB-B, the many-accessor classes cost no
  more per unit than the linear range, and the cost stays flat as accessors × primitives ×
  instances grows. Measured (P / E ns/unit): `quad` 4.92 / 8.54, `quadB` 4.60 / 8.04, and
  the end-to-end case, about 12× the accessor × primitive × instance product of `quadB`,
  4.46 / 7.74 (pre-fix 13.70 / 35.26, 26.72 / 77.23 and 47.61 / 83.66). All are below `τ`.
- **§9.C production mutants, mapped to the control that catches each.**

  | Mutant class | Caught by |
  | --- | --- |
  | Unit arithmetic (frozen weights, dropped step/level-0/`R0`/`B`/`Dec`/`J`/`Res`/`Prim`/`8·(L+1)` terms, sphere floor = box floor, node floor 128) | The 55 O-WB at-cap pairs (`acceptance/oracle_parity.rs`, `production_matches_every_c2wb_class_fixture`: `units` gives Contained, `units − 1` gives `workLimit`) and the per-term unit tests in `src/validate.rs` and `src/validate/containment.rs`; the oracle-side twins are `wb/mutants.py` (15/15) |
  | Stale chain bits | `chain_bits_keep_precedence_after_the_precheck_stops` (`src/validate.rs:4039`) and the four `wb/` chain-bit controls |
  | Default cap one step lower | The amended C1 limits pins (`c2wb_cap_amendment/2026-10-12`: every admitted report expects `containmentWorkUnits` 536870912), `tests/c2_containment.rs` and the Python API default-limits assertion |
  | Re-scanning JSON admission; quadratic `mark`; `build_levels` before the gate | Performance only: the pre-fix gate rows (deep JSON numbers, `quad`, `quadB`, end-to-end) and the phase-1 twin check; a regression puts those rows above `τ` |

  The slice-1 production mutants `precheck-depth-cap-off-by-one` and `reverse-tile-order`
  are re-pinned at the C2-WB head (`acceptance/production-mutants/report.json`).

**Proof limits.** One host, one build, one compiler. Classes outside the measured matrix are not
claimed. E-core figures include shared-L2 contention below the run threshold. C1's own phase-1
work is not in units. The bound is a calibration, not a guarantee.


## C2 slice-3 amendment (implicit archives, 2026-10-12)

Source: the frozen C2 slice-3 contract v3.1 (sha256 `8bc534290c5a90ace963e345336dea1e04c45fafb3c869672ee4af1ba03331d6`,
frozen on C2-WB head `92e8c04`), its ownership plan (`28285135…`), the precode and delta reviews it cites
and the coordinator's decisions OD-1(a), OD-2(a) and OD-9 (defer D4). The preparation package, with the
prototype probes that encode verdicts quarantined in `prototype-do-not-read/` (contract §9.G), is kept
outside the repository:
`~/.cache/rusty-tiles-evidence/architecture_audit/c2s3-preparation-20261012.tar.zst`, sha256
`b079f78d41638d1b3203b5a70a6f747038dd6447193322c1b8125f81d5e942d6`. C2-WB's formula, the default cap
2^29, τ_P = 8.10 and τ_E = 14.65 ns/unit and the class table are consumed unchanged.

**Claim.** When `checks` contains `decodedContentContainment`, the slice-1 claim holds for every content
reference of the archive, including every content emitted by the I1 walk of every implicit root
(`VisitedTile.contents`). For an emitted content of implicit tile `t` at level `l` under root `R`, every
drawn vertex lies, exactly, in the closed set of:

1. `t`'s effective volume (I1 §4): its `TILE_BOUNDING_BOX` row, FLOAT32 widened exactly; for `l = 0`
   without a row, exactly `R`'s JSON box (`DerivedRoot`);
2. each implicit ancestor's effective volume and `I_0`'s (`V_0`);
3. `R`'s own JSON volume `V_R`;
4. when `R` roots an explicit external tileset, the referencing content's `boundingVolume`, after
   `R.transform`;
5. every explicit ancestor of `R`, as in slice 1.

**Recorded interpretations.**

- **I4.** `I_0` and `R` are two levels; `V_0` is tested before `V_R`, both exactly (I1's `V_0 ⊆ V_R`
  check has the explicit-tile tolerance, so a vertex inside `V_0` but outside `V_R` is a proven Outside).
- **I5.** A tile at level `l ≥ 1` without a row (`Derived` cell) is not evaluated. A reference whose chain
  contains one is excluded with `derivedImplicitVolume` (chain-scoped: a derived tile that no reference
  tests does not exclude the archive). The rounded box used for `hierarchyBounds` never enters C2.
- **I6.** Subtree metadata tables do not set `tileMetadata`; a content-template header `metadata` entity
  does (it is copied to every emitted content).
- **I7.** Implicit levels have no content or external-content volume of their own.

**Text deltas to this contract (slice-3 §8).**

1. Enum: `implicitTiling` is replaced, in the same position, by `derivedImplicitVolume`; the enum keeps
   15 names.
2. Claim items and I4–I7 above.
3. Frame chain step 4: implicit levels enter with no step (`to_parent: None`); `R.transform` steps out of
   `V_R`'s level; an external-root `R`'s referencing content volume is tested at the next level (`l + 2`),
   before that level's ancestor volume.
4. B3: a chain has at most `hierarchy_depth + 2` levels (130 at the default 128): an implicit tile cannot
   be an ancestor of another implicit root, so a chain holds at most one `I_0`. `TileFact.volume` became
   `Option<VolumeFacts>`; the enum's tag niche keeps `size_of::<TileFact>()` unchanged (R's `size_of`
   record below).
5. Outside message: for implicit content, `depth` = `depth(R) + l` (I1's depth, not the chain length − 1);
   `ancestor:1 … ancestor:l` are the implicit ancestors down to `I_0`, `ancestor:l+1` is `R`
   (`V_R`), `externalContent:l+2` and `ancestor:l+2` are the referencing content and tile when `R` roots
   an external tileset, and higher `k` are explicit levels as in slice 1.
6. Proof limits: "implicit coverage (A2/D3)" becomes "derived implicit cells (`derivedImplicitVolume`)".
7. D4: slice 3 meets D4's precondition only; D4 is deferred (OD-9).

Implementation (F, `src/validate.rs` only): the `implicitTiling` exclusion is removed; each visited
implicit tile is a `TileFact` (parent `R` for `I_0`, else the path parent), whose chain bits carry
`derivedImplicitVolume` when its volume is `None`; each emitted content is registered with
`Containment::reference` after C1's reservation-consuming `reference_reserved`, never the charging
`ArchiveResources::reference`; `build_levels` treats a `None` volume as `InvalidState` (unreachable in
domain); a template header with `metadata` sets `tileMetadata`. `src/validate/containment*` and
`src/content_integrity/implicit*` are unchanged, so the M hoist (OD-8(c)) was not needed.

**Exit-status changes versus the C2-WB merge.** X3-1: success with `notInspected(implicitTiling)` becomes
success with `decodedContentContainment` for default point, vector and `mesh-to-3tz` output (all 17
forest witnesses, subject to the 2^29 cap; none is `workLimit`). X3-2: implicit archives with a proven
escape become InvalidInput (the 32 pre-amendment I1 fixtures, whose payloads skipped the y-up rotation,
are kept as S3-19 C2 controls and left the I1 manifest under I1 oracle amendment 6). The contract's
producer prediction for X3-2, "none", was falsified at integration: `point-cloud` on a degenerate thin
cloud (`tests/native_validate.rs`, `point_archive_and_feature_attributes_use_valid_core_types`) wrote a
row whose lower x face lies half an ulp (2^-52) inside its extreme drawn vertex, because tile bounds were
not rounded outward. Production and O-implicit-C2 both report Outside(own, depth 2, vertex 0). The fix
belongs to the point producer (outward-rounded tile bounds), which merges before slice 3; that test is
unchanged.
X3-3: rowless subtrees on a reference chain report `derivedImplicitVolume`. X3-4: header `metadata`,
rotated rows, `workLimit` and payload reasons replace `implicitTiling`. X3-6: the report enum change.

**Acceptance (R).** Production equals the independent oracle O-implicit-C2 on all 80 slice-3 controls,
the S3-18 sweep of the 181 I1 controls (R5 injection-only), the 34 regenerated forest archives with their
precheck totals, the five 9.D limit pairs and two C1 `references` pairs; I1 parity stays 181/181. All 16
slice-3 production mutants flip their paired control, and the 19 slice-1 mutants still flip. Performance
(§10): mf2-lod3 3.444 s best wall and 104.8 MiB peak RSS (ceiling 4.5 s and 116.9 MiB); C2-off within the
lane rule; per-class streaming 4.57–5.98 ns/unit on a P-core and 8.04–10.07 on an E-core, within
τ_P/τ_E. The `--explain` policy note for I1's validate cases is in the
[I1 contract](i1-implicit-read-contract.md). Full record, hashes and readings:
[slice-3 acceptance](../../bench/architecture_audit/c2_decoded_containment/README.md#slice-3-acceptance-r-2026-10-12).


## H11 amendment (corner fast path, 2026-10-13)

Source: the H11 plan (invariants V1–V8 and the soundness argument),
`~/.cache/rusty-tiles-evidence/h11-20261013/plan.md`, sha256 `c7512d808a889b0f5e793cd6cecb533d3ca8e4957a7b6cdebcef2b8732c3c4a5`. The claim, the domain, the
span definition (B1), the predicates, the work formula and every charge, the caps, the outcome
precedence, the report schema and the arena reservation (B3) are unchanged. The streaming step 3
("for each set bit in ascending order, decode the raw word and run the upward walk") is amended as
follows; its observable outcome is unchanged.

**Algorithm.** Per (instance, POSITION accessor), the marked vertices are taken in ascending chunks of
1,024 indices (16 bitmap words). For a chunk, one pass over its marked indices reads each raw word with
the walk's reader and keeps the componentwise raw minimum and maximum. The distinct corners of that raw
box are walked exactly, with the unchanged walk; if all are Contained the chunk is skipped, otherwise
every marked vertex of the chunk is walked in ascending order, as before. Only the all-minimum and
all-maximum corners are walked when every row `r < 3` of the instance composite and of every tile step
has at most one nonzero entry in columns 0–2 and every tested volume is a box; otherwise up to eight.
A corner test is attempted only when it has fewer corners than marked vertices and the stream's corner
credit covers it: each reference adds `T(u)/64` corner walks, a test spends its corner count and a
skipped chunk refunds its marked count. Accessor `min`/`max` and every other declared bound are never
read by this step.

**Why it is exact.** The decode is monotone per component (the signed clamp included), so every marked
vertex lies in the box spanned by the decoded corners. `W = G₃₃·k` does not depend on the vertex and each
tile step is affine with `W` fixed, so the point tested at each level is affine in the decoded vertex. Box
rows and spheres are convex, and so is their preimage under an affine map; the corners passing therefore
puts the whole box, and every vertex in it, inside every tested volume. Under the diagonal condition each
tested row is a function of one coordinate, so its extremes over the box are its values at the two
diagonal corners, and box rows are tested independently. Corner walks use the same exact arithmetic, so
no rounding is involved. Corners are raw words of the accessor's type and the span bound uses the type's
fixed worst-case pair, so they fit the arena; a corner faults only where a vertex would (`W ≤ 0` is
vertex-independent; non-finite words are refused by C1, and a non-finite word in a chunk sends it to
the per-vertex walk without a corner test). This fault equivalence relies on excluded volumes never
being streamed: a region or rotated box breaches inside the walk only after the earlier volumes pass,
so a corner could fault where the per-vertex walk returns an earlier Outside. Any exclusion
suppresses streaming (outcome precedence: exclusions before the precheck and the stream), so no
such volume reaches a corner walk; a change that streams past an exclusion must revisit this step.

**Work.** Charges are unchanged and admission reads nothing new. After every reference, the walks
performed are at most the marked vertices so far plus `⌊ΣT(u)/64⌋` over the references so far, and each
marked vertex is read at most twice. So "at most 24 limb multiplications per vertex unit" becomes at most
`24·(1 + 1/64)` in the worst case (every chunk inconclusive). C2-WB's τ and the 2^29 cap are
calibrations; the H11 record re-measures an all-inconclusive adversary
([acceptance](../../bench/architecture_audit/c2_decoded_containment/README.md#h11-corner-fast-path-2026-10-13)).

**Acceptance.** Production equals the oracle on every existing suite and on 16 new oracle-decided H11
controls (`h11/`); nine H11 production mutants (two diagonal corners regardless, tile steps ignored by
the diagonal rule, inward-rounded corners, skipped fallback, trusted declared bounds, a box missing the
last vertex, first-vertex report on an inconclusive test, ignored credit, ignored chunk offset) each flip
their paired control or test, and the slice-1 and slice-3 mutant matrix still flips.

## H7 amendment (legacy content Unsupported, PROPOSED 2026-10-13)

**Status: PROPOSED.** This takes effect with the H7 production change; until then the text
above describes `develop`. Rule and invariants:
[C1 amendment H7](c1-legacy-content-unsupported-amendment.md). The claim, the span definition,
the predicates and the C2-WB formula's structure are unchanged.

**Text deltas.**

1. **Domain predicate.** "admitted glTF/b3dm JSON" becomes "admitted glTF JSON". A b3dm, i3dm,
   pnts or cmpt payload never reaches C2. C1 reports it as Unsupported (outcome 1 of R8: C1
   outcomes, first failure in traversal order), whether it is detected by member suffix,
   by its first four bytes or by an implicit content template.
2. **`rtcCenter`.** The reason keeps its position in the frozen 15-name enum but is reserved:
   its only carrier, a b3dm feature table, is no longer admitted, so it is never reported.
   The `gltfExtension` row (`CESIUM_RTC`) is unchanged.
3. **C2-WB `J(u)`.** It counts the glTF JSON or GLB JSON chunk only. The "also inside a b3dm"
   clause and the b3dm feature- and batch-table terms are deleted. `B(u)`, `Dec(u)`,
   `Res(u)` and `Prim(u)` are unchanged.
4. **Ownership and disposition.** The Format owner no longer detects `RTC_CENTER`. The
   `unwrap_b3dm` and `FeatureTable` rows of "Disposition of existing code" are superseded:
   both are deleted.
5. **Oracle.** O-explicit reads GLB and glTF only. Legacy content is the oracle outcome
   `Unsupported` with `phase: "C1"` and facts `{format, member, rule: suffix|magic}`. A
   non-`glb`/`gltf` suffix is Undecided, because C1 refuses it. The composed-oracle pins in
   `c2o/implicit.py` were re-pinned for `check.py`, `gltf.py` and `i1_oracle.py`.

**Exit-status changes versus slice 3.**

- **XH7-1.** Success with `notInspected` `rtcCenter` becomes Unsupported (any b3dm with
  `RTC_CENTER`).
- **XH7-2.** Success, or any C2 outcome, of an archive that holds legacy content becomes
  Unsupported.

**Evidence to regenerate with the production change.** These are kept as committed now, and
all 20 legacy archives among them are Unsupported under the amended oracle:

- witnesses: `W01-ordered-node-U-RTC-tile-chain`, `RTC-binary-form` and `RTC-range-overrun`
  retire. `W01-noRTC-variant` (with its 9 ulp variants) and `matrix-and-TRS` are re-emitted as
  GLB;
- controls: the `RTC-added` pair retires;
- C2-WB: the six `json-*-b3dm*` classes and the mutant `J-without-b3dm-batch-table` retire.
  Their GLB twins remain.
