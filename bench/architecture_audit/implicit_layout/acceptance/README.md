# P bounded implicit forest: acceptance (R-helper, combined head e3a67bf)

These runs execute PO's frozen checker and probes, the perf lane and the A-8 reuse timing on real
outputs of the combined head `feat/implicit-forest` at `e3a67bf`. No production code was changed.

**Evidence binding (P final review F-4).** Every acceptance binary below was built at `e3a67bf`,
and every archive in this record comes from those binaries. After `e3a67bf`, P changed in only two
places:
- **`tests/fixtures/implicit_levels.cjs`** (`1ef4edb`). The probe was re-run 3/3 green with the
  e3a67bf archives (§5).
- **The P-review F-1 guard in `restore_payload`** (`src/implicit/tileset.rs`, `14dddd6`). It
  refuses an oversized recorded JSON length.
  - `implicit/tileset.rs` is part of the vector encoder hash, so the guard also changes vector
    archives.
  - All 36 producer runs were repeated with a binary built from `b51f83d`, the same compiled
    source as `28042c6` (`60584d3d…`), and compared member by member with the e3a67bf outputs
    (`binding-e3a67bf-to-b51f83d.json`).
  - **Point-cloud, mesh, glb-wrapper and `--noMeshopt` outputs are byte-identical.**
  - Every vector archive (default and `--explicit`) differs only in:
    - `tileset.json` `asset.extras.vectorBuildStateSha256`;
    - `vector-build.json` `config.encoder`;
    - its per-record `signature`s.
  - Layout, documents, subtrees and payloads are byte-identical, so every verdict below holds for
    the current head.

The other post-e3a67bf source changes are I1's (`walk.rs`, and the `lib.rs` test module).
The binaries, inputs and producer commands are shared with the I1 record
(`../../implicit_read/acceptance/README.md`, `producer-runs.json`):

| Binary | sha256 |
| --- | --- |
| Head, release `native-geospatial` | `a734eac596a2a26031a48b94e85e1fe2a4e38f7856c81e5cfdaf4a7aa327d6d0` |
| Head, release portable | `1ace7782ad6fb6ec0ad3d96fb5971e7ae43da7ccf3f7ca3e3863ef523cff71be` |
| Develop `766b92b`, release `native-geospatial` | `2a7c4ba6cfc21dedd595cecf92d01d7f8161c3442ca1057e12c9107a78230cc3` |

| Script | sha256 |
| --- | --- |
| `forest_check.py` | `76dbf281…a5f2` |
| `cesium/prepare_real.py` / `run_real.py` / `judge_deep.py` | `619cf351…66d4` / `dce0e36d…9d14` / `a8c02bac…f95` |
| `cesium/make_fixtures.py` / `run.py` / `judge_fixtures.py` / `probe.cjs` | `578884d8…ee63` / `6a3c3935…5cd` / `c4697828…e3dc` / `fb2d0fdb…d5fe` |
| `bench/perf_lane/process.py` / `cesium.py` / `cesium.cjs` / `corpus.py` | `0e734d91…1084` / `a4ff2697…8b8e` / `b2b67479…71ce` / `a6cd738c…5245` |
| `tests/fixtures/implicit_levels.cjs` | `3cba880e…ed1` at e3a67bf; `bb5ce1f6…f95b` after the follow-up fix |

Pins: Cesium 1.146.0, Chromium 1243 (SwiftShader) and Node 26.10.0. Browsers ran one at a time
at `nice -n 10`.

## Summary

| Item | Result |
| --- | --- |
| P forest checker, every forest output | **PASS**, with the reference limits below |
| P7 determinism | **PASS**: 34/34 archives `cmp`-identical across two builds |
| Perf lane process A/B | **PASS**: no gate flag and no `--explain` used |
| Perf lane Cesium, base against head | **PASS**: 0 Cesium errors and no head gate problems |
| Deep views (A-5.2), four cases × child-bounds 0 and 1 | **PASS** |
| Fixture suite (A-5.1) | **PASS** |
| `implicit_levels.cjs --require-documents` (A-5.3) | Failed as committed at e3a67bf (probe defect, archive sound). **PASS** 3/3 on 2 CPUs after the follow-up probe fix |
| A-8 reuse (mf2-lod3, small edit) | **PASS** on timing and on the corrected guarantees. Reuse = fresh does not hold at mf2-lod3 scale, which predates P; the contract clause is corrected |

## 3. Forest checker and P7

Results are in `forest-check/*.verdict.json`.

- **Vector, with `--explicit` as H.** Every required rule passes on all 19 cases:
  - W4 ×6;
  - vec-perf default, `--quantize` and `--meshopt`;
  - mf2-lod3 (12,530 tiles, 189 documents, 188 links, 1,225 roots);
  - the portable W4/W5 builds (9).

  `vectorBuild.reuseMap` is applicable and passes everywhere, so every payload-map record
  restores bytes to its sha256 name. The advisory `links.creationOrder`, `A4.topLevel` and
  `A4.contentUris` also pass.
