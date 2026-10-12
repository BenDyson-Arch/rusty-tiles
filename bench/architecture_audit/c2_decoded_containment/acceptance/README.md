# C2 acceptance (coordinator-owned)

Production is compared only against the independent oracle's committed expectations. No
verdict here is derived from production code.

| Evidence | What it shows | Where it runs |
| --- | --- | --- |
| `oracle_parity.rs`, compiled as `--lib c2_oracle_parity` | Production `inspect_selected` equals the oracle on every witness and control (outcome, reasons, Outside depth/level/role/node, C1 refusal kind) | CI and local. The 24 MB `controls/large/` archives are gitignored, so CI skips only those three ids |
| `parity-a547d1a.json` | Local release run with the large controls: 261 cases, 261 agree, 0 skipped, 0 C1-precondition refusals | Local, Linux |
| `corpus_parity.py` and `corpus-parity-a547d1a-release.json` | `validate --json` on the 27 producer artifacts (develop f2238cd release producers, manifest sha256 `28e4bee6…`) equals the oracle corpus manifest: 17 Contained, 10 NotInspected(implicitTiling), 0 Outside | Local, Linux |
| `oracle_parity.rs`, `production_matches_every_c2wb_class_fixture` | Production equals O-WB on all 102 `wb/` rows: 55 at-cap pairs, 4 chain-bit controls, and the totals and per-URI facts (`T`, `Idx`, `E`, `Dec`, `B`, `J`, `Res`, `Prim`, `Nodes`) of 43 matrix-size rows | CI (CI rows) and local (matrix-size rows, after `wb/generate.py --large`) |
| `wb-gate/` | The C2-WB section 9.D class-matrix calibration on P- and E-cores and the default-cap decision (2^29) | Local, Linux, measurement host only |
| `production-mutants/` | 19 hashed production mutants (12 mirroring the oracle's). Initially 17 flipped at least one fixture. The 2 survivors exposed corpus gaps, which the oracle closed in `122a63e`; `rerun-after-gap-fixtures.json` shows both now flip | Local, Linux |

Replay:

```sh
python3 -I bench/architecture_audit/c2_decoded_containment/witnesses/generate.py --large
C2_PARITY_OUT=parity.json cargo test --locked --release --lib c2_oracle_parity
```

The corpus replay needs the producer artifacts: regenerate them with the corpus generator
recorded in the corpus manifest, then run `corpus_parity.py BINARY RUN_DIR
corpus/manifest.json OUT`.

Measured cost (Linux, release, 2 CPUs; develop f2238cd compared with this branch).

| Unit class | Measured on | Cost |
| --- | --- | --- |
| Exact vertex unit (`T·Σ⌈s/64⌉²`) | 20k-point explicit point cloud. The oracle computes 5.99M units; validate goes from 48 ms to 108 ms | about 10 ns |
| Index unit (`Idx`) | C1 `legal_shared_indices`: 1000 primitives share a 1M-index accessor, about 1e9 units. Release 2.85 s (debug 44 s, which exceeded the C1 finite-work probe's 30 s) | about 2.8 ns |

(Superseded by the C2-WB gate below: the unit formula and the default cap changed.) The work cap, not C1's element ceiling, bounds C2 time: instancing, shared index accessors and
repeated references all multiply work. The original default of 2^32 units therefore allowed
about 43 s in release (minutes in debug). After the nonauthor review, the coordinator set the
default to 2^28. Correction (2026-10-12, C2 slice-3 precode review): this does not bound time uniformly. Adversarial cheap-unit shapes measured 193–776 ns/unit, about 52–208 s at 2^28; a corrected formula is planned. The original claim was that this bounds C2 work at the defaults to about 2.7 s at the most expensive
measured unit cost, and `legal_shared_indices` becomes `workLimit`. Re-reads are separately
capped by `containmentBytesRead`. The nonauthor review noted that per-reference C1
re-validation and drawn-vertex marking across many primitives have no dedicated unit; that
cost is bounded only indirectly, and timing archives with many primitives is open work.

Not established here: platform byte identity, performance or RSS of the precheck caps (the cap
values are proposals), implicit/RTC/region/rotated-box coverage, and any rendered-extent
claim.

## C2-WB gate (R, 2026-10-12)

The C2-WB work-bound correction (contract draft v2.1, sha256 `ea7b3acf…`, sections 5, 5.1, 6
and 9.D) sets the default `containmentWorkUnits` from a measured class matrix. This section is
the record. It is a calibration on one host, not a proof.

### Parity first (release, local, large controls generated)

| Harness | Result |
| --- | --- |
| `c2_oracle_parity::production_matches_every_oracle_witness_and_control` | 261 / 261 agree, 0 skipped, 0 C1-precondition refusals. Includes the re-pinned work pairs 5,040 / 5,039 and the repeated-index pair 48,008,577 / 48,008,576 |
| `c2_oracle_parity::production_matches_every_c2wb_class_fixture` | 102 / 102 (`C2WB_PARITY_OUT`), after the two reconciliations recorded in the oracle README ("R's reconciliation against production") |
| `i1_oracle_parity` | 181 / 181 controls (I1 `--large` generated), 41 / 41 corpus rows |

O-WB's readings, resolved against production: `Res` counts resolver calls (one per buffer or
image entry); `data:` URIs are not resources; "16 levels" means `L_r = 15`. `B` also counts
per resolved entry, which changed one O-WB fixture (`reading-res-duplicate-uri`, 5,578 to
5,602). C1 admits every matrix-size class fixture, including the meshopt INDICES and bits
streams and the 800-character decimals, once sibling leaves are unique.

### Method (section 9.D)

- **Binary.** Measurement build: source `e8cf53a` exported with only the default cap raised to
  2^62, so every class streams to completion (`rusty-tiles-measure-e8cf53a-cap2p62`, sha256
  `c39ee374…a8b`). Final build: the committed source with cap 2^29 (`f98d5b4c…acef`). Both are
  release builds, rustc stable, x86_64 Linux 7.2.9, Intel i7-12700KF (P-cores 0–15 with SMT;
  E-cores 16–19, one L2 cluster).
- **Arms.** A is the case; B is its `asset.gltfUpAxis = "Z"` twin (excluded before the
  precheck). Every A is checked Contained (or `workLimit` for the precheck arms) and every B
  `upAxisOverride` on every run. The case figure is the child CPU time (`wait4`) of A − B.
- **Clean runs.** Busy time is read from `/proc/schedstat` (ns). A run counts only if the
  pinned CPU's foreign busy share is ≤ 5% and the contention neighbour is ≤ 10% (P: the SMT
  sibling; E: the busiest other E-core). A pair counts only if both runs are clean. Each case
  needs 7 clean pairs within 30 attempts; its figure is the median of the 7, the minimum beside
  it. The harness waits for an idle core before a case and re-picks after 3 dirty attempts.
