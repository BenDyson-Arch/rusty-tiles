# C2 implementation ownership plan (v3, interface frozen)

## Status

v2 responded to the precode review. v3 freezes the interface per the delta review. v1 and v2
are preserved in `v1/` and `v2/`.

## Roles

Five roles, held by distinct people:

| Role | Who | Notes |
| --- | --- | --- |
| **O** | Oracle author | Never sees an implementation diff before O-explicit is frozen |
| **M** | Math author | — |
| **F** | Format/operation author | — |
| **R** | Coordinator | Cargo, corpus, CI, docs, merge |
| **N** | Fresh nonauthor reviewer | — |

Separation rules:

- O, M and F are three different people.
- N is none of O, M or F.
- R may also act as F only if a different person acts as N.

Everyone follows the AGENTS.md CPU policy:

- two workers;
- `nice -n 10`;
- only R runs Cargo.

**Start gates:**

- **All implementation:** starts from `develop` after PR #155 merges, re-pinned. PR #155 touches
  `src/main.rs`, `docs/CLI.md`, `docs/architecture/README.md`, `public-surface-inventory.md` and
  `tests/cli_contract.rs`.
- **F and M:** the delta review returned GO-WITH-CHANGES for both, conditional on B1–B3 and D1–D4
  being folded in. v3 folds them in.
- **Before coding:** the coordinator confirms that the v3 freeze satisfies those conditions. Once
  confirmed, the interface below is frozen for both authors.

## Slice 0a: O-explicit oracle (gates slice 1)

**O owns these new files** under `bench/architecture_audit/c2_decoded_containment/`:

- `README.md`: provenance, primary pins and replay.
- `oracle/`: pure Python 3 with `fractions` and `int`, and no third-party or repository imports.
  It provides:
  - GLB/glTF/b3dm reading, including b3dm feature-table `RTC_CENTER` key detection;
  - accessor decoding: f32, integer and normalized types, the signed clamp, and viewless
    accessors;
  - draw-index expansion that skips restart values;
  - **a NONE-filter EXT_meshopt_compression decoder** for the ATTRIBUTES, TRIANGLES and INDICES
    bitstreams, written from `primary/EXT_meshopt_compression.md` (sha256 `602608cb…`). The
    upstream golden vectors serve as its self-test only (R9);
  - the scene/node walk under the contract's admission rules;
  - the canonical formulas and the **normative span definition (v3)**, implemented independently
    from the contract text. The contract's worked example is a bit-exact self-test: `sInst = 5`,
    level spans `[23, 29]`;
  - signed-permutation box and sphere predicates;
  - the domain predicate, the 15-reason enum and the precheck units;
  - tileset traversal, including external tilesets, I1/I2 levels, and the tileset and glTF
    allowlists.
- `oracle/self_test/`: the meshopt golden vectors and exact arithmetic identities.
- `witnesses/`: materializes the prior 13 literal witnesses and the 17 context recipes per
  `verify/slice1-witness-expectations.json` (W13 uses the normalized meaning), plus the new
  slice-1 controls listed there. Every Contained case gets a one-ulp-outside mutant.
- `controls/`: data-control pairs, as a base fixture plus a mutated fixture, both hashed, with
  expected verdicts for each.
- `corpus/manifest.json`: per producer output, records the artifact hash, mode and oracle
  outcome (Contained, Outside with facts, or NotInspected with reasons).

**Inputs O may use:**

- the primary specs;
- `contract-draft.md`;
- `verify/slice1-witness-expectations.json`;
- artifacts from R.

O must not read `src/`. O also pins the `3DTILES_content_gltf_vector` README at revision
`c48ebdc` (open item O-a).

**R generates the producer corpus** and hands O **the artifact files and their hashes** (R9):

| Producer | Outputs |
| --- | --- |
| Vector | Plain, `--quantize` and `--meshopt`, each default and `--explicit` |
| Point cloud | Default and `--explicit` |
| F1c2 mesh | One output |
| W1 | Local and Wgs84, including near-window quaternions and deep rotated nesting |
| Legacy `mesh-to-3tz` | Explicit and implicit (meshopt by default) |
| `convert-to-implicit` | One output |

**Exit criterion.** Every producer row has an oracle outcome, and each prediction in
`producer-inventory.md` is confirmed or corrected.

## Slice 0b: O-implicit oracle (gates D3/A2, not slice 1)

