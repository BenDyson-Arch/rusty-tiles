# P-IMPL ownership plan (v3)

v1 and v2 are in `v1/` and `v2/`. v3 does three things:
- it moves vector reuse onto the published payloads (PD-11), in PV's scope;
- it drops v2's retained-source I1 ask;
- it records PD-13 as accepted.

P is the producer layout lane of the combined I1+P integration PR (I1 v3.1 ownership plan, role P;
I1 `contract-draft.md` `9674e60b…`). I1 owns the validator and the reader. P owns everything that
writes or self-reads the producers' implicit layout.

Within P, three authors work in parallel, each on files nobody else in P edits. The coordinator R
(I1's R) owns every Cargo build, assembles the combined branch, and runs the perf lane and the
browser checks. Line references are to `f2ed077`.

## Roles

| Role | Owns | Must not |
| --- | --- | --- |
| **PW**, writer author | The forest writer (layout, external documents, subtrees), the reversible payload rebase `rebase_payload` and its inverse `restore_payload`, their unit tests, and the shared expander rework (PD-8) | Edit producer modules; edit `src/validate.rs` or `src/content_integrity/*` |
| **PP**, point-cloud, mesh and wrapper author | The point-cloud, legacy-mesh and small-model wrapper integrations; `tests/fixtures/implicit_levels.cjs` (the **only** owner, R-12b) | Edit `src/implicit/*`; edit vector files |
| **PV**, vector author | The vector integration: report statistics from `ForestOutput`; **reuse from published payloads** (the `vector-build.json` payload map and the restore path, §4.7); reuse identity; docs text | Edit `src/implicit/*`; edit point-cloud or mesh files |
| **PO**, P oracle and acceptance author | Independent forest checks: stdlib Python that imports nothing from `src/`, derived from `evidence/v2/forest_model.py` and `evidence/layout_probe.py`. Also the Cesium probes, ported from `evidence/v2/cesium/` into `bench/` | Read the writer diff before the checks are frozen; edit `tests/fixtures/*` |
| **PN**, P nonauthor reviewer | Delta review of v2; final-source review of P's part of the combined PR | Be PW, PP, PV or PO |

PP and PV may be the same person, because their files are disjoint. Neither may be PW, so that a
consumer reviews the writer interface.

## File ownership (no overlap)

| Owner | Files |
| --- | --- |
| PW | `src/implicit/tileset.rs` (the whole file); the writer parts of `src/implicit.rs` (`Subtree`, `TileMetadata`, the re-exports on `:21`; **not `:20`**, which is I1-B's); `tests/implicit_metadata.rs`; `tests/implicit_subtree.rs`; a new `tests/implicit_forest.rs` |
| PP | `src/point_cloud.rs` (`:600-640`: the writer call, the members, the encoder string); `src/point_cloud/tiles.rs` (nothing else unless PD-5 changes); `src/tile.rs` (`:510-540`: the writer call, staging from `ForestOutput.members`, the encoder string); `src/tileset.rs` (`:153-177`, M2: **the writer call** becomes `write_forest(...)`, plus the encoder string, R-12a); `tests/native_point_cloud.rs`; `tests/native_implicit.rs` (including removing its vector reuse case (the `--reuseTileset` use at `:157`), which PV re-specifies in its new file); `tests/mesh_tile.rs`; `tests/mesh_crs.rs`; `tests/fidelity.rs` (implicit call sites only: `:142`, `:427`, `:515`); `tests/p1_point_oracle.py` (its implicit walker, `:278-339`, gains the forest and external documents and loses the JSON-template chain); `tests/fixtures/implicit_levels.cjs` |
| PV | `src/vector/pipeline/store.rs` (`:970-1046`: the writer call; replace the `expand_tileset` consumer at `:1003-1013`, and every report sum at `:1010-1036`, with `ForestOutput.tiles`; no retained sources); `src/vector/pipeline/reuse.rs`:<br>- `:268-270`, tiling identity `implicit-forest-v3`;<br>- `:318-420`, `restore` reads the payload map, calls `restore_payload`, verifies sha256 and stages raw bytes privately;<br>- `:433-466`, `publish` records the payload map in `vector-build.json` and publishes only referenced members;<br>`tests/native_vector.rs` (its reuse cases); a new `tests/vector_reuse_payloads.rs` (reuse from forest archives, including quantize and meshopt, and integrity failure when a published payload is altered); `tests/test_vector_reuse.py`; `tests/vector_acceptance_oracle.py` reuse cases; `README.md` `:201` and the `docs/VECTOR.md` reuse text, sent to R; `tests/fixtures/vector_compat.cjs` (`:68`); `tests/fixtures/vector_compat.py` (`:93-94`); `tests/vector_test_support.py` (`:37`); the reuse tests that assert archive layout; `docs/VECTOR.md` text, sent to R |
| PO | `bench/architecture_audit/implicit_layout/` (new): the forest checker, the fixture generator and probes (from `evidence/v2/cesium/`), the deep-view A/B (`run_real.py`, `compare_runs.py`), and frozen expectations |
| R | Cargo builds; the combined branch; the perf lane at `bb7d054` (A-6); `CHANGELOG.md` (`:28` rewritten; it must say that old vector reuse baselines are refused with "run a fresh conversion", `reuse.rs:285`); `docs/FORMATS.md`; `docs/CLI.md`; a new `docs/architecture/p-impl-layout-contract.md` (from design.md); the architecture README index |
| Nobody in P | `src/validate.rs`; `src/content_integrity/*` (I1-A and I1-B); `src/implicit.rs:20` (I1-B); `src/convert_implicit.rs` (deleted by #159); `src/validate/containment*` |

## The interface between PW and the integrators (agreed before coding)

```rust
// src/implicit/tileset.rs (PW)
pub(crate) enum TileRole { Skeleton, ExplicitLeaf, ImplicitRoot, Implicit }
pub(crate) struct RebaseRecord {      // design §4.7; recorded by PV in vector-build.json
    pub raw_sha256: [u8; 32],
    pub delta: [f64; 3],
    pub raw_root_node: String,        // exact JSON text of the raw scene-root node object
    pub raw_json_length: u32,         // raw JSON chunk byte length (alignment padding)
}
pub(crate) struct WrittenContent {
    pub member: String,               // published member name
    pub bytes: u64,
    pub rebase: Option<RebaseRecord>, // Some for payloads published under implicit-content/
}
pub(crate) struct WrittenTile {
    pub role: TileRole,
    pub extras: Value,                // as written; extras.encodedBytes per §4.5 of design.md
    pub contents: Vec<WrittenContent>,
}
pub(crate) struct ForestOutput {
    pub members: Vec<String>,         // every member written: documents, subtrees,
                                      // implicit-content, skeleton payloads
    pub tiles: Vec<WrittenTile>,      // exactly one per tile of H, in preorder (R-6)
    pub documents: usize,
    pub implicit_roots: usize,
}
pub(crate) fn write_forest(
    manifest: &mut Value,             // the producer's explicit hierarchy in; the root document out
    directory: &Path,
    scheme: SubdivisionScheme,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<ForestOutput, Error>;
```

- `write_tileset` and `write_tileset_recorded` are removed. Every producer calls `write_forest`
  once: point cloud, legacy mesh, the M2 wrapper and vector.
- The writer deletes the raw sources of payloads it rewrote into `implicit-content/`, for every
  producer. Skeleton payloads stay in place and appear in `members`.
- `pub(crate) fn restore_payload(published: &[u8], record: &RebaseRecord) -> Result<Vec<u8>, Error>`
  is the exact inverse of the writer's rebase. PV's reuse calls it and then checks
  `record.raw_sha256`.
- A change to this interface needs PW, PP, PV and R.

**PW-internal (R-12c, R-12d).**
- After I1-B removes the re-export at `implicit.rs:20`, PW either deletes `expand_tileset_bounded`
  and `ExpansionBudget`, or keeps them used inside the reworked `expand_tileset`. No dead code may
  remain under `-D warnings`.
- The reworked `expand_tileset` charges **one** budget across every implicit root and every
  external document of an archive.

## I1 interface items (R-13, PD-13; recorded on the I1 side by its coordinator)

1. **Retained-source gate: no change (v3).**
   - New outputs carry no `rustyTilesSourceContents`.
   - I1 already moves `validate.rs:1204-1217` into `prepare_tileset` unchanged, so older archives
     that carry the list under an implicit document root keep validating.
   - v2's request to honour the list on every document is withdrawn.
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
   - (c) a P output at the 32-level bound.
4. **Link tiles in `budgets.tiles` (PD-13, accepted).**
   - I1 excludes tiles whose only content is an external tileset document from the `budgets.tiles`
     comparison (`validate.rs:1662`).
   - The report's `tiles` counter still counts them (I1's definition).
   - P records no link count.

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
- I1 makes today's JSON templates Unsupported.

## Overlap check

| File | Touched by | Resolution |
| --- | --- | --- |
| `src/implicit.rs` | PW (writer, `:21`) and I1-B (`:20`) | R applies them as separate commits (I1 plan) |
| `src/vector/pipeline/reuse.rs:81-82` | Nobody edits it. The `include_str!` of `implicit/tileset.rs` changes the encoder hash automatically, so old vector caches are rejected | Intended; CHANGELOG note by R |
| `tests/fidelity.rs` | PP only (implicit call sites) | Not shared |
| `tests/fixtures/implicit_levels.cjs` | PP only. PO ports *copies* of the evidence probes into `bench/` and never edits `tests/fixtures/` (R-12b) | Not shared |
| `tests/native_implicit.rs` | PP only. PW's tests live in `tests/implicit_forest.rs`, and PV's reuse tests in `tests/vector_reuse_payloads.rs` | Not shared |
| `restore_payload` | Defined by PW in `src/implicit/tileset.rs`; called by PV from `reuse.rs` | Interface, not shared editing |
| `CHANGELOG.md`, `docs/*` | R only. Authors send text to R | Not shared |
