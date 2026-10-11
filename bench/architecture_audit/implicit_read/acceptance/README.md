# I1 acceptance (R-helper, combined head e3a67bf)

The I1 implicit reader on the combined head `feat/implicit-forest` at `e3a67bf` is compared with
the frozen O-implicit oracle (`../oracle`, `../controls`, `../corpus`). Every verdict here comes
from O's committed expectations. No production code changed. The only `src/` edit is the
test-only `#[cfg(test)] #[path]` include in `src/lib.rs`, which follows C2's pattern.

**Result at e3a67bf: FAIL.** One production defect (B-10.subnormal) remained. Every other case
agreed, or differed for a diagnosed O-side reason.

**Result after the follow-ups (`ce9838e` plus this record): PASS, 179/180, with 1 diagnosed
O-side difference and 0 production disagreements.** See "Follow-up" below.

## Follow-up: owner fix and oracle amendment merged

These commits are now in the branch:
- `5ffbf71` (owner): decides derived-error subnormality on the exact value, using
  `e_R < 2^(level−1022)`, before multiplying.
- `2ecbad1` (oracle author, post-freeze), merged as `ce9838e`. It changes three things:
  - C-11b's children are made distinct;
  - Z-payload's and C-8's points go through glTF's Y-up rotation;
  - `contentReferences` now counts external-tileset links, per the coordinator's ruling.

The merged oracle replays exactly:
- self-test 50/50;
- `generate.py --large --check`: 180 cases byte-identical;
- `check.py`: 167 + 13 cases, 0 disagreements;
- `corpus/run.py`: 41/41.

`corpus/results.json` was regenerated from the merged oracle. mf2-lod3 `contentReferences` is
now 12,715. The facts digests are unchanged.

The harness now compares `contentReferences` directly. Its sha256 was
`e7e8541e907578b7f52e6cf37c1fc75f4cd40e4c99098b3cb7ab44da0d9abb43` for these runs; after
`cargo fmt` (`456f614`, whitespace only) it is
`4317291be4bf615ec84375d606c183628fe2e14abbe37d3ae57a5868b163b74a`. A debug re-run of both
parity tests at `456f614` passes.