Covers subtree availability and `TILE_BOUNDING_BOX` row decoding, per the A2 plan. It is
scheduled with A2.

## Slice 1: explicit `.3tz` production check (M and F)

### Interface freeze (FROZEN in v3; changes need M, F and R)

This is the frozen interface, crate-private, in `src/validate/containment.rs`. F adds
`mod containment;` to `validate.rs`; this one-line edit is agreed in advance. Everything here
follows the contract's "Normative span definition" and "Work bound" sections.

```text
// ---- fixed-size inputs (stack): no serde, Value or I/O ----
enum Normalization { None, I8, U8, I16, U16 }            // k = 1, 127, 255, 32767, 65535
enum RawVertex { F32([f32;3]), Int([i32;3]) }             // raw words; the signed clamp is applied inside M
enum Local { Skip, Matrix([f64;16]), Trs { t:[f64;3], q:[f64;4], s:[f64;3] } }  // Skip = no transform fields
enum VolumeFacts { Box([f64;12]), Sphere([f64;4]), Region }
struct LevelFacts { to_parent: Option<[f64;16]>,         // None for the own level; None for a child without transform
                    volumes: [Option<(Role, VolumeFacts)>; 3] } // own/ancestor volume, content, externalContent
enum Role { Own, Content, ExternalContent(u32), Ancestor(u32) }
struct SpanCaps { span_bits: u64, node_depth: u64 }      // copied from ValidationLimits by F

// ---- static bound (pure; O re-implements from the contract text) ----
struct Pair(i32, i32);  enum Bound { Zero, P(Pair) }      // normative pair arithmetic
struct UriBounds { pair0: [Bound;4], s_inst: u32, s_pos: u32 }   // the per-URI hull (D4)
fn node_step_bound(g: &[[Bound;4];4], l: &Local) -> ([[Bound;4];4], u32 /*s_step*/)
fn position_bound(g: &[[Bound;4];4], n: Normalization) -> ([Bound;4] /*P0,P1,P2 after U, W*/, u32 /*s*/)
fn hull(a: &UriBounds, b: ...) -> UriBounds
fn level_spans(u: &UriBounds, levels: &[LevelFacts], caps: SpanCaps)
     -> Result<SmallSpanList /* s_0..s_L */, Exclusion /* NumericSpan | RegionVolume | RotatedBox */>
fn vertex_units(t_count: u64, spans: &SmallSpanList) -> Option<u64>      // T·Σ⌈s/64⌉², checked
fn node_units(s_step: u32) -> u64                                        // 1 for Skip, else 128·⌈s/64⌉²

// ---- exact evaluation (heap arena; fallible) ----
struct Arena { /* Vec<u64> limbs, reserved once with try_reserve_exact */ }
fn Arena::reserve(caps: SpanCaps) -> Result<Arena, AllocRefused>   // (16·(node_depth+1)+32) dyadics
fn admit_rotation(q: [f64;4], arena: &mut Arena) -> RotationOutcome      // Ok | InvalidZeroOrRange | QuaternionWindow (exact Σq²−1 vs 2^-18)
fn push_node(arena: &mut Arena, l: &Local)                               // composite stack push; Skip pushes nothing
fn pop_node(arena: &mut Arena)
fn walk(arena: &mut Arena, n: Normalization, raw: RawVertex, levels: &[LevelFacts]) -> Verdict
     // applies the top composite, then U (named step), then for each level tests its volumes in
     // Role order before applying to_parent. It never allocates.
enum Verdict { Contained, Outside { role: Role, level: u32 } }
```

**Notes on the frozen interface:**

- **No stack dyadics.** No dyadic value or array lives on the stack (B3); `Arena` is the only
  owner.
- **Allocation failure.** `AllocRefused` maps to **ResourceLimit** in `validate.rs`.
- **Capacity.** The 4096-bit capacity is runtime-sized from `containmentSpanBits`; it is not a
  const generic.
- **Who calls what.** F computes `UriBounds` (via `node_step_bound`/`position_bound`/`hull`)
  and `T`, `Nodes`, `Idx`, `E`, `Dec`, `B` and `maxNodeDepth` during C1's first inspection of a
  URI. `level_spans` and `vertex_units` run per reference in the precheck.
- **Bounded output.** `SmallSpanList` holds at most `hierarchy_depth + 1 = 129` u32 values in a
  fallible heap `Vec`.

