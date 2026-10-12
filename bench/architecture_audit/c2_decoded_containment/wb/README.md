# C2-WB class fixtures (O-WB)

This directory is O-WB's part of the C2-WB work-bound correction (contract draft v2.1,
`/tmp/rusty-tiles-c2wb-contract-20261012/`, sections 3.1, 3.2, 3.6, 5, 5.1 and 9.A1/9.B):

- one or more fixtures per class of the calibration matrix, at a CI size (committed)
  and at the matrix size (`--large`, generated into `large/`, not committed);
- one identity fixture per C2-WB weight;
- the phase-1 chain-bit controls;
- a reading probe for the one contract question the fixtures expose;
- the expected C2-WB unit total of each, which is the source of truth for every
  at-cap pair.

## Provenance and independence

- **Inputs read.**
  - The C2-WB contract package: `contract-draft.md`, `ownership-plan.md` and
    `change-log.md` (v2.1; `sha256sum -c SHA256SUMS` passes).
  - The existing C2 oracle (this directory).
  - The delta reviews `delta/review-delta.md` and `delta2/review-delta2.md`, with their
    `shapes.py` and `viewless.py`. The reviewer's shapes are re-implemented here, not
    imported.
  - The C1 contract's definition of a JSON value node
    (`docs/architecture/c1-payload-integrity-contract.md:201`).
- **Not read.** `src/`, R's `probes/`, and the J, S and F branches.
- **Pins.** See the main [README](../README.md), section "C2-WB amendment".
- **Two implementations of the formula.**
  - `hand.py` is a second implementation of 3.1/3.2 that does not import `c2o`.
  - For each fixture, `generate.py` derives `T`, `Idx`, `E`, `Dec`, `B`, `J`, `Res`,
    `Prim` and the node visits from the bytes it wrote, using stdlib `json` only.
    The tileset construction gives the per-level operation counts.
  - Every fixture states its span classes `q = ⌈s/64⌉²` by hand (derivations below).
    The two exceptions, `nodes-rotated` and its large twin, use generic quaternions;
    there the B1 spans are read from the oracle and everything else is applied by hand.
- **What a fixture must satisfy.**
  - The oracle's total equals the hand total.
  - Every per-URI fact agrees term by term.
  - Every hand-stated `q` equals the oracle's.
  - Every level's operation counts equal the construction.
- **Run conditions.** Pure Python 3 stdlib; tested with CPython 3.14.7. All runs were
  single-threaded under `taskset -c 0,1 nice -n 10`.

## Replay

From `bench/architecture_audit/c2_decoded_containment/`:

```sh
python3 -I wb/generate.py            # CI fixtures + at-cap pairs + worked-example identity; exit 0 when all agree
python3 -I wb/generate.py --large    # also the matrix-size fixtures into wb/large/ (5-6 min wall, about 260 s CPU here; 99 MB)
python3 -I wb/mutants.py             # C2-WB formula mutants against the CI at-cap pairs
python3 -I oracle/c2_oracle.py wb/large/e2e.3tz --precheck-only --no-detail \
        --limit containmentWorkUnits=4611686018427387904   # units of one large fixture
```

- **Determinism.** Fixtures are stored ZIPs with fixed timestamps and seeded content,
  so they are byte-reproducible. `manifest.json` pins every file's sha256, the large
  ones included.
- **Partial runs.** Without `--large`, the generator keeps the matrix-size rows of
  the existing manifest unchanged, so a CI replay leaves `manifest.json` byte-identical.
- **How large fixtures run.** They use the oracle's `--precheck-only` mode: units and
  facts only, no streaming. Their verdict is Contained by construction: every vertex
  lies inside every volume, as the oracle's full run shows for the CI twin. A Contained
  large row therefore reports `PrecheckPassed`.

## Manifest fields

`manifest.json` has one row per fixture and size. Each row holds:

- `units`: the expected C2-WB total, equal to `oracleUnits`.
- `terms`: the oracle's breakdown into vertex `T·V`, `Nodes`, `Idx`, `E`, `Dec`, `B`,
  `16J`, `160Res`, `4Prim`, `1152` and `8levels`.
- `payloadFacts`: the hand per-URI facts.
- `referencesHand`: per reference group, the count, levels, `q` per level, `V` and the
  units per reference.
- `at2^28` / `at2^29`: the verdict at that default cap.
- `capPair`: CI rows only. With `containmentWorkUnits = units` the verdict is
  Contained; with `units - 1` it is NotInspected(workLimit).