- **Point cloud (W1, W2, pc-deep) and legacy mesh (W6, W6d).** Archive-only: every rule passes.
  A-4 could not be applied, because their `--explicit` builds are a different hierarchy, not H:
  - W1 has 63 explicit tiles against 73 in the forest; pc-deep 1,443 against 2,233; W6 25 against
    73; W6d 233 against 314.
  - The point-cloud explicit build is the legacy partition, and the same holds at f2238cd (63 and
    1,443). Mesh `--explicit` is documented as "median partitioning".
  - With the `--explicit` reference the A4 rules fail as a consequence (`*.verdict.json`
    without `.archive-only`). This is a proof limit, not a writer defect.
  - Coordinator ruling: A-4 is inapplicable to point cloud and mesh, and only the archive-only
    checks apply. This is now recorded in the P contract's A-4 "Domain" note.
  - W7 (M2 wrapper) passes with its `--explicit` reference, A4 included.
- **`glb-to-3tz`** is not a forest producer: it writes one explicit tile, byte-identical to
  f2238cd. It fails `skeleton.S4` by construction and is out of P's scope.
- **Size (A-8).** The vec-perf default is 19,819,944 B against `--explicit`'s 19,447,030 B, which
  is 1.019× (≤ 1.1). mf2-lod3 is 83.6 MB against 74.6 MB (1.12×), but that case is not in the
  criterion. Develop's defaults were 37.8 MB and 149.3 MB.
- **P7 (`determinism-p7.json`).** All 34 archives from two head builds are byte-identical,
  `conversion.json` included. Point cloud has no `--reproducible` flag and is identical anyway.
- **Explicit outputs keep their layout and payloads, but their files change.**
  - Point-cloud `--explicit` `tileset.json` is now compact (pc-deep 2.93 MB → 0.42 MB), with
    identical parsed JSON and identical payloads.
  - Vector `--explicit` archives change in `tileset.json` (state hash) and `vector-build.json`:
    encoder revision and the empty `payloads` map.
  - Mesh `--explicit` is byte-identical to develop.

## 4. Perf lane (`perf/`)

`process.py --base develop --head head --repeat 3 --cpus 0,1` (ABBA) ran with CPUs 0 and 1 at
3% and 1% busy beforehand. Foreign CPU stayed ≤ 0.12 s per sample. **`gate.regressions = []`,
`explained = []`.**

| Case | Base best wall / RSS / bytes | Head best wall / RSS / bytes |
| --- | --- | --- |
| vector | 3.472 s / 89.9 MiB / 37,111,803 | 3.468 s / 92.5 MiB / **19,461,861** |
| validate:vector | 0.160 s / 88.3 MiB | **0.100 s** / 87.0 MiB |
| vector-explicit | 3.401 s / 85.8 MiB / 19,089,024 | 3.401 s / 85.5 MiB / 19,089,046 |
| validate:vector-explicit | 1.250 s | 1.215 s |
| point-cloud | 0.127 s / 46.6 MiB / 19,553,112 | 0.135 s / 46.9 MiB / 19,552,256 |
| validate:point-cloud | 0.058 s | 0.058 s |
| point-cloud-explicit | 0.164 s / 20,634,718 | 0.164 s / 20,627,754 |
| mesh / validate:mesh | 0.234 s / 0.044 s | 0.237 s / 0.044 s |
| mesh-local, glb-wrap, mesh-explicit, terrain, terrain-fixture, raster | within ±2% wall; identical bytes | |

The archive shrinks and the I1 reader makes validation faster or equal, so no case needed an
explanation. The I1 contract's own perf obligation (base = combined head with the I1 validator
commits reverted, plus the amplified ≥ 0.4 s point-cloud case) was **not** run. These runs use
develop as base, per the coordinator.

**Cesium** (`cesium.py`) covered the lane cases plus mf2-lod3 and pc-deep, in the order base1,
head1, head2, base2, with `--baseline base1` and CPUs 0,1 quiet beforehand:

| Case | Base s (1 / 2) | Head s (1 / 2) | Requests base → head | Bytes base → head |
| --- | --- | --- | --- | --- |
| vector | 2.23 / 2.19 | 2.13 / 2.10 | 6 → 5 | 1,563,790 → 1,572,286 |
| point-cloud | 2.83 / 2.70 | 3.28 / 3.01 | 11 → 11 | about equal |
| mesh | 2.52 / 3.03 | 2.53 / 2.48 | 11 → 11 | about equal |
| mf2-lod3 | 2.15 / 2.11 | 2.14 / 2.18 | 6 → 5 | 2,573,618 → 1,833,961 |
| pc-deep | 21.88 / 27.99 | 26.85 / 26.34 | 2,649 → 2,242 | 12,892,981 → 12,542,042 |
| terrain, raster, explicit variants | equal within noise | | equal | equal |

