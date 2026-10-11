# P-IMPL ownership plan (v2)

v1 is in `v1/ownership-plan.md`. v2 resolves R-12 and R-13 and adds the I1 asks that R-1 and R-3
create.

P is the producer layout lane of the combined I1+P integration PR (I1 v3.1 ownership plan, role P;
I1 `contract-draft.md` `9674e60b…`). I1 owns the validator and the reader. P owns everything that
writes or self-reads the producers' implicit layout.

Within P, three authors work in parallel, each on files nobody else in P edits. The coordinator R
(I1's R) owns every Cargo build, assembles the combined branch, and runs the perf lane and the
browser checks. Line references are to `f2ed077`.

## Roles

| Role | Owns | Must not |
| --- | --- | --- |
| **PW**, writer author | The forest writer (layout, external documents, subtrees, payload rebasing) and its unit tests; the shared expander rework (PD-8) | Edit producer modules; edit `src/validate.rs` or `src/content_integrity/*` |
| **PP**, point-cloud, mesh and wrapper author | The point-cloud, legacy-mesh and small-model wrapper integrations; `tests/fixtures/implicit_levels.cjs` (the **only** owner, R-12b) | Edit `src/implicit/*`; edit vector files |
| **PV**, vector author | The vector integration: report statistics from `ForestOutput`, reuse identity, retained-source policy, docs text | Edit `src/implicit/*`; edit point-cloud or mesh files |
| **PO**, P oracle and acceptance author | Independent forest checks: stdlib Python that imports nothing from `src/`, derived from `evidence/v2/forest_model.py` and `evidence/layout_probe.py`. Also the Cesium probes, ported from `evidence/v2/cesium/` into `bench/` | Read the writer diff before the checks are frozen; edit `tests/fixtures/*` |
| **PN**, P nonauthor reviewer | Delta review of v2; final-source review of P's part of the combined PR | Be PW, PP, PV or PO |

PP and PV may be the same person, because their files are disjoint. Neither may be PW, so that a
consumer reviews the writer interface.

## File ownership (no overlap)

| Owner | Files |
| --- | --- |
| PW | `src/implicit/tileset.rs` (the whole file); the writer parts of `src/implicit.rs` (`Subtree`, `TileMetadata`, the re-exports on `:21`; **not `:20`**, which is I1-B's); `tests/implicit_metadata.rs`; `tests/implicit_subtree.rs`; a new `tests/implicit_forest.rs` |
| PP | `src/point_cloud.rs` (`:600-640`: the writer call, the members, the encoder string); `src/point_cloud/tiles.rs` (nothing else unless PD-5 changes); `src/tile.rs` (`:510-540`: the writer call, staging from `ForestOutput.members`, the encoder string); `src/tileset.rs` (`:153-177`, M2: **the writer call** becomes `write_forest(..., SourcePolicy::Remove)`, plus the encoder string, R-12a); `tests/native_point_cloud.rs`; `tests/native_implicit.rs`; `tests/mesh_tile.rs`; `tests/mesh_crs.rs`; `tests/fidelity.rs` (implicit call sites only: `:142`, `:427`, `:515`); `tests/p1_point_oracle.py` (its implicit walker, `:278-339`, gains the forest and external documents and loses the JSON-template chain); `tests/fixtures/implicit_levels.cjs` |
| PV | `src/vector/pipeline/store.rs` (`:970-1046`: the writer call; replace the `expand_tileset` consumer at `:1003-1013`, and every report sum at `:1010-1036`, with `ForestOutput.tiles`); `src/vector/pipeline/reuse.rs` (`:268-270`: tiling identity `implicit-quadtree-v2` becomes `implicit-forest-v3`; `publish` content inventory `:433-466`, which keeps every `t/*` that `explicitRoot` references); `tests/fixtures/vector_compat.cjs` (`:68`); `tests/fixtures/vector_compat.py` (`:93-94`); `tests/vector_test_support.py` (`:37`); the reuse tests that assert archive layout; `docs/VECTOR.md` text, sent to R |
| PO | `bench/architecture_audit/implicit_layout/` (new): the forest checker, the fixture generator and probes (from `evidence/v2/cesium/`), the deep-view A/B (`run_real.py`, `compare_runs.py`), and frozen expectations |
| R | Cargo builds; the combined branch; the perf lane at `bb7d054` (A-6); `CHANGELOG.md` (`:28` rewritten; it must say that old vector reuse baselines are refused with "run a fresh conversion", `reuse.rs:285`); `docs/FORMATS.md`; `docs/CLI.md`; a new `docs/architecture/p-impl-layout-contract.md` (from design.md); the architecture README index |
| Nobody in P | `src/validate.rs`; `src/content_integrity/*` (I1-A and I1-B); `src/implicit.rs:20` (I1-B); `src/convert_implicit.rs` (deleted by #159); `src/validate/containment*` |

## The interface between PW and the integrators (agreed before coding)

```rust
// src/implicit/tileset.rs (PW)
pub(crate) enum SourcePolicy { Retain, Remove }      // vector: Retain; point, mesh, M2: Remove
pub(crate) enum TileRole { Skeleton, ExplicitLeaf, ImplicitRoot, Implicit }
pub(crate) struct WrittenTile {
    pub role: TileRole,
    pub extras: Value,                // as written; extras.encodedBytes per §4.5 of design.md
    pub contents: Vec<(String, u64)>, // published member name and its byte length, per slot
}
pub(crate) struct ForestOutput {
    pub members: Vec<String>,         // every member written or retained: documents, subtrees,
                                      // implicit-content, skeleton payloads, retained sources
    pub tiles: Vec<WrittenTile>,      // exactly one per tile of H, in preorder (R-6)
    pub documents: usize,
    pub implicit_roots: usize,
}
pub(crate) fn write_forest(
    manifest: &mut Value,             // the producer's explicit hierarchy in; the root document out
    directory: &Path,
    scheme: SubdivisionScheme,
    sources: SourcePolicy,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<ForestOutput, Error>;
```

- `write_tileset` and `write_tileset_recorded` are removed. Every producer calls `write_forest`
  once: point cloud, legacy mesh, the M2 wrapper and vector.
- `SourcePolicy::Remove` deletes only sources that were rewritten into `implicit-content/`.
  Skeleton payloads stay in place and appear in `members`.
- `SourcePolicy::Retain` keeps the rewritten sources too, and lists them in the extras of the
  document that holds their implicit root (§4.1).
- A change to this interface needs PW, PP, PV and R.

**PW-internal (R-12c, R-12d).**
- After I1-B removes the re-export at `implicit.rs:20`, PW either deletes `expand_tileset_bounded`
  and `ExpansionBudget`, or keeps them used inside the reworked `expand_tileset`. No dead code may
  remain under `-D warnings`.
- The reworked `expand_tileset` charges **one** budget across every implicit root and every
  external document of an archive.

## I1 asks (R-3, R-13; for I1-B, I1-O and R; recorded on the I1 side by its coordinator)

1. **Retained-source gate (I1-B, `validate.rs:1204-1217`, moved into `prepare_tileset` by I1).**
   - Remove the `doc["root"].get("implicitTiling").is_some()` condition.
   - For **every** tileset document the validator reads (`tileset.json` and every explicit
     external document), if `extras.rustyTilesSourceContents` is present:
     - it must be an array of strings, otherwise InvalidInput, as today;
     - each name resolves through `uri(document, name)`;
     - each is registered with `resources.reference`, and passes `payload()` (C1), as today.
   - Nothing else changes.
   - Control: a forest whose list is in an external document passes. Removing one listed name makes
     the archive fail with "unreferenced archive entries".
2. **Admission of explicit external documents.** No change is needed.
   - I1 v3.1 already admits implicit roots "on any explicit tile, including inside explicit
     external tilesets" (`contract-draft.md` §Supported domain, `:150`).
   - It keeps the existing explicit traversal for external tilesets (`:433-436`; today's
     `validate.rs:1364-1376` and `:1112-1126`).
   - The JSON-template exclusion applies only in `admit_root`, to implicit roots' templates (`:727`).
     A link tile is an explicit tile whose `content.uri` names an explicit external document, which
     is the ordinary core case.
3. **Witnesses for W11 (I1-O, R).** Add:
   - (a) a P output with a **translated non-root implicit root** (`transform` on an implicit root
     below an explicit tile). Only fixture `F` has one today;
   - (b) an implicit root inside an **explicit external document**;
   - (c) a retained-source list in an external document;
   - (d) a P output at the 32-level bound.
4. **Tile count (`validate.rs:1662`, `budgets.tiles`).** Under I1's counter definition, link tiles
   are explicit tiles, so `tiles` = |H| + link tiles.
   - A vector archive with |H| close to `maxTiles` and at least one link could be refused as
     exceeding the recorded tile budget. The same applies today to chunked layouts.
   - Recommendation for I1: do not count tiles whose only content is an external tileset document
     in the `budgets.tiles` comparison.
   - Alternative: P records the link count in `conversion.json`. See PD-13.

## Merge sequencing

1. #159 merges, including the convert retirement. This is I1's gate as well.
2. PW lands the writer and `tests/implicit_forest.rs` on the P branch. PO's checks are frozen
   before PO sees the writer diff.
3. PP and PV integrate on the P branch in parallel, rebased on PW. Their files are disjoint.
4. R assembles the combined PR (I1-A, I1-B, P) and runs all of the following on the combined head:
   - Cargo and CI;
   - W11, including the asks above;
   - PO's checks;
   - the perf lane;
   - the browser probes;
   - the wheel and Blender checks.
5. PN reviews P's final source. I1's N reviews I1's part.

There is no separate P merge:
- today's validator refuses implicit roots on explicit tiles (`validate.rs:1280-1284`);
- today's validator honours retained sources only under an implicit document root (`:1204`);
- I1 makes today's JSON templates Unsupported.

## Overlap check

| File | Touched by | Resolution |
| --- | --- | --- |
| `src/implicit.rs` | PW (writer, `:21`) and I1-B (`:20`) | R applies them as separate commits (I1 plan) |
| `src/vector/pipeline/reuse.rs:81-82` | Nobody edits it. The `include_str!` of `implicit/tileset.rs` changes the encoder hash automatically, so old vector caches are rejected | Intended; CHANGELOG note by R |
| `tests/fidelity.rs` | PP only (implicit call sites) | Not shared |
| `tests/fixtures/implicit_levels.cjs` | PP only. PO ports *copies* of the evidence probes into `bench/` and never edits `tests/fixtures/` (R-12b) | Not shared |
| `tests/native_implicit.rs` | PP only. PW's tests live in `tests/implicit_forest.rs` | Not shared |
| `CHANGELOG.md`, `docs/*` | R only. Authors send text to R | Not shared |
