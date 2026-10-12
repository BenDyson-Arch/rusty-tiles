# O-implicit: independent oracle for I1 (implicit effective-volume read owner)

Role **O** of the I1 ownership plan. This is the reference that production (A's owner, B's validator step) is judged
against. It is written from the contract and the pinned primary texts only. No file under `src/`, `bindings/` or
`tests/` was read, except `tests/p1_point_oracle.py:279-287` for the Morton/template layout. No other agent's worktree
was read, and rusty-tiles was never run.

The R-owned `implicit_read/README.md` is not written here. O writes only `oracle/`, `witnesses/`, `controls/` and
`corpus/`.

## Layout

| Path | What |
| --- | --- |
| `oracle/i1_oracle.py` | The oracle. Pure Python 3 stdlib (`zipfile`, `json`, `struct`, `fractions`, `hashlib`, `re`), with no repository imports |
| `oracle/self_test/test_oracle.py` | 50 identity checks (list below) |
| `witnesses/fixturelib.py` | Fixture primitives: stored 3TZ writer with index, binary subtree writer, minimal GLB, implicit tileset builder. Independent of the oracle code |
| `witnesses/generate.py` | Materializes every control, witness and resource case O owns and writes `controls/manifest.json` with the expected verdicts |
| `witnesses/check.py` | Runs the oracle on every manifest case in its own `python3 -I` process and compares the results. Writes `controls/oracle-results*.json` |
| `controls/fixtures/` | 162 small archives (2.9 MB), committed |
| `controls/large/` | 9 large archives (171 MB), gitignored, built only with `--large` |
| `corpus/manifest.json`, `corpus/run.py` | Coordinator-supplied producer artifacts, pinned by sha256, with expectations. Writes `corpus/results.json` and `corpus/facts/<id>.json` |

## Replay

All commands are single-threaded and pinned. Run them from `bench/architecture_audit/implicit_read/`:

```sh
P="taskset -c 0,1 nice -n 10 python3 -I"
$P oracle/self_test/test_oracle.py          # {"checks": 50, "failed": []}
$P witnesses/generate.py --large            # 180 cases, 172 files (small + large)
$P witnesses/generate.py --large --check    # every hash equals the manifest (byte-reproducible)
$P witnesses/check.py                       # {"cases": 167, "disagreements": 0}
$P witnesses/check.py --large               # {"cases": 13, "disagreements": 0}
$P corpus/run.py                            # 14 supplied cases agree; 12 witnesses pending
$P oracle/i1_oracle.py [--limit name=value ...] [--exact] [--summary] ARCHIVE...
```

Wall times on the coordinator host: self-test 0.9 s; small generation 0.6 s; large generation 3.4 s; small check
9 s. Large check 6 s in total, of which R7c takes 2.7 s and R8a 0.9 s.

## Inputs (sha256)