- **Units.** From O-WB's oracle only: `wb/manifest.json` for the class fixtures, and
  `oracle/c2_oracle.py --precheck-only` for the probe matrix, producer twins and the images
  class (`wb-gate/results/cases.json` pins every archive's sha256 and units).
- **Cases.** 96: O-WB's 43 matrix-size fixtures (`wb/large/`), R's probe matrix (29 chain,
  large-span and reference shapes from the v2 calibration, `wb-gate/probes/`, regenerated),
  the 9 b51f83d explicit producer twins, and R's images-by-URI class (OD-WB-7:
  `images_class.py`, 20,000 image entries × 20 references).
- **Load.** P-core runs used CPUs 3–7 and 13–15; E-core runs used 16–19. 717 P and 900 E
  attempts; the largest counted foreign share was 4.9% and the largest neighbour 9.99%. Host
  load average (1 min) ranged 0.97–10.1 over the session (`results/load-*.txt`; other users'
  work ran throughout, so many E-core runs were discarded and re-run).

### Result

| | τ (ns/unit, max over 95 decided cases) | Class | Cap | Seconds at the cap |
| --- | --- | --- | --- | --- |
| **E-core** | **14.65** | `probe:boxT-E300-L8` (box + step, 8 levels, translations near 2^300) | **2^29** | **7.87 s** |
| P-core | 8.10 | same | 2^29 | 4.35 s |

