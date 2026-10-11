# P-IMPL design change log

## v3 (2026-10-11): user decisions on PD-11, PD-12 and PD-13

v2 is preserved in `v2/`. v3 keeps v2's layout, document rule, bounds and Cesium evidence unchanged
and applies only these decisions.

| Item | Decision | What changed in v3 | Where |
| --- | --- | --- | --- |
| PD-11 | No duplicate retained sources in default archives. `--reuse-tileset` reads the archive's actual payloads | **Reversible rebase.** The writer changes only the GLB scene-root node object's text (replacing `translate_content`'s whole-document re-serialization) and returns per payload δ, the raw root node text, the raw JSON chunk length and the raw sha256. PV records these in a `vector-build.json` `payloads` map. Reuse restores exact raw bytes, checks the sha256 and stages them privately. The match identity, granularity and re-encoded set are unchanged. `rustyTilesSourceContents`, `SourcePolicy` and retained `t/*` copies are gone. v2's I1 retained-source ask is withdrawn: old archives with the list keep validating through I1's unchanged gate. Evidence: exactness 14,703/14,703 on production payloads (default, `--quantize`, `--meshopt`, both, and 12.5k LOD-chain); today's whole-document re-serialization is not invertible for meshopt (0/544); naive f64 un-baking misses 8/544 quantized payloads; reuse against a fresh build on realistic edits is in §5.4. Archive size on the model: vector perf 37.1 MB → 17.8 MB of tiles, and mf2-lod3 148 MB → 67 MB, about `--explicit`'s size | design §4.1, §4.5, §4.6, §4.7, §5.4, A-8; open-decisions PD-11; ownership (PV scope, PW `rebase_payload`/`restore_payload`); `evidence/v3/` |
| PD-12 | The coordinator's validator slice raises the entry and visit caps | Recorded the v3 per-tile counts as that slice's inputs. Vector: entries 1.08–1.20 and visits 1.09–1.23 per tile. At 100k vector tiles: 110,523–120,336 entries and 110,916–122,797 visits. Points: about 1.0 per tile. 2^17 admits every case with at least 6% margin; 2^18 gives at least 2.1× | design §5.1; PD-12; `evidence/v3/scale/` |
| PD-13 | Accepted: I1 excludes link tiles from `budgets.tiles` | Recorded as an I1 interface item. A-3's expected `tiles` uses I1's counter (it counts links), and `budgets.tiles` compares \|H\| | design §4.6, A-3; ownership "I1 interface items" 4 |

**Found while measuring (outside P; recorded for the vector owner).**
- GeoJSON features without a top-level `id` get ordinal source ids, so a single insertion or
  deletion shifts every later fingerprint.
- With such inputs, reuse restored 21 of 544 contents (perf) and 3,199 of 12,527 (12.5k), and was
  about 6× *slower* than a fresh build at 12.5k tiles (§5.4).
- With stable ids (GPKG fid, or a GeoJSON `id`), the same edit reuses like an edit-only change.


## v2 (2026-10-11): responds to PN's precode review NO-GO

| | |
| --- | --- |
| Review | `/tmp/pimpl-precode-review-20261011/review.md`, sha256 `16205c1c554eb9ed058f1ccef1db998bae23af4266e9772c1a2ea43c34ca51ae` |
| v1 | Preserved in `v1/` (`design.md` `24bf76ec…`, `open-decisions.md` `6e6e2950…` with the coordinator's resolution, `ownership-plan.md` `22effb0d…`, the coordinator's `SHA256SUMS`, and `open-decisions.pre-resolution.md`) |
| v2 evidence | `evidence/v2/`. The v1 evidence stays in `evidence/` as v1's record; files v2 supersedes are listed below |

The core choice is unchanged: the bounded implicit forest. PN confirmed the root cause and proved
it causally with the patched-Cesium ablation. v2 changes three things and corrects the evidence:
- how the explicit skeleton is stored (R-1);
- the depth bound (R-2);
- the I1/P retained-source interface (R-3).

| R | Review item | What changed in v2 | Where |
| --- | --- | --- | --- |
| R-1 | `tileset.json` scale against the validator's JSON limits (depth 64, 65,536 nodes, 8 MiB), and up-front load | **Re-decided (PD-2).** The explicit skeleton is now split into **explicit external tileset documents**. A skeleton tile that would overflow its document's budget becomes a *link tile* (box, error, refine, transform; `content.uri` = `tileset-d{n}.json`), and the tile itself is the root of that external document. Budgets per document: 16,384 value nodes, 2 MiB, 12 skeleton levels; JSON depth ≤ 32 is asserted. S = 4 stays for both schemes, chosen against S = 6 and 8 on evidence. Every document of every modelled case fits with at least 4× margin on nodes, 10× on bytes and 2× on depth. That includes vector at `maxTiles` 100k with `lodLevels` 3 and 16, real and full chains, and point octrees up to 350k tiles. I1 v3.1 admits explicit external tilesets through the existing traversal; I1 needs no admission change. P1 is restated | design §1, §3.2, §4.1, §4.6, §5.1, §5.3; open-decisions PD-2, PD-10; `evidence/v2/forest_model.py`, `scale/`, `scale_runs.sh` |
| R-2 | Reverse PD-5 | **Reversed.** The writer refuses more than 32 levels (level ≤ 31) for every producer with a typed `Error::Data`. That is today's effective vector and point bound, so the domain does not shrink. The point-cloud 64-level idea is withdrawn. JSON depth is bounded per document by the document rule, independently of L. A-1 now tests 32 levels (accepted, and validated by A-3) and 33 levels (the typed error) | design §4.2 P9, §4.4; PD-5; A-1 |
| R-3 | Retained vector sources | **Investigated; recommend (b), not (a).** The `t/<sha256>` sources are what `--reuse-tileset` reads from the previous archive (`reuse.rs:340-390`, via `vector-build.json` `explicitRoot`). README `:201` and VECTOR.md `:260-274` document reuse from the default implicit archive. So (a) would break a documented user feature. v2: a document that holds implicit roots lists their raw sources in its own `extras.rustyTilesSourceContents`, which keeps the list bounded by the document budget. The I1-B change: drop the `doc.root.implicitTiling` gate at `validate.rs:1204` and honour the list on every tileset document that has it. P4 and A-2 exempt listed sources. (a) and its size saving are recorded as a user decision (PD-11) | design §4.1, §4.6; PD-11; ownership "I1 asks" |
| R-4 | One per-root slot list | Each implicit root's distinct (slot, suffix) list drives both its templates and its subtree's content layers. A root without content gets no templates and no `contentAvailability` | design §4.1, P10 |
| R-5 | Refine | The implicit root's `refine` is the producer's. A differing `refine` below an implicit root is a typed error. Chunk-document REPLACE hard-coding is gone with `write_chunks` | §4.1, §4.4 |
| R-6 | `ForestOutput.tiles` covers H | One record per tile of H, in preorder, with role and written bytes. `encodedBytes` semantics are stated per role. The vector report computes every listed statistic from it | §4.5; ownership interface |
| R-7 | Literal `r{n}` | `n` is written as decimal digits at write time, for example `implicit-content/r17-{level}-{x}-{y}-0.glb` | §4.1 |
| R-8 | Strip `implicitChildIndex` | Stripped from skeleton, explicit-leaf and link tiles (and rows) | §4.1, P11 |
| R-9a | Overstated conformance | §3 and PD-1 now match PD-7: the "more specific" dispute remains; its client consequence is removed | §3.1, PD-1 |
| R-9b | Model-only sizes | The v2 model reads the producer's `--explicit` archive, so skeleton `extras` and `transform` are kept. Every size is labelled as a model of the production writer | §5 |
| R-9c | Views reached no vector leaf | Views now aim at a real vertex of the leaf's geometry, at 40 m and maximumScreenSpaceError 1e-4. `leafSelected` is reported per layout; null=null picks are reported separately and never counted as agreement | §5.2; `forest_model.py leaf_target`, `compare_runs.py` |
| R-9d | Receipts for the shipped scripts | Every v2 result records the sha256 of the exact script that produced it. v1's `cesium/real-vector-msse1-det.json` (probe `ffe8ff0c…`, not shipped) is superseded by PN's rerun and by v2's runs. It moves to `evidence/superseded/` | §9 |
| R-9e | Stale names and nice level | The text names the real files. Receipts record the actual niceness: the session runs at −4, so `nice -n 10` gives 6 | §9 |
| R-9f | `pc-deep` "did not settle" | Marked as contended (CPUs 0 and 1, shared) and non-comparative | §5.2 |
| R-9g | Pin I1 | I1 v3.1 `contract-draft.md` `9674e60b…`. OD-1b is withdrawn in v3.1 | header, §4.6 |
| R-10 | A-6 controls | ABBA ×2 rounds per case, one case per invocation, on the same pair (CPUs 13,15 = P-cores 6 and 7; siblings 12 and 14). Per-CPU busy is sampled around every invocation. Cases: vector perf, mf2-lod3 (12.5k tiles, LOD chains) and `pc-deep`; 24 runs, no gate problem. The lane's default framing loads no implicit root for either vector case, so an mf2-lod3 deep-view ABBA ×2 times implicit-root and document loading (`deep_abba.sh`): the forest is about 11% faster. Lane scripts are pinned to `bb7d054` (extracted); `cesium_cpus.py` differs only in its affinity line | §5.3, A-6; `perf_abba.sh`, `cpustat.py` |
| R-11 | A-4 payload equality | Defined: the BIN chunk is byte-identical; the JSON chunk is equal as parsed JSON except for scene-root `translation`/`matrix`; δ is compared after composing it with the implicit root's `transform`; the f64 summation order is stated | A-4 |
| R-12 | Ownership | (a) M2's writer call moves to `write_forest` (PP). (b) `tests/fixtures/implicit_levels.cjs` belongs to PP alone. (c) PW deletes `expand_tileset_bounded` and `ExpansionBudget` after I1-B's `:20` removal, or uses them inside `expand_tileset`. (d) One budget is shared across all roots and documents | ownership-plan |
| R-13 | I1 acceptance | A-3 lists the exact `checks` and `notInspected` sets and the tile counts. I1 is asked for W11 witnesses of a translated non-root implicit root, an implicit root inside an explicit external document, and a retained-source list in an external document | A-3, ownership "I1 asks" |
| R-14 | PD-4 at scale | The LOD-chain vector case (12,530 tiles) is measured with and without `cullWithChildrenBounds`, and against `--explicit`. The share of views selecting a coarser tile is reported | §5.2, PD-4 |

**New findings in v2 (executed):**
- **F-1.** Cesium 1.146 takes the metadata schema only from the root `tileset.json`
  (`Cesium3DTileset.js:2470`, `makeTile`), including for implicit roots inside external documents.
  - The first v2 model, which put the schema only in documents holding roots, failed every
    implicit tile load in an external document ("Cannot read properties of undefined (reading
    'classes')"). That run was discarded when the model was fixed.
  - It is reproduced deterministically by the fixture negative control `N`: `D` without the
    schema in the root (`evidence/v2/cesium/fixtures-results.json`).
  - Contract P12: `tileset.json` carries the schema whenever any implicit root exists, and every
    document holding an implicit root also carries it.
- **F-2.** The validator's archive-level caps (`archive_entries` 65,536 and `hierarchy_visits`
  65,536, both C1-owned and kept by I1) bound validated vector archives at about 32k tiles in
  **any** layout, today's included. Vector `maxTiles` 100k is outside the validated domain whatever
  layout is chosen. The forest lowers both counts per tile (§5.1). This is recorded as PD-12 for the
  coordinator, not as a P obligation.
