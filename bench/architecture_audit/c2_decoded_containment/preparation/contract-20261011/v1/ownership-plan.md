# C2 implementation ownership plan

## Roles

There are five roles, held by distinct people:

| Role | Who | Owns |
| --- | --- | --- |
| **O** | Oracle author | Independent reference; never sees an implementation diff before its own oracle is frozen |
| **M** | Math author | Exact predicates |
| **F** | Format/operation author | Payload stream, traversal, report |
| **R** | Coordinator | Shared Cargo builds, producer corpus generation, CI, merge |
| **N** | Fresh nonauthor reviewer | Final-source and evidence review |

The roles are separated as follows:

- O, M and F are three different people.
- N is none of O, M or F.
- R may also act as F only if a different person acts as N.

Every role follows the AGENTS.md CPU policy:

- two workers;
- `nice -n 10`;
- only R runs Cargo.

## Slice order (smallest first)

### Slice 0: independent oracle and witnesses (O only; no `src/` edits)

O owns these files, all new, under `bench/architecture_audit/c2_decoded_containment/`:

- `README.md`: provenance, pins and replay command.
- `oracle/`: pure Python 3 with `Fraction` and no third-party imports. It provides:
  - a GLB/glTF/b3dm reader;
  - an accessor decoder covering the f32, integer and normalized types, the signed clamp
    and viewless accessors;
  - draw-index expansion that skips restart markers;
  - a scene and node walker;
  - the U step;
  - a tileset walker with external tilesets and implicit subtree/metadata-row decoding, as
    the A2 plan requires;
  - exact box and sphere predicates. Use Gaussian elimination here, not Cramer, so this
    path stays distinct from M's.
- Meshopt: decode only through pinned upstream golden vectors. Otherwise mark the case
  out of oracle scope.
- `witnesses/`: materialize the prior 13 literal witnesses (with W13 updated per D2) and
  the 17 context recipes. Add the new controls listed in `contract-draft.md` §Independent
  acceptance step 2.
- `corpus/manifest.json`: one row per producer output, with input identity, mode and the
  oracle verdict.

O must not read or import `src/`, and must not copy formulas from it. Only the primary
specs and the contract text are inputs.

R generates the producer outputs:

- vector: plain, `--quantize` and `--meshopt`, each in implicit and `--explicit` mode;
- point cloud: implicit and `--explicit`;
- F1c2 mesh;
- W1: local and Wgs84, including near-window quaternions and matrices;
- legacy `mesh-to-3tz`;
- `convert-to-implicit`.

R hands O artifact hashes only.

