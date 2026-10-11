# C2 decoded-content containment: fresh nonauthor precode review

Reviewer: fresh nonauthor. I did not write any material under review.
Reviewed: `/tmp/rusty-tiles-c2-contract-20261011/` (contract-draft.md, producer-inventory.md,
open-decisions.md, ownership-plan.md, verify/), plus the two preparation packages it builds on.
Source: `/tmp/rusty-tiles-r2-foundation`, read-only. No cargo, rustc or build ran. Nothing was
edited outside this directory.

## Verdict: GO-WITH-CHANGES

The architecture holds up:

- an exact zero-tolerance predicate;
- forward evaluation in each volume's own frame, with no inversion;
- U applied exactly once;
- honest per-archive coverage (D1(b));
- explicit-only slice 1 (D3(a));
- no reuse of the `validate.rs` slack helpers;
- an oracle that is independent of `src/`;
- disjoint M/F/O/R file ownership.

The source citations are overwhelmingly accurate.

However, the representation chosen for the exact arithmetic (D7) is wrong as stated. Three other
problems also need fixing before code:

- the slice-1 RTC exclusion cannot be detected with the retained code;
- the work-cap design can turn archives that pass today into ResourceLimit failures;
- D3 is framed so that slice 1 can never start.

All of these are contract amendments, not research blockers. F's record and stream work may start
once R3, R6, R7 and R8 are amended. M must not start until R1 and R2 are amended and get a short
delta review.

## What was checked

### Source pins

All 27 `src/` and `docs/` hashes in `verify/source-pins.txt` match both `70d184f` and the current
HEAD `27fffd2`. The commit diff between the two touches only `src/raster.rs` and
`docs/architecture/r2-raster-contract.md`.

Compared with `origin/develop` (`6c5ea2a`), every pinned file is identical except `src/main.rs` and
`docs/CLI.md`. Line numbers cited in `main.rs` are correct at HEAD and shifted by +20 relative to
develop.

### Claims confirmed against source

| Area | Claims confirmed |
| --- | --- |
| `validate.rs` | 197-338 (helpers); 274 and 278 (1e-6 slack); 246 (1e-8 + 1e-12 orthogonality); 385-400 (`ValidationTask`); 402-404 (URI cache); 413-423 (kinds); 560-571; 575-608; 643-650; 679-685; 752-797; 892; 908-919; 965-979 |
| `validate/types.rs` | 5-17 (no options); 18-28 (four failures); 50-66 (exactly 15 `ValidationLimits` fields); `bindings/python/src/lib.rs:976-981` |
| `payload.rs` | 109-140 (`normalized()` returns a rounded f64); 292-337; 340-369; 396-401; 419-421; 447-451; 736-760; modes `[0,1,3,4]` only at 790-791; meshopt filter NONE only at 271-272 |
| `implicit/tileset.rs` | 117-121 (`implicitChildIndex` fallback to array index); 149-224 (translation at line 190 is `[dx, dz, −dy]` for roots of all scenes); 284-297; 403-418; 718-733; 1135 (metadata box substitution) |
| `metadata.rs` / `implicit.rs` | `metadata.rs:174-180`; `implicit.rs:188-208`, 260-292 |
| `mesh_archive/source.rs` | 660-668; 673-678; 681-683; 684-692 (`|dot| > 1e-6·|a||b|` gives Unsupported); 711-713; 714-717 (`|‖q‖−1| > 1e-6` gives InvalidInput) |
| W1 bounds and placement | `original_bounds.rs:128-139` (hull of raw and normalized); `placement.rs:11`, 84-125, 155-169 |
| `bbox.rs` | 216-224 (`aabb_to_box` midpoint, no outward step); 68-81 (first-scene fallback) |
| `partition.rs` | 21-45 (`next_up`, exact 0 on degenerate axes) |
| Point cloud | `tiles.rs:109-115`, 140-142, 300-316; `tileset_node.rs:7-9`, 57-72; `point_cloud.rs:585-608` |
| Vector | `model.rs:183-190`; `store.rs:500-529`, 976-982; `encoding.rs:593-632` |
| Terrain | `terrain.rs:246`, 322-331, 619-622 |
| `glb.rs` | 312-321 |
| Spec text | `3d-tiles-core.adoc:86`, 167, 752 and 790 quotes are accurate; transform semantics are at 390, 398-405 and 422; external tilesets at 738-742 |