`τ_E ≤ 14.9`, so the cap is 2^29 (contract section 6: the largest power of two with
`cap × τ_E ≤ 8 s`). The margin is 1.7%; the eight costliest classes are all 1-limb or small-span
chains at 13.6–14.65 (E). The final build reproduces the top rows (`results/final-*.jsonl.gz`):
`boxT-E300-L8` 8.12 P / 14.46 E, `extS-L16` 7.96 / 14.55, `e2e` 4.36 / 7.84. E/P per class:
min 1.47, median 1.74, max 2.13.

Never-measured classes (contract 5.1): long-significand decimals 5.23 P / 11.02 E (E/P 2.11,
the highest of the non-trivial classes); meshopt NONE decode per decoded byte: ATTRIBUTES bits
4.35 / 7.59, ATTRIBUTES raw 4.06 / 7.08, INDICES 2.55 / 4.41, TRIANGLES 2.47 / 4.27. Viewless
accessors: SCALAR 3.62 / 5.33, MAT4 1.99 / 3.42.

Pre-fix rows (contract 5) after WB-A and WB-B, E-core: deep numbers depth 62 (GLB) 22.22 → 2.33
(depth 1: 2.29, ratio 1.01, within the A2 1.25× linearity control); b3dm depth 60 21.05 → 2.04;
`quad` 35.26 → 8.54; `quadB` 77.23 → 8.04; end-to-end 83.66 → 7.74 (354.5M units, Contained at
2^29 in 2.78 s E).

Other 9.D items (final build, cap 2^29):

- **Precheck-only arms** (A is `workLimit`): 1,047,040 tiny references under a 100-document
  chain, A − B 0.58 s P / 0.99 s E; at depth 1, 0.09 / 0.09 s. O-WB's 100,000 references at
  103 levels: 0.58 / 0.99 s.
- **Phase-1 C2 time** (excluded B twins, depth 1 against depth 100): 100,000 references
  +0.002 s P / −0.006 s E; 1,047,040 references −0.044 s P / +0.042 s E. Both within 0.05 s.
- **Producers** (b51f83d explicit twins): 4.58–5.94 P / 7.66–10.12 E; at 2^29 the W5 twins
  use 38–40% of the cap, W2 26%, the others ≤ 7%. `mf2-lod3-explicit` (438,590,913 oracle
  units, 81.7%) is refused by C1 (`JSON exceeds value-node limit`, also with the b51f83d
  binary), so it has no timing; the implicit `mf2-lod3` is `implicitTiling` until slice 3.

Not done here: the 9.C production mutants (re-scanning JSON, quadratic `mark`, `build_levels`
before the gate, cap one step lower) were not re-run. The cap mutant is caught by the
`tests/c2_containment.rs` default-limits assertion.

