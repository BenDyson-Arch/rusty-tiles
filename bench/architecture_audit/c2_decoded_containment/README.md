# C2 decoded-content containment: O-explicit oracle (Slice 0a)

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

## Replay

From this directory:

```sh
python3 -I oracle/self_test/run.py            # 36 checks, exit 0
python3 -I witnesses/generate.py --large      # 255 cases, 15 control pairs; exit 0 when all agree
python3 -I oracle/mutants/run.py              # 12 mutants, each must flip its named fixture
python3 -I corpus/run.py CORPUS_DIR           # CORPUS_DIR = R's run2 directory (manifest.json + artifacts/)
python3 -I oracle/c2_oracle.py some.3tz --json
```

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

- **Self-test: 36/36 pass.** The checks are:
  - the upstream golden vectors: TRIANGLES `kIndexDataV1`, INDICES
    `kIndexSequenceV1` and ATTRIBUTES `kVertexDataV0`;
  - the spec's 4-bit sentinel and varint examples;
  - the worked example, bit-exact: `sInst = 5`, level spans `[23, 29]`,
    8 vertex units and 128 node units;
  - identity-node pair equivalence;
  - pair soundness: about 98k exact quantities over random chains, with 0
    values outside their pair;
  - f32 exactness, `R(q)` exact orthogonality, the quaternion window boundary,
    and the integer predicate against the `Fraction` predicate.
- **Witnesses and controls: 255 cases, all agree.** No expectation mismatches.
  - 14 literal (W01–W13, plus the W01 no-RTC variant), 20 context and 45
    control cases.
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

    | Judgement | Cases |
    | --- | --- |
    | `likely` | 1: `child-volume-escapes-but-content-inside` |
    | `possible` (escape of at most 4 ulps; depends on C1's slack) | 152 one-ulp mutants |
    | `possible` (only if C1 compares an external root with the referencing content volume) | 1: `external-content-volume-excludes` |
    | none | the other 101 |
- **Mutants: 12/12 flip their named fixture.**
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
   | `B` | Member bytes plus each distinct resolved external buffer or image member, counted once |
   | `Dec` | External and data-URI buffer bytes plus the decoded `byteLength` of **every** meshopt-compressed view, used or not |

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
14. **Index-typed node fields written as floats** (for example `"mesh": 0.0`)
    are InvalidInput(node).
15. **A tile `transform` whose last row is not `0,0,0,1`, or that is not 16
    finite numbers,** is InvalidInput by v3.3 (8). The oracle used to report
    `Undecided` here; it now reports `InvalidInput`. A tileset number literal
    that overflows binary64 is also `InvalidInput` now. Controls:
    `tile-transform-non-affine` and `tileset-number-overflow`.
16. **An overflowing number literal in glTF JSON** gives `InvalidNode` with a
    number reason. The contract makes it InvalidInput, but C1 might report it
    first.
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

## What remains

- Production verdicts for `witnesses/report.json`, `controls/manifest.json`
  and `corpus/manifest.json`, once M and F land. Each Outside row needs a
  recorded disposition. The corpus has none.
- R's measurement of the cap values. The work controls are pinned to the
  oracle's computed totals, so they must be regenerated if a formula
  reconciliation changes `E`, `B` or `Dec`.
- O-implicit (Slice 0b), for implicit coverage.