### Probes re-executed

I re-ran `verify/` from a copy under `rerun/`:

- `tight-bounds-observed.json`, `observed.json` and `witness-check.json` are byte-identical to the
  author's outputs.

I also wrote and ran a new probe, `span_probe.py`, with output in `span-probe-observed.json`. It is
described under R1.

## Required changes before implementation

### R1. D7: the exact-arithmetic domain is a bit-span limit, not a magnitude limit (blocks M)

**Where.** `open-decisions.md:248-274`, `contract-draft.md:153-170`,
`ownership-plan.md:106-111` and `:196`.

**What is wrong in the current text:**

- It claims that "nonoverlapping expansions, whose length is bounded by the f64 exponent span (≤
  about 40 components)".
- It describes "a short polynomial" for point images.
- It proposes a guard that is mainly about magnitude ("magnitudes near 2^±1000, subnormal
  coordinates").

**Why that is wrong.** Exact binary64 expansions fail when any TwoProduct error term underflows. In
the Dekker or Boldo condition, the operand exponents must satisfy `e_a + e_b ≥ −1022 + 53`, and this
applies to every *component*, not only to inputs. Low-order expansion components are tiny by
construction. The span of the exact value grows with every rotated level.

**Evidence.** I measured this with `span_probe.py`, using the contract's step-wise homogeneous form
`N' = n'·D·T' + H(q')·S'·N`, `D' = n'·D`, with no rescaling:

| Rotation style | Underflows below 2^-1074 at node depth | Exceeds the whole 2098-bit binary64 span at depth |
| --- | --- | --- |
| Generic f64 quaternion | 8 | 16 |
| Exporter-style near-axis quaternion (a component of about 1e-17) | 4 | 8 |

At depth 12 (generic), even perfectly packed 53-bit components would need at least 38 components.
Shewchuk nonoverlap does **not** imply 53-bit packing; the length is bounded only by the number of
set bits. The planned control "deep nested rotations (node depth 128)" (`open-decisions.md:268`)
therefore cannot be exact under the stated representation.

**Required amendment.** Pick and state one representation:

- **(i) Arbitrary-precision dyadic integers.** Use a hand-rolled fixed-limb type or an explicitly
  decided direct dependency. `num-bigint` exists only transitively in `Cargo.lock`. The domain is
  then simply "finite inputs", and the cost is bounded by explicit bit and step caps.
- **(ii) binary64 expansions.** This needs:
  - a run-time guard on every TwoProduct and TwoSum operand pair, including `|a| ≤ 2^995` for the
    Dekker split;
  - optional power-of-two rescaling;
  - `numericRange` beyond the guard.

**Further requirements, whichever option is chosen:**

- Delete or prove the "≤ about 40" claim.
- State the domain as a predicate that the oracle author O can evaluate independently from the
  contract text. Without that, `numericRange` expectations are not oracle-checkable.
- Record the expected coverage consequence for W1 user models. Under (ii), roughly 4–8 rotated
  levels become `numericRange`.
- Note that the D2 exact window comparison of `‖q‖²` falls under the same rule: components below
  about 2^-537 square into subnormals.

### R2. Work bounds, interface cost and undeclared exit-status changes

**Where.** `contract-draft.md:76`, `:204-219` and `:254`; `ownership-plan.md:93-97`;
`open-decisions.md:52-55`.

**a. Per-test cost is unbounded.** `containmentTests` counts position × volume pairs. The cost of a
single test grows with chain length (node depth + tile depth, up to 256) and with operand length
(R1). Add a step or work counter, or a combined C2 depth cap, so that total work has a stated bound.

**b. The interface multiplies work by ancestor count.** The frozen signature
`test(volume, tile_steps, nodes, raw, n)` is called once per (position, volume). It therefore
re-walks the full node and tile chain for every ancestor, which costs O(depth²) per position.
Replace it with a single upward walk per position. The walk tests the own volume, then the content
volume, then each ancestor in order, reusing the image at each step. Make the U step explicit in
the signature.

**c. "Repeats are charged" against a 64M test cap regresses valid archives.** C1 caps
`total_payload_elements` at 16M components, counting index components too (`payload.rs:416-441`,
`:640-654`). That allows about 9M index occurrences in a C1-passing indexed mesh. At 7 or more
volumes per position, this exceeds 64M and gives ResourceLimit on an archive that passes today. Do
one of the following:

- Test distinct referenced vertices per (instance, accessor), using a bounded visited bitmap of at
  most `accessorElements` bits. Undrawn vertices are still excluded, so the "visit undrawn vertices"
  control still bites.
- Derive the caps from existing limits so that no single-reference archive within the C1 limits can
  reach them.

An optional exact shortcut: once a child volume is exactly proven to lie inside its parent, testing
against further ancestors through that edge is implied. This is an exact implication, not a filter.

**d. D1's evidence list is incomplete.** It expects only proven-Outside changes. It must also
enumerate:

- new ResourceLimit outcomes from the four new counters;
- re-reads charged to `totalBytesRead`;
- any recharge of `total_payload_elements` caused by re-streaming. The contract does not say
  whether re-streaming recharges it; state this.
- new InvalidInput classes from node admission: cycles, shared children, matrix together with TRS,
  non-affine matrices, zero quaternions, and the D2 window (see R4).

### R3. Slice 1 cannot detect `RTC_CENTER` with the code it retains

**Where.** `contract-draft.md:69` and `:258`.

**What the source does.** `records::FeatureTable` admits only `BATCH_LENGTH`
(`payload/records.rs:54`). `unwrap_b3dm` (`payload.rs:292-337`) returns only the GLB slice. There
are no "RTC facts" to retain.

**Consequence.** A third-party b3dm with `RTC_CENTER` would be tested in the wrong frame. That gives
either a false InvalidInput or a false Contained.

**Required amendment.**

- Change the disposition to "Rework (F): expose presence of the `RTC_CENTER` key", in either the
  array form or the `{byteOffset}` form, and route it to `rtcCenter`.
- Confirm that `CESIUM_RTC` reaches the positional-extension exclusion.
- Add a slice-1 control.

### R4. D2: arithmetic correct; failure class and ownership need deciding

**Arithmetic.** I checked it exactly. `|‖q‖−1| ≤ 1e-6` implies `|‖q‖²−1| ≤ 2.000001e-6`. That
remains true after allowing for W1's f64 evaluation slack (2.0000010018e-6). Both values are below
2^-18 (3.8147e-6) and above 2^-20 (9.537e-7). So 2^-18 does admit everything W1 admits, and 2^-20
would not.

**Failure class.** Making the window a new hard InvalidInput is inconsistent with D2(iii). D2(iii)
tolerates non-decomposable matrices because soundness does not need the check, and soundness does
not need the quaternion window either. Do one of the following:

- map out-of-window to a `notInspected` reason (my preference, which is consistent with D1(b));
- keep InvalidInput, but list it as a D1 capability change once the Khronos validator threshold has
  been pinned.

Zero or nonfinite q stays InvalidInput.

**Ownership.** Remove or reconcile the inventory's "single format-owned constant shared by W1
admission and the C2 node preflight" (`producer-inventory.md:241-243`). It conflicts with two rules:

- #125: "no writer/validator shared helper";
- `ownership-plan.md:156`, which puts `src/mesh_archive/**` out of bounds.

### R5. State the frame-chain interpretations that turn into InvalidInput

**a. Ancestor reading.** Both core text passages (`3d-tiles-core.adoc:86` and `:752`) literally say
"content for child tiles are completely inside the parent's bounding volume". The contract applies
this to **every** ancestor (`contract-draft.md:35`). That transitive reading is defensible:

- hierarchical culling (`:835`);
- "a parent tile completely encloses all of its children" (`:790`).

It is still an interpretation, and it makes an Outside against a grandparent fail the archive.
Record it as a decision with that basis.

**b. Content volume of a `.json` reference.** Decide whether the `content.boundingVolume` of a tile
whose content is an external tileset participates for that tileset's payloads.

**c. Coverage in partially covered archives.** Define the per-(reference, volume) in-domain
predicate. For example:

- a region ancestor that sits above an in-domain own box;
- explicit ancestors above an implicit root.

`contract-draft.md:86-88` says that in-domain references are still streamed, but it does not say
which references these are.

### R6. Restate D3 as decided for slice 1

`open-decisions.md:14` says that D1–D3 block slice 1. D3's "evidence to settle"
(`:158-164`) begins with "A2's accepted effective-volume/ordinal owner", which is held. Read
literally, slice 1 can therefore never start.

The evidence already settles option (a) at the source level:

- implicit volumes are `TILE_BOUNDING_BOX` rows (`implicit/tileset.rs:1135`);
- vector uses array-slot cells (`:117-121`);
- the expander honours only `rustyTile` (`:718-733`).

Mark D3(a) as decided for slice 1, and move the A2 evidence to the slice-3 gate.

### R7. Report coherence, public surface and the allowlist (settle with D1 before code)

1. **C1 payload label.** `payload.rs:1102-1103` still inserts `"scene transforms, skins, animations,
   morph targets and rendered bounds"` for every payload. That would contradict
   `decodedContentContainment` in `checks`. Specify the rewording and give it to F.
2. **Public surface inventory.** `docs/architecture/public-surface-inventory.md:40` states that
   "decoded world-content bounds remain unclaimed". Add the file to the F or R list.
3. **Positional-extension allowlist.** Enumerate it explicitly, with a no-positional-effect
   rationale for each entry. It must cover the producer extensions, or default explicit vector
   output is never covered:
   - `EXT_mesh_features`
   - `EXT_structural_metadata`
   - `EXT_mesh_polygon`
   - `KHR_materials_unlit`
   - `KHR_mesh_primitive_restart`
   - `KHR_mesh_quantization`
   - `EXT_meshopt_compression`
   - texture extensions used by the legacy routes

   It must also state how tileset-, tile- and bounding-volume-level extensions are treated, for
   example `3DTILES_content_gltf_vector` and `3DTILES_bounding_volume_*`.
4. **Report schema.** Freeze the reason enum and the Outside fact schema before code, as already
   listed in D1.

### R8. Witness expectations and outcome precedence

**Context controls under D1(b).** The prior `witnesses.json` context controls expect "C2
Unsupported" for:

- ambiguous scene;
- unknown node positional extension;
- morph targets;
- skin;
- animation;
- GPU instancing;
- region.

Under D1(b), each of these must become `notInspected` plus a reason.

**RTC-dependent witnesses in slice 1:**

- W01 is a b3dm that uses RTC, and so are its derived controls `matrix-and-TRS`, `RTC-binary-form`
  and `RTC-range-overrun`.
- They need slice-1 expectations.

**Outcome precedence.** Add an explicit order of outcomes:

- malformed-node InvalidInput;
- not-inspected exclusion;
- ResourceLimit;
- Outside.

`matrix-and-TRS` on an RTC b3dm is exactly the case where this order matters.

### R9. The oracle must cover the meshopt producers

**The problem.** "Decode only through pinned upstream golden vectors, otherwise out of oracle scope"
(`ownership-plan.md:45-46`, `contract-draft.md:319-320`) cannot decode arbitrary corpus files.
Two corpus rows would therefore have no independent verdict:

- vector `--meshopt`;
- legacy `mesh-to-3tz`, which uses meshopt by default.

**Required amendment.** Do one of the following:

- O implements NONE-filter meshopt decoding from the pinned spec
  (`primary/EXT_meshopt_compression.md`), using the golden vectors only as its self-test;
- those rows are declared unaccepted, and the claim excludes them.

**Also fix:**

- R must hand O the artifacts as well as their hashes (`ownership-plan.md:65`).
- Each sensitive control must be specified as a pair of a fixture and a hashed mutant patch, for
  both production and the oracle. The current list mixes data controls and code mutants.

## Non-blocking notes

1. **Stale HEAD in the contract.** `contract-draft.md:5` cites HEAD `70d184fb`, while
   `source-pins.txt` records `27fffd27`. The content of every cited file is identical, so only the
   text needs fixing.
2. **Wrong line for `Root`.** `contract-draft.md:71` and `:257` cite `records.rs:40` for `Root`. The
   record is at `:37`; line 40 is `View`.
3. **The derived-cell "check" is a layout fallback.** `implicit/tileset.rs:284-297`, described at
   `producer-inventory.md:141-142`, is a writer fallback to `write_chunks` with external roots, not
   a validation gate. The conclusion is unaffected.
4. **P1 pass condition is overstated.** `producer-inventory.md:65` says "Pass whenever
   `rounding > 0`". `rounding` can be positive yet smaller than one f64 ulp of `extent/2`. The real
   condition is that `rounding` is at least the f64 slack. The same applies to vector.
5. **Limits of `tight_bounds_probe.py`:**
   - Q2's 40,000/40,000 comes from widening unconditionally, even when the subtraction is exact.
   - Q3 omits the `rounding` / `positionRoundingMetres` padding that P1 and vector add. Its 2,546
     Outside results are not a P1/vector prediction (`contract-draft.md:146-147`).
6. **The f32 quaternion bound is loose.** The stated bound is 2^-22 + 2^-46
   (`open-decisions.md:89-90`). The tight bound is 2^-23 + 2^-48. The stated bound is still valid.
7. **Consider a smaller slice 1.** Diagonal and signed-permutation boxes plus spheres would be
   enough. General boxes could move to a later slice, since no producer writes them and Cramer adds
   degree-3 bit growth. D5 would then leave slice 1, which better fits AGENTS.md's "smallest
   representation".
8. **Decouple the oracle phases.** Slice 0's exit criterion (`ownership-plan.md:67-69`) includes
   implicit oracle decoding. Split it so that the explicit oracle gates slice 1 and the implicit
   oracle gates D3/A2.
9. **Overlap with R2.** PR #155 touches:
   - `docs/CLI.md`
   - `docs/architecture/README.md`
   - `public-surface-inventory.md`
   - `src/main.rs`
   - `tests/cli_contract.rs`

   Start from post-merge develop.

   There is no overlap with `src/raster*` or `src/geospatial/native.rs`, and M is correctly
   forbidden from reading `raster/grid.rs`. M's module needs `mod containment;` in `validate.rs`,
   which is F's file, so agree that edit up front.
10. **`ValidationLimits` is a public API change.** It is a `pub` struct with `pub` fields and no
    `#[non_exhaustive]` (`types.rs:50-66`). Adding fields breaks Rust struct literals, not only the
    report. Record it as such.
11. **Possible third quaternion meaning (unverified).** From memory, which I did not check, CesiumJS
    `Matrix3.fromQuaternion` uses the homogeneous `H(q)` *without* dividing by `‖q‖²`. If so, that
    is a third meaning for non-unit q, and it makes pinning the consumer formula (already listed in
    D2) material.
12. **Correctly settled items.** I agree with:
    - D1(b);
    - D4 (unchanged in slice 1);
    - D5, if it is kept;
    - D6 (deferred);
    - forward-only evaluation with singular tile transforms supported;
    - not reusing `validate.rs:197-338`;
    - keeping the URI cache for integrity facts only.