`workedExample` holds the C2-WB 3.6 identity on the committed slice-1 fixture
`controls/fixtures/worked-example-adjusted-containmentSpanBits-29.3tz`:

```
400 + 256 + 4 + E 12 + Dec 0 + B 652 + 16·J 56 + 160·Res 0 + 4·Prim 1 + 1152 + 16 = 3,392
```

The hand facts come from the fixture's bytes, and the oracle gives the same total.

## Span classes (hand derivations, B1)

Every fixture uses volumes centred at the origin with power-of-two half-lengths or
radii, except the large-span classes. Every transform is the identity matrix, so a
step leaves the pairs unchanged.

| `q` | Where | Derivation |
| --- | --- | --- |
| 25 | f32 POSITION, box levels | The word pair is `(-149, 127)`, span 277. A zero centre adds nothing. A one-term translation or centre gives `(-149, 128)`, span 278. Both are 5 limbs, so `q = 25` |
| 81 | f32 POSITION, any level with a sphere | `d = P` (span 277). `d·d = (-298, 255)`. The three-term sum is `(-298, 257)`, span 556: 9 limbs, so `q = 81` |
| 1 | u16 (unnormalized) POSITION (`terms-*`) | `P = (0, 15)`. Box sums and sphere squares stay at or below span 36 |
| 64 | `large-span-box-8-limbs`, box centre `2^320` | `(-149,127) + (320,320) = (-149, 321)`, span 471: 8 limbs |
| 256 | `large-span-sphere-16-limbs`, sphere centre `2^340` | `d0 = (-149, 341)`. `d0² = (-298, 683)`. The sum of 3 is `(-298, 685)`, span 984: 16 limbs |
| node: 1 | dyadic translations, unit matrix or scale | `s_step ≤ 64`, so the visit is `max(256, 128) = 256` |

## Formula mutants (`mutants.py`)

`mutants.py` writes `mutants-report.json` (gitignored; the 2026-10-12 run is kept outside the
repository, sha256 `0195cdf1b7ca2d40ff7fc28ffaa6a91918851bfd823387d7d31fc87bfda13826`).

These are the unit-arithmetic subset of contract 9.C, applied to a copy of `c2o` and
scored on the 55 CI at-cap pairs. Each must flip its named pair.

| Mutant | Named pair | Pairs flipped |
| --- | --- | --- |
| frozen vertex weights (`units = q`) | `box-chain-L16` | 55 |
| drop the step term | `box-step-L16` | 5 |
| sphere floor = box floor | `terms-content-sphere` | 2 |
| drop the level-0 base | `terms-base` | 55 |
| drop `R0` (1152) | `tiny-refs-L4` | 55 |
| drop `B` | `json-flat-string-glb` | 55 |
| drop `Dec` | `meshopt-attributes-raw` | 9 |
| drop `J` | `unselected-nodes` | 55 |
| drop `Res` | `extbuf-20` | 4 |
| drop `Prim` | `shared-accessor-160x20` | 55 |
| node floor 128 | `terms-node` | 4 |
| drop `8·(L_r+1)` | `tiny-refs-L103` | 55 |
| `J` without the b3dm batch table | `json-flat-numbers-b3dm` | 6 |
| `Res` per distinct member (reading probe) | `reading-res-duplicate-uri` | 1 |

All 14 pass. The production mutants that concern performance (re-scanning JSON
admission, quadratic `mark`, `build_levels` before the gate) are R's perf controls,
not unit pairs.

## Chain-bit controls (C2-WB 3.3; verdict only)

| Fixture | Cap | Expected |
| --- | --- | --- |
| `chain-worklimit-then-rotated-content` | `containmentWorkUnits = 100`; reference 0 alone exceeds it | NotInspected([rotatedBox]): the content-volume exclusion of reference 1 overrides the earlier workLimit |
| `chain-worklimit-base` | same archive with an axis-aligned content box, same cap | NotInspected([workLimit]) |
| `chain-region-ancestor` | `containmentWorkUnits = 1`; region-only hierarchy (C1-passing), regions two levels up | NotInspected([regionVolume]) |
| `chain-external-rotated-content` | `containmentWorkUnits = 1`; external tileset reached through a rotated content volume | NotInspected([rotatedBox]) |

## Readings and contract ambiguities