### M owns (only these files)

- **`src/validate/containment.rs`.** The frozen interface above: the normative bound functions,
  admission, `walk` and the predicates.
- **`src/validate/containment/dyadic.rs`.** The arena-backed, sign-magnitude dyadic type:
  add, sub, mul, compare.
  - The capacity is `⌈(span_bits + 64)/64⌉` limbs.
  - Exceeding capacity is a debug assertion and an internal-invariant error. It can only happen
    if the static bound is wrong.
  - An optional i128 fast path may decide the same predicate.
- **Unit tests in those files:**
  - the worked example, bit-exact: `sInst = 5`, `s = [23, 29]`, 8 vertex units, 128 node units;
  - at cap and one over;
  - identity-node equivalence (`Skip` versus identity TRS versus identity matrix);
  - flat-sum `⌈log2 m⌉` for m = 2, 3, 4;
  - the (0,0) pass-through;
  - ZERO for `−0.0`;
  - the signed clamp;
  - `k` denominators;
  - sphere doubling;
  - the tiny-quaternion `numericSpan`;
  - one-ulp boundary pairs;
  - zero half-lengths;
  - singular tile transforms;
  - `AllocRefused` under an injected tiny reservation.

  Tests may use O's witness *data* (JSON only).

### F owns

- **`src/content_integrity/payload/records.rs`.**
  - Adds `scene`, `scenes`, `nodes`, `skins` and `animations` to `Root` (`:37`).
  - Adds a `Node` record, plus `weights` and `targets`.
  - Adds `RTC_CENTER` presence to `FeatureTable` (`:54`) (R3).
- **`src/content_integrity/payload/scene.rs`** (new). Responsibilities:
  - graph admission (InvalidInput rules in the contract);
  - scene selection;
  - exclusion facts;
  - per-instance static facts;
  - the phase-2 event walk.
- **`src/content_integrity/payload.rs`.**
  - `unwrap_b3dm` returns RTC_CENTER presence (R3).
  - `inspect` returns phase-1 C2 facts and accepts a phase-2 consumer callback that has a
    distinct error type. The phase-1 facts are `UriBounds`, `T`, `Nodes`, `Idx`, `E`, `Dec` (a
    new materialized-bytes fact), `B` and `maxNodeDepth`, aggregated per URI (D4).
  - The label at `:1102-1103` is split (R7.1).
  - There is no positions `Vec` and no second decoder.
- **`src/content_integrity.rs`.** Crate-private re-exports only.
- **`src/validate.rs`.**
  - The level chain replaces `parent: Option<Volume>`.
  - Exclusion collection over tileset and glTF allowlists, metadata, regions and up-axis.
  - The precheck: accumulated during traversal in traversal order, with early exit at the first
    violation. It covers units, bytes, decoded bytes and node depth (D1–D4).
  - The per-URI fact cache, at 120 B per URI; and the tile arena and reference list, all
    fallibly reserved (B3). Allocation refusal is ResourceLimit.
  - Serial per-reference streaming, with the URI cache kept for integrity and static facts only.
  - The distinct-vertex bitmap.
  - Outcome precedence.
  - Label and reason assembly.
  - Removing `"scene transforms"` on outcome 5.
  - The old `transform`/`inside`/`contains`/`scale_bound` stay untouched (D4).
- **`src/validate/types.rs` (O-b decided).** Add exactly five `pub u64` fields to
  `ValidationLimits`, with `Default` values and camelCase serialization:
  - `containment_span_bits` = 4096;
  - `containment_work_units` = 4,294,967,296;
  - `containment_bytes_read` = 2,147,483,648;
  - `containment_decoded_bytes` = 2,147,483,648;
  - `containment_node_depth` = 64.

  The values are R-measured proposals. This is a public struct-literal break, recorded in the
  migration notes.
- **Tests.** Rust integration tests that consume O's materialized fixtures by hash, and Python
  report tests in `bindings/python/tests/`.

### Out of bounds for M and F

- producers: `src/vector/**`, `src/point_cloud/**`, `src/mesh_archive/**`, `src/terrain*`,
  `src/implicit*`, `src/tile.rs`, `src/tileset.rs`;
- `src/archive3tz/**`;
- `content_integrity/json.rs` and `content_integrity/numbers.rs`.