| Input | sha256 |
| --- | --- |
| Contract `contract-draft.md` v3.1 (post-freeze amendments: see below) | `9674e60b959eea0a4a985b6beec7a278ec79c08579f6c19e58f5a070aee03496` |
| `change-log.md` including **v3.2 coordinator rulings** | `52b01d2b71436bcae5ecfb005693122559a794020c9d4522af40803a0c6898a3` |
| `ownership-plan.md` v3.1 | `95d41487b514c99e21cdc9f8fa9d5f4d3a96e049b1091ac1c23e85b1aa334ffc` |
| `open-decisions.md` v3.1 | `6a774c48df53c89ed70f1165caf3b54cf0eefaefdef36213dd023a318bbc4f09` |
| ImplicitTiling `README.adoc` (raw line numbers) | `14a37a3425520e46dd266c8afe82d884fdbcd4a8044739e0311198551f5ec672` |
| Metadata `Semantics/README.adoc` | `02ac07db39258c67e702e0b51dcc3411f96c1caee873aeec2894dbad625e85a2` |
| Core `README.adoc` | `b841fade126f8269d50c1c177e32a30acb01b3d59f1d73820984816378222f87` |
| `primary-fetch/ImplicitTiling_AVAILABILITY.adoc` | `c0df021962dd1ff91dcc02f17573d05f4ed57c444805de052f3c282a7a1ea519` |
| `primary-fetch/Metadata_README.adoc` | `4797a8836ca4899129828fcccbaefcedabbf29350ed0a264ccc83fbd1361ceaa` |
| `primary-fetch/…PropertyTable_README.adoc` | `8099715c66f09e2fd8bbd3358e0b0d71f7934a86e4b7c8dfdc2756d8090231d5` |
| `primary-fetch/…propertyTable.property.schema.json` | `6c4ccad8e2b9db2f344928c8bd4cce05bcee081cdd296eb74e7bc262bb37f692` |
| `primary-fetch/…propertyTable.schema.json` | `d84fc875a4746d06c0186c5391666ba3eb5f8a9a48db52d947d249735be7320f` |
| `primary-fetch/…class.property.schema.json` | `628e5642ccf07bd3c0a9a45648d39ed1c88e268d07cca735b4428058f901605b` |
| `primary-fetch/…Subtree_availability.schema.json` | `16da41d074dd3be58da9e0e2004cace966653b5f308228f01580b18e95a8ec2c` |
| `primary-fetch/…Subtree_buffer.schema.json` | `7edf13ac18d798f56f0e73571b062f84f0ea23e8f7aaf2783147ad869398f87a` |
| `primary-fetch/…Subtree_bufferView.schema.json` | `35c06c8d0fe52d8da07e4e96279c585785e12c7aa230e28cc64270f7bd9cca58` |
| `primary-fetch/…subtrees.schema.json` | `815328b35544f5b2c35a8fb8885f06c0e33f235f316bbc89823d21792214ceff` |
| `primary-fetch/…Subtree_subtree.schema.json` | `88bc7ec39bbc88aa8ad54dc968e6e214ed540546a1835d77d0ac34a5a9e7265a` |
| `primary-fetch/…tile.implicitTiling.schema.json` | `ea286e717ad875a1fe2c41b3d44c965f52a90793f7e906e5b43951e9d62af34e` |
| #151 `a2_semantics/octree.json` | `ec78244b10efb9ca564e037f51473b334ed4dbf4e4ece36feb918798e7fcee8b` |
| #151 `a2_semantics/quadtree.json` | `23d26bc05934a203da1505b07cf31a1fd896136bcacc482dd7eaa0dbd9665637` |
| Reviewer `probes/alias_budget.py` / `.out.json` | `1ee5cbeb…` / `5c189653e6f230249765ce881de20af15ce61c0c85bda177d0a568261cb970c3` |
| Reviewer `probes/w2_caps.py` | `b43a71f02ffabe2cc3f463686eb8d3161dfdaed21d26872148d8601a91056673` |
| Reviewer `delta/delta-check.md` | `abd37595108c7583812f75d336117ff9390dc6f813869d1312a7ca1bf5210e85` |
| Reviewer `delta/delta_attacks.py` / `.out.json` | `8e86229419e2…` / `e0680b10376511fd03f27f75286d714ed9338b934d0b0b47570c84151a04ee45` |
| Reviewer `delta/content_index.out.json` | `9a98d371db95595d9ea49fdcc02fa0431341b81a0aa7bff281eb8f0420f376eb` |
| `tests/p1_point_oracle.py:279-287` at `f2ed077` (blob `f66b2d34…`; the 9 lines) | `bac98cee533c53a0b8b2793c0ceb5215982756a2691704a08b0be034e2fe0b95` |
| Coordinator samples (producer `f2238cd`) | in `corpus/manifest.json`; they match `probes/receipt.json` |

