# I1 ownership plan (v3)

v1 and v2 are preserved in `v1/` and `v2/`. v3 resolves RC-10, RC-16, RC-17 and RC-19 from the
review. The overall model is a single combined integration PR (I1 plus P), with the convert
retirement (#159) landing first.

## Roles (held by distinct people)

| Role | Who | Constraints |
| --- | --- | --- |
| **A**: owner author | Writes `src/content_integrity/implicit*` | Does not read O before O is frozen. Does not edit the validator |
| **B**: validator integration author | Writes the consumer step in `src/validate.rs` | Does not read O before O is frozen. Does not edit the owner, the writers, producers or `implicit/tileset.rs` |
| **O**: oracle author | Writes O-implicit | **Never reads `src/`** |
| **R**: coordinator | Owns all Cargo builds (serial, nice 10, two CPUs, `--jobs 2`); assembles the combined integration PR from the I1 and P branches; corpus and witnesses on the combined head; mutants for both sides; perf at `bb7d054`; CI, wheel and Blender; docs; merge | May act as B only if a different person is N |
| **N**: reviewer | Precode review (done); **delta check of v3** (RC-1, RC-2, RC-3, RC-5, RC-6); final-source review of the combined PR's I1 part | Not A, B, O or the contract author |
| **P**: producer layout lane (design in `/tmp/rusty-tiles-pimpl-design-20261011`, in progress) | Separate authors | Owns the writers, the producer consumers of `expand_tileset` and its own tests. P's own nonauthor review covers P |

**O's permitted inputs.**
- The pinned primary text.
- This contract.
- The #151 data.
- R's artifacts and hashes, including W11.
- `tests/p1_point_oracle.py:279-287`, for Morton layout only.

## File ownership (no overlap)

| Owner | Files |
| --- | --- |
| A | `src/content_integrity/implicit.rs` (plus optional `implicit/{subtree,availability,metadata,walk}.rs`); unit tests inside; one `mod` line in `src/content_integrity.rs` |
| B | `src/validate.rs`: `node()` branch and walk loop; removals at `:56-62`, `:898`, `:1203-1263`, `:1600-1604`, test `:1758-1761`; counters; labels; the `#[cfg(test)] implicit_tile_facts` seam **and its parity tests in `mod tests`** (RC-16). `src/implicit.rs:20`, removing the `pub(crate) use` of `expand_tileset_bounded` and `ExpansionBudget` (RC-10). `tests/i1_implicit_read.rs` (CLI and public-API outcomes only). `bindings/python/tests/test_api.py` (parity cases; after #159 removed the convert test) |
| P | `src/implicit/tileset.rs` (the whole file; I1 does not touch it, RC-10), the writer parts of `src/implicit.rs` other than line 20, producers, `vector/pipeline/store.rs:1006` (the `expand_tileset` consumer), `tests/native_implicit.rs`, and P's docs |
| O | `bench/architecture_audit/implicit_read/{oracle,witnesses,controls,corpus}/` |
| R | `bench/architecture_audit/implicit_read/{README.md,primary,acceptance,production-mutants,oracle-mutants,perf}/`, `docs/architecture/i1-implicit-read-contract.md`, the architecture README index, `docs/VALIDATION.md` (`:143` rewritten on top of #159's sentence), `docs/CLI.md`, inventory `:40`, `:100`, `CHANGELOG.md` |
| Nobody in I1 | `src/validate/containment*`, `src/validate/types.rs`, `src/content_integrity/{json,numbers,payload*,meshopt}.rs`, `src/lib.rs`, `bindings/python/src/lib.rs`, `src/convert_implicit.rs` (deleted by #159) |

**Overlap check.**
- The only file both B and P could touch is `src/implicit.rs`. B removes line 20; P edits writer
  code. R applies them in the combined branch as separate commits.
- `src/validate.rs` is edited by #159 (a comment at `:1635`). B rebases after #159.

## Minimal vertical slice

1. **A.** The owner, contract §1–§7. Coding of §1, §2/§2a and §7 starts **after the N delta check**.
2. **O.** The oracle, witnesses and controls. O freezes after the N delta check. W11 is added when P
   produces output.
3. **B.** The migration on a branch rebased on #159.
4. **R.**
   - Assembles the combined PR: P commits, A and B commits, docs.
   - Regenerates W1–W11 and the negative witnesses (W3u, W5u, W8m, W9u) on the combined head.
   - Runs parity, controls C-*/D-*/T-*, mutants, R1–R7b, and perf (before = combined minus I1's
     validator commits).
   - Runs adapters, CI, wheel and Blender.
5. **N.** Final review of the I1 part. P's reviewer covers P.

**Outside the slice.** C2 slice 3 (obligations RC-12b and RC-12c recorded in the contract), the A2
writer, P's design, and removing `implicit::expand_tileset`.

## Start gates and merge order

| Gate | Gates | Status |
| --- | --- | --- |
| #157 (C2) merged | everything | **Met** (`f2ed077`) |
| OD-1, OD-1b, sequencing, convert retirement, content semantics and perf pin decided | everything | **Met** (coordinator, 2026-10-11) |
| Coordinator records OD-2 to OD-10 | N delta check | Open |
| **N delta check of v3** confirms RC-1, RC-2, RC-3, RC-5 and RC-6 | O freeze; A coding §1, §2/§2a, §7 | Open |
| R re-pins the primary texts in-repo, matching `probes/receipt.json` | O freeze | Open |
| O frozen before A's or B's diff is shown to O | acceptance runs | Open |
| A's interface reviewed by B and R; changes need A, B and R | B coding against it | Open |
| **#159 (T1, including the `retire/convert-to-implicit` commits) merged to develop** | rebase of B and of the combined PR | Open |
| **P's branch ready.** No point-cloud, vector or legacy `mesh-to-3tz` implicit root has a `.json` content template. Rows on every tile (OD-3), distinct subtree prefixes (OD-7) | assembling the combined PR | Open |
| **W11: P's outputs Accepted by O, with production equal to O**, on the combined head | merging the combined PR | Open |
| Perf at `bb7d054`, with an amplified base of at least 0.4 s (or the recorded closest case) | merging the combined PR | Open |

**Merge order.**
1. #159, which includes the convert retirement.
2. **One combined integration PR containing P and I1** (A, B and docs).

There is no separate P-first or I1-first merge.

**Why the combined PR is mandatory (RC-17).**
- If P places implicit roots on explicit tiles, which is one of the admitted layouts:
  - today's validator refuses that (`validate.rs:1280-1284`);
  - today's `expand_tileset`, used by `vector/pipeline/store.rs:1006` for `maximumTileBytes`, handles
    only a manifest-root implicit root (`expand_document`).
  So P cannot merge before I1.
- I1 cannot merge before P either: default point and vector outputs would turn Unsupported.
- P owns the `store.rs` consumer.
- A (the owner, dead code until B) is not merged separately.

## What N must check (final review)

1. **Independence.**
   - O imports nothing from the repository and never read `src/`.
   - Expectations were not edited to match production.
   - Mutants are hashed.
   - The P1 oracle was used for Morton layout only.
2. **Semantics.**
   - Matching by semantic; grep the owner for `rustyTile`, `boundingBox`, `geometricError` and
     `extras`.
   - LSB-first; strict rank; error halving; inherited refine; R and I_0 volumes.
   - JSON detection per §3. Uniform Unsupported.
   - The Unsupported semantic set includes CONTENT_URI and CONTENT_GROUP_ID.
3. **Exactness.**
   - No tolerance constant in the owner.
   - `Derived` is symbolic.
   - The rounded OD-3 evaluation is confined to `hierarchyBounds` and never enters `TileFact`.
4. **Work.**
   - §2a step order.
   - Distinct-source memoization; `Scan` charged before any pass.
   - Closed forms for constants; lazy child subtrees; no `C` enumeration.
   - Reservation before the rank table and index.
   - Live-path release.
   - The parsed subtree JSON is dropped before descending.
   - R7, R7a and R7b timings; R3 RSS.
5. **Report and exits.**
   - Labels exactly as specified (OD-4).
   - The exit table X-1 to X-14 matches the final diff.
   - The tolerance rows have controls (T-1 to T-3).
6. **Scope.**
   - I1 never edits `implicit/tileset.rs`, `containment*` or `types.rs`.
   - The C2 slice-3 obligations are recorded, not implemented.
   - No parent closure.