1. **`Res` with a repeated URI.**
   - The contract says "number of external resources C1 resolves …: buffers and images
     with a `uri` (each is a `reference()` plus a member read)". It does not say whether
     two entries that name one member count once or twice.
   - O-WB counts **entries** (`Res = 2` in `reading-res-duplicate-uri`). A per-member
     count gives 160 units less per reference; the mutant
     `Res-counts-per-distinct-member` shows the probe separates the two.
   - `Dec` counts each owned buffer (slice-1 model, unchanged).
   - **Resolved by R against production (2026-10-12).** C1 calls the resolver once per
     entry, and each call reads the member, so production counts `Res = 2` (this
     reading) **and** `B` once per entry (`B = 435`, not the slice-1 oracle's
     member-once 423). The oracle (`c2o/gltf.py`) and the hand facts now count `B` per
     resolved entry, matching "archive bytes read during C1 inspection"; the probe's
     total is 5,602.
2. **`data:` URIs are not `Res`.** They need no member read; their decoded bytes are in
   `Dec`. Image data URIs are C1-refused.
3. **`J` counts the root and only nonempty tables.**
   - The root counts, as the C1 contract says (`{}` is 1 node).
   - A b3dm table with JSON length 0 contributes no document.
4. **Per-reference terms apply to every reference**, including references to a URI with
   no drawn instance. There `T·V = 0`, but `1152 + 8·(L_r+1) + B + Dec + 16J + 160Res +
   4Prim + Nodes + Idx + E` is still charged. The contract's per-reference formula says
   so; slice 1 only skipped the span check (v3.2 (4)).
5. **"16 levels"** in the matrix names (`extTS-L16`, `sphere-chain-L16`, `box-*-L16`) is
   read as 16 levels (`L_r = 15`). `extTS-L32` has 32.
6. **The delta review's per-vertex `V = 120`** for its 4-level f32 box shapes is 125
   under the formula. f32 spans are 277–278, so `q = 25` exceeds the level-0 base floor
   20, giving `(25 + 25) + 3·25`. The reviewer's ns/unit figures for those shapes are
   about 4% high on the vertex share. This does not affect the contract's own
   calibration, which R recomputes from O-explicit.