- No case had a load failure, console or page error, failed or external request, or probe failure.
- head1 and head2 exit 0 with no gate problems.
- The only flag in the session is base2 against base1 on pc-deep: 21.9 → 28.0 s, base against
  itself. That spread brackets head's 26.3–26.9 s, so pc-deep load time is not judged.
- Point cloud is +0.3–0.45 s, below the 1 s floor.

## 5. Cesium deep views and fixtures

**Deep views** (`deep/<case>/judge.json`; `PIMPL_DETERMINISTIC=1`, `PIMPL_STABLE=30`, MSSE 1e-4 or 1
for pc-deep). Each case ran develop default against the head default on CPUs 13 and 15, whose
busy share was 11–24% during the runs. Siblings 12 and 14 stayed ≤ 2%.

| Case (view base) | Target path same | cb0 same selection | cb1: same / head ancestor | JSON / subtree / GLB requests (develop → head) | `judge_deep` |
| --- | --- | --- | --- | --- | --- |
| vec-perf (explicit) | 16/16 | 16/16 | 14 / 2 (12.5%) | 55/55/218 → 1/45/218 | pass |
| mf2-lod3 (explicit) | 16/16 | 16/16 | 3 / 13 (81%) | 151/151/518 → 45/79/518 | pass |
| mesh-deep (develop; W6d `--noMeshopt`) | 16/16 | 16/16 | 16 / 0 | 43/43/314 → 1/8/314 | pass |
| pc-deep (develop) | 16/16 | 16/16 | 16 / 0 | 208/208/2209 → 1/8/2209 | pass |

- In vec-perf and mf2-lod3, every non-null pick equals its counterpart and the null picks are
  null on both sides. The vec-perf and mf2-lod3 request splits and ancestor shares reproduce the
  design's §5.2 rows.
- Mesh outputs are meshopt-compressed by default, so `prepare_real.py` cannot place targets
  (0 views). The mesh case therefore used `--noMeshopt` builds of the W6d recipe:
  - develop `a699f7a8…`, head `b701588a…`;
  - the head build also passes the checker.
- The `--noMeshopt` producer argv and hashes are in `extra-producers.json`; the e3a67bf and b51f83d
  heads produce the same bytes.
- The meshopt default itself is covered by the checker and by I1 only. The use of `--noMeshopt`
  builds for the mesh case is now recorded in the P contract's A-5.2.

**Fixture suite** (`cesium-fixtures/`): **pass**.
- E, S, X, F and D select and pick 8/8, and their in-cell controls pass.
- A selects 0/8.
- N fails with "Cannot read properties of undefined (reading 'classes')".

**`implicit_levels.cjs --require-documents` (`implicit-levels/runs.json`): FAIL as committed.**

The fixtures follow CONTRIBUTING's recipe on the head binary:
- a 24×24 textured grid at `--cartographic-position-degrees 12.1 41.9 200` with
  `--max-triangles 2 --max-bytes 0 --tile-size 64`: 1,191 tiles, 43 roots;
- `p1_point_oracle.py --variant deep` with `--source-crs local --max-points 16`: 61 tiles, 21
  levels, 1 link, `tileset-d1.json`, 1 root.

Both archives pass `forest_check.py` (`implicit-levels/{mesh,cloud}.forest-check.verdict.json`) and `validate`. The fixture argv and hashes are in `extra-producers.json`. They were served by `rusty-tiles preview`
with an unpacked head vector output as the annotations layer, which the probe only hides.

1. On CPUs 13 and 15 the point cloud did not settle within the probe's window (SwiftShader on 2
   CPUs).
2. On CPUs 12–15, twice, the mesh passes (levels 0–5, 43 nested roots, fine selection all
   leaves). The probe then asserts "point-cloud did not cross an implicit-root boundary": Cesium
   selects only level 1 and instantiates levels 0–11, with no document and no root.
3. The cause: the deep chain sits in a 4 mm box (errors 0.0035 m) inside a 2 km dataset. At the
   probe's fine framing (3× the root radius) its screen-space error is about 1e-3 px, and Cesium's
   default `dynamicScreenSpaceError` subtracts its fog term. So nothing refines past level 1,
   even at MSSE 1e-7.
4. With one change, `dynamicScreenSpaceError=false` for the point cloud only
   (`implicit_levels_diag.patch`), the probe **passes**:
   - point cloud: levels 0–20, the document and the implicit root crossed, 44 implicit tiles,
     fine selection all leaves, a pick with LAS properties;
   - mesh unchanged;
   - no browser errors.

The archives are sound; the defect was in PP's probe.

