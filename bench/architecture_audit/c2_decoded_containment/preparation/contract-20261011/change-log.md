# C2 v2 change log

**Revision inputs.** v2 responds to the precode review
`/tmp/rusty-tiles-c2-precode-review-20261011/review.md` (sha256 `7bb397dd…`, verdict
GO-WITH-CHANGES).

**Preserved v1.** v1 is kept byte-for-byte under `v1/`, as the four `.md` files plus `verify/`.

**What this lane did and did not run.**

- Not run: Cargo, rustc, builds or producers.
- No production edits.
- Ran: small single-threaded `python3 -I` probes under `nice -n 10` only.
- All output is under this directory.

**Source pin.** `e83e880`, the final head of PR #155, in `verify/source-pins-e83e880.txt`.

- The pins are git-blob hashes of 29 files. 28 are identical to `70d184f`.
- `docs/CLI.md` changed only in a raster progress row, which nothing here cites.
- The worktree had moved to `1ca7ead` and was not used.

**Verify artifacts.** Hashed in `verify/manifest-v2.txt`.

## Required changes

| Change | What changed | Where |
| --- | --- | --- |
| **R1** D7 exactness | **Arbitrary-precision dyadic integers** were chosen over binary64 expansions. The type is fixed-capacity (4096-bit mantissa), hand-rolled and heap-free. The domain is a static exponent-pair bound over pinned canonical formulas, capped at `containmentSpanBits = 4096`; beyond the cap the archive is `numericSpan` (not inspected). O can compute the bound from the contract text. The "≤ about 40 components" and "depth 128" claims are withdrawn. New probe `verify/span_bound_probe.py` (output `span-bound-observed.json`): the bound stays within about 300 bits of the exact span; producer chains are ≤ 327 bits; the cap admits 16 generic or 12 near-axis rotated levels; this is the expected W1 coverage. The D2 window is now exact, with no subnormal squares | `contract-draft.md` "Exact evaluation"; `open-decisions.md` D7; `ownership-plan.md` M files and interface |
| **R2a** per-test cost | Static per-level units `u = ⌈span/64⌉²`. With the span cap, a unit is at most about 16 limb multiplications, so total work ≤ 16 × `containmentWorkUnits` (2^32) | contract "Work bound" |
| **R2b** quadratic re-walks | The interface changed to `instance_frame` plus a single upward `walk` that tests each level's volumes and then applies `to_parent`. U is an explicit named step | `ownership-plan.md` interface |
| **R2c** repeats leading to ResourceLimit | Distinct referenced vertices are deduplicated per (instance, accessor) with a bitmap of at most `accessorElements` bits. The precheck runs after traversal and before streaming. Every cap overflow becomes `workLimit` (not inspected), never ResourceLimit. Phase-2 re-reads are charged to a separate `containmentBytesRead`. C1's `totalBytesRead` and `total_payload_elements` are not charged again | contract "Work bound"; slice-1 control "9M index occurrences" |
| **R2d** D1 exit-status list | A complete list of exit-status changes, including the new InvalidInput classes from node admission in **any** archive, ResourceLimit only on host allocation refusal, and unchanged counters | contract "Exit-status changes"; `open-decisions.md` D1 |
| **R3** RTC detection | F reworks `FeatureTable` (`records.rs:54`) and `unwrap_b3dm` to expose the presence of the `RTC_CENTER` key in any form, giving `rtcCenter`. `CESIUM_RTC` is not allowlisted, giving `gltfExtension`; it is reachable because C1 requires extension objects to be declared (`payload.rs:355`). Slice-1 controls were added | contract domain table and dispositions; `ownership-plan.md` F; `verify/slice1-witness-expectations.json` |
| **R4** quaternion window | An out-of-window quaternion is not inspected (`quaternionWindow`). Zero, nonfinite and out-of-[−1, 1] components stay InvalidInput. The constant shared with W1 is removed; C2 has its own constant 2^-18, which admits everything W1 admits (the reviewer's arithmetic) | `open-decisions.md` D2; `producer-inventory.md` W1 section and table |
| **R5** interpretations | I1: transitive ancestors (basis lines 86, 752, 790, 835). I2: external-reference `content.boundingVolume` participates. I3: coverage is all-or-nothing per archive. I3 replaces v1's partial streaming and so removes the per-(reference, volume) coverage predicate | contract "Recorded interpretations" |
| **R6** D3 | Marked decided (a) for slice 1. The A2 evidence moved to the slice-3 gate | `open-decisions.md` D3 |
| **R7.1** C1 label | `payload.rs:1102-1103` splits into `"skins, animations, morph targets and rendered extent"` (always) and `"scene transforms"` (removed only on full coverage) | contract "Report and failure schema"; F list |
| **R7.2** public surface | Rewording for `public-surface-inventory.md:40` specified and assigned to R | contract dispositions; `ownership-plan.md` R |
| **R7.3** allowlists | Explicit glTF allowlist with a no-positional-effect rationale per entry; `EXT_mesh_polygon` primitives are tested on all vertices. Tileset allowlist: only `3DTILES_content_gltf_vector`, provisional (O-a). Other tileset, tile, content and boundingVolume extensions are `tilesetExtension`. Tile and content `metadata` is `tileMetadata` | contract "Allowlists" |
| **R7.4** schema | Frozen before code: 15-name reason enum, `notInspected` entry format and Outside template. Recorded that the `ValidationLimits` change is a public API change (review note 10; placement is O-b) | contract "Report and failure schema" |
| **R8** witnesses and precedence | Slice-1 outcome for every literal witness and context recipe, plus 11 new controls. W01 and its RTC-dependent controls get slice-1 expectations: W01 → `rtcCenter`; `matrix-and-TRS` → InvalidInput (node beats exclusion); `RTC-range-overrun` → `rtcCenter` in slice 1. W13 recomputed under the normalized meaning. Explicit outcome precedence: C1 (including node admission per payload, in traversal order) → exclusions → precheck → Outside or ResourceLimit → Contained | `verify/slice1-witness-expectations.json`, `verify/w13-normalized.json`; contract "Outcome precedence" |
| **R9** oracle meshopt and controls | O implements a NONE-filter meshopt decoder (ATTRIBUTES, TRIANGLES, INDICES) from the pinned spec (`602608cb…`); the golden vectors are its self-test only. R hands O the artifacts plus their hashes. Controls are split into data pairs (base and mutant fixtures, both hashed, expected verdicts for each) and implementation mutants (hashed patches for production and oracle separately, each with its paired fixture) | contract acceptance steps 1, 3 and 4; `ownership-plan.md` O and R |

## Non-blocking notes

| Note | Response |
| --- | --- |
| 1. Stale HEAD | Re-pinned to `e83e880`; the contract status was rewritten |
| 2. `records.rs:37` | Corrected: `Root` is at `:37` and `FeatureTable` at `:54` |
| 3. Writer fallback | The inventory now describes `:284-297` as a writer layout fallback |
| 4. P1 pass condition | Restated as "`rounding` ≥ f64 slack"; the same applies to vector |
| 5. Probe limits | The contract states Q2's unconditional widening and Q3's missing padding |
| 6. f32 bound | Tightened to 2^-23 + 2^-48 in D2 |
| 7. Smaller slice | Adopted. Slice 1 accepts only signed-permutation boxes and spheres; rotated boxes are `rotatedBox`; D5 moved out of slice 1 |
| 8. Oracle phases | Split into O-explicit (gates slice 1) and O-implicit (gates D3/A2) |
| 9. PR #155 overlap and `mod containment;` | Recorded as a start gate and an agreed edit |
| 10. Public API | Recorded; placement left as O-b |
| 11. Third quaternion reading | Added to D2's residual evidence, with a worst-case discrepancy estimate |
| 12. Agreed items | Kept |

## Deliberately left open, and why

- **D4 (`hierarchyBounds` retirement).** For implicit archives it is still the only spatial check
  until slice 3. Retiring it is a capability decision for the coordinator.
- **D5 (rotated boxes).** No producer writes them, and keeping them out makes slice 1 smaller.
  The interpretation evidence (CesiumJS OBB semantics) is unpinned.
- **D6 (RTC precision).** No producer writes RTC. Slice 1 only needs detection, which is now
  specified.
- **O-a (`3DTILES_content_gltf_vector`).** This lane has not read the README. Pinning it is O's
  independent task. If it defines positional semantics, explicit vector coverage drops rather
  than becoming a failure.
- **O-b (cap placement).** A cosmetic public-API choice owned by R.
- **Cap values** (4096 bits, 2^32 units, 2 GiB, node depth 64). These are engineering proposals.
  Per-unit timing and corpus totals require Cargo and producer runs, which are outside this lane.
  Retuning a cap never changes semantics, only coverage.
- **D2 consumer formulas** (CesiumJS, Blender loaders, Khronos validator threshold). These are
  unpinned here and could justify tightening the window later. Tightening reduces coverage; it
  never adds failures.
- **I3 versus partial streaming.** All-or-nothing coverage gives up Outside detection inside
  excluded archives. That trade was chosen for a smaller, unambiguous slice 1.
- **Reviewer-requested delta review.** M must not start until R1 and R2 receive a short delta
  review. This lane cannot self-certify that.

---

# v3: interface freeze (delta review)

**Revision inputs.**

- Delta review: `/tmp/rusty-tiles-c2-precode-review-20261011/delta/delta-review.md` (sha256
  `5f4b6fe8da67bbcaeafea9bf0c034cf9a38c471badd75ef631a01b6353aba378`). Verdict: GO-WITH-CHANGES
  for both F and M.
- Coordinator decisions: O-b, and the EXT_mesh_polygon lean.

**Preserved v2.** v2 is kept byte-for-byte under `v2/`, as the five `.md` files plus `verify/`.

**Rules followed.** No Cargo, builds or production edits; single-threaded `python3 -I -B` under
`nice -n 10`; output only under this directory. The source pin is unchanged: `e83e880`, in
`verify/source-pins-e83e880.txt`.

## Mapping

| Item | What changed | Where |
| --- | --- | --- |
| **B1** bit-exact spans | Normative span definition: input pairs (ZERO for ±0; fixed raw-word pairs; exact `k` pairs; the literal 2), the product rule with a new **(0,0) pass-through**, exhaustive term lists for flat sums, `G` starting at the identity, `U`, the tile step, box and sphere quantities, and the per-level spans `sInst`/`s₀`/`s_ℓ`. **Transform-less nodes (`{"mesh":0}`) are skipped.** With pass-through, explicit identity TRS and matrix nodes give the same level spans (297 = 297 = 297). The D2 window is outside `numericSpan` (≤ 2151 bits). Per-URI hull monotonicity is stated. Worked numeric example: max span 29, so the control is in domain at cap 29 and `numericSpan` at 28; 8 vertex units and 128 node units. New probe `verify/frozen_bound_probe.py` (output `frozen-bound-observed.json`) implements the text literally, includes the producer `{"mesh":0}` node, and found 0 violations in 304,455 quantity and prefix checks. It is deterministic: re-run hash identical | `contract-draft.md` "Normative span definition", "Worked example"; `verify/frozen_bound_probe.py`, `verify/frozen-bound-observed.json` |
| **B2** coverage | Admitted depth at 4096 bits: generic 16 box / **10 sphere**; near-axis 14 / **6**; a subnormal-scale quaternion component gives **1 / 0**. All excess is `numericSpan`. Producer chains are ≤ 297 bits; `--quantize` is 90 bits | contract "Expected coverage"; `producer-inventory.md` W1; `open-decisions.md` D7 |
| **B3** heap and peak bytes | Every dyadic lives in one fallible heap `Arena` (`try_reserve_exact`), runtime-sized from `containmentSpanBits`; no stack dyadics. Sizes: 544 B per dyadic at cap 4096; arena of `16·(depth+1) + 32` dyadics = **583,168 B** at depth 64; bitmap ≤ **166,667 B**; level chain ≤ 44 KiB; per-URI facts ≤ 7.5 MiB; tile arena ≤ 21 MiB; references ≤ 4 MiB. Peak C2-added memory ≈ **33.5 MiB facts + 0.8 MiB working** (not an RSS claim). Allocation refusal is **ResourceLimit**, before the first vertex test | contract "Representation and memory"; `ownership-plan.md` frozen interface (`Arena::reserve` → `AllocRefused`) |
| **D1** node work | `Nodes(u)` = Σ over node visits of 1 (skipped node) or `128·⌈s_step/64⌉²`. It is charged per reference | contract "Per-URI facts", "Per reference" |
| **D2** index scanning | `Idx(u)` = Σ over instances and primitives of the index count (or POSITION count for non-indexed). It is charged per reference. The repeated-index control is rebuilt over **3 payloads**: 3 × (9 POSITION + 3,999,990 index elements = 3,999,999 ≤ 4M); 11,999,997 total ≤ 16M; about 12M occurrences. Expected Contained, never ResourceLimit; `workLimit` at one below the sum | contract acceptance step 5; `verify/slice1-witness-expectations.json` |
| **D3** re-decode and revalidation | Per-reference charges: `E(u)` (C1 `elements_checked`) into units; `B(u)` into `containmentBytesRead`; `Dec(u)` (a new C1 materialized-bytes fact) into the new `containmentDecodedBytes` | contract "Per-URI facts", "What the units bound" |
| **D4** fact memory and precheck time | Facts are aggregated **per URI** (the `Pair0` hull plus scalars, 120 B each). The precheck accumulates during traversal in traversal order and **exits at the first violation**, with exactly one precheck reason. Time is O(references × (depth + 1)), about 33.8M pair operations at C1 limits | contract "Precheck (D4)" |
| Frozen formulas | Frozen: `units(r) = T(u)·Σ_ℓ⌈s_ℓ(r)/64⌉² + Nodes(u) + Idx(u) + E(u)`; the definition of `T(u)` (an accessor shared by two primitives of one instance counts once); the per-level `s`; the unit weights; the stream algorithm. Cap **values** stay deferred to R | contract "Work bound"; `ownership-plan.md` reviewer item 5 |
| **EXT_mesh_polygon** | **Decided: only core-primitive referenced indices are marked; `loopIndices` are not draws.** Evidence: C1 types the loop accessors but does not range-check their values against POSITION (`payload.rs:797-835`), and no spec is pinned. This matches the Claim and the bitmap. The Claim now defines "draws". O gives a non-gating all-vertex observation. New control: an out-of-volume loop-only vertex → Contained | contract Claim, allowlist row, "Stream algorithm"; witness JSON; inventory vector row |
| **O-b** | **Decided (coordinator).** Five named `pub u64` fields with `Default` values in `ValidationLimits`: `containment_span_bits` (4096), `containment_work_units` (2^32), `containment_bytes_read` (2 GiB), `containment_decoded_bytes` (2 GiB), `containment_node_depth` (64); serialized in camelCase. The **struct-literal break** is recorded in D1's list of public changes (item 6) and in the contract's exit/public-change table. F owns the `types.rs` edit | contract "Report and failure schema", "Exit-status changes"; `open-decisions.md` D1 item 6, O-b; `ownership-plan.md` F list |
| Interface | Frozen signature set: `Local::Skip`, `LevelFacts`, `SpanCaps`, pure bound functions (`node_step_bound`, `position_bound`, `hull`, `level_spans`, `vertex_units`, `node_units`), `Arena`, `admit_rotation`, `push_node`/`pop_node`, and a non-allocating `walk`. Any change needs M, F and R | `ownership-plan.md` "Interface freeze" |

## Still open, as agreed with the reviewer

- **D4** (`hierarchyBounds` retirement): open.
- **D5** (rotated boxes): open.
- **D6** (RTC precision): open.
- **O-a** (the `3DTILES_content_gltf_vector` README): open.
- **Cap values** (4096, 2^32, 2 GiB, 2 GiB, 64): proposals pending R's measurement of per-unit
  time and corpus totals.
- **Viewer quaternion formulas** and the Khronos validator threshold: open.
- **Optional per-accessor tightening of the position pair** (the delta review's coverage note):
  not adopted. It would change spans and require a contract delta. It only affects coverage.

## Gate

The coordinator confirms that v3 satisfies the delta review's conditions before F and M start
coding. This lane cannot self-certify that.

## v3 deliverable hashes (sha256)

| File | sha256 |
| --- | --- |
| `contract-draft.md` | `f840ff0b5d89b684de0411a714a768cd3307e03ccd79d58b280fd97d354c2538` |
| `producer-inventory.md` | `e391734cc5ef6070528d92d4b913eba1fe27db035095f5cde0f5dcc9dec1e080` |
| `open-decisions.md` | `45a2f150864fbe2a16bbcf02773dc9b56bae5294e76ab84960fb47a4712b3772` |
| `ownership-plan.md` | `99deab3cddd31c4be00c24ac0c7eb9d61f27eb6510cd4948f61d6d9ad8d73a72` |

Verify artifacts:

| File | sha256 |
| --- | --- |
| `verify/frozen-bound-observed.json` | `6ff24749f8fc4d4a56c96e66763e4223f5d7f0f44efd738cd0838e1609fed0ef` |
| `verify/frozen_bound_probe.py` | `694d3de919d7e19f3b1bb976011a79769c80a6f2eadf496857097cf8cac199cf` |
| `verify/model-stdout.txt` | `96ddfb1b497c4d70e67c86c1cabf365a11ecbc1dae803905c27c3547408b17ef` |
| `verify/model.py` | `40080211b50f27e7629bc4b6964e5731290f515c56f3f6ef653f37dd1e9d2b82` |
| `verify/observed.json` | `9d46031bb82b317b124a829ec1f463bd5d70395bde591af33996314420ac96f1` |
| `verify/pin.sh` | `07bff08e18c75f622955ae55269c18459e39b2917a6623fa6840902f8660e94f` |
| `verify/slice1-witness-expectations.json` | `3fd8a08b3b31d943c65a3fa583e00ee5568fa3b5231c655e19e92e2e25b60662` |
| `verify/source-pins-e83e880.txt` | `d7cbf0aeb0da786d4bd58be24966f4e3ad733d791defa7ffd08118888dc86fc0` |
| `verify/span-bound-observed.json` | `04b60bc1f9bf95b2831b5aca90968efe95749db36ab8fbfb35f84ad4b3d2ad14` |
| `verify/span_bound_probe.py` | `a32dd37a18d9796a0469d439c7cd08a1f5eed306fdde8c0a0044802870ab1494` |
| `verify/tight-bounds-observed.json` | `0ff3846b580bac49eb157fd7570e0c4a7df30179f9e47bc7138ee32f656ca4bb` |
| `verify/tight_bounds_probe.py` | `31055de2f01e1f9f971cd3e666caf0b8ec0e96d5ad22b4e27b96f4529c165846` |
| `verify/w13-normalized.json` | `356c36fa2040186f13f71c66c6758b670a0aafd217635332721b17c76f536a17` |
| `verify/witness-check.json` | `fa2fd089e5fb308c2fe3c7508e3ba12a899b76b23743de79bb192313b111dba2` |
| `verify/witness_check.py` | `08d76aa990e92e67b26c009d6652b0717e0693d875ba2256c07ecf11e6682c4b` |

The sha256 of `change-log.md` itself, and of every top-level file, is in `SHA256SUMS` (it cannot be embedded in this file).

## v3.1 (coordinator, delta2 H1)

- `contract-draft.md` Domain paragraph: in-domain is defined per reference from the per-URI hull `Pair0(u)`, matching the precheck. This replaces the per-(reference, instance) wording, which disagreed for multi-instance URIs.
- `verify/slice1-witness-expectations.json`: added the two-instance hull control.
- Reviewer: `delta2/delta2-review.md` sha256 `9647ec15d7096f3bbda8a774bfcb0f912aac9ee439413a483ea81664da4584b3`. Per that review, this fix needs no further review. v3 is preserved under `v3/`.
- ownership-plan.md N checklist: EXT_mesh_polygon line aligned with the v3 decision (referenced indices only).

## v3.2 interface amendments (coordinator R, accepted for M and F; raised independently by F and M)

1. `Normalization` carries the raw word: `enum Normalization { None(Word), I8, U8, I16, U16 }`, `enum Word { F32, Int8, Int16 }`. `position_bound` uses the word's pair from the span table (FLOAT (−149,127); i8/u8 (0,7); i16/u16 (0,15)) for both normalized and unnormalized words. W07 (unnormalized I16) depends on this.
2. `LevelFacts.to_parent` at `levels[l]` is the child tile's transform from frame l−1 into frame l. `levels[0].to_parent` is None, and so is the field for a child without a transform. `walk` does, for each l: apply `levels[l].to_parent` if l>0, then test level l.
3. Within a level, `walk` tests volumes in Role order Own < Content < ExternalContent < Ancestor, regardless of slot position. `k` in `ExternalContent(k)`/`Ancestor(k)` is the level index l (1 = nearest ancestor).
4. `Pair`, `Bound` and `UriBounds` fields are `pub(crate)`. M provides `UriBounds::identity()` (identity G) and `hull(a: &UriBounds, b: &UriBounds) -> UriBounds`. A URI with no instance skips `level_spans` and charges 0 vertex units.
5. `level_spans` returns a non-allocating `SmallSpanList<'a>` view over `(UriBounds, &'a [LevelFacts])`, so it has no allocation failure path.
6. `Arena` has a sticky `fault() -> bool` (capacity-invariant breach) with a debug assertion. F checks it after each `push_node`/`walk` and maps it to InvalidState (internal invariant), never to a verdict.
7. `admit_rotation` uses a fixed admission region inside the same arena: 4 values × 35 limbs, about 1.1 KiB, added to (16·(depth+1)+32) values. F reserves the Arena in phase 1 for admission. The B3 peak grows by that region.
8. M exposes `volume_exclusion(&VolumeFacts) -> Option<Exclusion>` (RegionVolume | RotatedBox). F uses it to collect every reason, so there is one definition. `level_spans` checks region and rotated box before spans, so they take precedence over numericSpan within a reference.
9. Accepted readings: `sInst` is the maximum `s_step` over node paths of mesh instances only; Nodes(u) and maxNodeDepth count every selected node. `s_step` includes a matrix node's raw L entries, per D1 "that node's L quantities". `Pair` arithmetic saturates, and a saturated pair is over the cap; this is unreachable under `containment_node_depth` 64. F does not push composites for selected subtrees without a mesh instance.
10. (coordinator, later) `Arena::fault()` maps to a new public `ValidationFailure::InvalidState(String)`, category ("invalid_state", 1), mirroring the producers' JobErrorKind::InvalidState. This is a public API change added to D1's list. Rejected alternatives: io (mislabels an internal invariant) and panic (validation runs on Python/Blender threads).
11. (coordinator) The C1 dated artifact `bench/architecture_audit/c1_payload_integrity/production_controls/context_controls/2026-10-10/compiled_context_controls.rs` is compiled by `src/lib.rs` as a live test module. F extends its `ValidationLimits` literal and declared JSON with the five C2 fields, at their defaults, in a separate labelled commit. R records the pin amendment (old/new sha256, reason) at integration.

## v3.3 decisions on oracle (O) contract questions (coordinator R)

O's slice-0a report: c2/oracle d201a49; self-test 36/36; 237/237 witness/control agreements; 12/12 mutants flip; corpus 17 Contained, 10 NotInspected(implicitTiling), 2 producer refusals, 0 Outside. Decisions:

1. Worked example. The contract claims only *in domain* at 29 (line ~730). The literal fixture's verdict is Outside: row-1 own half-length 0 at c=0, with vertices at 2.75..3. The Contained-at-cap / numericSpan-at-28 control uses O's adjusted fixture (own-box row-1 centre 3, z = 0), whose spans are identical, [23, 29]. M's at-cap test must use the same adjusted geometry.
2. The B2 coverage table reports depths *tested* (1, 2, 4, …, 16), not maxima. The text is reworded to "at least".
3. A `3DTILES_content_gltf_vector` extension object with `clip: true`, wherever it appears, is treated as a non-allowlisted use: `tilesetExtension` on the tileset/tile/content side, `gltfExtension` on the glTF side. Reason: the pinned README (c48ebdc, sha256 c8d14e4d…) permits clipped content outside the volume. No producer writes `clip`.
4. Traversal order is depth-first pre-order: a tile's contents (array order) before its children (array order). An external tileset is expanded in place at its referencing content. This fixes the first-Outside and precheck-first-violation order.
5. Outside message. `depth` is the tree depth of the tile whose content is referenced (root 0; an external tileset root is its referencing tile's depth + 1). `primitive` is the lowest primitive index in the instance's mesh whose draw references the reported vertex. `vertex` is the POSITION accessor element index.
6. `Nodes(u)` charges every selected node, including nodes in mesh-less subtrees, with `s_step` from the static `node_step_bound` along its path. Pair arithmetic only, no arena. F computes static bounds for all selected nodes but pushes exact composites only for instance paths.
7. `E`, `B` and `Dec` stay internal. Production exposes them through a crate-private test accessor, and acceptance reconciles them with O's model per work control. No public report change.
8. A duplicate in `children` or in a scene's `nodes`, or a node reachable twice, is InvalidInput (glTF node hierarchy must be disjoint trees). A non-affine or non-finite tile transform is InvalidInput (C1's existing `transform()`). O changes its "Undecided" for that case to InvalidInput.
9. Precedence with C1: C2 runs only when C1 succeeds. Where C1 `hierarchyBounds` refuses an archive (e.g. some one-ulp ancestor mutants or the external-content control), the expected production outcome is the C1 failure. The oracle records "C1-precondition" for such cases, and acceptance compares on the C1-passing subset plus the C1 refusal itself.
10. Core 1.1 `contents` arrays: each content entry is a separate reference with its own `content.boundingVolume`, tested in array order.
