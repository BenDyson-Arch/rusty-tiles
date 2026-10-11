# I1 ownership plan (v2)

This plan follows the AGENTS.md foundation-first process and the GPT-6.1 Sol fan-out. v1 is
preserved in `v1/`. Line references are to `f2ed077`.

**What changed in v2.**
- The validator migration is now gated on the producer layout change (OD-1, user decision).
- The `convert-to-implicit` disposition is a gate.
- File overlap with the producer layout lane is resolved.
- B's scope is smaller: the owner drives the walk, so there is no task variant and no root
  registry.

## Roles (held by distinct people)

| Role | Who | Constraints |
| --- | --- | --- |
| **A**: owner author | Writes `src/content_integrity/implicit*` | Must not read O's code until O is frozen. Must not edit validator files |
| **B**: validator integration author | Writes the consumer step | May read A's interface, but not O's code before O is frozen. Must not edit `content_integrity/implicit*`, producers or writers |
| **O**: independent oracle author | Writes O-implicit | **Never reads `src/`.** Sees no production diff before O is frozen |
| **R**: coordinator | Owns all Cargo builds (serial, `nice -n 10`, two CPUs, `--jobs 2`), corpus regeneration including the producer-change witnesses, mutant patches for both sides, the perf lane, CI, wheel and Blender checks, docs, and the merge order between the I1 PR and the producer-layout PR | May act as B only if a different person is N |
| **N**: fresh nonauthor reviewer | Precode review; final-source review | Not A, B, O or the contract author |
| (P: producer layout lane) | A separate design and implementation lane, not part of I1 | It owns the writers. I1 states only the profile its output must satisfy (below) |

**Separation rules.**
- A, B and O are three different people.
- N is none of A, B, O or the contract author.
- Only R runs Cargo, and builds are never raced.

**O's permitted inputs.**
- The pinned primary text, re-pinned by R under `bench/architecture_audit/implicit_read/primary/`.
- This contract.
- The #151 fixture data.
- Witness artifacts and their hashes, from R. These include W11, the producer-change outputs, once
  they exist.
- `tests/p1_point_oracle.py:279-287`, for Morton layout only.

## File ownership (no overlap)

| Owner | Files |
| --- | --- |
| A | `src/content_integrity/implicit.rs`, plus optional `src/content_integrity/implicit/{subtree,metadata,address,walk}.rs`; unit tests inside them; one line in `src/content_integrity.rs` |
| B | `src/validate.rs`: implicit-root branch and walk loop in `node()`, counters, report label, removals, `#[cfg(test)] implicit_tile_facts`. `tests/i1_implicit_read.rs` (new). `bindings/python/tests/test_api.py` (parity cases only) |
| B, delete only, **sequenced after P merges** | `src/implicit/tileset.rs:619-660` and `src/implicit.rs:20`. P also edits `implicit/tileset.rs` (`:284-602`), so R applies B's deletion hunk after P lands, or folds it into the combined branch. They are never edited concurrently |
| P (outside I1) | `src/implicit/tileset.rs:229-602` (writers), `src/implicit.rs` (writer), producers as needed, `tests/native_implicit.rs`, and their own docs |
| Convert disposition owner (R, or the A2 lane per the coordinator) | `src/convert_implicit.rs`, `tests/convert_implicit.rs`, `bindings/python/tests/test_api.py:985-998` if affected. A **second edit of `test_api.py`** besides B's must be sequenced by R |
| O | `bench/architecture_audit/implicit_read/{oracle,witnesses,controls,corpus}/` |
| R | `bench/architecture_audit/implicit_read/{README.md,primary,acceptance,production-mutants,oracle-mutants,perf}/`, `docs/architecture/i1-implicit-read-contract.md`, the architecture README index, `docs/VALIDATION.md`, `docs/CLI.md`, inventory `:40`, `:100`, `CHANGELOG.md` |
| Nobody in I1 | `src/validate/containment*`, `src/validate/types.rs`, `src/content_integrity/{json,numbers,payload*,meshopt}.rs`, `src/lib.rs`, `bindings/python/src/lib.rs` |

## Minimal vertical slice

1. **A.** The owner (contract §1–§7). Unit tests:
   - header and buffers;
   - LSB versus MSB;
   - availability rules;
   - Morton round trip;
   - rank;
   - semantic matching;
   - FLOAT32;
   - the reservation before allocation;
   - live-path release;
   - template variables;
   - JSON-template detection, with the level-0 rule.

   A can start now. The owner does not depend on the producer change.
