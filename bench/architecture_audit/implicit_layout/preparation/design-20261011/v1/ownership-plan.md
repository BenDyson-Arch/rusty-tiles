# P-IMPL ownership plan

P is the producer layout lane of the combined I1+P integration PR (I1 ownership plan v3, role P).
I1 owns the validator and the reader. P owns everything that writes or self-reads the producers'
implicit layout. Within P, three authors work in parallel, each on files nobody else in P edits.
The coordinator R (I1's R) owns every Cargo build, assembles the combined branch and runs the
perf lane and the browser checks. Line references are to `f2ed077`.

## Roles

| Role | Owns | Must not |
| --- | --- | --- |
| **PW**, writer author | The forest writer and its unit tests; the shared expander rework (PD-8) | Edit producer modules; edit `src/validate.rs` or `src/content_integrity/*` |
| **PP**, point-cloud author | The point-cloud and legacy-mesh integrations | Edit `src/implicit/*`; edit vector files |
| **PV**, vector author | The vector integration: report statistics, reuse identity, docs | Edit `src/implicit/*`; edit point-cloud or mesh files |
| **PO**, P oracle and acceptance author | Independent forest checks: stdlib Python that imports nothing from `src/`, and the Cesium probes | Read the writer diff before the checks are frozen |
| **PN**, P nonauthor reviewer | Precode review of this design, and final-source review of P's part of the combined PR | Be PW, PP, PV or PO |

PP and PV may be the same person, because their files are disjoint. They must not be PW, so that the
writer interface is reviewed by a consumer.

## File ownership (no overlap)

| Owner | Files |
| --- | --- |
| PW | `src/implicit/tileset.rs` (the whole file); the writer parts of `src/implicit.rs` (`Subtree`, `TileMetadata`, the re-exports on `:21`; **not `:20`**, which belongs to I1-B); `tests/implicit_metadata.rs`; `tests/implicit_subtree.rs`; a new `tests/implicit_forest.rs` |
| PP | `src/point_cloud.rs` (`:600-640`: the writer call, the members, the encoder string); `src/point_cloud/tiles.rs` (`:151` depth limit, PD-5); `src/tile.rs` (`:510-540`: staging from the writer's members, the encoder string); `src/tileset.rs` (`:153-177`, M2: the encoder string only); `tests/native_point_cloud.rs`; `tests/native_implicit.rs`; `tests/mesh_tile.rs`; `tests/mesh_crs.rs`; `tests/p1_point_oracle.py` (its implicit walker, `:278-339`, gains the forest and loses the JSON-template chain); `tests/fixtures/implicit_levels.cjs`; `tests/fidelity.rs` (implicit call sites only) |
| PV | `src/vector/pipeline/store.rs` (`:970-1046`: the writer call; replace the `expand_tileset` consumer on `:1003-1013` with the writer's statistics); `src/vector/pipeline/reuse.rs` (`:268-270`, tiling identity `implicit-quadtree-v2` becomes `implicit-forest-v3`; `publish` content inventory over the forest); `tests/fixtures/vector_compat.cjs` (`:68`); `tests/fixtures/vector_compat.py` (`:93-94`); `tests/vector_test_support.py` (`:37`, the implicit default for browser fixtures); `docs/VECTOR.md` |
| PO | `bench/architecture_audit/implicit_layout/` (a new directory): an independent forest checker derived from `evidence/layout_probe.py` and `evidence/relayout_forest.py`; the Cesium fixture and deep-view probes, ported from `evidence/cesium/`; frozen expectations |
| R | Cargo builds; the combined branch; the perf lane at `bb7d054`; `CHANGELOG.md` (`:28` rewritten); `docs/FORMATS.md`; `docs/CLI.md`; a new `docs/architecture/p-impl-layout-contract.md` (from design.md); the architecture README index |
| Nobody in P | `src/validate.rs`; `src/content_integrity/*` (I1-A and I1-B); `src/implicit.rs:20` (I1-B); `src/convert_implicit.rs` (deleted by #159); `src/validate/containment*` |

## The interface between PW and the integrators (agreed before coding)

```rust
// src/implicit/tileset.rs (PW)
pub(crate) struct ForestOutput {
    /// Every member the writer created or retained, including skeleton payloads it left in place.
    pub members: Vec<String>,
    /// Per available tile, in preorder: final payload bytes per slot (after any baked
    /// translation) and the tile's extras. This replaces re-reading the output (store.rs:1006).
    pub tiles: Vec<WrittenTile>,
}
pub(crate) fn write_forest(
    manifest: &mut Value,          // producer's explicit hierarchy in, forest manifest out
    directory: &Path,
    scheme: SubdivisionScheme,
    sources: SourcePolicy,         // Retain (vector reuse) | Remove (point, mesh): rewritten sources only
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<ForestOutput, Error>;
```

- `write_tileset` and `write_tileset_recorded` are removed. Each producer calls `write_forest`
  once.
- `SourcePolicy::Remove` deletes only sources that were rewritten into `implicit-content/`.
  Skeleton payloads stay where they are and appear in `members`.
- The boolean `retain_sources` becomes a two-variant enum. That is the only producer-dependent
  input.
- A change to this interface needs PW, PP, PV and R.

## Merge sequencing

1. #159 merges, including the convert retirement. This is I1's gate as well.
2. PW lands the writer and `tests/implicit_forest.rs` on the P branch. PO's checks are frozen
   before PO sees the writer diff.
3. PP and PV integrate on the P branch in parallel, rebased on PW. Their files are disjoint.
4. R assembles the combined PR (I1-A, I1-B, P) and runs all of the following on the combined head:
   - Cargo and CI;
   - W11 (I1);
   - PO's checks;
   - the perf lane;
   - the browser probes;
   - the wheel and Blender checks.
5. PN reviews P's final source. I1's N reviews I1's part.

There is no separate P merge, because today's validator refuses implicit roots on explicit tiles
(`validate.rs:1280-1284`), and I1 makes today's JSON templates Unsupported.

## Overlap check

| File | Touched by | Resolution |
| --- | --- | --- |
| `src/implicit.rs` | PW (writer, `:21`) and I1-B (`:20`) | R applies them as separate commits (I1 plan) |
| `src/vector/pipeline/reuse.rs:81-82` | Nobody edits it. The `include_str!` of `implicit/tileset.rs` changes the encoder hash automatically, so old vector caches are rejected | Intended |
| `tests/fidelity.rs` | PP only, for its implicit call sites (`:142`, `:427`, `:515`) | Not shared |
| `tests/native_implicit.rs` | PP only. PW does not edit it; PW's tests live in `tests/implicit_forest.rs` | Not shared |
| `CHANGELOG.md`, `docs/*` | R only. Authors send text to R | Not shared |