The exit criterion is per-producer Contained/Outside/OutOfDomain verdicts that confirm or
refute every prediction in `producer-inventory.md`. Slice 0 is also the decoded-content
oracle that A2 (#126) and F1 fidelity need, so it is useful even before slice 1 lands.

### Slice 1: production check for explicit `.3tz` (M and F)

The scope is the contract's "Supported domain (first slice)" table:

- explicit `.3tz` only;
- no RTC;
- no regions;
- implicit not inspected.

D1–D3 must be resolved before any code is written.

**Interface freeze (M+F, with R's sign-off) comes before either side codes.** The interface
is one private Rust signature file:

```text
src/validate/containment.rs (M), crate-private:
  enum Normalization { None, I8, U8, I16, U16 }            // raw integer + tag, never a quotient
  enum Local { Matrix([f64;16]), Trs { t:[f64;3], r:[f64;4], s:[f64;3] } }
  enum Volume { Box([f64;12]), Sphere([f64;4]) }          // region never constructed
  enum VolumeAdmission { Ok(Volume), DegenerateBox, Region }
  enum Verdict { Contained, Outside }
  struct NumericRange;                                    // maps to notInspected reason
  fn admit_volume(json-free [f64] facts) -> VolumeAdmission
  fn admit_rotation(q:[f64;4]) -> Result<(), RotationError>   // finite, nonzero, D2 window
  fn test(volume:&Volume, tile_steps:&[[f64;16]], nodes:&[Local],
          raw:[f64;3], n:Normalization) -> Result<Verdict, NumericRange>
```

The signature is fixed-size: no `serde_json`, no `content_integrity` import, no I/O and no
allocation proportional to content. Changing it needs M, F and R.

**M owns the math. These files and their tests are M's only:**

- `src/validate/containment.rs` (new): admission, the step-wise exact point chain, and the
  box (diagonal or signed-permutation direct, otherwise Cramer) and sphere predicates.
- `src/validate/containment/expansion.rs` (new):
  - TwoSum;
  - Dekker split and TwoProduct, with no dependence on FMA;
  - expansion sum, scale and compress;
  - the sign test;
  - the D7 magnitude-domain guard.
- Unit tests live in those files:
  - literal exact vectors derived by M;
  - O's witness data, as JSON data only, never code;
  - one-ulp boundary pairs;
  - signed-clamp and denominator cases;
  - deep-chain and magnitude-edge controls.

M must not read or reuse producer bound code: `mesh_archive/source/original_bounds.rs`,
`mesh_archive/placement.rs`, `partition.rs`, `terrain.rs`, `tileset_node.rs`, `bbox.rs` and
`raster/grid.rs`. This keeps the checker independent of the writers it checks.

**F owns format records, the stream and operation integration. These files are F's:**

- `src/content_integrity/payload/records.rs`:
  - adds `scene`, `scenes`, `nodes`, `skins` and `animations`;
  - adds node fields; mesh `weights`; primitive `targets`;
  - adds a positional-extension allowlist record.
- `src/content_integrity/payload/scene.rs` (new):
  - graph admission: references, strict tree, cycles, and matrix together with TRS;
  - scene selection and exclusion preflight;
  - the event walk: `NodeEnter`, `DrawPosition`, `NodeExit`.
- `src/content_integrity/payload.rs`:
  - `inspect` gains a consumer callback with a distinct error type;
  - the stream runs while views and accessors are live;
  - no positions `Vec` and no second decode.
- `src/content_integrity.rs`: crate-private re-exports only.
- `src/validate.rs`:
  - an ancestor arena of `(Volume, transform, parent)` replaces `parent: Option<Volume>`;
  - per-reference re-streaming, with the URI cache kept for integrity facts only;
  - counters;
  - the conditional `decodedContentContainment` label;
  - `notInspected` reasons;
  - the Outside facts.

  `transform`, `inside`, `contains` and `scale_bound` stay untouched for `hierarchyBounds`
  (D4).
- `src/validate/types.rs`: four new `ValidationLimits` fields (see the contract).
- Docs and tests:
  - `docs/VALIDATION.md`, `docs/CLI.md`;
  - Python report tests in `bindings/python/tests/`;
  - Rust integration tests that consume O's materialized fixtures by hash.

**Out of bounds for both M and F in slice 1:**

- producers (`src/vector/**`, `src/point_cloud/**`, `src/mesh_archive/**`, `src/terrain*`,
  `src/implicit*`);
- `src/archive3tz/**`;
- `content_integrity/json.rs` and `content_integrity/numbers.rs`, which are retained as-is.

Producer fixes for confirmed 1-ulp escapes are separate follow-ups owned by each producer's
migration. A fix is made only when O's corpus shows an Outside.

**R owns:**

- builds and focused test runs;
- corpus regeneration against the final source;
- exact-head CI, installed-wheel and official Blender checks;
- source and artifact pins;
- the evidence directory `bench/architecture_audit/c2_decoded_containment/acceptance/`;
- the final architecture doc `docs/architecture/c2-decoded-containment-contract.md`, with
  its README entry.

## What the nonauthor reviewer (N) must check

1. **Independence.**
   - O's oracle imports nothing from the repository and shares no formulas copied from
     `src/`.
   - M's code does not call or mirror producer bound code.
   - Expected witness values were never edited to match production.
2. **Frame order** in the final source:
   - node chain in parent·local order;
   - U applied exactly once;
   - each ancestor test applies tile transforms child-first and excludes the ancestor's own
     transform;
   - external tileset roots are included;
   - no inversion anywhere.
3. **Exactness.**
   - No tolerance, epsilon or slack constant appears in C2 code. Grep
     `src/validate/containment*` and the new `validate.rs` paths for `1e-`, `EPSILON` and
     `next_up`.
   - No f64-rounded transformed point or normalized quotient is used as truth.
   - The quaternion is used in homogeneous form scaled by `‖q‖²`.
   - The signed clamp is applied before the rational.
   - The Dekker split constant is correct, and the compression invariant holds.
   - The magnitude guard precedes every product.
4. **Selection and draw semantics.**
   - Scene selection follows the contract: no scene-0 default.
   - Exclusions are detected before position work.
   - The positional-extension allowlist is finite, and `extras` stay opaque.
   - Indexed draws visit only referenced indices.
   - Restart markers are skipped; repeats and shared-mesh instances are charged.
5. **Report honesty.**
   - `decodedContentContainment` is listed only when every content reference was covered.
   - Otherwise `decodedContentBounds` stays in `notInspected`, with a finite reason.
   - A proven Outside is InvalidInput carrying finite facts and no source strings or floats.
   - Counters are checked before the scan or predicate.
   - The URI cache never short-circuits a different reference.
   - A callback, resolver or format failure remains causal, with no partial success.
6. **Sensitive controls** (contract, acceptance step 3). Each mutant flips **both**
   production and oracle verdicts. Run against final binaries bound to source hashes.
7. **Producer corpus.** Production agrees with the oracle on every row. Every Outside or
   OutOfDomain row has a recorded disposition: producer fix, D-decision, or documented
   capability change. Inventory predictions that turned out wrong are corrected in the doc.
8. **No unintended change.**
   - C1 counters, labels and limits are otherwise unchanged.
   - `hierarchyBounds` behaviour is unchanged.
   - The Rust, CLI and Python reports match.
   - Applicable CI, wheel and Blender checks pass on the exact head.
9. **Scope claims.** There is:
   - no claim about implicit, RTC, region, deformation, rendered extent or whole-process
     memory;
   - no parent issue closure;
   - no main merge, tag or publication.

## Later slices (each needs its own contract delta and review)

| Slice | Content | Dependency |
| --- | --- | --- |
| 2 | b3dm `RTC_CENTER` JSON and binary (D6) | D6 evidence |
| 3 | Implicit coverage through A2's effective-volume owner (D3); then retire the `hierarchyBounds` rejection and its slack helpers (D4) | #126 A2 accepted |
| 4 | Region volumes | #120 geodesy owner |
| — | Interval filter in front of exact predicates | Measured need plus an equivalence proof; never as a fallback |
