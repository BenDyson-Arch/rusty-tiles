# P-IMPL delta review: v3 (responding to R-1 to R-14, plus PD-11, PD-12 and PD-13)

Reviewer: PN. Date: 2026-10-11. This is a delta review of the v1 NO-GO (`review.md` `16205c1c…`).

**Package.** `/tmp/rusty-tiles-pimpl-design-20261011/`. `SHA256SUMS` is OK for `design.md`,
`open-decisions.md`, `ownership-plan.md` and `change-log.md`. `evidence/v3/SCRIPTS.sha256` is OK (10/10).

**Rules followed.**
- No cargo was run and no repo was edited.
- The I1 branches `i1/owner` `ab4cfb7` and `i1/validator` `36f9ffe` were read with `git show` only.
- The one browser run used `taskset -c 13,15 nice -n 10`, after checking that 13 and 15 were idle.
- The model reruns used E-cores 17,19.
- Every output is under `delta/`.

## Verdict: **GO-WITH-CHANGES**

v3 closes the v1 blockers:
- **R-1.** The documents keep every JSON document far inside the per-document limits. My reruns match the
  design exactly.
- **R-2.** L ≤ 32, enforced as a typed error.
- **R-3.** Superseded by PD-11; no retained sources and closed names.

The link-tile layout is spec-clean. The reversible rebase is exact on every production payload I re-ran.

The implemented I1 has two cross-slice gaps:
- **D-1.** PD-13 is not implemented.
- **D-2.** A link document's bytes count against the per-tile byte budget, which is new.

Both are needed before the combined PR merges, but neither changes the P writer's interface. Implementation
may start. D-1 to D-5 must be closed in the combined PR; D-1 and D-2 need the coordinator's I1-side action.

## (1) R-1 soundness

**Reruns.** I re-ran `forest_model.py` on a copy, changed only so that it keeps every document instead of the
top five (`delta/scale/`):

| Case | Max nodes, bytes, depth | Documents | Small documents (< 500 nodes) |
| --- | --- | --- | --- |
| vec-perf | — | 1 | — |
| mf2-lod3 | 16,294 / 181,949 / 20 | 189 | 114 |
| mf2-lod16 | 16,294 / 198,125 / 31 | 291 | 102 |
| synthetic 100k, `lodLevels` 16, full | 16,328 / 204,817 / 30 | 5,745 | **5,556** (median 368 nodes) |

Every summary field matches `evidence/v3/scale/*-S4.json`.