**Follow-up (coordinator ruling): probe fixed and green.** `tests/fixtures/implicit_levels.cjs`
now makes two changes:
- it sets `dynamicScreenSpaceError=false` on the point cloud, with a comment explaining why;
- settling requires `tilesLoaded` on 10 consecutive rendered frames within 180 s.

The second change is needed because a longer timeout alone was not enough. The chain refines
one level per load, so `tilesLoaded` is briefly true between levels: a single sample stopped
early ("did not settle" after 13 s). The mesh's fine view could also exceed the old 30 s window
on 2 CPUs.

Re-run on CPUs 13 and 15 (2 CPUs), with the same archives and `--require-documents`: **3 of 3
pass**, in 17–18 s each, with identical results and no browser errors (`runs.json`
`fixedProbe`):
- mesh: levels 0–5, 43 nested roots, 1,222 implicit tiles; fine selection is all leaves at
  levels 2–5, and a pick succeeds;
- point cloud: levels 0–20, 1 document, 1 implicit root, 44 implicit tiles; the fine selection
  is all leaves, and a pick returns the LAS properties.

During the runs CPUs 13 and 15 were 23–29% busy. CPUs 0 and 1 carried 74–83% foreign load
(not this run). `implicit_levels_diag.patch` is the earlier diagnostic, kept for the record.

## 6. A-8 reuse timing (`reuse/reuse-ab.json`, `scripts/reuse_ab.py`)

Setup:
- mf2-lod3 (`--maxFeatures 2 --lodLevels 3 --reproducible --jobs 2`);
- the base is perf `vector.geojson` (`4f11f4ca…`), the edit is the design's small edit
  (`a59d29b0…`; 23 edited features);
- each binary reuses its own base build;
- 3 rounds, binary order ABBA, fresh/reuse order alternating, CPUs 0 and 1.

| | Fresh wall best (median) | Reuse wall best (median) | Reuse CPU s | Reused / rebuilt |
| --- | --- | --- | --- | --- |
| Develop | 21.81 (22.05) | 6.49 (10.20) | 5.72–6.35 | 12,469 / 62 |
| Head | 21.04 (37.33) | 6.65 (9.15) | 4.51–4.94 | 12,469 / 62 |

- **Head reuse ≤ 1.25× develop reuse plus 0.1 s:** 6.65 against 6.49 (1.025×): pass.
- **Head reuse < head fresh:** pass.
- Head reuse uses about 20% less CPU than develop's.
- The reused/rebuilt counts equal the design §5.4 row.
- In rounds 0 and 1, wall time is about 2× CPU time on several runs (I/O stalls on 80–150 MB
  archives, while the deep-view browser ran on 13 and 15). Best-of-3 is therefore the meaningful
  wall figure.

**Finding (pre-existing, recorded for the coordinator).** A-8's integration clause says the
reuse output is "byte-identical to a fresh v3 build of the edited input". On this input:
- head reuse differs from head fresh in 11,311 members; develop's reuse differs from its fresh
  build in 55;
- in both, reuse publishes **12,531** contents over **12,534** tiles, against fresh's 12,527 over
  12,530;
- the reused hierarchy H keeps 4 more tiles than a fresh partition of the edited input. That is
  existing reuse behaviour, present on develop.

P's preorder root names (`r{n}`) turn those 4 tiles into thousands of renamed members. Both reuse
outputs pass `forest_check.py` and `validate` (12,722 tiles). The repository test
(`tests/vector_reuse_payloads.rs`, `--lodLevels 2`) does not exercise this case.

**Follow-up (coordinator ruling): the contract clause is corrected.** The guarantees now stated
were executed on the head (`reuse/reuse-guarantees.json`, `scripts/reuse_guarantees.py`).
Comparisons exclude `conversion.json`, `geometry-reports.jsonl` and the 3TZ index, the PV test's
volatile set:

| Guarantee | perf-default | mf2-lod3 |
| --- | --- | --- |
| Unchanged input: reuse = baseline | 0 differing members (544 reused, 0 rebuilt) | 0 differing members (12,527 reused, 0 rebuilt) |
| Small edit: reuse = fresh | 0 differing members (534 reused, 10 rebuilt) | not guaranteed: 11,308 differing members (12,469 reused, 62 rebuilt, 12,531 published) |
| Byte-exact payload recovery | every reused payload passed reuse's sha256 check; `vectorBuild.reuseMap` passes on every vector forest | same |
| Valid archive | yes | yes: `forest_check.py` and `validate` pass |

## Not established here

- A-4 equivalence for point cloud and mesh (no matching H).
- mf2-lod16 and the mesh chunk case in the Cesium perf ABBA.
- The I1 amplified perf case.
- Medium-edit and ids-mixed reuse configurations, and perf-default reuse.
- GPU behaviour.
- Release readiness.
