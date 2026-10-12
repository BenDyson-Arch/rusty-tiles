# C2 acceptance (coordinator-owned)

Production is compared only against the independent oracle's committed expectations. No
verdict here is derived from production code.

| Evidence | What it shows | Where it runs |
| --- | --- | --- |
| `oracle_parity.rs`, compiled as `--lib c2_oracle_parity` | Production `inspect_selected` equals the oracle on every witness and control (outcome, reasons, Outside depth/level/role/node, C1 refusal kind) | CI and local. The 24 MB `controls/large/` archives are gitignored, so CI skips only those three ids |
| `parity-a547d1a.json` | Local release run with the large controls: 261 cases, 261 agree, 0 skipped, 0 C1-precondition refusals | Local, Linux |
| `corpus_parity.py` and `corpus-parity-a547d1a-release.json` | `validate --json` on the 27 producer artifacts (develop f2238cd release producers, manifest sha256 `28e4bee6…`) equals the oracle corpus manifest: 17 Contained, 10 NotInspected(implicitTiling), 0 Outside | Local, Linux |
| `oracle_parity.rs`, `production_matches_every_c2wb_class_fixture` | Production equals O-WB on all 102 `wb/` rows: 55 at-cap pairs, 4 chain-bit controls, and the totals and per-URI facts (`T`, `Idx`, `E`, `Dec`, `B`, `J`, `Res`, `Prim`, `Nodes`) of 43 matrix-size rows | CI (CI rows) and local (matrix-size rows, after `wb/generate.py --large`) |
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

C2-WB (contract draft v2.1, sha256 `ea7b3acf…`, sections 5, 5.1, 6, 9.D) sets the default
`containmentWorkUnits` from a measured class matrix: a calibration on one host, not a proof.
Raw runs, scripts and the full per-class table are kept outside the repository (listed by
sha256 below).

**Parity (release, large controls generated).** C2 witnesses and controls 261/261, including
the work pairs 5,040/5,039 and 48,008,577/48,008,576. `wb/` class fixtures 102/102: 55 at-cap
pairs, 4 chain-bit controls, totals and per-URI facts of 43 matrix-size rows (so C1 admits
every large class fixture). I1 181/181 and corpus 41/41. O-WB readings resolved against
production: `Res` and `B` count per resolver call (per buffer or image entry); `data:` URIs are
not resources; "16 levels" means `L_r = 15`.

**Method.** Intel i7-12700KF, Linux 7.2.9, release build of `e8cf53a` with only the default
cap raised to 2^62 (so every class streams), then re-checked with the final 2^29 build. A is
the case, B its `gltfUpAxis` twin; figure = child CPU of A − B. A run counts only with own-CPU
foreign busy ≤ 5% and neighbour ≤ 10% (P: SMT sibling; E: busiest other E-core of 16–19, from
`/proc/schedstat`); median of 7 clean pairs per case. Units come from O-WB's oracle only.
96 cases: O-WB's 43 matrix-size fixtures, R's probe matrix (29 shapes), 9 b51f83d producer
twins, an images-by-URI class. 717 P and 900 E attempts; load average 0.97–10.1.

| | τ (ns/unit) | Worst class | Default cap | At the cap |
| --- | --- | --- | --- | --- |
| **E-core** | **14.65** | `boxT-E300-L8` (box + step, 8 levels, 2^300 translations) | **2^29** | **7.87 s** |
| P-core | 8.10 | same | 2^29 | 4.35 s |

`τ_E ≤ 14.9`, so the cap is 2^29 (margin 1.7%). E/P per class: 1.47–2.13, median 1.74.

| Class | P ns/unit | E ns/unit |
| --- | --- | --- |
| 1-limb / small-span chains (worst 8 classes) | 7.21–8.10 | 13.6–14.65 |
| long-significand decimals (5.1, first measurement) | 5.23 | 11.02 |
| meshopt NONE: ATTRIBUTES bits / raw / INDICES / TRIANGLES (5.1, first measurement) | 4.35 / 4.06 / 2.55 / 2.47 | 7.59 / 7.08 / 4.41 / 4.27 |
| viewless accessors SCALAR / MAT4 | 3.62 / 1.99 | 5.33 / 3.42 |
| unselected `{}` nodes; selected skipped nodes | 5.97; 4.54 | 8.78; 7.56 |
| 100,000 tiny references, 4 / 103 levels | 4.15 / 5.25 | 7.12 / 9.02 |
| external buffers 20,000 × 20; images by URI 20,000 × 20 | 4.02; 4.17 | 7.07; 7.06 |
| deep JSON numbers GLB depth 1 / 62 (was 22.22 E at 62) | 1.43 / 1.41 | 2.29 / 2.33 |
| `quad` / `quadB` / end-to-end (were 35.26 / 77.23 / 83.66 E) | 4.92 / 4.60 / 4.46 | 8.54 / 8.04 / 7.74 |
| producer twins W1–W6d | 4.58–5.94 | 7.66–10.12 |

Other 9.D items (final build): precheck-only arm, 1,047,040 references under a 100-document
chain, A − B 0.58 s P / 0.99 s E; phase-1 excluded twins at depth 1 vs 100 differ by ≤ 0.044 s
(100,000 and 1,047,040 references). At 2^29 the W5 twins use 38–40% of the cap; mf2-lod3
explicit (438,590,913 units, 81.7%) is refused by C1 (`JSON exceeds value-node limit`, also at
b51f83d) and was not timed. Not re-run: the 9.C production mutants.

Evidence outside the repository (`/home/bend/.cache/c2wb-evidence/`):

| File | sha256 |
| --- | --- |
| `wb-gate/results/table.json` (all 96 classes, P and E) | `61da485bdf3f25f82e97a83c67c6fa8920fb40ba56f436f625064b8427957152` |
| `wb-gate/results/cases.json` (archive and unit pins) | `1ef2c9697bcf4bdd8522a0d543a0ec7fce5620fd5dc0082a04cb548bf8d4f253` |
| `wb-gate/results/gate-P.jsonl.gz` | `4729ddbcfc708849489e5f2f946e10c305edf6be4bb9847d40dd2ad5e5a6c511` |
| `wb-gate/results/gate-E.jsonl.gz` | `8b7f2f8e7e297e5916c75bbc589c3424a44822fa6525428dcea35d6f13f51e31` |
| `wb-gate/results/final-P.jsonl.gz` | `f362a28e18f0a6d129c6b86c6e63616fc0eac8438464124f6794f2743adcdd26` |
| `wb-gate/results/final-E.jsonl.gz` | `c908677b346c1d30fd8b7ee8027e9ff404f0c29389553e1bf0f52d4f46d58adf` |
| measurement binary (cap 2^62) | `c39ee37428b5a970c62d4721830e6726d5eb1c6980a1e328c77be3f0908f8a8b` |
| final binary (cap 2^29) | `f98d5b4c3e3613271d56ee633be452a57bc7a8157164999783a93d92efd9acef` |
| `wb/manifest.json` (units source, in the repository) | `1bfff2174863b9b8f48d5047afdc075390d0d643c416abf754142ccabc7cd0cc` |