| Class (case) | Units | P ns/unit (min) | E ns/unit (min) | E/P | P / E attempts |
| --- | ---: | --- | --- | --- | --- |
| `probe:boxT-E300-L8` | 44,302,409 | 8.10 (8.09) | 14.65 (14.26) | 1.81 | 8 / 7 |
| `probe:extS-L16` | 102,002,473 | 7.96 (7.93) | 14.34 (14.26) | 1.80 | 7 / 9 |
| `probe:sph-L16` | 54,002,473 | 7.95 (7.88) | 14.31 (14.19) | 1.80 | 8 / 9 |
| `probe:extTS-L16` | 147,002,473 | 7.97 (7.91) | 14.04 (13.89) | 1.76 | 8 / 9 |
| `probe:extTS-L32` | 148,702,597 | 7.97 (7.92) | 13.99 (13.83) | 1.76 | 7 / 7 |
| `probe:box-E300-L8` | 23,302,409 | 7.70 (7.69) | 13.97 (13.65) | 1.81 | 9 / 10 |
| `probe:sphT-L16` | 99,002,473 | 7.80 (7.78) | 13.90 (13.71) | 1.78 | 7 / 7 |
| `probe:boxT-L16` | 87,802,473 | 7.57 (7.53) | 13.62 (13.36) | 1.80 | 7 / 7 |
| `probe:box-L16` | 42,802,473 | 7.47 (7.36) | 13.39 (13.20) | 1.79 | 7 / 7 |
| `probe:box-E120-L8` | 22,802,409 | 7.37 (7.30) | 13.34 (13.09) | 1.81 | 8 / 9 |
| `probe:extT-L16` | 125,302,473 | 7.50 (7.48) | 13.17 (13.12) | 1.76 | 7 / 7 |
| `probe:boxT-f32-L16` | 89,102,301 | 7.49 (7.36) | 13.14 (13.03) | 1.76 | 7 / 8 |
| `probe:ext-L16` | 80,302,473 | 7.38 (7.36) | 12.87 (12.69) | 1.74 | 7 / 7 |
| `probe:box-f32-L16` | 44,102,301 | 7.21 (7.19) | 12.83 (12.51) | 1.78 | 7 / 7 |
| `probe:soloS` | 92,002,353 | 7.39 (7.38) | 12.78 (12.65) | 1.73 | 7 / 8 |
| `probe:box-L4` | 51,202,377 | 7.39 (7.35) | 12.65 (12.47) | 1.71 | 7 / 8 |
| `probe:soloB` | 78,002,353 | 6.88 (6.82) | 12.02 (11.97) | 1.75 | 8 / 7 |
| `probe:solo1B` | 53,002,353 | 6.37 (6.34) | 11.05 (10.96) | 1.74 | 7 / 7 |
| `wb:long-decimals` | 130,909,840 | 5.23 (5.11) | 11.02 (10.87) | 2.11 | 11 / 11 |
| `producer:W5-vec-perf-quantize-explicit` | 204,006,173 | 5.94 (5.87) | 10.12 (9.98) | 1.70 | 7 / 9 |
| `producer:W5-vec-perf-meshopt-explicit` | 214,224,609 | 5.34 (5.30) | 9.16 (9.07) | 1.71 | 9 / 7 |
| `producer:W5-vec-perf-explicit` | 207,610,517 | 5.42 (5.34) | 9.10 (9.01) | 1.68 | 7 / 9 |
| `wb:tiny-refs-L103` | 562,900,000 | 5.25 (5.22) | 9.02 (8.87) | 1.72 | 7 / 17 |
| `producer:W3-pc-deep-explicit` | 38,782,952 | 5.30 (5.23) | 8.92 (8.69) | 1.68 | 7 / 9 |
| `probe:refs-tiny-d100` | 559,300,000 | 5.26 (5.23) | 8.88 (8.76) | 1.69 | 7 / 7 |
| `wb:unselected-nodes` | 22,847,240 | 5.97 (5.90) | 8.78 (8.40) | 1.47 | 7 / 8 |
| `producer:W2-explicit` | 139,785,937 | 5.06 (5.06) | 8.68 (8.55) | 1.71 | 7 / 9 |
| `wb:quad` | 18,151,888 | 4.92 (4.81) | 8.54 (8.28) | 1.74 | 7 / 8 |
| `wb:refs-to-mid-payload` | 3,097,680,000 | 4.72 (4.70) | 8.38 (8.31) | 1.78 | 7 / 18 |
| `wb:box-chain-L16` | 8,822,345 | 4.46 (4.40) | 8.18 (7.83) | 1.83 | 7 / 15 |
| `wb:quadB` | 31,490,820 | 4.60 (4.56) | 8.04 (7.76) | 1.75 | 7 / 7 |
| `wb:box-step-L16` | 17,822,345 | 4.49 (4.46) | 7.95 (7.79) | 1.77 | 7 / 15 |
| `producer:W6d-explicit` | 1,910,238 | 5.13 (4.93) | 7.76 (6.98) | 1.51 | 8 / 7 |
| `wb:e2e` | 354,517,700 | 4.46 (4.40) | 7.74 (7.56) | 1.73 | 7 / 10 |
| `producer:W6d-nomeshopt-explicit` | 1,497,470 | 4.87 (4.71) | 7.69 (7.29) | 1.58 | 7 / 7 |
| `wb:nodes-translation` | 164,297,100 | 4.62 (4.60) | 7.68 (7.62) | 1.66 | 10 / 10 |
| `producer:W1-explicit` | 17,492,428 | 4.58 (4.54) | 7.66 (7.54) | 1.67 | 8 / 8 |
| `wb:meshopt-attributes-bits` | 572,989,556 | 4.35 (4.31) | 7.59 (7.46) | 1.75 | 7 / 10 |
| `wb:skipped-nodes` | 25,025,720 | 4.54 (4.46) | 7.56 (7.45) | 1.66 | 7 / 9 |
| `probe:refs-w5` | 709,434,000 | 4.26 (4.24) | 7.44 (7.28) | 1.75 | 7 / 8 |
| `probe:nodes-flat-T` | 86,423,740 | 4.46 (4.44) | 7.20 (7.08) | 1.62 | 7 / 12 |
| `probe:refs-tiny-d6` | 249,100,000 | 4.22 (4.18) | 7.19 (7.08) | 1.70 | 7 / 7 |
| `probe:refs-w3` | 331,980,000 | 4.02 (3.97) | 7.16 (7.08) | 1.78 | 7 / 7 |
| `wb:tiny-refs-L4` | 236,200,000 | 4.15 (4.09) | 7.12 (6.99) | 1.71 | 7 / 10 |
| `probe:refs-tiny-d1` | 232,600,000 | 4.12 (4.04) | 7.11 (7.02) | 1.73 | 7 / 8 |
| `wb:meshopt-attributes-raw` | 612,761,908 | 4.06 (4.03) | 7.08 (7.02) | 1.74 | 11 / 10 |
| `wb:extbuf-20000` | 100,623,560 | 4.02 (3.98) | 7.07 (6.82) | 1.76 | 7 / 7 |
| `R:images-by-uri-20000` | 102,224,020 | 4.17 (4.16) | 7.06 (6.85) | 1.69 | 7 / 7 |
| `wb:one-box-1M` | 66,002,229 | 3.88 (3.84) | 7.04 (6.89) | 1.81 | 7 / 12 |
| `probe:boxT-E500-L8` | 103,202,409 | 3.72 (3.70) | 6.53 (6.51) | 1.76 | 8 / 13 |
| `probe:sph-E300-L8` | 90,802,409 | 3.59 (3.58) | 6.45 (6.36) | 1.80 | 7 / 7 |
| `wb:nodes-rotated` | 299,925,900 | 3.87 (3.85) | 6.42 (6.31) | 1.66 | 7 / 22 |
| `wb:extbuf-2000` | 10,063,560 | 3.67 (3.65) | 6.39 (6.31) | 1.74 | 7 / 8 |
| `probe:nodes-flat` | 72,411,870 | 3.59 (3.59) | 6.03 (5.87) | 1.68 | 7 / 7 |
| `probe:refs-w6d` | 446,100,000 | 3.39 (3.38) | 5.90 (5.79) | 1.74 | 8 / 12 |
| `probe:box-E500-L8` | 58,402,409 | 3.30 (3.28) | 5.88 (5.76) | 1.78 | 7 / 7 |
| `probe:refs-w6dn` | 391,940,000 | 3.34 (3.29) | 5.63 (5.60) | 1.68 | 8 / 9 |
| `probe:extTS-f32-L8` | 187,902,237 | 3.04 (3.03) | 5.37 (5.29) | 1.77 | 7 / 8 |
| `wb:viewless-SCALAR` | 80,049,480 | 3.62 (3.60) | 5.33 (5.30) | 1.47 | 8 / 9 |
| `wb:meshopt-indices` | 144,889,508 | 2.55 (2.54) | 4.41 (4.35) | 1.73 | 7 / 7 |
| `wb:meshopt-triangles` | 152,889,636 | 2.47 (2.43) | 4.27 (4.17) | 1.73 | 10 / 8 |
| `wb:sphere-chain-L16` | 27,862,345 | 2.16 (2.10) | 3.97 (3.79) | 1.84 | 7 / 15 |
| `wb:shared-accessor-16000x40` | 4,857,579 | 2.47 (2.37) | 3.93 (3.67) | 1.59 | 7 / 9 |
| `wb:large-span-box-8-limbs` | 14,402,229 | 2.05 (2.02) | 3.72 (3.62) | 1.81 | 7 / 8 |
| `wb:extTS-L16` | 78,082,345 | 2.01 (2.00) | 3.56 (3.50) | 1.77 | 7 / 30 |
| `wb:extTS-L32` | 155,842,473 | 1.99 (1.98) | 3.54 (3.44) | 1.78 | 10 / 9 |
| `wb:own-content-sphere-1M` | 259,002,229 | 2.00 (1.98) | 3.47 (3.41) | 1.73 | 7 / 26 |
| `wb:viewless-MAT4` | 80,047,000 | 1.99 (1.93) | 3.42 (3.35) | 1.72 | 9 / 7 |
| `probe:sph-E500-L8` | 92,482,405 | 1.91 (1.89) | 3.40 (3.37) | 1.78 | 7 / 7 |
| `probe:extTS-E500-L8` | 117,922,405 | 1.72 (1.70) | 3.06 (3.02) | 1.78 | 7 / 7 |
| `probe:sphT-E500-L8` | 164,162,405 | 1.54 (1.53) | 2.78 (2.74) | 1.80 | 7 / 9 |
| `wb:json-deep-numbers-glb-62` | 108,175,300 | 1.41 (1.25) | 2.33 (2.28) | 1.66 | 7 / 7 |
| `wb:json-flat-numbers-glb` | 108,119,500 | 1.38 (1.24) | 2.31 (2.11) | 1.67 | 13 / 10 |
| `wb:json-deep-numbers-glb-1` | 108,120,300 | 1.43 (1.38) | 2.29 (2.13) | 1.61 | 9 / 9 |
| `wb:json-deep-numbers-glb-30` | 108,146,500 | 1.49 (1.32) | 2.28 (2.13) | 1.53 | 9 / 12 |
| `wb:json-flat-numbers-b3dm` | 108,124,300 | 1.23 (1.12) | 2.10 (1.92) | 1.70 | 8 / 9 |
| `wb:json-deep-numbers-b3dm-62` | 108,180,300 | 1.23 (1.20) | 2.09 (2.04) | 1.69 | 10 / 10 |
| `wb:json-deep-numbers-b3dm-30` | 108,151,500 | 1.28 (1.16) | 2.06 (1.89) | 1.61 | 7 / 9 |
| `wb:json-deep-numbers-b3dm-60` | 108,178,300 | 1.26 (1.09) | 2.04 (1.92) | 1.62 | 7 / 9 |
| `wb:json-deep-numbers-b3dm-1` | 108,125,500 | 1.34 (1.28) | 2.04 (1.85) | 1.51 | 8 / 8 |
| `wb:shared-accessor-16000x200` | 17,687,759 | 0.93 (0.90) | 1.56 (1.42) | 1.68 | 7 / 9 |
| `probe:refs-json64k` | 271,656,000 | 0.90 (0.86) | 1.55 (1.51) | 1.71 | 7 / 7 |
| `wb:large-span-sphere-16-limbs` | 52,802,229 | 0.89 (0.88) | 1.52 (1.47) | 1.72 | 7 / 16 |
| `probe:nodes-deep` | 14,775,187 | 0.87 (0.85) | 1.47 (1.44) | 1.68 | 7 / 7 |
| `wb:json-gltf-1MiB-refs` | 1,051,015,000 | 0.76 (0.75) | 1.38 (1.37) | 1.81 | 7 / 7 |
| `wb:json-flat-string-glb` | 52,548,300 | 0.75 (0.75) | 1.37 (1.32) | 1.82 | 7 / 10 |
| `probe:refs-json1m` | 1,050,954,000 | 0.79 (0.78) | 1.36 (1.30) | 1.72 | 7 / 7 |
| `wb:json-deep-string-glb-60` | 52,602,300 | 0.76 (0.75) | 1.34 (1.31) | 1.77 | 7 / 8 |
| `wb:json-deep-string-b3dm-60` | 52,607,100 | 0.68 (0.64) | 1.17 (1.15) | 1.73 | 7 / 11 |
| `probe:boxS-scale500-L8` | 253,712,405 | 0.29 (0.29) | 0.53 (0.52) | 1.78 | 8 / 11 |
| `probe:boxS-scale1000-L4` | 289,142,373 | 0.24 (0.24) | 0.44 (0.43) | 1.87 | 7 / 9 |
| `probe:extS-scale1000-L4` | 216,592,373 | 0.22 (0.22) | 0.42 (0.40) | 1.86 | 7 / 7 |
| `probe:sphS-scale1000-L2` | 399,902,357 | 0.16 (0.16) | 0.28 (0.27) | 1.78 | 8 / 10 |
| `probe:refs-bin256k` | 1,057,896,000 | 0.09 (0.08) | 0.16 (0.16) | 1.92 | 7 / 7 |
| `probe:refs-bin1m` | 1,050,906,000 | 0.07 (0.07) | 0.16 (0.16) | 2.13 | 7 / 8 |
| `producer:mf2-lod3-explicit` | 438,590,913 | C1 refuses (`JSON exceeds value-node limit`) | — | — | — |