7. **The C1 admission of the large shapes was not checked** (no production run by
   O-WB). R's production check (2026-10-12) found every multi-reference fixture
   refused by C1's tileset schema: the identical sibling leaf tiles violate the
   `children` `uniqueItems` rule. `kid_layout` now gives leaf `i` the geometric error
   `i·2^-24` (leaf 0 keeps 0, so single-reference fixtures are byte-identical); units
   are unchanged. After that, C1 admits every CI and matrix-size fixture
   (`acceptance/oracle_parity.rs`, `production_matches_every_c2wb_class_fixture`).
   Assumptions made:
   - every JSON document stays at or below 131,072 value nodes, 8 MiB and depth 64;
   - per-payload components stay at or below 4M;
   - tile chains are split across external tilesets every 25 tiles to respect JSON
     depth.

   Two shapes are new relative to the reviewer's, so R should confirm C1 passes them
   first: the INDICES and bits-mode meshopt streams (written here from the spec and
   decoded by the oracle's pinned decoder), and the long-significand literals.

## Classes and expected units

### Section 5 / 5.1 matrix classes

CI units are the at-cap values: `containmentWorkUnits = units` gives Contained and `units - 1` gives workLimit. Matrix-size units come from `--large`. The last column gives the verdict at the 2^28 / 2^29 default caps (Contained by construction).

| Class (contract row) | Fixture | CI shape | CI units | Matrix shape | Matrix units | 2^28 / 2^29 |
| --- | --- | --- | --- | --- | --- | --- |
| two spheres + step per level, 16 levels (extTS-L16) | `extTS-L16` | external chain, two spheres and a step per level | 252,189 | external chain, two spheres and a step per level | 78,082,345 | Contained / Contained |
| two spheres + step per level, 32 levels (extTS-L32) | `extTS-L32` | external chain, two spheres and a step per level | 501,149 | external chain, two spheres and a step per level | 155,842,473 | Contained / Contained |
| sphere chain, 16 levels | `sphere-chain-L16` | nested tiles, payload at the leaf | 91,485 | nested tiles, payload at the leaf | 27,862,345 | Contained / Contained |
| box + step, 16 levels | `box-step-L16` | nested tiles, payload at the leaf | 59,357 | nested tiles, payload at the leaf | 17,822,345 | Contained / Contained |
| box chain, 16 levels | `box-chain-L16` | nested tiles, payload at the leaf | 30,557 | nested tiles, payload at the leaf | 8,822,345 | Contained / Contained |
| own + content sphere, 1M vertices | `own-content-sphere-1M` | one tile | 261,221 | one tile | 259,002,229 | Contained / Contained |
| one box, 1M vertices | `one-box-1M` | one tile | 68,221 | one tile | 66,002,229 | Contained / Contained |
| large spans (box 8 limbs) | `large-span-box-8-limbs` | one tile | 11,429 | one tile | 14,402,229 | Contained / Contained |
| large spans (sphere 16 limbs) | `large-span-sphere-16-limbs` | one tile | 36,005 | one tile | 52,802,229 | Contained / Contained |
| translation-only / rotated nodes | `nodes-translation` | 50 translation-only nodes x 2 references | 58,774 | 15000 translation-only nodes x 20 references | 164,297,100 | Contained / Contained |
| translation-only / rotated nodes | `nodes-rotated` | 50 rotated nodes x 2 references | 103,934 | 15000 rotated nodes x 20 references | 299,925,900 | workLimit / Contained |
| 10 tiny references, depth 1 | `tiny-refs-L4` | 10 references to a one-vertex payload, 4 levels each | 23,620 | 100000 references to a one-vertex payload, 4 levels each | 236,200,000 | Contained / Contained |
| 10 tiny references, depth 100 | `tiny-refs-L103` | 10 references to a one-vertex payload, 103 levels each | 56,290 | 100000 references to a one-vertex payload, 103 levels each | 562,900,000 | workLimit / workLimit |
| 20,000 references to a W3 payload (own mid-size payload) | `refs-to-mid-payload` | 20 references to a 1,000-vertex, 3,000-index payload | 3,097,680 | 20000 references to a 1,000-vertex, 3,000-index payload | 3,097,680,000 | workLimit / workLimit |
| unselected {} nodes, 60,000 x 20 references | `unselected-nodes` | 600 unselected {} nodes x 2 references | 27,524 | 60000 unselected {} nodes x 20 references | 22,847,240 | Contained / Contained |
| selected skipped nodes, 30,000 x 20 | `skipped-nodes` | 300 selected skipped {} children x 2 references | 28,564 | 30000 selected skipped {} children x 20 references | 25,025,720 | Contained / Contained |
| external buffers, 2,000 / 20,000 x 20 | `extbuf-20` | .gltf with 20 external buffers (one 12-byte, the rest 4-byte) x 2 references | 14,516 | see below |  |  |
| external buffers, 2,000 / 20,000 x 20 | `extbuf-200` | .gltf with 200 external buffers (one 12-byte, the rest 4-byte) x 2 references | 104,356 | see below |  |  |
| primitives sharing one accessor, 16,000 x 40 / x 200 instances | `shared-accessor-160x4` | 160 POINTS primitives on one accessor x 4 instances, one reference | 22,495 | see below |  |  |
| primitives sharing one accessor, 16,000 x 40 / x 200 instances | `shared-accessor-160x20` | 160 POINTS primitives on one accessor x 20 instances, one reference | 38,299 | see below |  |  |
| JSON: flat dense numbers GLB / b3dm, 1 MiB flat string | `json-flat-numbers-glb` | numbers-glb, 1200 numbers x 5 references | 119,950 | numbers-glb, 120000 numbers x 50 references | 108,119,500 | Contained / Contained |
| JSON: flat dense numbers GLB / b3dm, 1 MiB flat string | `json-flat-numbers-b3dm` | numbers-b3dm, 1200 numbers x 5 references | 120,430 | numbers-b3dm, 120000 numbers x 50 references | 108,124,300 | Contained / Contained |
| JSON: flat dense numbers GLB / b3dm, 1 MiB flat string | `json-flat-string-glb` | string-glb, 10240 characters x 5 references | 63,150 | string-glb, 1048576 characters x 50 references | 52,548,300 | Contained / Contained |
| 1 MiB JSON payload x 1,000 references | `json-gltf-1MiB-refs` | .gltf with a 10240-character extras string and a data-URI buffer x 10 references | 126,790 | .gltf with a 1048576-character extras string and a data-URI buffer x 1000 references | 1,051,015,000 | workLimit / workLimit |
| deep string (GLB extras), depth 60 | `json-deep-string-glb-60` | string nested 60 deep in glb x 5 references | 68,550 | string nested 60 deep in glb x 50 references | 52,602,300 | Contained / Contained |
| deep string (b3dm batch table), depth 60 | `json-deep-string-b3dm-60` | string nested 60 deep in b3dm x 5 references | 69,030 | string nested 60 deep in b3dm x 50 references | 52,607,100 | Contained / Contained |
| deep numbers (GLB extras), depth 1 | `json-deep-numbers-glb-1` | numbers nested 1 deep in glb x 5 references | 120,030 | numbers nested 1 deep in glb x 50 references | 108,120,300 | Contained / Contained |
| deep numbers (GLB extras), depth 30 | `json-deep-numbers-glb-30` | numbers nested 30 deep in glb x 5 references | 122,650 | numbers nested 30 deep in glb x 50 references | 108,146,500 | Contained / Contained |
| deep numbers (GLB extras), depth 62 | `json-deep-numbers-glb-62` | numbers nested 62 deep in glb x 5 references | 125,530 | numbers nested 62 deep in glb x 50 references | 108,175,300 | Contained / Contained |
| deep numbers (b3dm batch table), depth 1 | `json-deep-numbers-b3dm-1` | numbers nested 1 deep in b3dm x 5 references | 120,550 | numbers nested 1 deep in b3dm x 50 references | 108,125,500 | Contained / Contained |
| deep numbers (b3dm batch table), depth 30 | `json-deep-numbers-b3dm-30` | numbers nested 30 deep in b3dm x 5 references | 123,150 | numbers nested 30 deep in b3dm x 50 references | 108,151,500 | Contained / Contained |
| deep numbers (b3dm batch table), depth 60 | `json-deep-numbers-b3dm-60` | numbers nested 60 deep in b3dm x 5 references | 125,830 | numbers nested 60 deep in b3dm x 50 references | 108,178,300 | Contained / Contained |
| deep numbers (b3dm batch table), depth 62 | `json-deep-numbers-b3dm-62` | numbers nested 62 deep in b3dm x 5 references | 126,030 | numbers nested 62 deep in b3dm x 50 references | 108,180,300 | Contained / Contained |
| one accessor per primitive, 6,000 x 20 (quad) | `quad` | 60 primitives each with its own accessor x 2 instances, one reference | 42,842 | 6000 primitives each with its own accessor x 20 instances, one reference | 18,151,888 | Contained / Contained |
| 5,000 accessors x 16,000 primitives x 40 (quadB) | `quadB` | 50 accessors x 160 primitives (primitive i uses accessor i mod A) x 4 instances, one reference | 62,746 | 5000 accessors x 16000 primitives (primitive i uses accessor i mod A) x 40 instances, one reference | 31,490,820 | Contained / Contained |
| end-to-end, 5,000 x 15,500 x 500 (one reference; the many-accessor case) | `e2e` | 50 accessors x 155 primitives (primitive i uses accessor i mod A) x 5 instances, one reference | 69,220 | 5000 accessors x 15500 primitives (primitive i uses accessor i mod A) x 500 instances, one reference | 354,517,700 | workLimit / Contained |
| viewless accessors, unused 4M-component accessor x 20, SCALAR / MAT4 (5.1) | `viewless-SCALAR` | unused viewless SCALAR accessor, 4000 components x 2 references | 12,956 | unused viewless SCALAR accessor, 3999992 components x 20 references | 80,049,480 | Contained / Contained |
| viewless accessors, unused 4M-component accessor x 20, SCALAR / MAT4 (5.1) | `viewless-MAT4` | unused viewless MAT4 accessor, 4000 components x 2 references | 12,948 | unused viewless MAT4 accessor, 3999872 components x 20 references | 80,047,000 | Contained / Contained |
| long-significand decimals in node matrix/TRS and accessor min/max (5.1) | `long-decimals` | 2 matrix + 2 TRS selected nodes and 10 bounded accessors with 800-character literals x 2 references | 104,636 | 150 matrix + 150 TRS selected nodes and 2500 bounded accessors with 800-character literals x 20 references | 130,909,840 | Contained / Contained |
| meshopt NONE filter, worst mode per decoded byte (5.1): attributes-raw | `meshopt-attributes-raw` | 300 vertices via ATTRIBUTES (raw groups) x 2 references | 97,922 | 1000000 vertices via ATTRIBUTES (raw groups) x 4 references | 612,761,908 | workLimit / workLimit |
| meshopt NONE filter, worst mode per decoded byte (5.1): attributes-bits | `meshopt-attributes-bits` | 300 vertices via ATTRIBUTES (bits groups) x 2 references | 91,882 | 1000000 vertices via ATTRIBUTES (bits groups) x 4 references | 572,989,556 | workLimit / workLimit |
| meshopt NONE filter, worst mode per decoded byte (5.1): triangles | `meshopt-triangles` | 300 vertices via ATTRIBUTES (raw groups), 900 u32 indices via TRIANGLES x 2 references | 112,330 | 100000 vertices via ATTRIBUTES (raw groups), 3000000 u32 indices via TRIANGLES x 4 references | 152,889,636 | Contained / Contained |
| meshopt NONE filter, worst mode per decoded byte (5.1): indices | `meshopt-indices` | 300 vertices via ATTRIBUTES (raw groups), 900 u32 indices via INDICES x 2 references | 111,098 | 100000 vertices via ATTRIBUTES (raw groups), 3000000 u32 indices via INDICES x 4 references | 144,889,508 | Contained / Contained |
| external buffers, 2,000 / 20,000 x 20 | `extbuf-2000` | (CI twin: `extbuf-20`) | | .gltf with 2000 external buffers (one 12-byte, the rest 4-byte) x 20 references | 10,063,560 | Contained / Contained |
| external buffers, 2,000 / 20,000 x 20 | `extbuf-20000` | (CI twin: `extbuf-200`) | | .gltf with 20000 external buffers (one 12-byte, the rest 4-byte) x 20 references | 100,623,560 | Contained / Contained |
| primitives sharing one accessor, 16,000 x 40 / x 200 instances | `shared-accessor-16000x40` | (CI twin: `shared-accessor-160x4`) | | 16000 POINTS primitives on one accessor x 40 instances, one reference | 4,857,579 | Contained / Contained |
| primitives sharing one accessor, 16,000 x 40 / x 200 instances | `shared-accessor-16000x200` | (CI twin: `shared-accessor-160x20`) | | 16000 POINTS primitives on one accessor x 200 instances, one reference | 17,687,759 | Contained / Contained |

### Per-term identities (CI; u16 points, every floor applies)

`terms-base` is 4 u16 vertices on one box tile: `V = 20 + 25`. The tileset-only variants change no payload fact, so their difference from the base is exactly the weight times `T = 4`, plus 8 per added level.

| Fixture | What changes | Units | Δ vs base |
| --- | --- | --- | --- |
| `terms-base` | u16 points, one tile box: V = 20 + 25 | 2,593 | — |
| `terms-content-box` | + content box: +25 per vertex | 2,693 | +100 |
| `terms-content-sphere` | + content sphere: +32 per vertex (sphere floor) | 2,721 | +128 |
| `terms-parent` | + parent box, no transform: +25 per vertex, +8 | 2,701 | +108 |
| `terms-parent-step` | + parent box, child transform: +(30 + 25) per vertex, +8 | 2,821 | +228 |
| `terms-parent-sphere-step` | + parent sphere, child transform: +(30 + 32) per vertex, +8 | 2,849 | +256 |
| `terms-node` | non-skipped node (translation): Nodes 1 -> 256 | 2,940 | +347 |
| `terms-json` | + extras {k: [0, 0, 0, 0]}: J + 6 | 2,713 | +120 |
| `terms-prim` | two primitives on one accessor: Prim 2, Idx 8, T unchanged | 2,701 | +108 |
| `terms-res` | .gltf with one external buffer: Res 1, Dec and B count it | 2,784 | +191 |
| `terms-two-refs` | two references (root and child) to one URI: the per-URI facts twice | 5,294 | +2701 |

### Reading probe and chain bits (CI)

| Fixture | Units / verdict |
| --- | --- |
| `chain-worklimit-then-rotated-content` | NotInspected ['rotatedBox'] at `{"containmentWorkUnits": 100}` |
| `chain-worklimit-base` | NotInspected ['workLimit'] at `{"containmentWorkUnits": 100}` |
| `chain-region-ancestor` | NotInspected ['regionVolume'] at `{"containmentWorkUnits": 1}` |
| `chain-external-rotated-content` | NotInspected ['rotatedBox'] at `{"containmentWorkUnits": 1}` |
| `reading-res-duplicate-uri` | 5,602 |
