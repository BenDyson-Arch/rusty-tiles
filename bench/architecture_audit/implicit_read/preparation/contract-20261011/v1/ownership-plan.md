# I1 ownership plan

This plan follows the AGENTS.md foundation-first process and the GPT-6.1 Sol fan-out. Line
references are to `f2ed077`.

## Roles (held by distinct people)

| Role | Who | Constraints |
| --- | --- | --- |
| **A**: owner author | Writes `src/content_integrity/implicit*` | Must not read O's oracle code until O is frozen. Must not edit validator files |
| **B**: validator integration author | Writes the consumer step | May read A's interface, but not O's code before O is frozen. Must not edit `content_integrity/implicit*` |
| **O**: independent oracle author | Writes O-implicit | **Never reads `src/`.** Sees no production diff before O is frozen. Inputs are listed below |
| **R**: coordinator | Owns all Cargo builds (serial, `nice -n 10`, two CPUs, `--jobs 2`), corpus regeneration, mutant patches for both sides, the perf lane, CI, wheel and Blender checks, documentation, and the merge | May act as B only if a different person is N |
| **N**: fresh nonauthor reviewer | Precode review of this contract; final-source review | Not A, B, O or the author of this contract draft |

**Separation rules.**
- A, B and O are three different people.
- N is none of A, B, O or the contract author.
- Only R runs Cargo. Agents never race builds or copy a binary while it is building.

**O's permitted inputs.**
- The pinned primary text: the five retained and twelve fetched files, re-pinned by R under
  `bench/architecture_audit/implicit_read/primary/`.
- This contract.
- The #151 fixture *data*.
- Witness artifacts and their hashes, from R.
- `tests/p1_point_oracle.py:279-287`, for Morton layout only.

## File ownership (no overlap)

| Owner | Files |
| --- | --- |
| A | `src/content_integrity/implicit.rs`, plus optional `src/content_integrity/implicit/{subtree,metadata,address}.rs`; unit tests inside them; **one line** in `src/content_integrity.rs` (`pub(crate) mod implicit;`) |
| B | `src/validate.rs`: task, `node()` branch, counters, report labels, removals and the `#[cfg(test)] implicit_tile_facts` seam. `tests/i1_implicit_read.rs` (new). `tests/native_implicit.rs` (expectation updates only, if R's re-verification requires them). `bindings/python/tests/test_api.py` (parity cases) |
| B, delete only | `src/implicit/tileset.rs:619-660` (`expand_tileset_bounded`, `ExpansionBudget`, `expansion_limit`) and the `pub(crate) use` at `src/implicit.rs:20`. A two-hunk deletion; nothing else under `src/implicit*` |
| O | `bench/architecture_audit/implicit_read/oracle/`, `…/witnesses/` (recipes, plus the corrected #151 materializer), `…/controls/` (base and mutant pairs, `manifest.json` with hashes and expected verdicts), `…/corpus/manifest.json` (O's outcome per witness) |
| R | `bench/architecture_audit/implicit_read/{README.md, primary/, acceptance/, production-mutants/, oracle-mutants/, perf/}`, `docs/architecture/i1-implicit-read-contract.md` (final), the `docs/architecture/README.md` index line, `docs/VALIDATION.md` (`:143` rewrite and new labels), `docs/CLI.md` (validate notes), `docs/architecture/public-surface-inventory.md:40,100`, `CHANGELOG.md`, the perf-lane case additions or receipt |
| Nobody | `src/validate/containment*`, `src/validate/types.rs` (no new fields, OD-5), `src/content_integrity/{json,numbers,payload*,meshopt}.rs`, `src/implicit.rs` (apart from B's one line), the writers in `implicit/tileset.rs:1-602`, all producers (`point_cloud*`, `vector/**`, `tile.rs`, `tileset.rs`, `convert_implicit.rs`), `src/lib.rs`, `bindings/python/src/lib.rs` |

The bindings need no change because the report struct is unchanged. If B finds an adapter
change necessary, R takes that file over.

## Minimal vertical slice

1. **A.** The owner covering contract §1–§7, with unit tests:
   - header and buffers;
   - LSB versus MSB bit order;
   - parent, content and child rules;
   - Morton round trip;
   - rank;
   - semantic matching by `semantic`;
   - FLOAT32 widening;
   - the reservation before allocation;
   - template variables.
2. **B.** The validator migration, with the report exactly as specified. The existing
   `native_implicit`, `c1_validate` and `c2_containment` suites must pass, with expectation edits
   listed and justified.
3. **O.** Frozen O-implicit, plus witnesses and controls.
4. **R.** Final-source corpus (W1–W10), production-versus-O parity, data controls, production and
   oracle mutants, resource controls R1–R6, the perf lane before and after, Rust/CLI/Python parity,
   CI, installed wheel and Blender.
5. **N.** Final-source and evidence review.

**Explicitly outside the slice.**
- C2 slice 3: its own delta for `Derived` cells and implicit references.
- The A2 writer and the OD-1 standards verdict.
- Removing `implicit::expand_tileset`, which belongs to G6.
- Producer changes.

## Start gates

| Gate | Status |
| --- | --- |
| PR #157 (C2) merged | **Met.** `f2ed077` |
| Coordinator records OD-1 to OD-8 resolutions in this directory, with OD-1 also noted on #125 and #126 | Open |
| R re-pins the primary texts in-repo, with hashes matching `probes/receipt.json` | Open |
| Fresh nonauthor precode review of the contract, inventory and decisions returns GO or GO-WITH-CHANGES, and its changes are folded in | Open |
| O-implicit frozen (self-tests pass; controls hashed) before A's or B's diff is shown to O | Open; gates the acceptance runs, not coding |
| Perf lane available: merged, or the scripts pinned by hash as in the contract | Open |
| A's interface reviewed by B and R before B starts coding against it; changes need A, B and R | Open |

## What N must check

1. **Independence.**
   - O imports nothing from the repository and never read `src/`.
   - Control expectations were not edited to match production.
   - The mutant patches are hashed.
   - The P1 oracle was used only for Morton layout.
2. **Semantics.**
   - Semantics are matched by `semantic`, never by class or property name (grep the owner for
     `rustyTile`, `boundingBox`, `geometricError` and `extras`).
   - Bits are read LSB-first.
   - Rank equals the strict popcount.
   - Errors halve per level.
   - Refinement is inherited.
   - Placeholder and level-0 volumes are both present.
   - Core external-tileset semantics apply: no link merging, no equality requirement.
3. **Exactness.**
   - The owner has no tolerance constant (grep `1e-`, `EPSILON`, `as f32`).
   - `Derived` is symbolic.
   - Rows are bit-exact.
   - Any rounded evaluation is confined to `hierarchyBounds` (OD-3) and never reaches `TileFact`
     or `VolumeFacts`.
4. **Bounds.**
   - The reservation precedes the rank table and every visit.
   - Each cap is checked before the read or allocation it guards.
   - No allocation is sized by an unverified declared count.
   - The R3 RSS measurement is present.
   - ResourceLimit never acts as a verdict.
5. **Report.**
   - The labels are exactly as specified.
   - The count changes match the contract table.
   - The exit-status table is complete against the final diff.
6. **Scope.**
   - `containment*`, producers and `types.rs` are untouched.
   - No claim about containment, conformance, metadata semantics or memory.
   - No parent closure.