**Can any input exceed the limits?**
- **Nodes and bytes: no.** The packer reserves the link cost for every frontier child. It accepts a child only
  when the exact unit cost fits a quarter of the validator caps, and a unit is bounded by the branching
  factor (one skeleton tile plus at most 8 children). The only escape is a single unit over 16k nodes, which
  §4.4 makes a typed error.
  - The model never checks the *document root's* unit against the budget. It checks only children. The
    production writer must assert P8 on every finished document, not only depth (A-1's "unit near the
    budget" case covers this).
- **Depth: bounded, with one level of margin.**
  - The deepest path I measured is 31:
    `root` + 13 `children` levels + `content.extensions.3DTILES_content_gltf_vector.vector`.
  - DOC_LEVELS = 12 fixes the maximum at 1 + 2·13 + 4 = 31. P8 asserts ≤ 32.
  - One more level of nesting in any content header or extras would turn every deep conversion into a
    writer error.
  - The validator allows 64. → **D-3.**

**Tiny documents: yes, but bounded, and not harmful.**
- With long LOD chains, the 12-skeleton-level cut, not the node budget, creates one small document per chain
  that crosses the cut: 5,556 of 5,745 in the 100k `lodLevels` 16 case.
- Each costs 1 entry, 1 request and 3 visits. That is already inside the PD-12 counts (1.23 visits per
  tile).
- A path crosses at most ⌈(L−S)/12⌉ + (node splits) documents, and the measured deep views need fewer
  requests than today.
- Packing by real JSON depth (budget ≤ 48) instead of skeleton levels would remove most of them. This is
  optional (part of D-3).

**Spec cleanliness of link tiles: clean.**
- Core `:731-741`: a tile pointing to an external tileset omits `children`, and transforms compose (the link
  tile carries T; the document root has no transform).
- Core `:911`: "Any class that is defined in the schema of an external tileset shall also be defined in the
  top-level tileset schema". P12 meets it, and this is the spec basis for F-1.
- I reproduced F-1 (`delta/cesium/results-DN.json`):
  - the fixtures regenerate byte-identically;
  - `D` is 8/8 selected and picked, with 3 JSON requests;
  - `N` is 0/8, with 4× "Cannot read properties of undefined (reading 'classes')";
  - `F` is 8/8.

**Does I1 admit the layout unchanged? Traversal yes; budgets no** (`delta/agent-i1code/notes.txt`;
`V` = `i1/validator:src/validate.rs`).
- **Admitted.**
  - External documents are followed (V:1481-1493), and implicit roots inside them are admitted (V:1369-1377;
    tests at `i1_implicit_read.rs:422-443`, `:628-655`).
  - The JSON-template Unsupported is raised only from `admit_root` (`implicit.rs:513-518`).
  - Each document is read with the full JSON limits (V:878-887).
  - Visits: each document adds Tileset + ExitTileset, plus the link node, which matches §4.6.
  - The schema is taken from the document holding the root (V:1250-1269). A document with roots but no schema
    is InvalidInput, so P12's per-document half is also required by I1. The root half is checked only by A-2.
- **D-1: PD-13 is not implemented.**
  - V:1931-1935 is develop's `check.tiles > budgets.tiles`.
  - `check.tiles` counts links (V:1416).
  - Vector records `budgets.tiles = maxTiles` (`store.rs:1029`).
  - So any output with |H| + links > maxTiles fails "hierarchy exceeds recorded tile budget". That includes
    near-cap default runs and any user `--maxTiles` close to |H|.
- **D-2 (new): a link document counts as tile bytes.**
  - The member size is added before the `.json` branch (V:1471-1477) and checked against `budgets.bytes`
    (V:1331-1335, `tile_budgets`).
  - `--maxBytes` accepts values down to 4,096 (`vector.rs:198`). The default is 4 MiB.
  - Documents reach about 250 KB measured, with a 2 MiB budget. Any vector run with `--maxBytes` below its
    largest document fails validation.

## (2) PD-11: exactness and failure modes

**Exactness.** I re-ran `rebase_model.py` on the five production archives (`delta/rebase-rerun.json`):
- perf default, `--quantize`, `--meshopt`, both, and mf2-lod3: **14,703/14,703**, matching on every field.
  This covers round-trip byte exactness, the sha256 equal to the immutable name, the BIN chunk untouched, and
  the result parsing equal to production's bake.
- The archive sha256 values are identical to the receipts.
- The mechanism is sound by construction. Only the root node's text span changes. The JSON chunk is
  right-trimmed of space padding and re-padded to the recorded raw length.

**`--meshopt`.** The splice preserves raw key order, so today's whole-document re-serialization (0/544
invertible) is no longer the path.

**`--quantize`.** The raw root carries a translation (and possibly scale). Adding δ to `translation` is correct
under TRS. The recorded raw text avoids the 8/544 f64 un-bake misses.

**Stale or corrupt records.**
- Restore checks sha256(result) against the immutable name. A wrong `rootNode`, a wrong `jsonLength` or an
  altered member all fail with "previous content checksum failed". That is a hard integrity error, as today
  for a corrupt `t/*`.
- `vector-build.json` stays hash-bound to `tileset.json` (`vectorBuildStateSha256`, `reuse.rs:184-193`).
- **Gaps (D-4).** The model *asserts* in these cases, where production must raise typed errors:
  - a `jsonLength` shorter than the restored text;
  - not exactly one scene root;
  - a missing entry together with a missing `t/<sha>` member, which today gives a raw zip error.
- `manifestSha256` binds only `tileset.json`, not `tileset-d*.json`. That is harmless, because reuse reads
  `explicitRoot` and the payloads map, never the documents. Note it.

**Generality (D-4).**
- The design defines `rebase_payload` for "the root node" with `translation` only. `translate_content` today
  handles several scene roots and `matrix` roots. Every measured producer payload has one root and no matrix:
  vector, `pc-deep`, mesh, mesh-local and glb-wrap (`delta/` survey).
- Specify either support for N roots and `matrix`, with one record per root, or a typed error.

**Older archives.**
- **Validate.** The `rustyTilesSourceContents` gate moved unchanged (V:1273-1289), so pre-v3 *single-tree*
  archives still validate. Pre-v3 *chunked* archives become Unsupported under I1 (`i1_implicit_read.rs:259-275`):
  vector perf, mf2, `pc-deep`, which is most real default vector output. That is I1's decision, but users need
  to be told (D-5).
- **Reuse.** Every pre-v3 archive is refused through the encoder hash: "run a fresh conversion". Also say
  this in the CHANGELOG (A-7 already lists it).

**Size.**
- The `payloads` map is about 200–300 B per rewritten payload in `vector-build.json` (about 20–30 MB at 100k
  tiles).
- That is within reuse's read limit of max(16 MiB, maxTiles·4096) (`reuse.rs:134`). Note it.

## (3) Evidence