Producer fixes for oracle-confirmed escapes are separate follow-ups, owned by each producer's
migration.

### R owns

- builds and focused test runs;
- corpus generation, and regeneration on the final source;
- implementation-mutant patches for both production and oracle (R9), each hashed and run
  against its paired fixture;
- exact-head CI, installed-wheel and official Blender checks;
- pins;
- `bench/architecture_audit/c2_decoded_containment/acceptance/`;
- documentation:
  - `docs/VALIDATION.md` and `docs/CLI.md`;
  - `docs/architecture/public-surface-inventory.md:40`, reworded per the contract (R7.2);
  - the final `docs/architecture/c2-decoded-containment-contract.md` and its README entry;
- the O-b decision.

## What the nonauthor reviewer (N) must check

1. **Independence.**
   - O imports nothing from the repository, including in its meshopt decoder.
   - M does not mirror producer bound code.
   - Witness expectations were not edited to match production.
   - Mutant patches are hashed.
2. **Frame order.**
   - The node composite is built root-first in parent·local order.
   - U is applied exactly once.
   - Each level tests its volumes before applying `to_parent`.
   - An ancestor's own transform is never applied before testing that ancestor.
   - External roots are included, and external-content volumes are tested at the referencing
     level (I2).
   - Nothing is inverted.
3. **Exactness.**
   - C2 code contains no tolerance constant. Grep `src/validate/containment*` and the new
     `validate.rs` paths for `1e-`, `EPSILON`, `next_up` and `as f32`.
   - No rounded f64 point or normalized quotient is used.
   - The homogeneous `H(q)` and `‖q‖²` are used as specified.
   - The signed clamp is applied before the rational.
   - The fixed capacity is covered by the static bound: an overflow is an assertion, not a
     silent wrap.
   - The fast path, if present, is equivalent to the general path.
4. **Domain.**
   - The exclusions match the frozen enum and the allowlists exactly.
   - `RTC_CENTER` is detected in both JSON and binary form.
   - `CESIUM_RTC` and `EXT_mesh_gpu_instancing` yield `gltfExtension`.
   - Primitives with `EXT_mesh_polygon` are tested only on indices referenced by their core draw (v3); loop vertices are not drawn.
   - Coverage is all-or-nothing per archive (I3).
5. **Work.**
   - Distinct-vertex dedupe works per (instance, accessor), and undrawn vertices are never
     tested.
   - `EXT_mesh_polygon` loops are not marked.
   - The precheck runs during traversal with early exit. Its formulas match "Work bound"
     exactly: `units = T·Σ⌈s_ℓ/64⌉² + Nodes + Idx + E`, plus bytes, decoded bytes and node depth.
   - The per-URI facts are aggregated as a hull.
   - No new counter yields ResourceLimit.
   - `totalBytesRead` and `total_payload_elements` are not charged by phase 2.
   - All dyadic storage, the bitmap and the fact tables are fallible heap allocations; refusal
     maps to ResourceLimit. No dyadic array is on the stack.
   - The worked example and the at-cap controls reproduce bit-exactly in both M's tests and O.
6. **Report.**
   - Outcome precedence is exactly as in the contract.
   - The label appears only for outcome 5.
   - The reason entries are sorted and deduplicated.
   - The Outside template carries only member names and integers.
   - The payload-label split is in place.
   - The `ValidationLimits` change is recorded as a public API change.
7. **Controls.**
   - Every data pair yields both expected verdicts in production and in the oracle.
   - Every implementation mutant flips its paired fixture in its own implementation.
   - A mutant that flips nothing fails review.
8. **Corpus.**
   - Production equals the oracle on every row.
   - Every Outside, `workLimit` or `numericSpan` row has a disposition.
   - Wrong inventory predictions have been corrected.
9. **Scope.**
   - No claim about implicit, RTC, regions, rotated boxes, deformation, rendered extent or
     memory.
   - No parent closure, main merge, tag or publication.

## Later slices

| Slice | Content | Gate |
| --- | --- | --- |
| 2 | `RTC_CENTER` semantics (D6) | D6 evidence |
| 3 | Implicit via A2's effective-volume owner, then retire `hierarchyBounds` (D3, D4) | A2 accepted; O-implicit |
| 4 | Rotated boxes (D5); regions | D5 evidence; #120 geodesy owner |
| — | Partial-coverage streaming (I3 alternative) | Its own contract delta |
