# P-IMPL oracle: bounded implicit forest checks (PO)

PO's independent acceptance checks for P-IMPL, the bounded implicit forest (design v3). They cover
A-2 (forest checker and its sensitive controls), A-4 (equivalence to an `--explicit` build) and the
Cesium probes of A-5. The checks were frozen before PO saw the writer diff.

**Refreeze 1** follows the coordinator's rulings on PO's spec questions, which rest on the writer's
documented contract, not its code. The changes:
- `DOC_LEVELS` is 10 (review D-3).
- Documents must be compact JSON.
- A row with δ = 0 is copied bit for bit, and a payload with δ = 0 is a byte copy.
- The payload-map sentinel `rootNode: ""`, `jsonLength: 0` is accepted.
- An affine `matrix` root takes δ in `matrix[12..14]`. A matrix combined with TRS, a non-affine
  matrix, or several scene roots is refused.

See "Rulings" below.

Nothing here claims release readiness. A passing checker run shows that one archive meets the
contract as stated below. It does not show that the writer is correct for other inputs.

## Independence and provenance

- PO read no file under any `src/` and did not see the writer diff (branch `p/writer`).
- The base is `develop` `68b7531`, branch `p/oracle`. All files live under this directory.
- Everything is stdlib Python, except the Cesium probe (`probe.cjs`, Node + Playwright).
- The contract comes from these inputs:

| Input | sha256 |
| --- | --- |
| `/tmp/rusty-tiles-pimpl-design-20261011/design.md` (v3) | `1c84e8ece066b1d433ad51926ead80b3265642b1cc0385325bd201f77a3e3aae` |
| `…/open-decisions.md` | `10f5cb7d8d0e316187b242f450d73def16074ef1c21ecb55802ababe9ecfaa85` |
| `…/ownership-plan.md` | `b1ebaaa0062657f6aa63f2a47c94e3964bb12f9d297f065d881c0aeb23526a36` |
| `…/evidence/v2/forest_model.py` (derivation source; not imported) | `8de71a3d62db4e823a4fe3f768641bca4400685c9fcf57b24088a20451a696e5` |
| `…/evidence/v3/forest_model.py` (used only for the cross-checks) | `a3c9eeaff550fb308898a8ce6b1acfc2f4911c9fac2859613bff79e8921bfd93` |
| `…/evidence/v3/rebase_model.py` (restore semantics) | `2f934b76afc03c73fe54751c5bf12072e986e0a696b94ee3796e0d5d57a436db` |
| `…/evidence/layout_probe.py` (derivation source) | `c00ad119a30315f94467c28e8a04d453bb65f6cd90e6d8768bce80cceb277304` |
| `…/evidence/v2/cesium/` probes and `compare_runs.py`, `cpustat.py`, `target_path.py` | listed in `SHA256SUMS.inputs` |
| I1 v3.1 `contract-draft.md` (Morton order and template rules) | `9674e60b959eea0a4a985b6beec7a278ec79c08579f6c19e58f5a070aee03496` |
| `docs/architecture/c1-payload-integrity-contract.md` `:200-201` (JSON depth and value-node semantics) | in-tree at `68b7531` |

`SHA256SUMS.inputs` gives the hash of every input, including the sample and mf2 archives used for the
cross-checks. `SHA256SUMS` gives the hash of every file in this directory except itself.

## Files

