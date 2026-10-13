# C2 decoded-content containment: O-explicit oracle (Slice 0a)

> **Bulk evidence moved out of the tree (2026-10-12).** 386 of 548 files remain:
> this README, what CI or tests read, live tooling (oracle, witness generators,
> self-tests, mutant sets) and the C2-WB (#164) additions. The full directory
> last existed at
> `e29cd19d2f236dc688e91a80a45170d4a573e20b`
> (`git archive e29cd19d2f23 bench/architecture_audit/c2_decoded_containment | tar -x`). Local archive:
> `~/.cache/rusty-tiles-evidence/architecture_audit/c2_decoded_containment.tar.zst`, SHA-256
> `9174fd77147b1abcb79cc388ffc371e67ef68c7b3a38bde7af3edf0e834cd25c`.
> Paths below that no longer exist refer to that snapshot.

This directory is the independent reference for the C2 slice-1 claim. When
`checks` contains `decodedContentContainment`, every vertex drawn by the
selected static scene must lie inside these volumes, each in its own frame:

- its tile volume;
- its content volume;
- every external-reference content volume on the way up;
- every ancestor tile volume.

A vertex is placed in those frames after the node transforms, the glTF y-up to
z-up step `U` and the tile transforms.

Role O wrote it from the frozen contract and the primary specs only. It does
not read `src/`, `tests/` or `bindings/`, does not run rusty-tiles, and imports
nothing from the repository. It is pure Python 3 standard library.

| Path | What it is |
| --- | --- |
| `oracle/c2_oracle.py` | CLI: `python3 -I oracle/c2_oracle.py ARCHIVE.3tz --json [--limit containmentSpanBits=29 ...]` |
| `oracle/c2o/` | `numbers` (exact JSON numbers, f32/f64 to Fraction), `meshopt` (NONE-filter decoder), `gltf` (GLB/glTF/b3dm, accessors), `scene` (admission, selection, walk), `spans` (normative span definition), `volumes` (box/sphere predicates), `tileset` (3TZ, traversal, levels), `check` (precedence, domain, precheck, streaming) |
| `oracle/self_test/` | `run.py` plus `meshopt_vectors.json` (pinned upstream golden vectors and spec examples) |
| `oracle/mutants/` | The 12 contract implementation mutants as hashed patches against the oracle, with the fixtures each flips |
| `witnesses/` | `generate.py` and `build.py` (fixture writer, independent of `c2o`), pinned `inputs/`, generated `fixtures/` and `report.json` |
| `controls/` | `manifest.json` with base/mutated fixture pairs, hashes and expected verdicts; `fixtures/`. `large/` is generated, not committed |
| `corpus/` | `run.py` and `manifest.json`: the oracle outcome for each artifact of R's producer corpus |
| `wb/` | C2-WB (O-WB): the calibration-matrix class fixtures, per-term identities, chain-bit controls, the independent hand formula and the formula mutants. See [wb/README.md](wb/README.md) |
| `h11/` | H11 corner fast path: `generate.py` (fixtures through `witnesses/build.py`, verdicts from the unmodified oracle), `fixtures/` (14) and `report.json`. See [H11](#h11-corner-fast-path-2026-10-13) |

## Replay

From this directory:

```sh
python3 -I oracle/self_test/run.py            # 59 checks, exit 0
python3 -I witnesses/generate.py --large      # 261 cases, 16 control pairs; exit 0 when all agree
python3 -I oracle/mutants/run.py              # 13 mutants, each must flip its named fixture
python3 -I corpus/run.py CORPUS_DIR           # CORPUS_DIR = R's run2 directory (manifest.json + artifacts/)
python3 -I wb/generate.py [--large]           # C2-WB class fixtures (wb/README.md)
python3 -I wb/mutants.py                      # 14 C2-WB formula mutants, each must flip its named pair
python3 -I oracle/c2_oracle.py some.3tz --json [--precheck-only] [--no-detail]
```

All runs in this record were single-threaded under `taskset -c 0,1 nice -n 10`.

Without `--large`, the generator skips the 24 MB repeated-index control and its
pair. Those fixtures are written as stored ZIPs, so their hash is reproducible
and does not depend on zlib:
`57c359962778525bb60a583382fafa5912edc436008cc21ddf490a38f64cb763`.
The full generator, including `--large`, runs in about a second
single-threaded. The corpus run takes a few seconds. Tested with
CPython 3.14.7.

## Oracle outcomes

| `outcome` | Meaning |
| --- | --- |
| `Contained` | Outcome 5. Every drawn vertex is inside every tested volume, as an exact rational. |
| `Outside` | Outcome 4. `facts` names the first proven escape in streaming order: `reference`, `uri`, `member`, `depth` (tree depth of the tile whose content is referenced: root 0, an external root its referencing tile's depth + 1; v3.3 (5)), `level`, `role` (`own`, `content`, `externalContent` or `ancestor`), `k` (the level index, per reading v3.2 (3)), `node`, `mesh`, `primitive` (lowest primitive index whose draw references the vertex), `accessor`, `vertex` (POSITION element index) and the exact coordinate. |
| `NotInspected` | Outcome 2: every exclusion reason, sorted, from the frozen 15-name enum. Or outcome 3: one precheck reason, `numericSpan` or `workLimit`, with `precheckStop`. |
| `InvalidNode` | InvalidInput for a malformed consumed scene/node record, including duplicates in `children` or a scene's `nodes` (v3.3 (8)). This is the first payload in traversal order whose records fail admission. |
| `InvalidInput` | InvalidInput for a tileset defect met in traversal order (v3.3 (8)): a non-affine or non-finite tile `transform`, or a tileset number literal that overflows binary64. Payloads referenced earlier in traversal order are admitted first. |
| `Undecided` | The oracle could not decide. Causes: one of its own limits ran out; the input has a format it does not model (`pnts`, `i3dm`, `cmpt`); or the data is something C1 should reject. This is **never a verdict**. |

`c1Caveats` lists conditions under which production may stop in C1 before C2.
It covers three cases from the contract text:

- a non-allowlisted `extensionsRequired` entry;
- `JOINTS_n`/`WEIGHTS_n` attributes;
- undeclared extension objects.

`caveats` lists what the oracle did not examine, for example implicit
subtrees. `precheck` reports, per reference, `sInst`, `levelSpans`, the units,
`bytes`, `decoded` and `maxNodeDepth`, plus the running totals.

### Default C2 caps

The oracle's defaults mirror `ValidationLimits`:

| Limit | Default |
| --- | --- |
| `containmentSpanBits` | 4096 |
| `containmentWorkUnits` | 2^29 = 536,870,912 (C2-WB WB-6, from the measured class matrix; 2^28 from PR #157 until C2-WB, 2^32 before that) |
| `containmentBytesRead` | 2 GiB |
| `containmentDecodedBytes` | 2 GiB |
| `containmentNodeDepth` | 64 |

Override any of them with `--limit NAME=VALUE`.

The lower work-units default changes no expectation. Under the C2-WB formula
(2026-10-12) the largest archive totals are 48,008,577 units among the witnesses
and controls (the repeated-index control) and 17,492,428 in the corpus (point
cloud explicit); no witness, control or corpus verdict changes. The default cap
itself is R's decision (C2-WB WB-6); the oracle keeps 2^28 until it changes.

### Oracle work bounds

| Bound | Default |
| --- | --- |
| `maxMemberBytes` | 1 GiB per member |
| `maxTiles` | 1,000,000 tiles |
| `maxVertexLevelTests` | 200,000,000 drawn-vertex × level evaluations |

Override them with `--oracle-limit NAME=VALUE`. When a bound runs out, the
result is `Undecided`. JSON nesting deeper than Python's recursion limit
(about 1000) is also `Undecided`.

The vertex test is pure integer arithmetic. Each level's exact affine map,
which includes `|q|²` denominators, `1/k` and `2^-149`, is cleared to a common
integer denominator once per (instance, accessor, level, volume). The
self-test checks this integer predicate against the plain `Fraction`
predicate on 8,000 random cases.

## What the oracle does and does not decide

- **It decides:**
  - node admission;
  - all 15 exclusion reasons over explicit tilesets, including external
    tilesets;
  - the precheck, with the normative spans, units, bytes, decoded bytes and
    depth;
  - exact streaming in the contract order.
- **It does not model C1.** That includes integrity, CRC and the index,
  C1 limits, `hierarchyBounds`, and C1's refusals such as sparse accessors or
  image data URIs.
  - Data that C1 must reject usually makes the oracle `Undecided`.
  - The oracle decodes sparse accessors itself.
- **It does not expand implicit tilesets.** An implicit archive is
  `NotInspected(implicitTiling)`. Its template payloads are not
  admission-checked, so production could report `InvalidInput` for a
  malformed node inside an implicit subtree where the oracle reports
  `implicitTiling`. Every such report carries a caveat. That expansion is
  O-implicit (Slice 0b).
- **It does not apply RTC.** RTC is slice 2. `rtcCenter` is key presence only.
- **`E(u)`, `B(u)` and `Dec(u)` are C1 facts that the oracle models.** See
  contract question 9. The work controls are built so that every plausible
  model gives the same totals.

## Pins

### Contract (v3.3) and coordinator files

| File | sha256 |
| --- | --- |
| `contract-draft.md` (v3.3, read for this regeneration) | `b888942bf2c7b389104043c6f8df2f44ce73ff0282a98214cd23c12967ed2a64` |
| `contract-draft.md` (v3.1, first oracle build) | `0f1d669a8da36dc24e31bae32137e237dc6b7af54f3992ae20c227f4636addd7` |
| `verify/slice1-witness-expectations.json` (v3.1) | `5086105c7bef651ccd62aeeafcc523aa36b742988610933a1d35befa983b1bcb` |
| `SHA256SUMS.v3.1` | `4d4c63d05b60844b0b210b8fda73affbcdf22da759542a2cd871e886dbe1892c` |
| `ownership-plan.md` (Slice 0a) | `18d99741ca3a2bfe73f93b1b7242e3b7c6b878aa64114b1a7bb545a123725316` |
| `producer-inventory.md` | `e391734cc5ef6070528d92d4b913eba1fe27db035095f5cde0f5dcc9dec1e080` |
| `verify/w13-normalized.json` | `356c36fa2040186f13f71c66c6758b670a0aafd217635332721b17c76f536a17` |
| `verify/frozen-bound-observed.json` | `6ff24749f8fc4d4a56c96e66763e4223f5d7f0f44efd738cd0838e1609fed0ef` |
| `change-log.md` (v3.3 decisions on O's questions) | `1ee34c2f80a11cc53a02148316c1417563e2a2f070cb38c3bab0119c4eec4b97` |
| `change-log.md` (v3.2 amendments, first oracle build) | `0e1ccc19a7b06be9485292d4187a0cb36f7eff2381d05508b2c6b49969a7d804` |

Notes on these rows:

- The first `ownership-plan.md` hash, `99deab3c…`, was replaced in
  `SHA256SUMS.v3.1`, and the text of Slice 0a is unchanged.
- `verify/frozen-bound-observed.json` is evidence the contract cites. Only its
  data was read, for B2. The probe code (`frozen_bound_probe.py` and the other
  `verify/*.py`) was **not** read.
- The v3.3 contract differs from v3.1 only in the worked-example acceptance
  sentence. The v3.3 decisions themselves are in `change-log.md` § v3.3.

### Domain preparation

| File | sha256 |
| --- | --- |
| `witnesses.json` | `cee818541b58c1cda6fe1e5ba0d7f86e3688d1515ffff7bc22e727f89e438cb2` |
| `README.md` | `5056fcd3121e603089766d7b79a6d95fb174f20ff3bcd9e1e6f30a695db65ffd` |

Copies of `witnesses.json`, the expectations and `w13-normalized.json` are in
`witnesses/inputs/`. The generator re-checks their pins.

### Primary specs (`primary/`)

| File | sha256 |
| --- | --- |
| `3d-tiles-core.adoc` | `b841fade126f8269d50c1c177e32a30acb01b3d59f1d73820984816378222f87` |
| `3d-tiles-gltf.adoc` | `b2a1fbc95fee4fed6bf650e068506c4097d3cc60dbfb290ed7e50f4f654f0a31` |
| `EXT_meshopt_compression.md` | `602608cb4e3402f189e8c4b235be680093ea07c68aff87ab5d974992283555b0` |
| `feature-table.adoc` | `3158281f21460941ccfd90a41dc0bffda04aeae5e5de296a5ba8197cc1934ece` |
| `gltf-2.0.1.adoc` | `10aebb6a8362a155b196448912bf6250bf046f150431fa73c6cd657c44185aff` |
| `KHR_mesh_quantization.md` | `f59a411d42b52ff7099b98d57fcb2bf4cb2835e7f543d55ab2b08e40c9bec6dc` |
| `modern-b3dm.adoc` | `539762ac799aa8b685035647682b54a40b728904c51ee0685838e29a1ce04f3e` |
| `node.schema.json` | `bd4463cf13f1154fb83fc39b311d8e9d0af1f263115c3f47cd93416b12a5b30a` |
| `scene.schema.json` | `44798bc160abcd8f4bb5e11937c14bc5a88d205932e17c91c5dc337d66bac38c` |

### Fetched by O

| Source | sha256 |
| --- | --- |
| `3DTILES_content_gltf_vector` README at `c48ebdc8db43dc00917b4f200eff5e2131d7e493` | `c8d14e4db0d18f8771b9d2c314a1160019bfc885aa9bfc3c2709fc693973624e` |
| meshoptimizer `demo/tests.cpp`, tag `v0.22`, commit `4affad044571506a5724c9a6f15424f43e86f731` | `365b75f5580b68656e3c3f56943e3d186c49fe206e8e0d248aea2c4a9d4ba70b` |

- **Vector README.** Fetched 2026-10-11 from
  `https://raw.githubusercontent.com/CesiumGS/3d-tiles/c48ebdc8db43dc00917b4f200eff5e2131d7e493/extensions/3DTILES_content_gltf_vector/README.md`.
  This resolves open item O-a: the README defines no transform of decoded
  positions, so the allowlist entry stays. See question 3 about `clip`.
- **meshoptimizer tests.** Used for golden vectors only. They are copied with
  provenance into `oracle/self_test/meshopt_vectors.json`.

### Producer corpus (R, run2)

| Item | Value |
| --- | --- |
| `manifest.json` | `28e4bee6c457de66580b5fb0d125d77d062aca7aba2df34a4cba0940be0d37e4` |
| Source | `develop f2238cd`, native-geospatial release build |
| Binary sha256 | `369c3c81…` |

Every artifact is checked against its hash in R's manifest before it is read.
The artifacts were treated as data and are not committed.

## Results

- **Self-test: 59/59 pass** (36 before C2-WB). The checks are:
  - the upstream golden vectors: TRIANGLES `kIndexDataV1`, INDICES
    `kIndexSequenceV1` and ATTRIBUTES `kVertexDataV0`;
  - the spec's 4-bit sentinel and varint examples;
  - the worked example, bit-exact: `sInst = 5`, level spans `[23, 29]`;
    under C2-WB 3.6, `V = (20 + 25) + (30 + 25) = 100`, 400 vertex units and
    256 node units (slice 1 had 8 and 128);
  - one identity per C2-WB weight (level-0 base, step, box and sphere floors,
    q above the floors, the node floor) and the value-node count;
  - identity-node pair equivalence;
  - pair soundness: about 98k exact quantities over random chains, with 0
    values outside their pair;
  - f32 exactness, `R(q)` exact orthogonality, the quaternion window boundary,
    and the integer predicate against the `Fraction` predicate.
- **Witnesses and controls: 261 cases, all agree.** No expectation mismatches.
  - 14 literal (W01–W13, plus the W01 no-RTC variant), 21 context, 48
    control and 2 review cases.
  - **Review cases (PR #157 nonauthor review B1).** The generator rebuilds the
    reviewer's `probe/empty_mesh_span.py` (sha256 `db8b8886…`). Its output is
    byte-identical to the reviewer's files.
    - Both cases nest three nodes rotated by `[2^-1000, 0, 0, 1]`, ending in a
      mesh with no primitives.
    - **`review-B1-empty-mesh-only`** expects Contained. The URI has no drawn
      instance, so it skips the span check (v3.2 (4)).
    - **`review-B1-empty-mesh-plus-real`** adds a triangle instance at the
      scene root and expects NotInspected(numericSpan) with `sInst` 6008.
      `sInst(u)` covers the empty mesh's instance path too.
  - 176 one-ulp mutants.
  - Every literal frame value the oracle exposes matches its witness exactly.
    That covers W13 under the normalized meaning, W06's rational value and
    W11's external-root and parent frames.
  - All 15 control pairs agree.
  - **`c1Precondition`** (v3.3 (9)) is the generator's static judgement of
    whether C1 `hierarchyBounds` refuses a fixture first. It checks declared
    nesting exactly: tile in parent, content in tile, external root in its
    referencing tile and, conditionally, in that content volume. Acceptance
    compares on the C1-passing subset plus the C1 refusal itself.

    The model covers box, sphere and region nesting in any combination. It also
    treats as refusals what production's first parity run showed C1 refuses:
    region volumes nested with box or sphere volumes; non-allowlisted
    `extensionsRequired` entries; `JOINTS_n`/`WEIGHTS_n` attributes; and
    non-finite JSON numbers.

    | Judgement | Cases |
    | --- | --- |
    | `likely` | 6, the C1-refused fixtures below |
    | `possible` (escape of at most 4 ulps; depends on C1's slack) | 152 one-ulp mutants and `external-content-and-ancestor-both-exclude` |
    | `possible` (only if C1 compares an external root with the referencing content volume) | 1: `external-content-volume-excludes` |
    | none | the other 98 |

  - **C1-refused fixtures.** Production's first parity run observed six
    fixtures stop in C1. Their `expected` is
    `{"outcome": "C1Refused", "kind": ...}`, which the harness compares on
    `kind` only. Their C2-level expectation moves to `oracleExpected`, and the
    oracle (which has no C1 model) is judged on that.

    | Fixture | `kind` | C1 message | C1-passing variant for the C2 reason |
    | --- | --- | --- | --- |
    | `child-volume-escapes-but-content-inside` | `invalid_input` | hierarchyBounds | none needed; the oracle's C2 verdict is Contained |
    | `GPU-instancing-required` | `unsupported` | required extension | `GPU-instancing-not-core-instances` (optional; already present) |
    | `region-bound` | `unsupported` | mixed region/Cartesian containment | `region-only-hierarchy` (new) |
    | `selected-skin` | `unsupported` | attribute semantic `JOINTS_0` | `selected-skin-without-joint-attributes` (already present) |
    | `rotated-TRS-chain-depth-16-with-sphere` | `invalid_input` | content bounds escape tile | `rotated-TRS-chain-depth-16-with-sphere-enclosed` (new) |
    | `tileset-number-overflow` | `unsupported` | JSON exceeds finite owned-document representation | none; see question 16 |

    - **The old sphere fixture was a real defect.** Its tile box had
      half-lengths of about 1e-16 around a content sphere of radius 100. The
      enclosed variant uses a ±128 box, keeps the spans (4216), and replaces it
      in the `rotated-chain-sphere` control pair.
    - **`selected-skin-without-joint-attributes` is malformed glTF.** glTF
      2.0.1 says "the skinned mesh **MUST** have vertex attributes that are used
      in skinning calculations" (`gltf-2.0.1.adoc:1598`). Because C1 refuses
      `JOINTS_n`, no valid glTF skin can pass C1. This fixture is therefore the
      closest C1-passing form for the `skin` reason, as the contract's own note
      describes.
- **Production mutation testing (parity run 2) added two fixtures.**
  - **`external-content-and-ancestor-both-exclude`** expects Outside at
    level 1 with role `externalContent` and depth 1.
    - Both volumes at level 1 exclude the vertex, so only Role order (v3.2 (3))
      gives `externalContent`. Slot order would report `ancestor`. The 13th
      oracle mutant, `slot-order-within-level`, checks this.
    - Exact declared nesting is impossible here: the vertex lies in the
      external root box, so an external root exactly inside the referencing
      tile's box would put the vertex inside that box too.
    - The referencing tile box therefore excludes the vertex by a quarter ulp,
      and the external root escapes it by the same amount. The fixture passes
      C1 only through `hierarchyBounds` slack; `c1Precondition` is `possible`.
  - **`repeated-index-three-payloads-units-at-sum`** (`--large`) sets
    `containmentWorkUnits` to the hand-derived deduplicated sum and expects
    Contained. The derivation is the `REPEATED_INDEX_UNITS` constant in
    `witnesses/generate.py`.
    - Slice 1: `3 × (3·(25+25) + 1 + 3,999,990 + 3,999,999) = 24,000,420`.
    - **C2-WB (2026-10-12):** per payload `3·75 + 1 + 3,999,990 + 3,999,999 +
      Dec 0 + B 8,000,624 + 16·53 + 160·0 + 4·1 + 1152 + 8·2 = 16,002,859`, so
      the sum is **48,008,577** (`V = max(20,25) + 25 + 25 = 75`; `B` and
      `J = 53` are counted from the payload bytes by the generator, not by `c2o`).
      The oracle's total agrees.
    - The `units-minus-1` fixture now takes its cap from the same constant.
    - New pair `repeated-index-work-units-at-cap` separates per-vertex from
      per-occurrence charging of `T`.
- **Mutants: 13/13 flip their named fixture.** Under C2-WB, `reverse-tile-order`
  flips 19 and `skip-ancestors` 26 (18 and 25 before): units now depend on the
  per-level volumes and steps, so each also flips the work-units at-cap pair.
  - Omit U and double U flip 130 fixtures each.
  - The 1e-6 slack flips 177.
  - Reusing the URI verdict and defaulting to scene 0 flip one fixture each.
    Each flips exactly the control designed for it.
- **Corpus: 29 runs, 27 artifacts.**
  - 17 artifacts are `Contained`: the vector explicit ×6, the point cloud
    explicit, the F1c2 mesh local and WGS84, the W1 plain, near-window and
    deep20 runs local and WGS84 (×6), and the legacy explicit split and small
    runs.
  - 10 are `NotInspected(implicitTiling)`: the vector implicit ×6, the point
    cloud implicit, the legacy implicit ×2 and convert-to-implicit.
  - 2 runs were refused by the producer and have no artifact.
  - No `Outside`. See `corpus/manifest.json` for each prediction's
    disposition.

### Corpus notes on `producer-inventory.md`

- **W1 deep20 is a correction.** It has 20 nested generic rotated nodes, yet
  its maximum span is 2589 (`sInst` 2316), so it is `Contained` at the 4096
  cap. The prediction "numericSpan beyond about 16 generic box levels" does not
  hold for it. See question 2.
- **Near-window is confirmed.** `|q|² − 1 = 1.8e-6` is inside the 2^-18 window.
- **Legacy explicit outputs have zero margin.** The tightest vertex lies
  exactly on a box face: slack 0, for both split and small. That is consistent
  with "midpoint rounding with no outward step". They pass only because the
  rounding happened to land on the face.
  - F1c2 `mesh-local-split` has a margin of 0.125 ulp.
  - W1 plain has a margin of 8 ulp.
  - Vector and point cloud margins are at least about 4e7 ulp, from their
    rounding padding.
- **This build writes vector polygon fills as GLB with `EXT_mesh_polygon`, not
  b3dm.** Lines are indexed `LINE_STRIP` without `KHR_mesh_primitive_restart`.
  - The informative all-vertex polygon observation finds 0 POSITION vertices
    outside any volume.
- **The `meshopt` artifacts support the attribute-tail reading S1.** Decoded
  POSITION values equal the declared accessor min/max on 87 of 87 components.

## Contract questions (recorded readings)

Each item gives the oracle's chosen reading. The coordinator ruled on items
1–6, 9, 10, 13, 15 and 18 in `change-log.md` § v3.3, and the oracle and
fixtures follow those rulings. Each ruled item says which decision applies.

1. **The worked example cannot be "Contained at 29".** Decided by v3.3 (1):
   the literal fixture is an in-domain, expected-`Outside` case at 29, and the
   adjusted fixture is the Contained-at-cap / numericSpan-at-28 control.
   - The own box has `c₁ = 0`, `h₁ = 0`. Every vertex has tile row 1 equal to
     `3 − 0.25·z/65535 ∈ [2.75, 3]`, so no vertex can lie inside it.
   - The literal fixture gives `Outside` at 29 and `numericSpan` at 28.
   - The oracle adds an adjusted fixture with `c₁ = 3` and `z = 0`. Its spans
     are identical (`sInst = 5`, `[23, 29]`), and it gives `Contained` at 29
     and `numericSpan` at 28.
   - Any one-ulp change to a worked-example volume value raises its spans above
     29. Its one-ulp mutant therefore runs at the default cap.
2. **The B2 coverage table reports tested depths, not maxima.** Decided by
   v3.3 (2).
   - `frozen-bound-observed.json` evaluates only depths 1, 2, 4, …, 16, so each
     B2 entry is the deepest *tested* depth that was admitted.
   - The admitted depth is at least that value. For example, box-only generic
     is at least 16, sphere generic at least 10 and near-axis at least 14/6.
   - For comparison:
     - W1 deep20 is admitted at depth 20 (span 2589).
     - The oracle's generic chain (seed 20261011) has span 2107 box-only at
       depth 16 and 4216 with a sphere.
   - The rotated-chain control uses depth 16.
3. **The vector extension's `clip` allows escapes that C2 would fail.**
   - The pinned README defines no position transform, so the allowlist entry
     stays.
   - However, `content.extensions.3DTILES_content_gltf_vector.clip: true`
     explicitly allows content to extend outside `tile.boundingVolume` (a
     "buffer" region).
   - With `clip: true`, C2 would report a spec-valid buffer as `Outside`
     against the own tile volume.
   - No producer writes `clip`.
   - Decided by v3.3 (3): a `3DTILES_content_gltf_vector` object with
     `clip: true` is a non-allowlisted use. It is `tilesetExtension` on the
     tileset, tile, content or bounding-volume side; on the glTF side the
     extension is not allowlisted anyway, so it is `gltfExtension`. Controls:
     `vector-content-without-clip` (Contained), `vector-content-clip-true` and
     `vector-clip-on-glTF-side`.
4. **Traversal order is not defined.**
   - It decides which `Outside` comes first and which precheck reason is
     recorded.
   - Oracle reading: depth-first pre-order; a tile's content(s) before its
     children; an external tileset expanded in place. Adopted by v3.3 (4).
5. **The Outside message fields `primitive` and `depth` are underspecified.**
   - With bitmap dedupe, `primitive` is ambiguous when several primitives share
     one POSITION accessor. The oracle reports the lowest-index primitive of
     that instance's mesh that draws the vertex.
   - Decided by v3.3 (5): `depth` is the tree depth of the tile whose content
     is referenced (root 0; an external root is its referencing tile's depth
     + 1). The oracle reports it as `facts.depth`, next to `level`.
     `primitive` is the lowest primitive index whose draw references the
     vertex, which is what the oracle already reported. Mutant and control
     expectations now include `depth`.
6. **`Nodes(u)` is undefined for subtrees without a mesh instance.**
   - Readings (e) and (9) say `Nodes(u)` counts every selected node, at
     `128·⌈s_step/64⌉²` for each non-skipped one. They also say "F does not
     push composites for selected subtrees without a mesh instance". Without a
     composite, `s_step` for those nodes is undefined.
   - The oracle computes composites for every selected node and charges them.
     `sInst` covers instance paths only, as accepted.
   - Production can differ only for non-skipped mesh-less leaves. None occur
     in the witnesses or the corpus.
   - Adopted by v3.3 (6): F charges mesh-less subtrees too.
7. **Raw matrix entries and `sInst`.**
   - The span text counts only products and flat sums. Accepted reading (d)/(9)
     puts a matrix node's raw `L` entries into `s_step` and makes
     `sInst = max s_step`.
   - The oracle adopts the accepted reading. The two can differ only when no
     `G·L` product reaches a matrix entry (a singular composite column) and
     that entry's span, at most 53, is both the maximum and above the cap. That
     cannot happen at caps of 53 or more.
   - The oracle agrees with readings (a), (b), (c), (e) and (f).
8. **Hull versus instance.** v3.1 settles this. The oracle uses the per-URI
   hull `Pair0(u)`. The two-instance control gives `numericSpan` (span 4558),
   and the one-instance control gives `Contained` (558).
9. **`E(u)`, `B(u)` and `Dec(u)` need exact C1 definitions.**

   | Fact | Oracle model |
   | --- | --- |
   | `E` | Σ over **all** document accessors of count × components |
   | `B` | Member bytes plus the member bytes of every resolved external buffer or image **entry** (C2-WB: a member named by two entries is read, and counted, twice, as C1's resolver does) |
   | `Dec` | External and data-URI buffer bytes plus the decoded `byteLength` of **every** meshopt-compressed view, used or not |
   | `J` (C2-WB) | Value nodes (root plus every scalar/container value; keys are not nodes, C1 contract "JSON value nodes") of the glTF JSON or GLB JSON chunk (also inside a b3dm) plus the b3dm feature-table and batch-table JSON, each when its length is nonzero |
   | `Res` (C2-WB) | Buffer and image **entries** whose `uri` is not a `data:` URI (a repeated URI counts per entry; see wb/README.md, readings) |
   | `Prim` (C2-WB) | Σ over selected instances (nodes with `mesh`) of the mesh's `primitives` length |

   The work and repeated-index controls are built so that these models
   coincide with any natural alternative. Each of those fixtures has only
   POSITION and index accessors, every compressed view is used and there are
   no external files. v3.3 (7): production exposes these facts through a
   crate-private test accessor, and acceptance reconciles them with this model
   per work control.
10. **Core `contents` (3D Tiles 1.1) is not mentioned in the contract.** The
    oracle treats each element as a reference with its own content volume.
    `3DTILES_multiple_contents` is `tilesetExtension`. Adopted by v3.3 (10):
    entries are separate references in array order. Controls:
    `contents-array-both-inside` and `contents-array-second-outside`.
11. **The scope of `regionVolume`, `rotatedBox`, `tileMetadata` and
    `tilesetExtension` is read literally.**
    - A region or rotated box on a tile with no payload beneath it is never
      tested, so it is not excluded.
    - Tileset-level `metadata`, `schema` and `groups`, and `content.group`, are
      not exclusions.
    - Extension objects are checked only on the tileset, tiles, contents,
      bounding volumes and the `extensionsUsed`/`extensionsRequired` lists.
      They are not checked on `asset`, `properties` or `implicitTiling`.
12. **External-tileset detection** uses the `.json` suffix of the
    percent-decoded path, case-sensitively.
13. **Duplicates in `children` and `scenes[*].nodes` are InvalidInput(node).**
    A node repeated in one `children` array counts as two parents. A repeated
    scene root violates glTF `uniqueItems`. The contract says only "no node
    has two parents". Adopted by v3.3 (8). Controls: `duplicate-child` and
    `duplicate-scene-root`.
14. **Exact integer representations are integers.** This reading changed on
    2026-10-11, to align with C1's admitted profile.
    - A JSON number whose exact decimal value is an integer reads as that
      integer, in index fields too. Examples: `1.0`, `1e0`, `10e-1`,
      `1000.000e-3`, `-0.0`, `0e999…`.
    - C1 admits such fixtures (`primary-local-gltf-integer-representations`,
      `exact-count-*`, `reference-zero-*` in the C1 payload controls).
    - A non-integral value in an index field is InvalidInput(node).
    - The oracle used to reject every float-typed index.
15. **A tile `transform` whose last row is not `0,0,0,1`, or that is not 16
    finite numbers,** is InvalidInput by v3.3 (8). The oracle used to report
    `Undecided` here; it now reports `InvalidInput`. A tileset number literal
    that overflows binary64 is also `InvalidInput` now. Controls:
    `tile-transform-non-affine` and `tileset-number-overflow`.
16. **An overflowing number literal** (for example `1e400`) fails only where
    it is consumed.
    - In a consumed glTF node field it is InvalidInput(node).
    - Opaque glTF JSON, such as `extras` or extension-owned data, may hold it.
      C1 admits `opaque-huge-number`.
    - Anywhere in a tileset JSON it is `InvalidInput`, because the tileset
      document is owned whole.
    - The contract "Numbers" paragraph calls it InvalidInput "as in the
      existing tileset number handling".
    - Production's C1 JSON layer instead refuses a tileset literal of `1e999`
      as **Unsupported** ("JSON exceeds finite owned-document
      representation").
    - Either the contract sentence or that C1 kind needs correcting. The
      fixture expects the observed C1 refusal, and the oracle keeps the
      contract text.
17. **Restart values.** The oracle drops the maximum index value from every
    primitive. C1 admits restart only for indexed `LINE_STRIP` with
    `KHR_mesh_primitive_restart` in both `extensionsUsed` and
    `extensionsRequired`, and rejects the maximum value elsewhere under the
    core rule. They differ only on input C1 rejects.
18. **`hierarchyBounds` interplay with the controls.**
    - Base fixtures nest exactly: each ancestor box is the exact image of its
      child box.
    - One-ulp mutants at ancestor levels, and centre-shift mutants of
      zero-half-length rows, break declared nesting by one ulp. Whether
      production reaches C2 for them depends on C1's slack. Each such row is
      noted in `report.json`.
    - In the I2 control, the content volume of the referencing tile excludes
      the external root's volume. If C1 compares external roots against that
      content volume, `hierarchyBounds` precedes C2.
    - For v3.3, the I2 control was rebuilt so that its referencing tile box
      encloses both the external root and the content box. The only possible C1
      refusal left is that external-root-in-content comparison.
    - Decided by v3.3 (9): every report row carries a `c1Precondition`
      judgement wherever declared nesting is broken (see Results).
19. **W13 needs a separating box.** The raw-polynomial and normalized readings
    differ by about 1.04e-15, below one ulp at |x| ≈ 17.5. Only a box with a
    tiny half-length, whose faces lie strictly between the two readings, can
    observe the difference. The W13 fixtures use such a box.

### Spec readings (EXT_meshopt_compression)

- **S1. Attribute tail.**
  - The spec says "a baseline element stored verbatim, padded to 32 bytes" but
    not where the element sits in the tail. The oracle reads it from the
    **last** `byteStride` bytes, with the padding first.
  - The upstream vector cannot tell the two apart, because its baseline is
    zero.
  - The corpus supports the reading: decoded POSITION equals the declared
    min/max on 87 of 87 components.
- **S2. 4-bit packing formula.** The formula reads
  `(delta1 << 0) | (delta1 << 4)`, which is a typo. The worked example fixes
  the meaning: delta0 is in the high nibble.
- **S3. Third varint example.** The spec gives
  `0xff 0xa0 0x05 => 0x1fd005`, which is not unsigned LEB128. LEB128 gives
  `0x1507f`. The decoder follows LEB128, which the spec text and the upstream
  vectors both use.
- **S4. Strictness.** The decoder rejects all of these:
  - unread trailing bytes;
  - FIFO reads of undefined entries;
  - a `codeaux` table that breaks its stated constraints;
  - non-NONE filters.

  A stream production accepts but the oracle rejects shows up as `Undecided`,
  never as a verdict.

## C1 payload-control amendment (2026-10-11)

C2 changed the report of every admitted C1 payload control. The amended
expectations were derived from this oracle. They are in
`bench/architecture_audit/c1_payload_integrity/production_controls/c2_amendment/2026-10-11/`,
whose `README.md` gives the rules, pins and end-to-end results.

Running the oracle over those C1-admitted fixtures exposed three gaps in its
coverage of the C1 profile. None of them changes any witness, control or corpus
verdict.

- integer representations, now question 14;
- consumed-only overflow, now question 16;
- b3dm embedded GLBs followed by the b3dm's 8-byte padding.

Unreadable ZIP members now make the oracle `Undecided` instead of crashing.

## C2-WB amendment (O-WB, 2026-10-12)

O-WB re-implemented the C2-WB unit arithmetic (contract draft v2.1, sections 3.1,
3.2 and 3.6) in `c2o` from the contract text, the existing oracle and the delta
reviews only. It did not read `src/`, R's `probes/`, or the J/S/F branches.

| Where | Change |
| --- | --- |
| `c2o/check.py` | `level_operations` (per-operation weights `max(F, ⌈s/64⌉²)`, F = 20 base, 30 step, 25 box, 32 sphere), `reference_units` (`Dec + B + 16·J + 160·Res + 4·Prim + 1152 + 8·(L_r+1)`), `Prim`; per-reference `V`, `levelOps`, `referenceUnits`; a `terms` breakdown of the totals; `precheck_only` and `detail` options |
| `c2o/scene.py` | Node visits: 1 skipped, otherwise `max(256, 128·⌈s_step/64⌉²)`; per-span visit counts |
| `c2o/gltf.py`, `c2o/numbers.py` | `J` (now also parses the b3dm batch-table JSON) and `Res` |
| `c2o/tileset.py` | Sibling references share ancestor levels at distance ≥ 2 (memory only; 100k references × 103 levels) |
| `witnesses/generate.py` | `REPEATED_INDEX_UNITS` re-derived by hand (48,008,577) |
| `preparation/contract-20261011/verify/frozen_bound_probe.py` | `--wb` writes `frozen-bound-observed-wb.json` (gitignored; C2-WB units of the same chains and the 3.6 worked example by the probe's own span engine; the 2026-10-12 output, sha256 `2926e31aa50dfddd33cbb938bd8b74fd7777054cea7995daf575e5e36c5fd543`, is kept outside the repository); the slice-1 output is byte-identical |

**At-cap pairs whose expected totals changed** (fixture bytes unchanged; only the
cap values in `controls/manifest.json`):

| Pair | Fixture | Slice 1 | C2-WB |
| --- | --- | --- | --- |
| `containmentWorkUnits-at-cap-and-one-over` | `work-containmentWorkUnits-at-sum` / `-sum-minus-1` | 422 / 421 | **5,040 / 5,039** |
| `repeated-index-work-units` (`--large`) | `repeated-index-three-payloads` / `-units-minus-1` | — / 24,000,419 | — / **48,008,576** |
| `repeated-index-work-units-at-cap` (`--large`) | `repeated-index-three-payloads-units-at-sum` / `-units-minus-1` | 24,000,420 / 24,000,419 | **48,008,577 / 48,008,576** |

`work-base-meshopt` breaks down as vertex 390 (T 3 × V 130) + Nodes 513 + Idx 3 +
E 12 + Dec 42 + B 1,404 + 16·94 + 4·1 + 1152 + 8·2. The bytes, decoded, depth and
span pairs are unchanged. Every other witness and control keeps its verdict at
the default cap; their per-row totals in `witnesses/report.json` are informative.

Corpus totals (no verdict changes): vector explicit 10,939–84,490, point cloud
explicit 17,492,428, F1c2 mesh 2,731,984, W1 plain 11,597, near-window 23,837,
deep20 1,535,056, legacy explicit 979,358 and small 4,060 (`corpus/manifest.json`).

Pins (read for this amendment):

| File | sha256 |
| --- | --- |
| C2-WB `contract-draft.md` v2.1 | `ea7b3acff2d4768d1ab68333b6730c080b30eb7495060c449bbe0a8bef8eac97` |
| C2-WB `ownership-plan.md` v2.1 | `3334b3acc6de0fd8c369e8f68b2c39f91116ce8e0f1f7010357ec4bd97cb3fe4` |
| C2-WB `change-log.md` v2.1 | `6e1bb99ae0632c2959f46f6259871fe4f6f733ef764f0c6bb7d7b1796f1e2f64` |
| `delta/review-delta.md` | `fbb99fafde66af13d0da67fa09c96bd69ec0bf36c696c5a71e9c8ca68c6bd2ab` |
| `delta2/review-delta2.md` | `8f28d536ce88ea9dbf533795c6b9c3821b2c57492d26ab79d64fb62cdbf12c04` |
| `docs/architecture/c1-payload-integrity-contract.md` (value-node meaning) | `3cb5ad341a73cdda77a9dd4a1925cf9800af6e37e49b902eb6db0c4f0c3ee1e3` |

**R's reconciliation against production (2026-10-12).** The first production run of
the `wb/` fixtures (`acceptance/oracle_parity.rs`,
`production_matches_every_c2wb_class_fixture`) found two differences, both fixed in
the fixtures and oracle, not in `src/`:

- C1 refused every multi-reference `wb/` fixture (59 of 102 rows): identical sibling
  leaf tiles break the tileset schema's `children` `uniqueItems`. `wb/generate.py`
  now gives leaf `i` the geometric error `i·2^-24`. No unit total changed.
- `B` with a repeated URI (`reading-res-duplicate-uri`). The C1 payload contract (`docs/architecture/c1-payload-integrity-contract.md:330-331`): "Repeated resource declarations charge repeated actual bytes; no digest/URI deduplication changes that meaning."
  So `B = 435` per reference; the slice-1 oracle counted each member once (423), which the
  C1 clause rules out. `c2o/gltf.py` and the hand facts now count per entry (production
  also gives 435). The probe's total is 5,602, its at-cap pair 5,602 / 5,601. No
  witness, control or other `wb/` total changed. The `Res` reading (per entry) and
  `data:` exclusion match production as written.

After both, production agrees on all 102 `wb/` rows: the 55 at-cap pairs, the 4
chain-bit controls, and the totals and per-URI facts of the 43 matrix-size rows
(local, with `--large`). That also confirms C1 admits every large class fixture,
including the meshopt INDICES/bits streams and the 800-character decimals.

## Slice 3: implicit archives (O, 2026-10-12)

Role O of the slice-3 ownership plan (sha256 `28285135…`) wrote this from the frozen slice-3 contract v3.1
(`contract-draft.md`, sha256 `8bc534290c5a90ace963e345336dea1e04c45fafb3c869672ee4af1ba03331d6`), the C2, I1
and C1 contracts under `docs/architecture/` and the primary specs. It did not read `src/`, `tests/`,
`bindings/` or the quarantined prototype probes (contract 9.G). `probes/validate-corpus-b51f83d.json` (marked
readable) was used only for the archive hashes.

| Path | What it is |
| --- | --- |
| `oracle/c2o/implicit.py` | O-implicit-C2 (contract 9.A): composes O-implicit (`implicit_read/oracle/i1_oracle.py`, `evaluate()`) and O-explicit (`c2o/`, as amended by C2-WB), both pinned by sha256 (`PINS`; drift is reported as `pinDrift`). Builds the section 4 chains, applies I5–I7, never produces `implicitTiling`, reports section 3 Outside facts (`facts.volume` = `own`, `ancestor:k`, `externalContent:k`; `facts.depth` = depth(R) + l) |
| `oracle/c2_oracle.py --implicit [--i1-limit NAME=VALUE]` | CLI flag. A non-Accepted O-implicit verdict is the outcome, with `"phase": "I1"` |
| `oracle/self_test/slice3.py` | 14 checks: pins; explicitization identity (9 fixtures, verdict, units and every per-level vertex coordinate); chain length = depth(R) + 1 + l + 1 and depth = depth(R) + l on 706 references; root order (and a swapped-record negative); external-root placement at l + 2 |
| `oracle/mutants/slice3.py` | 15 oracle mutants (contract 9.E, oracle side) as hashed patches in `oracle/mutants/slice3/`, report `slice3-report.json` |
| `controls/slice3/` | `generate.py` (with `synthetic.py`, `forest.py`), `manifest.json`, committed `fixtures/`, `check.py`, `results.json` |
| `corpus/slice3.py`, `corpus/slice3-manifest.json` | Forest witnesses (9.B), their precheck totals, the coverage statement and the 9.D limit pairs |

Replay (from this directory; single-threaded, `taskset -c 2,3 nice -n 10`):

```sh
python3 -I oracle/self_test/slice3.py                       # 14 checks, 0 failed
python3 -I controls/slice3/generate.py --forest DIR --pre-amendment DIR   # 80 cases; --check: 0 mismatches
python3 -I controls/slice3/check.py                         # 80 cases + 181 I1 cases, 0 disagreements
python3 -I oracle/mutants/slice3.py                         # 15/15 flip their named control
python3 -I corpus/slice3.py CORPUS_DIR                      # 34 archives, 5 pairs, 0 failed
```

`--forest` is R's b51f83d forest directory; `--pre-amendment` holds the I1 controls as generated at `92e8c04`.
Forest-derived controls above 120 kB are written to `/home/bend/.cache/c2s3-evidence/slice3/` and are listed in
the manifest by sha256 only (`file: "evidence:…"`).

**Limits.** O passes the #160 defaults to O-implicit (`hierarchy_visits` 2^18, `references` 2^20,
`member_bytes` and `document_decoded_bytes` 128 MiB, `document_items` 2^17), and the slice-3 default cap
`containmentWorkUnits` = 2^29 explicitly. `c2o/check.py`'s `DEFAULT_LIMITS` (O-WB's file) said 2^28 when
this section was written; R set it to 2^29 at integration (see below), so the bare explicit oracle and
O-implicit now agree. I1 controls (S3-18, S3-19) run at their manifest limits over O-implicit's own defaults, as the
I1 parity test does.

### Results

- **Forest witnesses (b51f83d, 17 forest + 17 twins): all Contained**, except the `mf2-lod3-explicit` twin,
  which O-implicit refuses as ResourceLimit (`document_items`), as production does. No witness is `workLimit`
  at 2^29. Maximum span 290 bits (F-MF2); FLOAT levels 277–290, quantized 69–78; at most 19 chain levels.

  | Witness | Units | Witness | Units |
  | --- | --- | --- | --- |
  | F-W1 | 7,074,373 | F-W5 / -quantize / -meshopt | 202,516,294 / 198,748,288 / 209,130,354 |
  | F-W2 | 70,582,811 | F-MF2 | **447,748,743** (12,527 references, 11,300 implicit) |
  | F-W3 | 31,659,297 | F-W6 / F-W6d / F-W6dn | 2,430,561 / 2,602,924 / 2,050,284 |
  | F-W4 (6 archives) | 11,347–102,823 | F-W7 | 4,193 |

- **9.D limit pairs (each end run by the oracle):** F-MF2 `containmentWorkUnits` 447,748,743 Contained /
  447,748,742 `workLimit`; `containmentSpanBits` 290 / 289 (`numericSpan`); `containmentBytesRead`
  59,996,880 / 59,996,879, the precheck-only arm, stops at the last reference (12,526); F-W6d
  `containmentDecodedBytes` 57,736 / 57,735; repeated-URI fixture (K = 3, 9 references)
  `containmentWorkUnits` 20,562 / 20,561.
- **Controls: 80 cases, all agree.** Every base and mutant is Accepted by O-implicit (so passes
  `hierarchyBounds` and `geometricErrorOrder`), except S3-16.depth-1, the intended I1 ResourceLimit.
  - S3-1 (W6), S3-2 (W4-polygons, W1, W5-quantize), S3-3 (W1), S3-4 and S3-5 (W7, W6), S3-6 (W1), S3-7
    (W5-quantize) from producer output.
  - Synthetic: S3-8, S3-9 to S3-14, S3-16, S3-20, the S3-17 role family (own, ancestor:1, ancestor:2 = I_0,
    ancestor:3 = V_R, externalContent:4, ancestor:4, all depth 3; it also carries S3-4, S3-5 and S3-15), the
    repeated-URI pair and a two-root fixture.
- **S3-18 (181 I1 controls after amendment 6):** 65 Contained; 106 refused by I1 (verdict passes through);
  derived 4 (B-4, B-12b, R8a, R8a.nopad); `tileMetadata` 2 (C-12b.group.base, C-12b.uri.base); outside C2 in
  production 4 (C-10 and R1b.full refused in C1, R5 by injection only, Z-payload explicit-only). None is Outside.
- **S3-19:** 32 pre-amendment fixtures, all Outside(own), with their old hashes.
- **Mutants: 15/15** flip their named control (an unmutated copy flips nothing). The 9.E row "register with the
  charging `ArchiveResources::reference`" is C1 accounting and has no oracle counterpart.

### Readings (please confirm or correct)

1. **S3-4 on F-W6 (9.C).** `I_0` carries its own content and it lies on every `V_0` face, so shrinking `V_0`
   gives Outside(**own**) at reference 0, not `ancestor:l`. `ancestor:l` is covered by S3-17.ancestor2.
2. **Whole-subtree-tight mutants (9.C steps 1 and 3).** With `c'` fixed, `h'` can be bound by the
   opposite-face constraint; then `pred(h')` still reaches `e(X)` and the mutant stays Contained (W5-quantize's
   first leaf did this). O chooses X and the face where the `e` constraint binds and records both.
3. **S3-8 on producer output** is not built. A tight explicit parent of the vector roots would need every
   `V_R` and row below it tightened too (vector padding is millions of ulps), so the control is synthetic.
   S3-6 uses W1 because the single-tile vector witnesses carry no root translation.
4. **The witnesses are b51f83d**, not the slice-3 head (9.B). Re-run `corpus/slice3.py` and
   `controls/slice3/generate.py` on R's head-regenerated archives (update `WITNESSES` / `forest.INPUTS`).

### Slice-3 acceptance (R, 2026-10-12)

R integrated F (`feat/c2-implicit-containment`, rebased onto develop `1b1ff9d`) and O (`c2s3/oracle`
`e83263e`) and owns everything below. Contract v3.1 (`8bc53429…`) governs. Raw outputs are outside the
repository under `/home/bend/.cache/c2s3-evidence/` (listed by sha256 at the end).

**R integration fixes to O's files (recorded, no verdict changed).**
- `c2o/check.py` `DEFAULT_LIMITS["containmentWorkUnits"]` was O-WB's pre-C2-WB 2^28; it is now 2^29, and
  `c2o/implicit.py` re-pins `c2o/check.py`. No slice-1 witness or control lies between the caps (the two
  `wb/large` rows that do already expected Contained at 2^29). Self-tests 61/61 and 14/14; O's slice-3
  oracle mutants still 15/15 (plus the unmutated copy).
- Forest witnesses (9.B) regenerated at the integration head (below); `corpus/slice3.py` `WITNESSES` and
  `controls/slice3/forest.py` `INPUTS` re-pinned, then O's generators re-run unchanged.
- `implicit_read/oracle/README.md` ("Accounting") notes that O-implicit's `referencesCharged` omits C1's
  tileset-document reads, so the C1 `references` boundary is `referencesCharged + tilesets`.

**Forest corpus (9.B).** Every 9.B command and its `--explicit` twin (plus `W6d --noMeshopt` and its twin),
release `--features native-geospatial` binary of the integration source (sha256 `6ade8f52…`; the validator
and producer sources are identical at the final head), `produce.json` sha256 `bf050f45…`. Point, mesh and
glb archives are byte-identical to b51f83d; vector archives differ only in `vector-build.json`'s encoder
digest and `tileset.json`'s `vectorBuildStateSha256`. O's re-run: 34 archives, totals, spans, chain lengths
and the 5 limit pairs identical to O's b51f83d reading; 80 controls (only forest-derived hashes change) and
80 + 181 checks with 0 disagreements.

**Production parity (release lib tests, `acceptance/oracle_parity.rs`; large and evidence files present).**

| Test | Result |
| --- | --- |
| Slice-1 witnesses and controls | 261/261 |
| C2-WB class fixtures | 102/102 |
| Slice-3 controls (verdict, reasons, Outside volume/depth/member/vertex, Contained totals) | 80/80 |
| S3-18 sweep over the 181 I1 controls | 180/181 (R5 is injection-only): 65 Contained, 106 I1-refused with the same kind, 4 `derivedImplicitVolume`, 2 `tileMetadata`, C-10 and R1b.full C1 refusals, Z-payload Contained; plus the 2 `references` pairs |
| Forest witnesses and twins (verdict and units) | 34/34; `mf2-lod3-explicit` is ResourceLimit (`document_items`) in both |
| 9.D pairs from O's totals | 5/5: F-MF2 `containmentWorkUnits` 447,748,743 / −1 `workLimit`, `containmentSpanBits` 290 / 289 `numericSpan`, `containmentBytesRead` 59,996,880 / −1 `workLimit`, F-W6d `containmentDecodedBytes` 57,736 / −1, repeated-URI 20,562 / 20,561 |
| I1 parity / W corpus | 181/181 / 41/41 |

**X3-2 counterexample (producer).** The native test `point_archive_and_feature_attributes_use_valid_core_types`
(thin LAS: 60 points, x 0–10, y 0–1e-6, scale 1e-7; `point-cloud --sourceCrs local --maxPoints 10`) now
fails `validate`: tile 2/1/1/3's row (c_x = −1.5254237000000002, h_x = 0.932203412055969) has its exact
lower x face 2^-52 above the drawn x = −2.4576271120559694 of vertex 0. O-implicit-C2 reports the same
Outside (own, depth 2, member `implicit-content/r0-2-1-1-3-0.glb`, vertex 0), so C2 is right and the
contract's "producer prediction: none" for X3-2 is falsified on degenerate input. Coordinator decision: the
point producer rounds tile bounds outward in a separate slice that merges first; the test is unchanged.
Resolved by #183 (merged into develop as `a4f14e6`): the test passes at the re-run below, unchanged.

Not checked in production: the precheck-only arm's stop point (the oracle records reference 12,526 of
12,527; production exposes no stop index).

**`references` pairs (9.E "charging reference").** The first witness took O-implicit's `referencesCharged`
as the boundary and production refused it. The witness was wrong: `references` keeps its C1 meaning (I1
§2a step 10; C1 contract "C1 reads/references": actual caller resource visits), and C1 charges the read of
every tileset document. The pairs use `referencesCharged + tilesets`: BQ 28 admits, 27 refuses; C-11 (an
external implicit-root tileset) 17 and 16.

**Production mutants (9.E).** `acceptance/production-mutants/slice3/`: `mutants.py` and the 35 hashed
patches; the report is evidence. Every mutant flips its paired control, and the unmutated tree flips
nothing. Production-side names follow O's oracle named controls (O's reading 1): skip `I_0` →
S3-17.ancestor2, merge `I_0` into `R` → S3-17.ancestor3 (also S3-5), own row only → S3-17.ancestor1 (also
S3-3.W1). The 129-level capacity is synthetic, as in O's twin (13 = S3-16's `hierarchy_depth + 1`).

| Mutant | Paired control | Cases flipped |
| --- | --- | --- |
| keep the `implicitTiling` exclusion (now the derived bit, archive-wide) | F-W7 (F-W1 with evidence) | 164 |
| skip the `I_0` level | S3-17.ancestor2 | 62 |
| skip `V_R` (`I_0` takes `R`'s transform, external volume and parent) | S3-17.ancestor3 | 93 |
| own row only | S3-17.ancestor1 | 64 |
| `R.transform` before `V_R` | S3-8.syn | 87 |
| external content volume at `R`'s level | S3-17.externalContent4 | 1 |
| `Cell::rounded_for_hierarchy_bounds` for derived tiles | S3-9 | 6 |
| archive-scoped derived exclusion | S3-10 | 5 |
| derived detected in `build_levels` only | S3-20 | 1 |
| FLOAT32 rows through their shortest decimal | S3-14.syn.base | 10 |
| subtree tileMetadata as `tileMetadata` | F-W7 | 148 |
| ignore header `metadata` | S3-12.header | 3 |
| `depth` = chain length − 1 | S3-17.own | 52 |
| charging `ArchiveResources::reference` for emitted contents | BQ and C-11 `references` pairs | 2 |
| units per URI | repeated-uri.cap-1 | 64 |
| fixed chain capacity (synthetic) | S3-16 | 18 |

The 19 slice-1 production mutants (`../scripts/mutants.py`; `skip-ancestors` and `skip-content-volume`
re-expressed for the slice-3 `build_levels`) were re-run on the same suites: all 19 flip cases, and 10 flip
slice-3 controls or forest witnesses. The other 9 (scene default, signed clamp, depth-cap off-by-one, raw
quaternion polynomial, URI verdict cache, normalized quotient, content volume, undrawn vertices, slot order)
concern features the forest output and slice-3 controls do not contain (no scene-less payloads, rotations,
normalized positions or content volumes on implicit references); their slice-1 controls catch them.

**Size (B3).** `size_of`: `TileFact` 376 B, `Reference` 32 B, `LevelFacts` 472 B;
`Option<VolumeFacts>` = `VolumeFacts` = 104 B, so `TileFact` did not grow. Chains are at most
`hierarchy_depth + 2` levels; observed at most 19 (F-MF2).

**Performance (contract §10; Intel i7-12700KF, Linux, release; `validate --json`, child pinned to one
quiet P-core with its SMT sibling idle, 5 clean runs per arm).** Before = develop `b13a86d` release binary
(sha256 `706dbc1e…`; validator sources identical to the C2-WB merge); after = the integration binary;
C2-off = after on the `gltfUpAxis = "Z"` twin.

| Archive | Before s | After s | C2-off s | RSS before / after / C2-off MiB |
| --- | --- | --- | --- | --- |
| mf2-lod3 (gating) | 0.841 | **3.444** | 0.859 | 102.5 / **104.8** / 104.8 |
| W5 vec-perf / quantize / meshopt | 0.101 / 0.098 / 0.116 | 1.265 / 1.334 / 1.254 | 0.102 / 0.100 / 0.116 | ≤ 90.2 / ≤ 88.3 / ≤ 88.0 |
| W2 | 0.086 | 0.428 | 0.091 | 87.8 / 85.5 / 85.7 |
| W3-pc-deep | 0.185 | 0.340 | 0.187 | 88.8 / 86.7 / 86.7 |
| W1 | 0.048 | 0.079 | 0.048 | 87.1 / 84.8 / 84.6 |
| W4 (6), W6, W6d, W6dn, W7 | 0.039–0.057 | 0.039–0.068 | 0.040–0.054 | ≤ 87.5 / ≤ 85.2 / ≤ 85.2 |
| `--explicit` twins (17; C2 on before and after) | within the lane rule (largest growth +0.099 s, W5-quantize-explicit 1.263 → 1.362) | | | ≤ 102.0 / ≤ 97.3 / ≤ 97.2 |

1. **C2-off parity:** every C2-off arm is within the lane rule (1.25×, 0.1 s floor, 16 MiB) of before, and
   mf2-lod3 C2-off (0.859 s, 104.8 MiB) is within it of I1's absolute values (1.007 s, 100.9 MiB).
2. **Precheck-only arm:** F-MF2 at `containmentBytesRead` = 59,996,879 (`workLimit` after the last
   reference) minus its twin: 0.027 s child CPU (P-core, median of 7 clean pairs). C2-WB's 1,047,040-reference
   construction under a 100-document chain: 0.596 s P / 0.991 s E (C2-WB: 0.58 / 0.99); depth 1: 0.068 /
   0.098 s.
3. **Per-class streaming (A − twin, median of 7 clean pairs, oracle units):**

   | Class | Witness | P ns/unit | E ns/unit |
   | --- | --- | --- | --- |
   | FLOAT points | F-W2 | 4.57 | 8.04 |
   | FLOAT vector | F-MF2 | 5.98 | 9.83 |
   | quantized vector | F-W5-quantize | 5.94 | 10.07 |
   | many references | F-W3 | 4.93 | 8.21 |
   | zero-slack mesh | F-W6d | 5.16 | 8.30 |

   All are within C2-WB's τ_P = 8.10 / τ_E = 14.65 and its producer-twin range (P 4.58–5.94, E 7.66–10.12).
4. **Absolute ceiling:** mf2-lod3 best wall 3.444 s ≤ 4.5 s, peak RSS 104.8 MiB ≤ 116.9 MiB. The M hoist
   (OD-8(c)) is not required. Re-checked with the native release binary of the final merge `806954b`
   (sha256 `dbd7a9bb…`): 3.261 s, 107.4 MiB (W5 1.123 s).
5. **Lane:** without C2 the validate cases do not move; with C2 the six implicit archives above 0.1 s growth
   (W2, W3, W5 ×3, mf2-lod3) exceed 1.25× on wall only, the case §10 item 5 covers with the recorded
   `--explain` (items 1–4 pass). No RSS flag (largest growth +2.3 MiB).

Debug builds are slower; no claim is made for them.

| Evidence (`/home/bend/.cache/c2s3-evidence/`) | sha256 |
| --- | --- |
| `forest-439b436/produce.json` (corpus argv, binary and archive hashes) | `bf050f45d0de3925bfaf2c5d81d029a95eebaf708e3eac4747e4d87c196af819` |
| `twins/twins.json` (`gltfUpAxis` twins) | `e0f5193c8779adf0505ff42b2aa26ac0acdf48c78f0147493a5e72babb96e14d` |
| `mutants/report.json` (36 rows, at `8c7bc9f`) | `d656c2f1e7071c36908d8e3b1e896e1f10c296f4dd3ce2ab3ed35e82224878b0` |
| `perf/forest.json` (before / after / C2-off) | `3c774dddeba5bf0a360d572bbfdea8f79879ab9718d679901c5f34b21d93fd15` |
| `perf/gate-P.jsonl`, `perf/gate-E.jsonl` (per-class, C2-WB `gate.py`) | `138ea744…`, `ce1e26b7…` |
| `perf/arm-mf2.json` (precheck-only arm) | `d9accb9323ea648bd168afc2daaaef0f581cf287a0172143637db03d14780949` |
| `arm/SHA256SUMS` (C2-WB arm archives, regenerated) | `082e6346aa3967c887e6b63b2afacf070eef0d1b07070c66f4ca4d173a4c076f` |

Proof limits: one host and one compiler; per-class figures are measurements, not bounds; the precheck stop
index is oracle-only; derived cells, RTC, regions and rotated boxes stay outside the claim; D4 is deferred.

### Re-run after merging develop (R, 2026-10-13)

The branch merged develop `a4f14e6` (#177, #178, #180, #181, #184, #185 and #183, which rounds point tile
bounds outward and widens implicit-writer drift) at `2d85208`, without conflicts. Develop changes no
validator source (the branch's `src/` diff against develop is still `src/validate.rs` alone), so the
re-run is about the producer output that the corpus and the forest-derived controls pin. Corpus re-pin at
`f661e4e`. Contract v3.1 governs. **No verdict changed** anywhere below; only totals and hashes moved.

- **Forest corpus (9.B):** `produce_slice3.py` (same argv as before) with the release `native-geospatial`
  binary at `2d85208` (sha256 `b279325b…`). 23 of the 36 archives change: W1, W2-explicit and
  W3-pc-deep-explicit (#183), and every vector archive and twin (#185). All W6/W7 mesh and glb archives are
  byte-identical. #184 (terrain) has no forest witness. `corpus/slice3.py` `WITNESSES` and
  `controls/slice3/forest.py` `INPUTS` were re-pinned, then O's generators were re-run unchanged.
- **O's re-run.** Self-test 14/14. Corpus: 34 archives, every verdict, span and chain length as before (all
  Contained except `mf2-lod3-explicit`, ResourceLimit `document_items`). Point and mesh units are unchanged.
  Vector units moved: F-MF2 447,082,855; F-W5 / -quantize / -meshopt 202,075,102 / 198,307,104 /
  207,876,074; F-W4 11,323–99,955. Limit pairs 5/5: F-MF2 `containmentWorkUnits` 447,082,855 / −1
  `workLimit`, `containmentSpanBits` 290 / 289 `numericSpan`, `containmentBytesRead` 59,330,992 / −1 (stops at
  reference 12,526), F-W6d `containmentDecodedBytes` 57,736 / −1, repeated-URI 20,562 / 20,561. Controls: 80
  cases, `--check` 0 mismatches. 13 forest-derived rows change hash (F-W1, F-W4-polygons,
  F-W5-vec-perf-quantize, S3-2 ×6, S3-3 ×2, S3-6, S3-7), and only their totals move: outcome, Outside volume,
  depth, member and vertex are unchanged. `check.py`: 80 + 181, 0 disagreements. Oracle mutants 15/15, and
  the unmutated copy flips nothing.
- **Production parity** (release lib tests, large and evidence files present): slice-1 261/261, C2-WB
  102/102, slice-3 controls 80/80, S3-18 sweep 181 cases with 0 disagreements (R5 injection-only, plus the
  2 `references` pairs), forest 34/34 and the 5 pairs, I1 181/181, W corpus 41/41.
- **Production mutants:** 36/36 at `f661e4e`, with patches byte-identical to the 2026-10-12 run. Every
  slice-3 mutant flips its paired control. The clean tree flips nothing. All 19 slice-1 mutants flip
  cases. Every mutant flips exactly the same case set as at `8c7bc9f`. Driver fix:
  `mutants.py` counted the `#[ignore = "…"]` status `ignored, perf helper: …` as a failed test, so it
  marked `clean` failed even though the clean run had 7 passed, 1 ignored and 0 failed. It now accepts any
  `ignored` status. Only the clean row's pass bit depended on this.
- **X3-2:** `tests/native_validate.rs::point_archive_and_feature_attributes_use_valid_core_types` passes,
  unchanged. `cargo test --features native-geospatial`: 1,019 passed, 0 failed, 11 ignored. The native
  vector and point oracles pass, and so does clippy `-D warnings` (native-geospatial). The local CI replay of
  job `rust` passes every step (`steps.txt`): it now also runs the tree budget, the perf-lane controls and
  the H1 tree oracle, the steps develop added. Native-jpeg tests: 905 passed, with the 82-execution
  receipt. `cargo package` also passes.
- **Perf lane** (`bench/perf_lane/process.py`, base = develop `a4f14e6` release `native-geospatial` (sha256
  `53c62f90…`), head = `2d85208`, 5 ABBA rounds on P-core pair 6,7, no contended best-wall sample). All
  producer outputs are identical to develop's except the volatile vector diagnostics, so Cesium was not
  re-run. The validate rows:

  | Case | develop s | head s | RSS develop / head MiB | Flag |
  | --- | --- | --- | --- | --- |
  | validate:vector (implicit) | 0.094 | 1.127 | 87.3 / 86.9 | wall ×12.0 |
  | validate:vector-explicit | 1.134 | 1.138 | 92.5 / 92.5 | |
  | validate:point-cloud (implicit) | 0.058 | 0.191 | 87.3 / 87.4 | wall ×3.3 |
  | validate:point-cloud-explicit | 0.288 | 0.291 | 88.9 / 88.8 | |
  | validate:mesh (implicit) | 0.044 | 0.099 | 84.3 / 86.4 | (+0.055 s, under the floor) |
  | validate:mesh-explicit / mesh-local / glb-wrap | 0.082 / 0.211 / 0.060 | 0.082 / 0.208 / 0.061 | ≤ 88.9 / ≤ 88.8 | |

  The two wall flags are the §10 item 5 case: implicit archives now get the containment that their
  `--explicit` twins already had, and the implicit vector cost (1.127 s) matches its twin's (1.138 s). No
  RSS flag. Forest (`perf_validate.py`, develop vs head, same corpus): mf2-lod3 0.837 → **3.295 s**,
  100.4 → **104.5 MiB** (ceiling 4.5 s, 116.9 MiB). W5 ×3: 0.09–0.11 → 1.10–1.16 s. W2: 0.083 → 0.387 s.
  W3: 0.178 → 0.329 s. The other implicit archives grow by at most +0.029 s. Every explicit twin is
  within ±0.01 s. Per-class ns/unit and the precheck-only arm were not re-measured (validator unchanged).

| Evidence (`/home/bend/.cache/c2s3-evidence/`) | sha256 |
| --- | --- |
| `forest-2d85208/produce.json` | `99b4452aba473ade3656a04d52760bf4abd91553ac8d0bee36f7364d14c76156` |
| `r2/parity/{c2,c2wb,s3,sweep,forest,i1,i1-corpus}.json` | `877fa29e…`, `acd8ab87…`, `b68dec3e…`, `49bb69c6…`, `dd82e78a…`, `f0eaea4a…`, `d22fb7c5…` |
| `r2/mutants/report.json` (36 rows, at `f661e4e`) | `443b9566fd2f4d10b6f8513e74f4708f44bd97bf781034c11e4f6c89455fa6af` |
| `r2/perf/process.json` (perf lane, develop vs head) | `60b9f471e882f5725122f4993feb4e33a2c5556710b8710de178953129d94696` |
| `r2/perf/forest.json` (forest, develop vs head) | `921e0d03f8b880bce41e31d86fb2017c8a4d63e448d92495adb58ad64a1c517a` |
| `r2/native-geospatial-tests.log`, `r2/ci/steps.txt` | `75b7c2be…`, `86dd723f…` |

### Slice-3 acceptance, §9.F adapters and final-audit record (R, 2026-10-13)

**Report parity (contract §9.F).** At head `448fdfc98ae816938d13f6368d20cf18a9f64b9f`, `rusty-tiles validate --json`
(release CLI, sha256 `e217d5584c7a319c0a3901202e66cc2cf6a98503df2cf61eaf17196a6623a81f`) and
`rusty_tiles.validate` from an installed wheel were run on **194 archives**: the 36 forest-2d85208
archives, the 16 `slice3/` controls and the 142 `witnesses/fixtures/*.3tz` explicit C2 controls. The CLI JSON
object and the Python dict were compared key by key, and error cases by code (`invalid_input` / `DataError`,
`resource_limit` / `ResourceLimitError`, `unsupported` / `UnsupportedError`) and message. **0 differences**; the
contract allows none. 71 archives report (35 forest, 8 slice-3, 28 witnesses) and 123 refuse, 114 of them with the
same Outside message in both adapters. Archives with their sha256 (full list of 194 in `f1/archives.sha256`):

| Archive | Kind | sha256 |
| --- | --- | --- |
| `forest-2d85208/W1.3tz` (F-W1) | Contained report | `ffb3eccbab629f34e18a4b111eb1e9d18211b8d6cedd6c35953f40a893d2c900` |
| `forest-2d85208/W5-vec-perf.3tz` (F-W5) | Contained report | `b67c411cddac63738896fc1049c593630e33078789f6724399c614dd9d10868a` |
| `forest-2d85208/W5-vec-perf-quantize.3tz` (F-W5) | Contained report | `057a79103f1bbce10bbb25c7bd67337b32163b29090a4ecf9abd705f58b7780b` |
| `forest-2d85208/mf2-lod3.3tz` (F-MF2) | Contained report | `2c526cbcb6171fe107b8f409db31139d74f922dd92d412a88e1d7745f9dede87` |
| `forest-2d85208/W6.3tz` (F-W6) | Contained report | `751e6fbbe86cb18b1f7c71135edc72b307db55bb054aae93945de478f34187dd` |
| `forest-2d85208/W6d.3tz` (F-W6d) | Contained report | `9af81d4242850b5dd3c4601d596a69cf60b6f36592e8b04030483d64f9d1cbe0` |
| `forest-2d85208/mf2-lod3-explicit.3tz` (twin) | `resource_limit` ("JSON exceeds value-node limit"), same in both | `91ba9ce90fb3cb8c884559af9d331a7a7403c766b4d4fb958d782300d226f17b` |
| `slice3/S3-1.W6.3tz` | Outside(own), same message | `a9a4812ce850f51f…` (full in `f1/archives.sha256`) |

**Installed wheel.** `maturin 1.15.0 build --release --locked` at the head produced
`rusty_tiles-0.4.0-cp310-abi3-linux_x86_64.whl`, sha256 `9b9e5ece4f7b4683bcf683bb6648324392d4a2f679cc19cb9e4e04d2b3c8ae04`.
It was installed into a fresh venv, and the parity run above used `python -I` there. `scripts/test_python_wheel.py`
(clean venv, empty PATH) passed: 45 tests, OK. The wheel therefore includes #176's meshopt header check, which
the earlier `final/wheels` build did not.

| Evidence (`/home/bend/.cache/c2s3-evidence/f1/`) | sha256 |
| --- | --- |
| `compare.json` (per-archive CLI and Python result, differences) | `b05c0132ea95bed5a348bb966e14fce3fb03e72f6ba3258b258c500f44e6b7a5` |
| `archives.sha256` (194 archives) | `0d2eefe00dcd9a0a185bcd5bc2ee1ace24b2decb9916053cc081ffd29740e2d3` |
| `wheel-test.log` | `97b0904a93431af64eedc1eedae17084c438d4ab72e21d10d52c637f8c867ac9` |
| `maturin.log` | `6943e0a695b9a44047fb1ee62465c31440e6bed30234ada3b40d058aa36522a3` |

**Known limits.** A 44.6 KB payload-free implicit archive (one QUADTREE subtree, 262,139 available tiles, no
content) peaks at 157 MiB RSS under `validate` at head versus 62 MiB on develop, because slice 3 registers one
376-byte `TileFact` per visited implicit tile. This is bounded by `hierarchy_visits`, fallible and allowed by
§5.4. Registering implicit tile facts lazily or compactly is a follow-up.

## H11: corner fast path (2026-10-13)

H11 skips exact per-vertex walks that a corner test proves Contained (contract
[H11 amendment](../../../docs/architecture/c2-decoded-containment-contract.md#h11-amendment-corner-fast-path-2026-10-13)).
Plan with invariants V1–V8 and the soundness argument, written before the code and amended after the
first measurements: `~/.cache/rusty-tiles-evidence/h11-20261013/plan.md`. Bulk evidence is in that
directory.

**Controls (`h11/`).** `python3 -I h11/generate.py` writes 7 pairs with the witness builder and records
the unmodified oracle's report for each; `--check` re-derives hashes and outcomes. Production must agree
(`production_matches_every_h11_fast_path_control`, 14/14). Every pair targets fast-path mutants: an
off-diagonal sphere escape (`h11-sphere-offdiagonal`), a rotated node over a box chain
(`h11-rotated-box`), a unique maximum one f32 ulp past the face as the last vertex
(`h11-ulp-last-vertex`), a u16 maximum (`h11-u16-maximum`), viewless zero words under declared bounds
inside the volume (`h11-declared-bounds`), an escape in the third chunk (`h11-third-chunk`) and an
all-inconclusive but contained sphere (`h11-inconclusive-sphere`).

**Parity** (release lib tests, `--large` C2, C2-WB and I1 fixtures generated, `C2S3_EVIDENCE_DIR` and
`C2S3_FOREST_DIR` set as in the slice-3 record): PARITY_ROW.

**Production mutants** (`acceptance/production-mutants/h11/mutants.py`, release, paired control or
test):

| Mutant | Paired | Also flips |
| --- | --- | --- |
H11_MUTANT_ROWS

The 36-row slice-1/slice-3 matrix (`slice3/mutants.py`, placement now overridable with `C2M_*`):
S3_MATRIX_ROW

**Performance** (release `native-geospatial`, develop `7fad70c` vs H11; best wall of 5 clean runs on a
quiet P-core, peak RSS within ±1 MiB everywhere):

| Archive | develop s | H11 s | before slice 3 s |
| --- | --- | --- | --- |
| mf2-lod3 | 3.380 | **1.646** | 0.837 |
| W5 vec-perf / quantize / meshopt | 1.145 / 1.198 / 1.209 | 0.147 / 0.150 / 0.182 | 0.09–0.11 |
| W2 | 0.396 | 0.120 | 0.083 |
| W3-pc-deep | 0.337 | 0.308 | 0.178 |
| W5 / W2 / W3 `--explicit` | 1.144 / 0.761 / 0.342 | 0.152 / 0.116 / 0.231 | (C2 since slice 1) |
| Perf lane validate: vector / vector-explicit | 1.149 / 1.170 | 0.147 / 0.150 | 0.094 (implicit) |
| Perf lane validate: point-cloud / -explicit | 0.198 / 0.294 | 0.067 / 0.070 | 0.058 (implicit) |
| Perf lane validate: mesh-local / mesh / mesh-explicit / glb-wrap | 0.218 / 0.103 / 0.085 / 0.062 | 0.065 / 0.052 / 0.051 / 0.046 | |
| Adversary A1 (1.2M axis points on the unit sphere, every test inconclusive) | 0.370 | 0.370 | |
| Adversary A2 (3,000 payloads of 9–20 such points; the credit binds) | 0.147 | 0.148 | |

Every other forest archive and twin moves by at most 0.01 s. On mf2-lod3 the exact walk falls to 8.8% of
samples; what remains of the slice-3 growth is the phase-2 C1 re-inspection (stream step 1), outside H11.
The adversaries (and their `gltfUpAxis` twins, unchanged) show no measurable cost when every corner test
fails. The value-identical hoist of per-accessor constants was not done (plan amendment 3).

## What remains

- Production verdicts for `witnesses/report.json`, `controls/manifest.json`
  and `corpus/manifest.json`, once M and F land. Each Outside row needs a
  recorded disposition. The corpus has none.
- Done (2026-10-12): R's measurement of the cap value (C2-WB WB-6): 2^29, from
  `τ_E = 14.65` ns/unit (`acceptance/README.md`, "C2-WB gate"). The work controls and the
  `wb/` at-cap pairs are pinned to the oracle's computed totals, so they must be
  regenerated if a formula reconciliation changes `E`, `B`, `Dec`, `J`, `Res`
  or `Prim`.
- Done (2026-10-12): slice-3 production verdicts, S3-18, the forest witnesses and the limit pairs at the
  slice-3 integration head (R, below).
- Done (2026-10-13): the same re-run after merging develop `a4f14e6` (#183 point bounds, #185 vector
  output), with the X3-2 native test passing (R, "Re-run after merging develop").
