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