2. **O.** Frozen oracle, witnesses and controls. O can start now. W11 is added when P produces
   output.
3. **B.** The validator migration. It may be written now on a branch, but **merges only after P,
   or with P in one combined PR**.
4. **R.** Final-source corpus (W1–W11, plus the negative W3u, W5u, W8u and W9u), parity, controls,
   mutants, resource controls, perf before and after (before = P head without I1), adapters, CI,
   wheel and Blender.
5. **N.** Final review.

**Outside the slice.**
- C2 slice 3.
- The A2 writer.
- The producer layout itself (P).
- The `convert-to-implicit` redesign.
- Removing `implicit::expand_tileset`.

## Start gates

| Gate | Gates | Status |
| --- | --- | --- |
| PR #157 (C2) merged | everything | **Met** (`f2ed077`) |
| OD-1 decided | everything | **Met** (user: strict reading; producers change) |
| The coordinator records OD-1b, OD-2 to OD-8 and the convert disposition | precode sign-off | Open |
| R re-pins the primary texts in-repo, matching `probes/receipt.json` | O freeze | Open |
| A fresh nonauthor precode review returns GO or GO-WITH-CHANGES, folded in | A, B and O coding | Open |
| O frozen before A's or B's diff is shown to O | acceptance runs | Open |
| A's interface reviewed by B and R; changes need A, B and R | B coding | Open |
| **Producer layout change (P) merged to develop, or combined with I1 in one PR.** No implicit root emitted by point-cloud, vector or legacy `mesh-to-3tz` may have a `.json` content template | **I1 validator migration merge** | Open |
| P's output satisfies the I1 profile, checked by O on W11 (Accepted, with production equal to O). P output keeps TILE_BOUNDING_BOX rows on every tile (OD-3) and distinct subtree prefixes per root (OD-7) | I1 merge | Open |
| **`convert-to-implicit` disposition recorded and implemented**: retire, restrict to single node, or redesign. Today every multi-node output is Unsupported under I1 and its `:205` self-validation would fail. `tests/convert_implicit.rs:238`, `:349`, `:536-571` and `:573` need re-verification | I1 merge | Open (coordinator or user) |
| Perf lane available, merged or pinned by hash; the baseline taken on the P head | I1 merge | Open |

**Merge order.**

1. P, or P and the convert disposition.
2. The I1 validator migration. Alternatively, one combined PR that R assembles. Under either order,
   develop never holds a validator that reports default producer output as Unsupported.
3. The owner (A) may merge earlier as dead code only if the coordinator wants that. The
   recommendation is to merge it together with B, so that no unused module lands.

## What N must check

1. **Independence.**
   - O imports nothing from the repository and never read `src/`.
   - Expectations were not edited to match production.
   - Mutants are hashed.
   - The P1 oracle was used for Morton layout only.
2. **Semantics.**
   - Matching by `semantic`; grep the owner for `rustyTile`, `boundingBox`, `geometricError` and
     `extras`.
   - LSB-first bits.
   - Strict rank.
   - Errors halve per level.
   - Inherited refinement.
   - Placeholder and level-0 volumes both present.
   - JSON templates detected statically and case-insensitively. Level-0 availability is
     InvalidInput per OD-1b; everything else is Unsupported.
   - No JSON content is ever followed from an implicit tile.
3. **Exactness.**
   - No tolerance constant in the owner.
   - `Derived` is symbolic.
   - Rows are bit-exact.
   - The OD-3 rounded evaluation is confined to `hierarchyBounds`.
4. **Bounds.**
   - The reservation precedes the rank table and every visit.
   - Live-path bytes are checked before each read, and released deterministically.
   - No allocation is sized by an unverified declared count.
   - R2b distinguishes live-path from cumulative accounting.
   - The R3 RSS measurement is present.
5. **Report and order.**
   - The label is exactly as specified.
   - Counts are unchanged for admitted shapes.
   - The exit-status table is complete.
   - The merge order respects the P gate.
   - The convert disposition is implemented.
6. **Scope.**
   - `containment*`, `types.rs` and the writers are untouched by I1 authors.
   - No claim about containment, conformance, metadata semantics or memory.
   - No parent closure.