| File | What it is |
| --- | --- |
| `forest_check.py` | The forest checker (A-2, A-4). Writes a JSON verdict with a result for each rule |
| `make_forest_fixtures.py` | Synthetic producer hierarchies H. Each is written as an `--explicit` reference and as a forest by PO's own v3 writer, plus 57 negative controls |
| `run_controls.py` | Runs the checker over the fixtures and judges each run against its frozen expectation |
| `expectations.json` | The frozen expectations: rules, controls, Cesium fixture outcomes, deep-view requirements and real-archive expectations |
| `results/` | Recorded runs: `controls.json` and `fixtures-manifest.json` (the generated archives' sha256), `model-*.verdict.json`, `develop-*.verdict.json`, `cesium-fixtures*` and `deep-smoke-vec-perf*` |
| `cesium/make_fixtures.py`, `probe.cjs` | Verbatim copies of the design's 8-leaf select/pick fixtures (E/A/S/X/F/D and the N schema-placement control) and the probe |
| `cesium/run.py`, `run_real.py` | The design's runners. The only changes are the perf lane's `CESIUM_DIR`/`NODE_PATH`/`CHROMIUM` overrides and version fields in the receipt |
| `cesium/compare_runs.py`, `cpustat.py` | Verbatim copies |
| `cesium/target_path.py` | A port of the design's `target_path.py`. Parents and errors come from `prepare_real.py` instead of the model |
| `cesium/prepare_real.py` | New. Prepares a deep-view A/B from real archives in any layout: unpacked trees, views, URI→key maps, parents, errors |
| `cesium/judge_fixtures.py`, `cesium/judge_deep.py` | New. Judge the fixture suite (A-5.1) and a deep-view A/B (A-5.2) against `expectations.json` |

## The forest checker

```sh
python3 -I bench/architecture_audit/implicit_layout/forest_check.py FOREST.3tz \
  [--explicit EXPLICIT.3tz] [--out VERDICT.json] [--examples N]
```

- `FOREST.3tz` is a default (forest) archive. `--explicit` takes an `--explicit` build of the same
  input. That build is the producer hierarchy H, and passing it enables the `A4.*` rules.
- Exit status: 0 when every required rule passes, 1 when one fails, 2 on a usage error.
- The verdict holds:
  - `rules`: for each rule, its status (`pass`, `fail` or `not-applicable`), whether it is required,
    its design source, the violation count and up to N examples;
  - `failedRules` and `verdict`;
  - `stats`: tiles, levels, skeleton tiles, explicit leaves, roots, implicit tiles, links,
    documents, link nesting, members, document maxima, the root document, and I1's `tiles` counter
    and `hierarchy_visits` as design §5.1 counts them;
  - the checker's and the archives' sha256.

### Profile

These are the design §4.1 constants. There is no option to change them.
- S = 4.
- `DOC_NODES` 16,384 value nodes. C1 semantics apply: the root, containers and scalars count;
  keys do not.
- `DOC_BYTES` 2 MiB (2,097,152 bytes).
- `DOC_BYTES` is measured on compact bytes. Every document must be compact JSON, as ruled.
- `DOC_LEVELS` 10, by the coordinator's ruling (review D-3; design v3 said 12).
- JSON depth ≤ 32, with the root at depth 0.
- L ≤ 32.

### Rules

Required unless marked advisory.

| Rule | Source | Checks |
| --- | --- | --- |
| `archive` | 3TZ, core | The zip is readable. Member names are unique. `tileset.json` is a tileset. `@3dtilesIndex1@` is ignored |
| `skeleton.S4` | §1, §4.1 | The root is the single implicit root iff h(root) < 4. Explicit tiles with children have h ≥ 4. Their children are explicit leaves (h = 0), skeleton tiles or implicit roots with 1 ≤ h < 4. Heights are measured on the effective tree, so a link and its document root count as one tile |
| `documents.budget` | P8 | Every `tileset.json` and `tileset-d{m}.json` has ≤ 16,384 value nodes, ≤ 2 MiB and JSON depth ≤ 32 |
| `documents.levels` | §4.1, ruling D-3 | Skeleton tiles are ≤ 10 tile levels below their document root. Links and unit children are ≤ 11 |
| `documents.compact` | ruling | Every `tileset.json` and `tileset-d{m}.json` is compact JSON, with no whitespace outside strings |
| `links.wellFormed` | §4.1, P1 | `content.uri` is `tileset-d{m}.json` (and nothing else). The member exists. Each document is referenced by exactly one link. Documents are numbered 1..D−1 contiguously. A link has no children and no `implicitTiling`. The document root has no transform, has `refine`, has children and is not an implicit root. The document's `geometricError` equals its root error. `extensionsUsed` and `asset.version` match `tileset.json` |
| `links.creationOrder` (advisory) | §4.1 | Link numbers are increasing in breadth-first order, over documents taken in m order |
| `P6.linkCoherence` | P6 | A link's box, error and refine equal its document root's |
| `P1.noJsonTemplate` | P1, U-1 | No implicit-root content template whose normalized last segment ends in `.json` (case-insensitive). Every member that parses as a tileset is `tileset.json` or a linked document. No `implicit-tileset-*` members |
| `P2.singleSubtreeRoots` | P2 | `subtreeLevels == availableLevels == h + 1 ≤ 4`, where h is the deepest available level. The subtree URI is `subtrees/r{n}-{level}-{x}-{y}[-{z}].subtree`. Exactly one `subtrees/r{n}-*` member exists. `childSubtreeAvailability` is exactly `{"constant": 0}`. The root tile has no children. Availability is closed: the root is available and no tile is orphaned. The subtree binary is well formed (lengths, 8-byte alignment, `availableCount`, no bits past the stream) |
| `P3.rowsEverywhere` | P3, P12 | The `tileMetadata` table's class (from the root schema) has a FLOAT64[12] `TILE_BOUNDING_BOX` and a FLOAT64 `TILE_GEOMETRIC_ERROR`, with no offset, scale, noData or default. `count` equals the available tiles, and the buffers hold exactly one row per tile. Values are finite. The level-0 row equals the root tile's box and error, bit for bit |
| `P4.namesClosed` | P4, PD-6 | `r{n}` prefixes are distinct, and template and subtree prefixes agree. Every name a template produces for available content exists. Every member is referenced by a tile, a template, a link, a subtree or a `.gltf` resource. Two exceptions are allowed: the report sidecars `conversion.json` and `geometry-reports.jsonl`, and `vector-build.json`, which counts only if `asset.extras.vectorBuildStateSha256` equals its sha256 |
| `P9.levelBound` | P9 | L = h(root) + 1 ≤ 32 |
| `P10.slots` | P10, R-4 | The number of layers equals the number of templates. A root with no content has neither `content`/`contents` nor `contentAvailability`. Templates are `implicit-content/r{n}-{level}-{x}-{y}[-{z}]-{slot}.{glb,gltf,b3dm}`, in strictly sorted `(slot, suffix)` order. Every layer has available content and marks only available tiles |
| `P11.noImplicitChildIndex` | P11 | No `implicitChildIndex` in the extras of any tile in any document, or in any row's extras |
| `P12.schema` | P12, F-1 | `tileset.json` has `schema` iff an implicit root exists, and its `id` is `rustyTilesImplicit`. Every document that holds an implicit root has the same schema |
| `PD11.noRetainedSources` | PD-11 | No `rustyTilesSourceContents` anywhere in any document. No unreferenced `t/` member. No implicit tile names a `t/` member |
| `payloads.wellFormed` | §4.1 | Implicit payloads are GLB (or GLB inside b3dm) with correct lengths and 4-byte chunks. A `.gltf` appears only at an implicit root |
| `vectorBuild.reuseMap` | §4.7 | Applies when `vector-build.json` exists. It has a `payloads` map. Each entry `{member, delta, rootNode, jsonLength}` names a published `implicit-content/` member, and the design's `restore_payload` reproduces bytes whose sha256 is the key. The zero-δ sentinel
(`rootNode: ""`, `jsonLength: 0`) restores the published bytes unchanged. With `--explicit`, every raw payload published only under `implicit-content/` has an entry, and no entry is extra |
| `A4.tree` | P5, A-4 | Expanding every document and root gives exactly H's tiles and parent/child relation. Under implicit roots, H's child slot (`implicitChildIndex`, else the array index) gives the coordinates through I1's Morton rule. Explicit children carry no slot, so they are matched by box, error and transform, then by box and error, then by position |
| `A4.roles` | §4.1 on H | Each tile's role (skeleton, explicit leaf, implicit root, implicit) is the one S = 4 assigns it in H |
| `A4.explicitTiles` | P5, §4.1 | An explicit tile, or a link plus its document root, equals H's tile object. The comparison drops `children` and `implicitChildIndex`, compares refine by its effective value, compares `content.uri` separately, and treats empty and absent extras as equal. An implicit root carries H's transform, box, error, effective refine and per-slot content headers. H keys that a row cannot represent fail, and so does a refine change below a root |
| `A4.rows` | P3, A-4 | See "A-4 arithmetic" below |
| `A4.extras` | P5, §4.5 | Row extras equal H's extras without `implicitChildIndex`. `encodedBytes` (when H has it) equals the published bytes of the tile's contents |
| `A4.payloads` | P5, A-4, §4.1 | Explicit payloads are byte-identical to H's. An implicit root's own payload is a byte copy. Other implicit payloads must meet four conditions: the same b3dm tables; byte-identical BIN and later chunks; the same parsed JSON outside the scene-root nodes; and the root `translation` equal to raw + (δ0, δ2, −δ1) bit for bit. An affine raw `matrix` takes the same addition at `[12..14]`;
a matrix combined with TRS, a non-affine matrix, or more than one scene root fails. A payload with
δ = 0 must be a byte copy. In addition, the JSON bytes outside the scene-root node objects must be identical: the root-node splice of §4.1 and §4.7. A `.gltf` is a byte copy. Layer membership follows the slot list |
| `A4.names` | PD-6, R-7 | Each implicit root's `r{n}` is its preorder index in H, in H's children-array order |
| `A4.contentUris` (advisory) | §4.1 | Explicit-tile content URIs equal the reference build's. The two modes may name payloads differently, for example point-cloud `t/{n}.glb`; bytes are compared by `A4.payloads` |
| `A4.topLevel` (advisory) | §4.1 | `tileset.json` `geometricError`, `extensionsUsed` and `asset.version` equal the reference's |

### A-4 arithmetic (as implemented, with the summation order stated)

- For an implicit tile t below root r:
  - δ(t) = (((0.0 + T₁) + T₂) + … + T_k), per component in f64, where T₁…T_k are the translations
    of the child transforms on the path from r (exclusive) to t (inclusive), taken parent to child.
  - Expected row centre = B(t).c + δ(t), one f64 addition per component.
  - Half-axes and error are H's, bit for bit.
  - For t = r, and for every t with δ(t) = 0, the expected row is B(t) itself, bit for bit, so −0.0
    stays −0.0 (P3's level-0 rule and ruling Q1).
- Composing with r's own transform (equal to H's, checked bit for bit) gives the world frame. So the
  head's per-root δ composed with each implicit root's transform is checked as two exact facts:
  the transform and the row.
- Payload translation: raw `translation` (absent means 0.0) + (δ0, δ2, −δ1), one f64 addition per
  component. An affine `matrix` root takes the addition at `[12..14]`. When δ = 0 the payload is a
  byte copy (ruling).
- All number comparisons use f64 bit patterns, so −0.0 ≠ 0.0. JSON integers equal floats of the
  same value.

### What the checker does not prove

- **P7 determinism.** One archive cannot show it. Build twice and `cmp` the archives.
- **Decoded containment.** C2 slice 3 owns it.
- **The exact document partition.** The design leaves the packing accounting open beyond the
  budgets, so only the budgets, the levels, the numbering and link coherence are checked.
- **Cesium behaviour.** See A-5 below.
- **The I1 report.** `stats.i1TilesCounter` and `stats.i1HierarchyVisits` are the model's counting
  (design §5.1), given as inputs for A-3. The validator is the judge.

## Fixtures and controls

```sh
W=~/.cache/perf-lane/work/pimpl-oracle
python3 -I bench/architecture_audit/implicit_layout/make_forest_fixtures.py $W/fx      # deterministic; manifest.json pins every archive sha256
python3 -I bench/architecture_audit/implicit_layout/run_controls.py $W/fx results/controls.json
```

**Positives.** PO's v3 writer writes each one, and each must pass with and without `--explicit`.

| Case | Exercises | Tiles / L / documents / links / link nesting |
| --- | --- | --- |
| `quad-single` | h(root) < 4: the single root `r0`, a contentless routing tile, vector headers and extras, `vector-build.json` with its payload map, 4- and 8-aligned and unsorted-key raw GLBs | 7 / 3 / 1 / 0 / 0 |
| `m2-single` | The M2 wrapper shape: one tile, ADD, a `.gltf` with an external buffer, copied with its resource | 1 / 1 / 1 / 0 / 0 |
| `oct-mixed` | OCTREE with sparse `implicitChildIndex` slots: explicit leaves, roots of heights 1–3, a contentless root, a root with two slots (glb + b3dm), b3dm payloads, raw roots with and without translation; a δ = 0 tile with a −0.0 box centre (row and payload copied bit for bit) and an affine-`matrix` root payload | 64 / 7 / 1 / 0 / 0 |
| `quad-wide` | The **node budget** (heavy skeleton extras): 97 documents, links two levels deep | 4,095 / 12 / 97 / 96 / 2 |
| `quad-bytes` | The **byte budget** (300 KB string extras): documents up to 1.81 MB | 255 / 8 / 8 / 7 / 1 |
| `quad-chain32` | The **32-level bound** and **`DOC_LEVELS`** = 10: a 28-tile LOD chain above a height-3 root gives L = 32, 3 documents and a link two levels deep. The JSON depth is 26 | 43 / 32 / 3 / 2 / 2 |

**Negatives.** There are 57, each violating a named rule. A run agrees when every target rule has
status `fail` and the verdict is `fail` (or `pass` if the only targets are advisory). Controls that
do not need the reference are run both with and without it. Without the reference, `A4.*` targets
are not judged, because those rules apply only with `--explicit`.
- **A-2's ten controls:**
  - `neg-json-template`;
  - `neg-child-subtree-bit`;
  - `neg-missing-row`;
  - `neg-row-1ulp` (A4.rows) and `neg-row0-1ulp` (P3, without a reference);
  - `neg-repeated-prefix`;
  - `neg-unreferenced-member`;
  - `neg-retained-source`, which is both PD-11 and P4;
  - `neg-doc-node-budget`;
  - `neg-link-box`;
  - `neg-missing-root-schema`, the archive analogue of the Cesium `N` control.
- **The other rules:** `neg-truncated-archive`, `neg-source-list`, `neg-doc-schema`, `neg-schema-id`, `neg-skeleton-s3`,
  `neg-skeleton-s5` and `neg-leaf-as-root` (which also target `A4.roles`), `neg-levels-mismatch`, `neg-two-subtrees`,
  `neg-orphan-availability`, `neg-l33`, `neg-doc-bytes`, `neg-doc-depth`, `neg-doc-levels` (11 skeleton levels), `neg-doc-pretty` (a pretty-printed `tileset.json`),
  `neg-link-refine`, `neg-link-children`, `neg-link-dangling`, `neg-link-twice`,
  `neg-doc-root-transform`, `neg-doc-numbering`, `neg-link-order` (advisory),
  `neg-old-chunk-document`, `neg-tile-child-index`, `neg-row-child-index`, `neg-extra-layer`,
  `neg-slots-unsorted`, `neg-missing-content`, `neg-unavailable-content`, `neg-corrupt-payload`,
  `neg-vector-build-hash` and `neg-reuse-record`.
- **A-4 controls (need `--explicit`):**
  - `neg-reuse-missing-entry`;
  - `neg-error-changed`;
  - `neg-row-extras`;
  - `neg-tile-dropped`;
  - `neg-payload-bin`;
  - `neg-payload-translation`, a 1-ulp error in the baked translation;
  - `neg-payload-reserialized`, today's whole-document re-serialization of an unsorted-key
    (meshopt-like) GLB;
  - `neg-explicit-payload`;
  - `neg-prefix-not-preorder`;
  - `neg-root-refine`;
  - `neg-root-transform-dropped`;
  - `neg-row-signed-zero`: a δ = 0 row computed as B + 0.0, which turns −0.0 into +0.0;
  - `neg-zero-delta-rebased`: a δ = 0 payload rewritten instead of copied;
  - `neg-matrix-as-translation`: δ written to `translation` beside the raw `matrix`.

**Recorded result** (`results/controls.json`): **111 of 111 runs agree**. That is 6 positives × 2
modes, plus 57 negatives (42 run in both modes and 15 with the reference only). The negatives
target 26 of the 28 rules: every required rule, plus the advisory `links.creationOrder`. The two
untargeted rules are the advisory `A4.contentUris` and `A4.topLevel`. Each run lists its
collateral failures.

### Cross-checks against the design's independent model

The design's v3 `forest_model.py` writes forests from the real `--explicit` archives. These are
not the production writer. Results are in `results/model-*.verdict.json`.
- **The statistics match design §5.1 exactly:**
  - mf2-lod16: 291 documents, max 16,294 nodes, 198,125 bytes, JSON depth 31, 22,197 visits;
  - mf2-lod3: 189 documents and 14,321 visits;
  - vec-perf: 1 document, 4,955 nodes and 594 visits;
  - pc-deep: 258 nodes, 4,021 bytes and 2,243 visits.
- **`pc-deep` passes every rule** (archive-only, because there is no matching explicit build).
- **mf2-lod16 now also fails `documents.levels`** (318 tiles), because the model packs at the old
  `DOC_LEVELS` of 12.
- **The vector models fail only where the model predates v3:**
  - `A4.extras` on `encodedBytes`: the model keeps the raw value, but §4.5 requires the baked
    length;
  - `P4.namesClosed`: the model writes no `vector-build.json`, yet keeps the hash in `asset.extras`;
  - `A4.payloads` splice failures: 20 payloads on mf2-lod3 and 18 on mf2-lod16, because the model
    re-serializes the whole JSON chunk. Python prints `-5.208847142057493e-05` where the raw text
    has `-0.00005208847142057493`. The v3 writer must avoid exactly this (§4.1, §5.4).
- **The develop archives fail as expected** (`results/develop-*.verdict.json`). Their JSON
  templates, child subtrees, retained sources and `implicit-tileset-*` documents all fail.
- **Develop's point-cloud `tileset.json` is pretty-printed today**, so it fails
  `documents.compact`. PP's output must be compact.

## Cesium probes (A-5)

Pins:
- Cesium 1.146.0;
- Playwright 1.63.0;
- Chromium 153.0.8010.12 (`~/.cache/rusty-tiles-137-playwright/chromium-1243/chrome-linux64/chrome`) on SwiftShader;
- Node 26.10.0;
- Python 3.14.7.

The path defaults are the perf lane's caches. Override them with `CESIUM_DIR`, `NODE_PATH` and
`CHROMIUM`. Run one browser at a time, under `taskset -c A,B nice -n 10`, on a pair whose busy
share is low (`cpustat.py snap` and `diff` around each run, siblings included). The recorded runs
used CPUs 13 and 15, the SMT threads of P-cores 6 and 7.

**A-5.1, fixture suite** (run from this directory):

```sh
python3 -I cesium/make_fixtures.py $W/cesium-fx
taskset -c 13,15 nice -n 10 python3 -I cesium/run.py $W/cesium-fx $W/fixtures-results.json
python3 -I cesium/judge_fixtures.py $W/fixtures-results.json
```

Frozen expectations:
- E, S, X, F and D select and pick 8/8, and their in-cell control passes;
- A selects 0/8, which is the placeholder cull;
- N fails with "Cannot read properties of undefined (reading 'classes')" and selects 0/8.

Recorded in `results/cesium-fixtures-results.json` and `.judge.json`: **pass** (CPUs 13 and 15 at about 11% busy; siblings ≤ 4%). Every variant's fixture tree digest and every
view's selection and pick are identical to the design's `fixtures-results.json`.

**A-5.2, deep-view A/B on real archives.** The first case is the baseline (develop) and the second
is the combined head:

```sh
P=bench/architecture_audit/implicit_layout/cesium; R=$W/real-<case>
python3 -I $P/prepare_real.py $R --base explicit explicit=<head --explicit>.3tz develop=<develop default>.3tz head=<head default>.3tz
export PIMPL_DETERMINISTIC=1 PIMPL_STABLE=30 PIMPL_MSSE=0.0001   # pc-deep: PIMPL_MSSE=1
for b in 0 1; do
  python3 -I $P/cpustat.py snap > $R/cb$b.cpu0
  PIMPL_CHILD_BOUNDS=$b taskset -c 13,15 nice -n 10 python3 -I $P/run_real.py $R/views.json $R/res-cb$b.json \
    develop=$R/develop:develop head=$R/head:head > $R/cb$b.log 2>&1
  python3 -I $P/cpustat.py diff $R/cb$b.cpu0 > $R/cb$b.cpuload
done
python3 -I $P/judge_deep.py --cb0 $R/res-cb0.json --cb1 $R/res-cb1.json --parents $R/parents.json --errors $R/errors.json
python3 -I $P/compare_runs.py $R/res-cb1.json $R/parents.json      # the design's table columns
```

How `prepare_real.py` works:
- **Content keys.** It maps every served content URI of every layout to a content key: sha256 of
  the BIN chunks plus the parsed glTF JSON without the scene-root translation or matrix.
- **Why that key.** The layouts differ only in that translation (A-4). So one producer tile has the
  same key in develop, head and explicit (544 of 544 shared on vec-perf), and no
  layout-specific map is needed.
- **Views.** Each view targets a real vertex of a base-layout leaf (R-9c). On vec-perf, a target
  computed from each layout's own payload and offsets agrees within 9e-13 m.
- **Limits.** Only unquantized, uncompressed POSITION accessors can serve as targets. Run views on
  default (not `--quantize`/`--meshopt`) archives, as the design did.

The requirements (`judge_deep.py`, frozen in `expectations.json`):
- **R1.** Everything loads, with no tile failures and no failed requests.
- **R2.** The same target-path tile in every view.
- **R3.** The same non-null picks. A null pick on one side only is a failure; null on both sides is
  reported apart and never counts as agreement.
- **R4.** With `cullWithChildrenBounds: false`, identical selections.
- **R5.** With defaults, a differing view must have the head selecting a strict ancestor of a
  baseline-selected tile, never the reverse, with equal pixel coverage. The share is reported.

The cases are vector perf, mf2-lod3, mf2-lod16, `pc-deep` and the mesh chunk case.

**Smoke run of the pipeline** (`results/deep-smoke-vec-perf*.json`): develop vec-perf against the
design's v3 model forest, standing in for a head archive. See "Recorded smoke" below.

## Running against the combined branch (for the coordinator)

1. **Checker, for every W11 / A-3 head archive and every P output with an `--explicit` build of the
   same input:**
   ```sh
   python3 -I bench/architecture_audit/implicit_layout/forest_check.py HEAD.3tz --explicit HEAD-explicit.3tz --out HEAD.verdict.json
   ```
   - **Expected:**
     - `verdict: pass`, with every required rule `pass` (or `not-applicable`);
     - `links.creationOrder` and `A4.topLevel` pass;
     - `A4.contentUris` may fail only where the two modes name payloads differently. Report it
       either way.
   - For vector archives, `vectorBuild.reuseMap` must be applicable and pass.
   - Without an `--explicit` build, run archive-only and report that A-4 was not applied.
2. **Fixture suite and deep views** as above, one browser at a time.
3. **Determinism (P7).** Build the same input twice with `--reproducible` and `cmp` the archives.
   Leave out the volatile `conversion.json` fields as A-8 does.

## Rulings and remaining interpretations

The coordinator ruled on PO's questions (refreeze 1):

| Question | Ruling, as checked |
| --- | --- |
| Q1 signed zero | A row with δ = 0 is B copied bit for bit, so −0.0 stays −0.0. Other non-root rows are B + δ in f64 |
| Q2 `{slot}` | Unchanged. The content index or the slot-list position is accepted, with strictly sorted, distinct `(slot, suffix)` pairs |
| Q3 sidecars | Accepted. `conversion.json` and `geometry-reports.jsonl` may be unreferenced |
| Q4 `DOC_LEVELS` | Now 10. Skeleton tiles ≤ 10 levels below the document root; links and unit children ≤ 11. The measured worst case is JSON depth 28 against 32 |
| Q5 creation order | Unchanged. Breadth-first numbering across documents is advisory |
| Q6 matrix roots | An affine `matrix` takes δ in `[12..14]`. A matrix with TRS, a non-affine matrix, or several scene roots is refused, and the checker fails it |
| Q7 headers | Disagreeing headers within one root's slot are a typed writer error, so the checker fails them (`A4.explicitTiles`) |
| Q8 `encodedBytes` | The sum of the published bytes of the tile's contents |
| Q9 reuse map | `payloads` with `{member, delta, rootNode, jsonLength}`. A δ = 0 payload is a byte copy whose record is `rootNode: ""`, `jsonLength: 0`, and its restore returns the bytes unchanged |
| Compactness | Every tileset JSON document must be compact JSON, and the budgets are measured on compact bytes. PO applies this to linked documents as well as `tileset.json` |

Remaining PO readings:
- The JSON keys of the reuse map are camelCase as in §4.7 (`rootNode`, `jsonLength`). Snake-case
  keys would fail `vectorBuild.reuseMap`.
- The document root's `refine` is required (core requires it on a tileset root) and must equal
  the link's.
- Absent and empty extras are treated as equal.
- `content` or `contents` is accepted for a single slot.
- Extras on an implicit-root tile's JSON are optional, but must match H when present.

## Freeze

- The checks were frozen at this directory's first commit on `p/oracle` (`08a9c36`) and refrozen
  after the coordinator's rulings (refreeze 1).
- `SHA256SUMS` pins every file, and `expectations.json` holds every expected outcome.
- Later changes must be new commits that state the reason. None may follow from reading the writer
  diff unless it is recorded as such.

## Recorded smoke

`results/deep-smoke-vec-perf*`:
- **Setup.**
  - Develop's `vec-perf.3tz` (`3fae0aca…`) is the baseline. The head stand-in is the design's v3
    model forest of `vec-perf-explicit.3tz` (`dd22037a…`).
  - Views come from the explicit build.
  - Deterministic, 30 stable frames, MSSE 1e-4.
  - CPUs 13 and 15 at nice 10: busy 10–12%; siblings 12 and 14 at ≤ 5%.
- **`judge_deep.py`: pass.**
  - cb0: 16/16 identical selections.
  - cb1: 14/16 identical. Both differing views (12.5%) have the head selecting an ancestor, with
    equal coverage.
  - The target-path tile is the same in 16/16 views in both runs.
  - 2 non-null picks are equal, and 14 views are null on both sides.
  - Requests are 55/55/218 (JSON/subtree/GLB) against 1/45/218.
- **This reproduces the design's §5.2 vec-perf rows exactly**, through the new layout-independent
  maps.
- **This is not evidence about the production writer.** It shows that the pipeline runs
  end to end on real archives.
- The views, parents and errors are stored gzipped. `prepare.json` records the archives' sha256.
