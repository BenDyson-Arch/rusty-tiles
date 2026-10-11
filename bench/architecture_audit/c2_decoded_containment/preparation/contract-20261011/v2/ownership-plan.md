# C2 implementation ownership plan (v2)

## Status

v2 responds to the precode review. v1 is in `v1/ownership-plan.md`.

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
- **F:** may start once R3, R6, R7 and R8 are accepted.
- **M:** may start only after a short delta review of R1 and R2 (contract "Exact evaluation" and
  "Work bound"; decision D7).

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
  - the canonical formulas and the static span bound;
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

### Interface freeze (M, F and R sign before coding)

The interface is crate-private, in `src/validate/containment.rs`. F adds `mod containment;` to
`validate.rs`; this one-line edit is agreed in advance.

```text
// Inputs (fixed-size, no serde/Value/IO)
enum Normalization { None, I8, U8, I16, U16 }          // k = 1, 127, 255, 32767, 65535
enum RawVertex { F32([f32;3]), Int([i32;3]) }           // raw words; never a rounded quotient
enum Local { Matrix([f64;16]), Trs { t:[f64;3], q:[f64;4], s:[f64;3] } }
enum VolumeFacts { Box([f64;12]), Sphere([f64;4]), Region }
enum Exclusion { RegionVolume, RotatedBox, QuaternionWindow, NumericSpan, WorkLimit }
enum Role { Own, Content, ExternalContent(u32), Ancestor(u32) }

fn admit_volume(f: VolumeFacts) -> Result<Volume, Exclusion>
fn admit_rotation(q: [f64;4]) -> Result<(), RotationOutcome>          // InvalidInput vs QuaternionWindow
fn instance_frame(path: &[Local]) -> Result<InstanceFrame, Exclusion> // composite G, root-first
fn level_chain(levels: &[Level]) -> Chain        // Level { volumes: [(Role, Volume)], to_parent: Option<[f64;16]> }, own level first
fn static_units(frame: &InstanceFrame, n: Normalization, chain: &Chain) -> Result<u64, Exclusion>  // NumericSpan beyond containmentSpanBits
fn walk(frame: &InstanceFrame, n: Normalization, raw: RawVertex, chain: &Chain) -> Verdict
     // applies G, then U (explicit named step), then for each level: test volumes in order, apply to_parent
enum Verdict { Contained, Outside { role: Role, level: u32 } }
```

Notes on the interface:

- The `walk` is single-pass, so per-vertex cost is O(levels) (R2b).
- U is a named, explicit step.
- `Chain` and `InstanceFrame` hold fixed-capacity dyadics. Their size is bounded by
  `containmentNodeDepth` and the hierarchy depth.
- Changing the interface requires M, F and R.

### M owns (only these files)

- `src/validate/containment.rs`: admission, instance frame, chain, static bound and units, walk,
  predicates.
- `src/validate/containment/dyadic.rs`: a fixed-capacity 4096-bit sign-magnitude dyadic type with
  add, sub, mul, compare and the exponent-pair bound. An optional i128 fast path may decide the
  same predicate.
- Unit tests in those files:
  - literal exact vectors derived by M;
  - O's witness *data* (JSON only, never code);
  - one-ulp boundary pairs;
  - signed clamp;
  - `k` and `‖q‖²` denominators;
  - span at the cap and one over;
  - deep translation chains;
  - deep rotated chains (excluded);
  - zero half-lengths;
  - singular tile transforms.

M must not read producer bound code:

- `mesh_archive/**` (`original_bounds.rs`, `placement.rs`, `partition.rs`);
- `terrain.rs`, `tileset_node.rs`, `bbox.rs`;
- `raster/grid.rs`.

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
    distinct error type.
  - The label at `:1102-1103` is split (R7.1).
  - There is no positions `Vec` and no second decoder.
- **`src/content_integrity.rs`.** Crate-private re-exports only.
- **`src/validate.rs`.**
  - The level chain replaces `parent: Option<Volume>`.
  - Exclusion collection over tileset and glTF allowlists, metadata, regions and up-axis.
  - The precheck (`containmentWorkUnits`, `containmentBytesRead`, node depth).
  - Serial per-reference streaming, with the URI cache kept for integrity and static facts only.
  - The distinct-vertex bitmap.
  - Outcome precedence.
  - Label and reason assembly.
  - Removing `"scene transforms"` on outcome 5.
  - The old `transform`/`inside`/`contains`/`scale_bound` stay untouched (D4).
- **`src/validate/types.rs`.** The four `containment*` limit fields, placed per O-b.
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
   - Primitives with `EXT_mesh_polygon` are tested on all vertices.
   - Coverage is all-or-nothing per archive (I3).
5. **Work.**
   - Distinct-vertex dedupe works, and undrawn vertices are never tested.
   - The precheck happens before streaming, and its units formula matches the contract.
   - No new counter yields ResourceLimit.
   - `totalBytesRead` and `total_payload_elements` are not charged by phase 2.
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