These support the claims:
- the scale tables (reproduced);
- F-1 (reproduced);
- rebase exactness (reproduced);
- the reuse table: I recomputed every median and count in §5.4 from the raw `reuse-bench.json` (52 runs, all
  exit 0). It matches.

**Caveats, already labelled in the design:**
- The v3 reuse *timing* is today's binary plus a modelled restore (0.43 ms per payload in Python). A-8 gates
  the real figure.
- Archive sizes are model figures.
- The LOD-chain deep views show the forest selecting a strict ancestor in **13/16** views against today (v1
  had 2/12). The target path and non-null picks are identical, and the result matches `--explicit`. This is
  much larger than the PD-4 the coordinator first accepted. It should be re-acknowledged explicitly, together
  with the note that "equal pixel coverage" is a coverage metric, not detail equality (D-6).

**Citation slips:**
- I1 `:705` is "Counters". The `checks` ordering is at `:713-714`.
- The "`admit_root` Unsupported" statement is at `:688-689`, not `:727`.

## (4) R-item closure

| R | Status |
| --- | --- |
| R-1 | Closed (documents). Residual items are D-1, D-2 and D-3 |
| R-2 | Closed (L ≤ 32 typed error; A-1 tests 32 and 33) |
| R-3 | Superseded by PD-11; closed (no list, P4 closed names, A-2 control) |
| R-4 | Closed (P10) |
| R-5 | Closed (§4.4) |
| R-6 | Closed (§4.5; links have no record) |
| R-7 | Closed |
| R-8 | Closed (P11) |
| R-9a to R-9g | Closed. `evidence/superseded/` exists, receipts carry script hashes, I1 is pinned to v3.1 |
| R-10 | Closed (ABBA ×2, sibling sampling, deep-view ABBA) |
| R-11 | Closed (A-4) |
| R-12 | Closed: (a) M2 call, (b) the `implicit_levels.cjs` owner, (c) and (d) budget and dead code |
| R-13 | Closed. Fix the `:705` citation |
| R-14 | Measured. Re-acknowledge it (D-6) |

## Required changes

1. **D-1 (I1-B, before merge).** Implement PD-13 on `i1/validator`: compare `budgets.tiles` against |H|,
   excluding link tiles. Add an A-3 case where |H| ≤ maxTiles < |H| + links (for example `--maxTiles` set to
   |H|).
2. **D-2 (coordinator decision; I1-B or PW, before merge).**
   - Preferred: exclude external-tileset `.json` members from per-tile `bytes` in `tile_budgets`. They are not
     encoded tile bytes, in the same spirit as PD-13.
   - Alternative: the writer caps document bytes at min(2 MiB, maxBytes). That couples P to a vector option
     and multiplies documents.
   - Add an A-3 case with a small `--maxBytes`.
3. **D-3 (PW).**
   - Make the document depth bound a packing criterion, not a post-hoc assertion with one level of margin:
     link when the real JSON depth would exceed a budget (for example 48), or derive DOC_LEVELS from it.
   - Restate P8 as ≤ that budget (≤ 64 enforced).
   - Assert P8, both nodes and bytes, on every finished document, including the document root's own unit.
   - Optionally this also removes most tiny documents.
4. **D-4 (PW and PV).**
   - Define `rebase_payload` and `restore_payload` for several scene roots and `matrix` roots: support them,
     with one record per root, or raise a typed error.
   - Make every malformed-record case a typed reuse error ("previous content checksum failed" or "invalid
     previous payload record"), never a panic:
     - a `jsonLength` shorter than the restored text;
     - a root count mismatch;
     - an unparsable published JSON;
     - a missing member.
   - Extend A-8's negative cases to these.
5. **D-5 (R, docs).** CHANGELOG and VECTOR.md must state that:
   - pre-v3 *chunked* default archives (most vector and deep point outputs) are reported Unsupported by
     `validate` under I1;
   - pre-v3 archives are not reuse baselines.
6. **D-6 (coordinator record).** Re-acknowledge PD-4 at the measured scale: 13/16 off-target coarser
   selections on LOD-chain vector, equal to `--explicit`.
7. **Note (D-7).** Fix the I1 citations (`:705` → `:713-714`; `:727` → `:688-689`). Note the `payloads`-map
   size and that `manifestSha256` covers only `tileset.json`. Bump the `vector-build.json` state `version`, or
   state why the encoder hash suffices.

## Files

All under `delta/`:
- `scale/`: the full-document reruns.
- `ev/v3/`: the script copy, plus `forest_model_alldocs.py`.
- `rebase-rerun.json`.
- `cesium/`: the D, N and F rerun.
- `agent-i1code/`: the I1 branch extracts and notes, which I verified at V:1331-1335, :1466-1515 and
  :1928-1937.