| Run (release, merged head) | Cases | Agree | O-side | Production disagreements |
| --- | --- | --- | --- | --- |
| `parity-merged-release.json` (O defaults) | 180 | **179** | 1 (C-11b) | **0** |
| `corpus-parity-merged-release.json` | 41 | **41** | 0 | 0 |
| `parity-merged-release-production-defaults.json` (informational) | 180 | 178 | 1 | R9 only (#160 default rescale, as before) |

- **B-10.subnormal** now agrees: Unsupported, "derived geometric error underflow at level 5".
- **Z-payload** and **C-8** now agree: Accepted.
- **C-11b** gives InvalidInput, "geometricError increases at depth 0", so it still does not reach
  OD-7.
  - **Cause.** The amended `tileset.json` keeps top-level `geometricError` 400 but wraps the two
    links in a root whose error is 500. Production bounds each document root by its document's
    top-level `geometricError` (for an external root, the minimum of that and the link's error).
    That is the explicit skeleton's `geometricErrorOrder`, unchanged from develop. O models the
    order against the path-parent tile only.
  - **Evidence that OD-7 itself agrees.** `scripts/c11b_variant.py` raises only that top-level
    value to 500 and repackages with O's `write_3tz` (`5898f6e4…`). Both O and production then
    give Unsupported "repeated subtree member" (`ext/st/0.0.0.0.subtree`).
  - **Who acts.** O's fixture should set the top-level error to at least 500, or O should model
    the document-level bound. The harness pins C-11b as O-side with this diagnosis.

## Identities

| Item | sha256 |
| --- | --- |
| Head `e3a67bf`, release `native-geospatial` (`rusty-tiles`) | `a734eac596a2a26031a48b94e85e1fe2a4e38f7856c81e5cfdaf4a7aa327d6d0` (rebuilt from the clean tree after a forced recompile: identical) |
| Head `e3a67bf`, release portable | `1ace7782ad6fb6ec0ad3d96fb5971e7ae43da7ccf3f7ca3e3863ef523cff71be` |
| Develop `766b92b`, release `native-geospatial` | `2a7c4ba6cfc21dedd595cecf92d01d7f8161c3442ca1057e12c9107a78230cc3` (equal to `~/.cache/vcaps/bin/rusty-tiles-proposed-native`) |
| `oracle_parity.rs` (this harness) | `7fc832899f995521c977c865a4a70c6f30c189b0cdc2541997c23034b5b7112d` |
| Producer inputs | `producer-runs.json` `inputs`. These are the c2-corpus run2 inputs and the perf-lane `points.las` and `vector.geojson`, all equal to the I1 receipt. W6d's input is `scripts/deep_mesh_glb.py 12` (`5442ba6c…`), a stdlib port of `textured_grid_glb(12)` whose PNG bytes differ from the `image` crate's |

Builds used `CARGO_BUILD_JOBS=2`, `taskset -c 0,1` and `nice -n 10`, under the shared cargo lock.
Producers ran on CPUs 2,3 and validation on 4,5, both at `nice -n 10`.

## 1. Oracle parity: production `inspect_selected` and `implicit_tile_facts`

`oracle_parity.rs` is compiled as `--lib i1_oracle_parity`. For every manifest case it runs
production under **O's frozen default limits** (`i1_oracle.py:29-39`, the pre-#160 values) plus the
case's own limits. It compares four things:
- the outcome kind, including the ResourceLimit field;
- for Accepted, the canonical tile list's `tilesSha256` (O's digest form): volume hex, error hex,
  depth, parent level, refine, `(slot, member)`;
- `tiles` and `implicitTiles`;
- `contentReferences`.

Production's `content_references` also counts explicit external-tileset contents (unchanged from
develop; see "Counting convention"). It is therefore compared with O's `contentReferences +
tilesets − 1`.

```sh
python3 -I bench/architecture_audit/implicit_read/witnesses/generate.py --large
I1_PARITY_OUT=p.json I1_CORPUS_PARITY_OUT=c.json cargo test --locked --release --lib i1_oracle_parity
```

| Run | Cases | Agree | O-side (diagnosed) | Production defect |
| --- | --- | --- | --- | --- |
| `parity-e3a67bf-release.json` (O defaults, large controls generated) | 180 | **177** | 2 (C-11b, Z-payload) | **1 (B-10.subnormal)** |
| `parity-e3a67bf-release-production-defaults.json` (`I1_PRODUCTION_DEFAULTS=1`, informational) | 180 | 176 | 2 | 1, plus R9 (see below) |

For the 74 cases O expects Accepted that production also accepted, the tile facts equal O's
bit for bit. That includes R7a, R7c and R8a with 37,449 tiles each.

### Disagreements

| Case | O | Production | Whose reading differs |
| --- | --- | --- | --- |
| **B-10.subnormal** | Unsupported ("derived error would be a nonzero subnormal", tile 5/9/1/0) | **Accepted**; level-5 tiles report error `0x0010000000000000` | **Production defect.** e_R = `0x1.fffffffffffffp-1018`, so the exact e_R·2⁻⁵ = `0x1.fffffffffffffp-1023` is a nonzero subnormal. `derived_error` (`src/content_integrity/implicit/walk.rs:56-66`) multiplies first and tests `is_normal()` on the **rounded** product, which rounds up to `0x1p-1022`. That breaks the contract (§4 "Deeper tiles": `e_R·2^−l`, "exact when e_R = 0 or the result is normal. A subnormal nonzero result is Unsupported"), and the reported error is not exact. Reproduced through the CLI: `validate B-10.subnormal.3tz` gives `ok: true`, 24 tiles. The one-ulp pair B-10.normal (Accepted) and B-10.l1 (Unsupported) both agree. The existing unit test (`walk.rs:417`) uses `MIN_POSITIVE` at level 1, where the product is exact, so it cannot catch the rounding |
| C-11b | Unsupported (OD-7: an implicit-root external tileset referenced twice) | InvalidInput: `tileset schema: [...] has non-unique elements` | **O's fixture.** The two children are identical JSON objects, and the 3D Tiles core tile schema has `children.uniqueItems: true` (`docs/schema/tileset.schema.json:790`). The schema check precedes I1, so the control cannot reach OD-7. It is insensitive, and O should make the two children differ (for example in `geometricError`) |
| Z-payload | Undecided; production note "Accepted" | InvalidInput: `decoded content outside bounding volume … vertex=0` | **O's stale production note.** O's point (16, 8, 2) is glTF Y-up, which is (16, −2, 8) in the tile frame, outside the box [0,32]×[0,16]×[0,4]. C2 decoded containment merged after O froze. C1 does accept the GLB, and C2 marks every implicit payload NotInspected, so no Accepted implicit control is affected (all 74 agree). If C2 later covers implicit content, every O fixture payload placed at a box centre without the Y-up rotation will be Outside |
| R9 (production defaults only) | ResourceLimit:references (182,500 + 182,500 > 262,144) | InvalidInput: `missing archive entry: c/3-0-0-0.glb` | **Default-limit change, not a defect.** #160 raised `references` to 1,048,576. Under O's frozen 262,144, production gives ResourceLimit:references at the first child subtree (agrees). The contract records this rescale (`i1-implicit-read-contract.md`, "Validator caps (#160)"). O's R9 still uses K = 2,500, so it discriminates only at the old ceiling |

The harness pins the two O-side rows to their exact production outcome (`O_SIDE`). It fails if
either changes, or if any other disagreement appears. B-10.subnormal is deliberately not listed,
**so `production_matches_every_i1_oracle_witness_and_control` fails at e3a67bf until it is
fixed.**

### Readings in O's "Contract questions", as executed

| Q | O's provisional reading | Production |
| --- | --- | --- |
| Q2 (R1b limit 25) | Reservation refuses at the last subtree | Agrees: `R1b` gives ResourceLimit:hierarchy_visits ("subtree tile availability exceeds remaining hierarchy visits"), and `R1b.full` gives InvalidInput ("invalid GLB v2 header/length") |
| Q3 (root depth 0) | BQ: depth 5 Accepted, depth 4 ResourceLimit | Agrees (R4, R4-1) |
| Q4 (R8a length) | 25 B | Agrees: R8a Accepted, facts equal to R8a.nopad |
| Q5 (cap base) | Includes the directory of the document holding R | Agrees on R8b.1024 / R8b.1025. R8b is at the archive root, so this does not discriminate |
| Q6 (dedup key) | `(kind, view, bits)` | No verdict depends on it; R7a is Accepted in both |
| Q7 (escape window) | `%` at −1 or −2 in the same segment | Agrees on C-7 / C-7b / C-7f |
| Q8 (tolerance norm) | Euclidean over the nine components | Agrees on T-1. The margins make it insensitive |
| Q9 (`document_items`) | Every JSON value | Agrees: R7c is Accepted (production under O's 65,536) |
| Q10 (table property checks) | InvalidInput | Agrees on every B-family case |
| Q11 (explicit refine absent) | Undecided | No fixture exercises it |

### Resource bounds (release, from the parity run)

| Control | Expected | Production | Wall |
| --- | --- | --- | --- |
| R7 | InvalidInput (post-walk) ≤ 5 s | InvalidInput | 0.14 s |
| R7a | Accepted ≤ 5 s | Accepted | 0.15 s |
| R7b | ResourceLimit:total_bytes_read ≤ 5 s | ResourceLimit ("availability scan exceeds total_bytes_read work limit") | 0.04 s |
| R7c | Accepted ≤ 5 s | Accepted | 0.39 s |
| R8 | Unsupported ≤ 5 s | Unsupported ("template variables eliminated by dot-segments") | 0.11 s |
| R8a | Accepted, ≤ 2× no-padding and ≤ 60 s | Accepted | 0.68 s against 0.57 s (1.18×) |
| R2 / R2-1 | Accepted / ResourceLimit before the sixth read | Agrees (refused at `subtrees/5/0/0.subtree`) | < 0.01 s |
| R3.L8 / R3.L32 | ResourceLimit at step 9 | ResourceLimit:hierarchy_visits | < 0.01 s |

The per-control RSS (R3 within 16 MiB of W1) and the implementation mutants (contract §D) were
not run here.

## 2. W corpus W1–W11

The producers ran on the combined head (`producer-runs.json`, `scripts/produce.py`). The 33 new
artifacts were added to `../corpus/manifest.json` with their sha256 values, and
`corpus/run.py` was run (`../corpus/results.json`, `../corpus/facts/`). Production was compared
twice:
- through the harness's second test, `production_matches_the_oracle_on_the_w_corpus`
  (`corpus-parity-e3a67bf-release.json`);
- through the CLI `validate --json`, native and portable (`validate-cli-e3a67bf.json`).

| Witness | Artifact(s) | O | Production facts and counts | CLI `validate` |
| --- | --- | --- | --- | --- |
| W1 | point-cloud `cloud.las --maxPoints 1000` | Accepted, 73 tiles, 76 visits | equal | ok, 73/73 |
| W2 | perf `points.las --maxPoints 2000` | Accepted, 428 tiles, 438 visits, 8 roots | equal | ok, 428/428 |
| W3 (pc-deep) | `cloud.las --maxPoints 40` | Accepted, 2,233 tiles, 2,243 visits | equal | ok |
| W4 ×6 | vector points-lines and polygons × default / `--quantize` / `--meshopt` | Accepted | equal | ok |
| W5 (vec-perf) ×3 | perf `vector.geojson` default / `--quantize` / `--meshopt` | Accepted, 547 tiles, 594 visits, 45 roots | equal | ok, 547/544 |
| W6 | `mesh-to-3tz split.gltf --maxTriangles 512 --maxBytes 0` | Accepted, 73 tiles | equal | ok |
| W6d | deep mesh (`--maxTriangles 2 --maxBytes 0 --tileSize 64`) | Accepted, 314 tiles, 8 roots | equal | ok |
| W7 (M2 wrapper) | `mesh-to-3tz example.gltf` | Accepted, 1 tile | equal | ok |
| W8s | f2238cd convert archive `460c2505…` | Accepted | equal | ok |
| W8m | A2 `vector-flat-rounded-implicit.3tz` `d1d19138…` | Unsupported | Unsupported | exit 2, "external tileset content template" |
| W10 | portable builds of W4 ×6 and W5 ×3 | Accepted | equal | ok |
| W11 | every output above, plus mf2-lod3 (`--maxFeatures 2 --lodLevels 3`: 12,530 tiles, 189 documents, 188 links, 1,225 roots) | Accepted, `tiles` 12,718, 14,321 visits | equal (`tilesSha256` included) | ok, 12,718/12,715, 0.97 s |
| Earlier samples | W2.f2238cd, R1, R2b, R2, W3u, W5u, W9u ×4 | as recorded | equal (R2b.W2.190943 and R2.W2.live refused by C1, per O's notes) | n/a |

**41 of 41 corpus artifacts agree**, and every forest output is Accepted by both O and production.

- **P A-3 report shape.** For all 16 outputs, `checks` and `notInspected` equal develop
  `766b92b`'s report on its own default archive of the same input, with these changes:
  `implicitAddressingAndAvailability` follows `hierarchyBounds`, it leaves `notInspected`, and
  `recordedBudgets.implicitTileExtras` joins. `decodedContentBounds`,
  `decodedContentBounds.implicitTiling` and `metadataSemantics` remain.
- **glb wrapper.** `glb-to-3tz` still writes one explicit tile, byte-identical to f2238cd's
  corpus (`7efd6da4…`, `420810c9…`). It is not a forest producer.
- **Native and portable.** Point-cloud and mesh outputs are byte-identical. The vector archives
  differ in payload bytes in the `--explicit` builds too, so the difference lies in the
  native/portable geometry path, not the forest writer. W10 only requires Accepted.

### Counting convention (for the coordinator)

Production's `contentReferences` counts each explicit external-tileset content, as develop
does. On mf2-lod3 it is therefore 12,715 = 12,527 emitted payloads + 188 link tiles, and `tiles`
is 12,718 = |H| + links (PD-13). P's A-3 text ("`contentReferences` = emitted contents") holds
only if link contents count as contents. O counts payload contents only (C-11, T-3.base and
mf2-lod3 differ by exactly the number of external tilesets).

## CI replay (local, `scripts/ci_replay.sh`, `ci-replay-summary.txt`)

This replays `.github/workflows/ci.yml` at `dd05dc4`, a clean tree, using its own target
directories, CPUs 0 and 1, `nice -n 10` and the shared cargo lock.

| Job | Steps | Result |
| --- | --- | --- |
| `rust` | foundation model and monitor; `cargo test --features native-jpeg --test-threads=1` (lib 549 passed, 0 failed); payload-context receipt; the C1, F1a–F1d2, model-wrapping, vector and point oracles and resource probes (21 steps); portable JPEG | all pass |
| `rust` | `cargo fmt --all --check` | **failed at `dd05dc4`** on this harness file only; fixed in `456f614`, after which it passes |
| `rust` | `cargo package --locked --no-verify` | failed locally only because the gitignored, generated `controls/large/` archives are present; with them moved aside it passes (102.0 MiB, 9,276 files). CI never has them |
| `python-wheel` (cargo and unittest steps) | `test_wheel_runners`; clippy `-p rusty-tiles-python -D warnings`; `cargo test -p rusty-tiles-python` | pass |
| `portable` (host target) | `cargo test --locked --target x86_64-unknown-linux-gnu` | pass |
| `native-geospatial` | `cargo test --features native-geospatial` (905 passed, 0 failed); native vector and point oracles; clippy `--all-targets --features native-geospatial -D warnings` | pass, and clippy is re-run clean at `456f614` |

**Not replayed locally:**
- `python`: it needs the conda GDAL/laspy/pyproj Python environment and npm-installed
  Cesium/three/gltf-validator/Playwright;
- the wheel build and installed-wheel test (no maturin here);
- `python-free-runtime` (docker image);
- `release-acceptance`.

These stay with exact-head CI.

The producer artifacts and the release binaries used for sections 1 and 2 are from `e3a67bf`.
The merged changes do not touch producers (`5ffbf71` changes validation only), and the merged
parity runs in this follow-up exercise validation at the merged head.

## Not established here

- Implementation mutants (contract §D).
- R3's RSS bound.
- Python, wheel and Blender parity (§F).
- mf2-lod16 and the 32-level bound on production output.
- Exact-head CI.

The I1 perf obligation is recorded with the P lane in
`../../implicit_layout/acceptance/`. Nothing here claims release readiness.