The reviewer probes were used only to size controls (R7, R7b, R7c, R8, R9 and W2's window). The probes'
`uri()` transcription was used to settle one point: which characters count as control characters
(`ord < 32` or `127 ≤ ord < 160`). The contract states everything else about `uri()`.

## What the oracle mirrors

- **Root admission**, in the contract's precedence order:
  - fields, scheme, levels;
  - volume kind and root shape;
  - templates in slot order, then the subtree template, each through §3 steps 1–6;
  - template-header `metadata` classes.
- **§3 normalization.** Structural and exact: a template is a list of segments, and each segment is a list of
  `lit`/`var` parts. Static parts are percent-decoded once; `.`, `..` and empty segments resolve from the directory
  of the document holding R. The other steps:
  - the escape guard (per segment, v3.2);
  - JSON detection on the last normalized segment, ASCII case-insensitive;
  - variable survival;
  - the 1,024 B cap.

  The self-test checks normalize-then-substitute against `uri(base, substituted raw)` on 4,000 random templates.
- **Subtree admission.**
  - §1 steps 1–5, with the v3.2 magic rule.
  - §2a steps 1–12 in order:
    - sources deduplicated by `(kind, view, bits)`;
    - `Scan` charged to `total_bytes_read` before any bit pass, with the scan message;
    - one memoized pass per distinct `View`;
    - closed forms for `Const0` and `Const1`;
    - the `P_t` reservation, then the `E` reservation.
  - §5: classification once per `(document, class)`; eager row checks; the DC-8 count rule.
- **Walk.**
  - Depth-first pre-order: a tile, then its contents in slot order, then its children in ascending index. Child
    subtrees are admitted lazily, and `C` is never enumerated.
  - Strict rank `popcount(tile[0..i))`.
  - `Row`, `DerivedRoot` and `Derived` volumes; `Derived` cells are exact `Fraction`s computed from `V_R`.
  - Errors are the row value, else `e_R·2^-l`. A nonzero subnormal at `l ≥ 1` is Unsupported.
  - `hierarchyBounds` and `geometricErrorOrder` are checked against the path parent; `I_0` is checked against R.
  - The live path is released when a subtree completes.
  - Post-walk content rule (`:299`).
- **Accounting.**
  - Visits: one each for `Tileset`, `Node` and `ExitTileset`, plus `P_t` per subtree. W2 on the `f2238cd` sample
    gives **431**.
  - `references`: one per explicit content, external tileset, subtree member and external buffer, plus each
    reserved `E`.
  - Depth: `depth(R) + level`.
  - Live-path bytes: subtree plus external buffers on the current path. Cumulative bytes are reported as well.
  - `total_bytes_read`: member reads plus `Scan`.
  - `member_bytes`.

## Verdicts and record format

- `verdict.kind`: one of `Accepted`, `InvalidInput`, `Unsupported`, `ResourceLimit` (with `verdict.limit`, the
  `ValidationLimits` field name), or `Undecided`.
- `reason` and `stage` are O's own wording. They are not production messages.
- `InvalidState` would be an oracle bug: reservation or emission identities. It never occurred.

**Accepted record.** `tiles` holds the implicit tiles in traversal order. Each entry has these fields:

| Field | Value |
| --- | --- |
| `doc` | member name of the tileset JSON holding R |
| `level`, `x`, `y`, `z` | global coordinates; `z = 0` for QUADTREE |
| `depth` | `depth(R) + level` |
| `parent` | the path-parent level; `-1` for `I_0`, meaning R |
| `volume` | `row:<12 × hex>`, `derived-root`, or `derived:<12 × root hex>;<level>;<x>,<y>,<z>` |
| `error` | hex |
| `refine` | R's effective refine |
| `contents` | `[[slot, member], …]` |

`hex` is the big-endian IEEE-754 binary64 bit pattern as 16 lowercase hex digits, so `struct.pack(">d", v).hex()`.
FLOAT32 rows are widened exactly before encoding. The root hex in `derived:` is R's JSON box.

`tilesSha256` is the sha256 of `json.dumps(tiles, sort_keys=True, separators=(",", ":"), ensure_ascii=True)`. B's
`implicit_tile_facts` seam must emit this form for parity. `--exact` adds `exact.centre` and `exact.halfAxes` as
`Fraction` strings for derived cells; these are excluded from the digest.

`counts` holds:
- `visits`, `referencesCharged`, `bytesReadModelled`, `scanBytes`;
- `subtrees`, `implicitTiles`, `emittedContents`;
- `tiles`: explicit plus implicit, with R and `I_0` counted once;
- `contentReferences`;
- `maxLivePathBytes`, `cumulativeSubtreeBytes`, `maxDepth`;
- `classScans`.

`unreferencedMembers` is informational only (see "Limits").

## Bounded work

The oracle never enumerates `C`, and constant sources are never read. It stops with `Undecided` if any of these is
exceeded: 4,000,000 visited tiles, 8 GiB of bit work, or 4,000,000 emissions (`ORACLE_BUDGET`). Large bitstreams
use sparse set-bit scans or big-integer level passes. R7 runs in 0.4 s, R7c in 2.7 s, and R8, including the 8 MB
normalization, in 0.4 s.

## Cases and agreement

**180 manifest cases.** 162 small and 9 large archives, plus limit variants on shared files.

| Family | Cases | Notes |
| --- | --- | --- |
| A (availability) | 27 | Includes A-12a to A-12d: four double-defect step-order pins |
| B (rows and metadata) | 22 | |
| C (addressing, templates, placement, semantics) | 55 | The 8 C-7 forms are each tested available and never-available |
| D (container) | 18 | |
| T (tolerances) | 7 | |
| R (resources) | 22 | Includes R2's synthetic six-subtree fixture and the R2b caps, R7, R7a, R7b, R7c, R8, R8a (+ no-padding companion) and R8b |
| W9u | 4 | |
| W9′ | 20 | Link-free a2Tile variants of every #151 node, FLOAT64 and FLOAT32 |
| Bases | 2 | |
| Z-payload | 1 | |

Expected verdicts: Accepted 74, Unsupported 48, InvalidInput 46, ResourceLimit 11, Undecided 1.

**Pairing.** 108 cases name a paired base. The manifest records the member-level diff, with the bit Hamming distance
where lengths are equal. One-bit or one-ulp mutants include:
- A-5, A-6, A-7 and A-8 (one availability bit);
- D-3 (one padding bit);
- B-2 (one ulp);
- B-10 (one ulp of `e_R`).

The resource pairs differ by a limit of exactly 1: R1, R1b, R2, R4 and R6.

**Agreement.** All 180 cases agree with the expectations recorded from the contract (`controls/oracle-results*.json`).
Every expected failure was audited to fire at the intended rule and step, not incidentally. Every Accepted fixture
has no unreferenced member.

**Corpus.** Coordinator samples, producer `f2238cd`:
- W2: Accepted; 431 visits, 428 tiles, 234 subtrees; cumulative 246,584 B, maximum live 23,864 B.
- R1 at 431/430.
- R2b at 190,944 and 246,583.
- W3u, W5u and the 4 W9u conversions: all Unsupported.

All agree. **Pending:** W1, W2 (combined head), W3, W4, W5, W6, W6d, W7, W8s, W8m, W10 and **W11**. R supplies the
files and hashes; add them to `corpus/manifest.json` and run `corpus/run.py`.

**Production expectations that differ from O's domain.** O excludes C1, so these are recorded in `production`:

| Case | O | Production |
| --- | --- | --- |
| C-10 (`.glb` member that is tileset JSON) | Accepted | InvalidInput |
| R1b.full (corrupt payload, full visits) | Accepted | InvalidInput |
| R5 (injected rank-table refusal) | Accepted | ResourceLimit under injection only |
| Z-payload (explicit-only archive, outside I1) | Undecided | Accepted |
| W2 with `document_decoded_bytes` < 190,944 | Accepted (live path alone) | refused by C1 |

**Z-payload must be run first.** It checks that production's C1 accepts O's minimal GLB, which every Accepted
control relies on.

## Modelling choices and limits

- **Payloads are opaque.** O checks existence and reference accounting only (C1 is excluded, per RC-7).
- **The explicit skeleton is modelled only where I1 depends on it:**
  - visits, depth and references;
  - box-in-box `hierarchyBounds` with transforms;
  - `geometricErrorOrder`;
  - external tilesets and cycles.

  O is Undecided when a check involves a non-box explicit volume or a scaling transform (core `:407`).
- **Containment.**
  - The tolerance is `1e-6·(1 + ‖container half-axis components‖₂)`, evaluated in binary64.
  - Axis-aligned containers are decided exactly.
  - Other orthogonal containers are decided in binary64, with an Undecided guard band of 1e-9 relative.
  - A derived cell inside a derived parent from the same root holds exactly (self-tested).
- **Error order.** `child > parent + 1e-8·(1 + parent)` is a violation, evaluated in binary64.
- **`total_bytes_read`.** O charges its own reads and `Scan`. Hash/CRC and payload reads are not modelled. If O's
  charges plus 2 × the archive's member bytes could exceed the limit, the verdict is Undecided unless it is already
  `ResourceLimit(total_bytes_read)`.
- **JSON.**
  - Strict UTF-8, no BOM, no NaN/Infinity, and duplicate keys are InvalidInput.
  - `document_items` counts every JSON value, containers included; `json_depth` counts container nesting.
  - Exceeding either is ResourceLimit (C1 is the authority).
- **Unreferenced members.** O does not decide the existing unreferenced-entry check, because exempt members such as
  `conversion.json` belong to production. It reports them. Every O fixture has none in its Accepted cases.
- **Explicit-only archives** are outside I1, so they are Undecided.

## v3.2 coordinator rulings (applied)

All nine rulings were checked against the pinned texts, and none contradicts them.

| Ruling | Oracle | Controls |
| --- | --- | --- |
| A member whose first non-whitespace byte is `{` (BOM allowed) is a JSON-format subtree: Unsupported. Other bad magic is InvalidInput | Applied | D-7, D-7b (Unsupported); D-4.magic (InvalidInput). This replaces O's earlier reading of D-4 as Unsupported |
| `extensions` inside `extras` does not count | Applied | D-11b (Accepted, facts equal to BQ); D-11 (top-level `extensions`: Unsupported) |
| Table-level `offset`/`scale` on an interpreted column is Unsupported | Already the case | B-7t |
| Subnormal derived error applies at level ≥ 1 only | Applied | B-10.root (subnormal `e_R` at level 0: Accepted); B-10.l1 (Unsupported) |
| The escape guard is per segment | Applied | C-7f (`c%/{level}`: InvalidInput, malformed escape) |
| Integer fields must be JSON integers | Already the case | A-9d.float (`6.0`), A-9d.exp (`6e0`), D-2.float (`0.0`): InvalidInput |
| A duplicated semantic of any kind in a class is InvalidInput | Already the case | B-5 (interpreted), B-5b (opaque `NAME`) |
| Rows are checked eagerly at table admission | Already the case | B-9 fires at `§5` of the subtree |
| OD-7 is validation-wide | Already the case | C-11b: the same implicit-root external tileset referenced twice is Unsupported |

## Post-freeze amendments (2026-10-12)

These were requested by the coordinator after acceptance parity on `feat/implicit-forest`, where production was
right. No verdict expectation changed. Only the three named archives and one counter changed; every other fixture
hash is identical, and generation stays byte-reproducible.

1. **C-11b** (sha256 now `06d5f32c…`). The two explicit children were identical JSON objects, so the core tile
   schema's `uniqueItems` made production refuse the archive as InvalidInput before OD-7 could apply. The second
   child's `geometricError` is now 399 instead of 400. Both still reference the same implicit-root
   `ext/tileset.json`, so the control tests OD-7 (Unsupported, repeated subtree member).
2. **Z-payload** (now `08e06a9f…`) and **C-8** (`leaf.glb`, now `1bf5e5c0…`). These explicit payloads placed their
   point without glTF's y-up to z-up rotation `U(x, y, z) = (x, -z, y)` (core `:441`). Once decoded, the point lay
   outside the tile box, and C2 (merged after the freeze) refused Z-payload.
   - `fixturelib.glb_tile_point` now writes the glTF point as `(a, c, -b)`, so that `U` maps it to the tile-frame
     point `(a, b, c)`.
   - C-8 was not refused, because C2 excludes archives with implicit tiles. It is fixed anyway, being the only other
     explicit payload.
   - Implicit payloads keep the raw `glb_point`. Their positions matter to no I1 or C2 check today, but **C2 slice 3
     will need them rotated too**. (Done by amendment 6.)
3. **`contentReferences`** now counts every content entry, external-tileset links included, as production does. A
   new `externalTilesetLinks` count shows the link part. This changes counts only, no verdicts. Examples: C-11
   goes 10 → 11, C-11b → 12, T-3.base → 1. The corpus facts (`corpus/facts/*.json`, tile lists) did not depend on it.

4. **Document-level error rule; C-11b again** (C-11b now `5898f6e4…`; new control **T-4** `38668d70…`).
   - **Defect.** C-11b's `tileset.json` had root `geometricError` 500 under a top-level `geometricError` of 400.
     Production refuses that under a pre-I1 explicit rule, before OD-7. The mismatch was in `tileset.json`, the
     referencing document; `ext/tileset.json` is consistent (64/64).
   - **Fixture.** The top-level value is now 500, and C-11b again reaches OD-7 (Unsupported, repeated subtree member).
   - **Oracle.** The model now applies the rule to the root of every tileset document: the root's error must be
     at most the document's `geometricError`, with the same tolerance as `geometricErrorOrder`
     (`1e-8·(1 + e)`). T-4 sits 2 × that tolerance above the document value, so the control is robust to the
     tolerance reading.
   - **Effect.** No other fixture or corpus verdict changed. There are now 181 cases.

5. **A-6 and A-8 now ship the members their mutants name** (A-6 now `fa6ae6c1…`, A-8 now `04488b5b…`).
   - **Reason:** independent controls must discriminate the I1 mutants drop-content-implies-tile and ge-vs-gt-on-A.
     Both mutants previously ended in "missing archive entry" (InvalidInput) instead of passing, so neither control
     flipped.
   - **Provenance:** the member names come from the `diagnosis` fields of R's
     `acceptance/production-mutants/report.json` on `feat/implicit-forest`. Only that results file was read, after
     the freeze; no mutant patch or `src/` file was read.
   - **A-6:** adds `content/2/1/1.glb`, the empty neighbour (2,1,1) that the mutant indexes the stray bit to. Its point
     is the tile centre. No other member is added, so the unreferenced-entry check cannot mask the mutant.
   - **A-8:** built as BQ plus the tile (6,18,2) at level A = 6. This sets child bit 12 in subtree (3,2,0), under the
     available (5,9,1), and ships `subtrees/6/18/2.subtree` (tile, row, content bit, `Const0` children) and
     `content/6/18/2.glb`.
   - **Diff to BQ:** each changed member differs by exactly one bit (the root subtree for A-6, subtree (3,2,0) for
     A-8); the rest are added members.
   - **Verdicts:** unchanged, and at the targeted rule. A-6 is InvalidInput at §2a.7 ("content available where the
     tile is not"); A-8 is InvalidInput at §2a.6 ("child subtree bit at or beyond availableLevels").
   - **R:** re-run both mutants to confirm they now flip.

6. **Implicit payloads are rotated (C2 slice 3, 2026-10-12).** Requested by the C2 slice-3 contract (frozen v3.1,
   sha256 `8bc53429…`, §7 last row and §9.C "Sequencing of amendment 6"). It lands before production stops
   excluding implicit archives from C2, because an unrotated row-bearing fixture would then be production
   InvalidInput (Outside) against O-implicit's Accepted.
   - **Fixtures.** Every implicit payload is now written with `glb_tile_point`, so `U` maps the stored point to the
     tile-frame cell centre (the `Implicit.payloads` default and the six explicit `glb_point` calls in
     `generate.py`). `glb_tile_point` now writes `0.0 - y`, so a zero coordinate stays `+0.0`; a payload at the
     origin is byte-identical to before, and Z-payload and C-8 are unchanged.
   - **B-1.** Its two leaf rows are swapped, so a payload at the cell centre lies outside its own row: C2 would
     report Outside (own) and production would refuse an I1 control. B-1's payloads now sit at the centre of their
     tile's row. I1 does not read payloads, so its verdict and `tilesSha256` are unchanged.
   - **C2 reading of the amended controls.** Under the slice-3 oracle (`c2o/implicit.py`), every case O-implicit
     accepts is Contained or NotInspected except R1b.full (corrupt payload; Undecided, production refuses it in C1).
     None is Outside, so production keeps every I1 verdict once C2 streams implicit content.
   - **Scope.** 126 of the 181 cases (122 files) changed hash, including the large R8a and R8a.nopad; 55 are
     unchanged (their payloads sit at the origin, or they have none). The old and new hash, size and verdict of each
     changed case are in `controls/amendment6.json`. The pre-amendment manifest is the one at `92e8c04`
     (sha256 `1a86392b…`).
   - **Re-run.** All 181 cases were re-run (`witnesses/check.py`, small and `--large`): 0 disagreements. Against
     the pre-amendment results, every verdict, reason, stage and `tilesSha256` is unchanged, and so is every count,
     including `bytesReadModelled`, `cumulativeSubtreeBytes`, `maxLivePathBytes` and `scanBytes`. The oracle never
     reads payload bytes, so only the manifest's archive `bytes` changed. No limit pin (`total_bytes_read`,
     `member_bytes`, `document_decoded_bytes`) is on a changed case, so no pin moved.
   - **The unrotated fixtures leave this manifest.** They are no longer I1 controls. The ones C2 can use are kept,
     with their old hashes, as C2 slice-3 control S3-19 (`c2_decoded_containment/controls/slice3/`).
   - **R:** re-run I1 parity (`production_matches_every_i1_oracle_witness_and_control`) on the new manifest before
     any C2 parity run.

## Contract questions (my readings; please confirm or correct)

- **Q1 (D-4 magic).** Resolved by v3.2 ruling 1.
- **Q2 (R1b "one short of the full count").** `ExitTileset` is charged after the walk. At full − 1 = 26, the
  failure is therefore at the exit, after the corrupt payload, so production gives InvalidInput and the control
  cannot discriminate.
  - O sets the R1b limit to Tileset + Node + Σ`P_t` − 1 = **25**, one short of what the last reservation needs.
  - Reservation then refuses at the last subtree's §2a.9, while per-visit charging reaches the corrupt payload first.
  - R1b.full uses 27.
- **Q3 (depth convention, R4).**
  - The root tile of `tileset.json` has depth 0, and an external tileset's root has the referencing tile's depth
    + 1.
  - The violation is `depth > hierarchy_depth`.
  - On BQ (max level 5): 5 is Accepted and 4 is ResourceLimit.
  - If the validator counts root depth as 1, both shift by one.
- **Q4 (R8a length).** The contract says the normalized `c/{level}-{x}-{y}-{z}.glb` is 27 B; it is **25 B** counting
  variables as their token text. The verdict is unaffected.
- **Q5 (cap base).** O counts the 1,024 B cap on the full normalized template, including the directory segments of
  the document holding R, because normalization starts there. R8b is at the archive root, so the verdict there is
  unaffected; a root in `ext/` would gain 4 B.
- **Q6 (dedup key).** O reads `kind` as the source kind (`Const0`/`Const1`/`View`). A content layer naming the tile's
  view therefore shares the tile source and is counted once in `Scan`. This affects only R7a's `Scan` (19 MB less),
  not any verdict.
- **Q7 (escape guard window).** "A `%` within two characters before a `{`" is read as `%` at offset −1 or −2 in the
  same segment. So `%7A{x}`, a complete escape, is not guarded and decodes to `z`.
- **Q8 (tolerance norm).** "‖all half-axis components‖" is read as the Euclidean norm of the container's nine
  components. T-1's margins (0.11× and 2×) make the verdicts robust to the alternatives (inner box, max-norm).
- **Q9 (`document_items` convention).** R7c's tileset has **65,368** items under O's count (every value, containers
  included), 168 under the limit. R7 and R7b subtree JSON have 60,019 and 60,014. If C1 counts differently (for
  example keys too), R7c turns into a JsonLimits refusal.
- **Q10 (table property checks).** O makes these InvalidInput:
  - a table property that is not in its class;
  - an empty `properties` (schema `minProperties: 1`);
  - a table whose class does not exist, including unreferenced tables.

  O does not enforce `required` on opaque (non-interpreted) properties, because the contract names only interpreted
  ones.
- **Q11 (explicit refine).** If the root of `tileset.json` has no `refine`, O is Undecided. No fixture does this.

## Independence statement

- O never read `src/`, `bindings/` or `tests/` beyond `p1_point_oracle.py:279-287`. That excerpt was used for
  layout only and is restated in the self-test as the global/local concatenation identity.
- No expectation was edited to match production; production has not been shown to O.
- The fixtures are generated by `fixturelib.py`, which shares no code with the oracle: member names, availability
  bits and rows are recomputed independently.
- The payload is O's own minimal POINTS GLB, which mirrors the shape of the #151 payloads.