Replay (measurement host; paths are in the scripts):

```sh
python3 -I wb/generate.py --large                     # O-WB matrix-size fixtures (wb/large/)
python3 -I acceptance/wb-gate/probes/wbcases.py WORK  # R's probe matrix
python3 -I acceptance/wb-gate/probe_units.py WORK/cases.json WORK/gate-cases.json
python3 -I acceptance/wb-gate/cases.py REPO TWINS     # class fixtures + producer twins
acceptance/wb-gate/run_gate2.sh                       # P and E passes until every case is decided
python3 -I acceptance/wb-gate/table.py table.json gate-P.jsonl -- gate-E.jsonl
```

| File | sha256 |
| --- | --- |
| `wb-gate/results/table.json` | `61da485bdf3f25f82e97a83c67c6fa8920fb40ba56f436f625064b8427957152` |
| `wb-gate/results/cases.json` | `1ef2c9697bcf4bdd8522a0d543a0ec7fce5620fd5dc0082a04cb548bf8d4f253` |
| `wb-gate/results/gate-P.jsonl.gz` | `4729ddbcfc708849489e5f2f946e10c305edf6be4bb9847d40dd2ad5e5a6c511` |
| `wb-gate/results/gate-E.jsonl.gz` | `8b7f2f8e7e297e5916c75bbc589c3424a44822fa6525428dcea35d6f13f51e31` |
| `wb-gate/results/final-P.jsonl.gz` | `f362a28e18f0a6d129c6b86c6e63616fc0eac8438464124f6794f2743adcdd26` |
| `wb-gate/results/final-E.jsonl.gz` | `c908677b346c1d30fd8b7ee8027e9ff404f0c29389553e1bf0f52d4f46d58adf` |
| `wb/manifest.json` (units source) | `1bfff2174863b9b8f48d5047afdc075390d0d643c416abf754142ccabc7cd0cc` |
| measurement binary (cap 2^62) | `c39ee37428b5a970c62d4721830e6726d5eb1c6980a1e328c77be3f0908f8a8b` |
| final binary (cap 2^29) | `f98d5b4c3e3613271d56ee633be452a57bc7a8157164999783a93d92efd9acef` |
| C2-WB `contract-draft.md` v2.1 | `ea7b3acff2d4768d1ab68333b6730c080b30eb7495060c449bbe0a8bef8eac97` |
