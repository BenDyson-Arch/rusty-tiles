# Mesh bake-time history: v0.3.0 to 2026-10-24

Read-only forensic audit at develop `f97cd930`. No conversions, builds, tests or
benchmarks were run. **The completed 10M-triangle sequence is approximately
67 → 82–126 → 1,063 → 1,000 CPU-s/Mtri**, for H8 leaves, H17 untextured LOD,
#251 textured LOD, and #251+#260 respectively. These are different products and
worker configurations, not a controlled regression curve. A same-model v0.3.0
timing was **not recovered**; the recovered release baseline is a small fixture.

## Recorded timeline

CPU = user + system seconds, divided by **source** triangles for CPU-s/Mtri.
RSS is GiB; scratch is decimal GB. `—` means unrecorded, not zero. `M` is
`mgal_detail_offset_2048.glb`, 9,999,999 triangles, 89 JPEG 2048 atlases; `F` is
the same geometry with original 8192 textures. `T` is textured `tex3.glb`,
788,425 triangles; `C` is its vertex-colour `crop3.glb` counterpart. Source keys
below resolve to exact evidence files. Dates initially follow evidence series;
the dating exceptions below prevent treating directory names as execution dates.

| Series / source | Source / commit | Tool; input; profile | Jobs / CPU affinity | Load recorded | Wall s | CPU s | CPU-s/Mtri | RSS GiB | Peak scratch GB |
|---|---|---|---|---|---:|---:|---:|---:|---:|
| Release 10-07, measured 10-13 [L] | v0.3.0 `99b9605b` | `mesh-to-3tz`; 96,800-triangle fixture, 512 texture; CLI JPEG default | Rayon 2 / 6–7 | 17.1 start¹ | 0.114 | 0.14649 | 1.51 | 0.070 | — |
| 10-11 [L0] | `f2238cd` legacy path | `mesh-to-3tz`; same fixture | Rayon 2 / 0–1 | 6.38 start¹ | 0.229 | 0.31680 | 3.27 | 0.089 | — |
| 10-13 [L] | `69aa85e` legacy path | `mesh-to-3tz`; same fixture | Rayon 2 / 6–7 | 17.1 start¹ | 0.248 | 0.33983 | 3.51 | 0.089 | — |
| H8 10-13 [A0] | `rt-v6`, pre-review #199 | `mesh-leaf-payloads`; M; balanced | **auto→20** / 20 visible, affinity — | 5.12 start; 57.3% busy | 88.81 | 660.74 | 66.07 | 6.93 | 5.066 sampled |
| H8 10-13 [A] | `6dd69fd`, #199 | `mesh-leaf-payloads`; M; balanced | **auto→20** / 20 visible, affinity — | 25.1 start; 92.1% busy | 122.60 | 668.92 | 66.89 | 7.61 | 6.596 sampled |
| H16 10-13 [B] | `3a52ecb` | `mesh-large-to-3tz`; M converted to vertex colours; balanced | 4 / 12–15 | — | 193.64 | 78.50 | 7.85 | 0.182 | — |
| H17 series [C0] | `128321f`, #209 pre-review | `mesh-large-to-3tz`; M; balanced | 4 / — | not bound to this record | 389.90 | 1,257.78 | 125.78 | 2.73 | — |
| H17 series [C] | `0d70f08`, #209 | `mesh-large-to-3tz`; M; balanced | 4 / — | README range 7.6–12 | 396.51 | 1,259.38 | 125.94 | 2.65 | — |
| H17 repeat [C2] | same | same | 2 / — | same series range | 480.05 | 819.39 | 81.94 | 1.96 | — |
| VS-C evidence 10-14 [D] | H16 producer; exact producer SHA absent | `mesh-large-to-3tz`; F; balanced, **leaf-only content** | 4 / 8–11 | — | 2,131.67 | 5,772.73 | 577.27 | 12.41 | — |
| Scratch 10-14 [E0] | `0072977` | `mesh`; M; **lossless** | 4 / 12–15 | 27.7 | 293.0 | 637 | 63.70 | 1.96 | 6.551 spill² |
| Scratch 10-14 [E1] | `7e53fe6`, #230 | same | 4 / 12–15 | 6.6 | 271.9 | 637 | 63.70 | 1.83 | 4.825 spill² |
| H27 10-20 [F0] | `21815e51`, #237 | `mesh`; T; balanced | 8 / 8–15 | 26.4 | 117.6 | 478 | 606.27 | 8.16 | — |
| H27 10-20 [F] | `bc6b3710`, #237 | same, corrected shading attributes | 8 / 8–15 | 5.1 | 119.6 | 471 | 597.39 | 8.85 | — |
| H28 10-21 [G] | `1f899917`, #244 evidence | `mesh`; T; balanced | 8 / 0–3,16–19 | 11.2 | 183.3 | 536 | 679.84 | 9.14 | —³ |
| 10-21 **failed** [H] | `f7648d07` per donor README | `mesh`; M; balanced | 4 / 8–15 | 2.7 | 880.7 | 2,603 | 260.30† | 9.33 | — |
| 10-22 **failed** [I3] | #246 diagnostic fix3 | `mesh`; M; balanced | 4 / 8–15 | 9.0 | 2,797.2 | 4,681 | 468.10† | 13.83 | ~254 sampled |
| 10-22 **failed** [I4] | #246 diagnostic fix4 | same | 4 / 8–15 | 9.7 | 3,084.7 | 5,018 | 501.80† | 13.78 | ~254 sampled |
| #251, executed **10-23** [J] | `bccac563` final evidence binary | `mesh`; M; balanced | **4** / 0–3,16–19, SCHED_IDLE | 19.3 | 6,457.1 | **10,632** | **1,063.20** | 13.09 | **118.737 sampled** |
| #251+#260 10-23 [K] | `c01a24f8` + `dc8ff0db` | same, tighter proxy atlas | **4** / same, SCHED_IDLE | 20.6 | 3,612.2 | **9,998** | **999.80** | 9.37 | —³ |

¹ Perf-lane environment load covers the suite, not each tiny mesh sample. Its
summary pairs CPU with the best-wall sample; RSS is the suite's per-case peak.
² Maximum logical `spill` bytes in [E]'s one-second samples, excluding staged
output/work; `mgal-develop.split.samples:82` and `mgal-B.split.samples:88`.
The later [J] monitor samples its scratch directory every ten seconds; these
are observed peaks with different boundaries/cadences, not exact disk maxima.
³ `capture.run_scratch_peak_bytes` is **not** a measurement of all H27 retained
packets: [G] reports 0.479 GB and [K] 4.943 GB there. Neither is presented as a
whole-bake peak. † Failed work divided by full input size, **not throughput**.

**Corrections to the motivating chronology.** [A]'s `stage-timing.json` records
20 delivery jobs, not 8. [J,K] report `requested_jobs: "4"`; eight is the affinity
CPU count. [J]'s first progress timestamp is 2026-10-23 17:12:47 Brisbane despite
its `bake-bounded-20261022` directory. H17's `0d70f08` commit is dated 10-14 even
though the evidence series is named 10-13. [D]'s archive mtime is 10-13; VS-C
validated it on 10-14. `4e55659` in its `identity.txt` identifies the **validator**,
not the 2,132-second producer. [D] has 1,959 payloads / 2,167 members and no
proxy count, versus H17's 847 proxies / 3,014 members [C].

## Where the time went

These are **phase elapsed seconds**, with percentages of measured run wall.
They cannot be converted into phase CPU by multiplying by jobs. H17's leaf
figure is derived: 390.200 − 46.003 − 190.644; its older `leaves` timer encloses
capture and LOD. H8 has 971 leaves versus 1,959 in H17/current M, so even the
leaf comparison is not a fixed-work A/B.

| Phase | H17 [C] | #251 [J] | #251+#260 [K] |
|---|---:|---:|---:|
| Capture | 46.0 (11.6%) | 79.6 (1.2%) | 113.0 (3.1%) |
| Leaf build | 153.6 (38.7%) | 168.1 (2.6%) | 137.4 (3.8%) |
| LOD pre-pass | included below | 17.9 (0.3%) | 14.5 (0.4%) |
| Simplification | included below | 63.5 (1.0%) | 61.4 (1.7%) |
| Seam resolution | included below | 86.5 (1.3%) | 97.5 (2.7%) |
| LOD finish: certificates, fills, encoding/readback | included below | 109.2 (1.7%) | 108.9 (3.0%) |
| **LOD subtotal** | **190.6 (48.1%)** | **277.1 (4.3%)** | **282.3 (7.8%)** |
| **Proxy texture bake** | absent | **5,919.1 (91.7%)** | **3,046.4 (84.3%)** |
| Archive | 6.2 (1.6%) | 10.7 (0.2%) | 22.7 (0.6%) |

Unclassified orchestration accounts for the small remainder. [H]'s four-line
log only establishes capture 63.95 s and another 816.68 s before donor failure;
it does **not** time each inner phase. [I3,I4] put LOD at ~199/~193 s despite
multi-thousand-second incomplete runs: the new bottleneck is downstream.
[D] records 2,115.8 s in the enclosing leaf stage and 15.7 s in delivery, with
no finer attribution; the 16× source texel-area difference from M precludes
assigning its cost to H27, H28 or certification.

### Proxy work, retries and the misleading “halving”

| Recorded proxy step | #251 [J], worker-s | #260 [K], worker-s | Share of five named worker timers, before → after |
|---|---:|---:|---:|
| Charts | 7.58 | 21.38 | 0.07% → 0.22% |
| **Raster / appearance sampling** | **9,560.12** | **9,128.43** | **90.89% → 95.62%** |
| Encode/delivery, including mips and image gates | 559.85 | 132.45 | 5.32% → 1.39% |
| Assemble | 7.47 | 7.02 | 0.07% → 0.07% |
| Commit, including packet persistence/readback | 383.28 | 257.21 | 3.64% → 2.69% |
| Source decode (**nested in raster**, not additive) | 333.04 / 2,354 decodes | 96.43 / 614 decodes | 3.17% → 1.01% of named total |
| Child texel tile reads | 1,394,648 | 615,767 | count, not seconds |
| Retried tiles / successful tiles | 69 / 847 | 0 / 847 | retry count does not measure discarded CPU |

`Instant::elapsed` powers these timers (`src/mesh_large/bake/mod.rs:327`,
`integration.rs:396`, `src/mesh_large/phases.rs:75`). They include descheduling,
I/O and overlapping workers; they are **not CPU seconds** and need not sum to
rusage. Successful-tile statistics also do not fully account for discarded
attempts. An exact CPU cost split between raster, verification and I/O is absent
for the full model; attributing 96% of its *CPU* to raster would overstate the data.

#260 reduced proxy elapsed time **48.5%**, whole-run elapsed **44.1%**, but
user+sys **only 6.0%** (9,598+1,034 → 9,282+716). CPU/wall rose **1.65 → 2.77**.
That is a mixture of less work, less I/O/admission pressure, better utilization
and different contention; load averages alone cannot apportion it. H8 supplies
another control: wall rose 38.0% from [A0] to [A], CPU only 1.2%. CPU accounting
removes time not scheduled, but CPU frequency, core type, cache contention and
retry paths can still affect CPU seconds. The H17 jobs-2 versus jobs-4 CPU
spread is another reason not to call their delta a pure feature cost.

| Proxy depth | #251 elapsed s / retries [J] | #260 elapsed s / retries [K] |
|---|---:|---:|
| 8 | 39.1 / 0 | 29.9 / 0 |
| 7 | 990.7 / 24 | 549.6 / 0 |
| 6 | 1,975.0 / 34 | 1,001.2 / 0 |
| 5 | 442.5 / 6 | 246.2 / 0 |
| 4 | 153.5 / 2 | 50.7 / 0 |
| 3 | 333.1 / 1 | 273.4 / 0 |
| 2 | 530.6 / 0 | 429.1 / 0 |
| 1 | 547.7 / 2 | 454.0 / 0 |
| 0 | 906.5 / 0 | 12.2 / 0 |

The root's collapse is real elapsed evidence, not proof of a 74× CPU speedup.
Depths 7–6 account for 52% of #260 proxy wall; depths 3–1 still take 1,156.5 s
(38%). Low parallelism and expensive queries remain after all retries disappear.

### Feature attribution and avoidable work

| Feature / change | Evidence and defensible attribution | Verdict |
|---|---|---|
| **H17 certified LOD** (#209, handoff #216) | 190.6 s / 48% of its original full run [C]. Recent pre-pass + simplification + seam + finish remain ~193–282 s [I,J,K]. Finish is not a certificate-only timer. | Decoded-geometry two-way bounds and crack-free same-depth seams are promised work; external joins and topology consensus are necessary mechanisms, but their implementation cost is not a lower bound. No isolated certificate CPU measurement. |
| **H27 textured proxies** (#237) | Adds the stage now taking 84–92% of full-model wall [J,K]. Pre-integration 122k subscene `prof-timing.log:1` has 77.457 s source-nearest inside 96.312 s raster; optimized `prof3-timing.log:1` has 0.279 s source-nearest inside 5.428 s raster. Larger T's 7,021 s / 18,819 CPU-s debugging run became 117.6 s / 478 CPU-s, but intermediate code/configuration differed [F0]. | Appearance reconstruction is inherent; exhaustive candidate visits, repeated identical image work and excessive page/ledger sizes are not. Historical BVH and nearest-first fixes already landed; do not sell their old wins as remaining savings. |
| **H28 batching** (#244) | T's H27 471 CPU-s vs H28 536 (+13.8%); wall +53.3%, different affinity/load [F,G]. H28 changes class packing, exact UV ownership checks and primitive merging, while disabling default proxy ID textures; root textures 110→1, leaf primitives 1,439→156 [G README]. | No isolated H28 timer or controlled bake A/B. Cannot assign the +65 CPU-s specifically to batching or claim it explains the full-model regression. KTX2/fidelity gates predate H28. |
| **KTX2 effort and image verification** | H17 leaf worker timers: Basis 477.31 s; codec verify 47.83; fidelity 13.52; sparse mips 32.67 [C timing]. H8 reviewed: 830.66, 90.50, 24.05, 71.97 [A timing]. Verify+fidelity are 10.1% / 10.6% of non-capture/non-LOD named leaf timers respectively, not total CPU. Current balanced effort remains 1 (UASTC pack level 0), `leaves/delivery.rs:448`; #260 proxy delivery is just 1.39% of named worker timers. | Required all-level validation costs something, but encoder effort did not suddenly increase with H28. Turning off verification is neither justified nor the largest available lever. |
| **Codec/gate escalation** (#229/#234) | Each failed page step re-encodes and verifies: profile → UASTC level 2 → level 3 → PNG only for eligible level-0 samplers (`leaves/delivery.rs:724,936`). [J,K] **leaf** reports record zero escalated pages, zero codec/gate causes and zero PNG fallbacks. Proxy step counts are not exposed by those leaf aggregates. | Legitimate recovery, potentially repeated work on difficult inputs. No evidence it dominates M; cannot infer zero proxy escalation or invent a separate time share. Successful codec decodes are already reused at `delivery.rs:962`; a blanket “duplicate decode for every fidelity gate” claim is false. |
| **Scratch and commit** | [I] ~254 GB spill before failure; [J] 118.737 GB on a completed run. Packets were ~330 B/texel/level; a d6 parent required 15.5 GB of child ownership. #251 stores a 109-byte terminal sample plus RGBA, streams packets and reads tiles lazily [J README]. Commit is 3.64→2.69% of named worker timers; persistence also affects raster reads and waiting. | Witness data is required; duplicate frames/base arrays, unused retained leaf pixels and whole-child decoding are representation waste. `bake_pass.rs:798`, `lod/mod.rs:3108`, `leaves/producer.rs:407,825` call `sync_all`; no log separates fsync from serialization/readback or host I/O. Do not attribute the whole gap to fsync. |
| **Jobs / memory admission** | Develop `bake/integration.rs:234,327` divides the allowance equally, then reruns refused tiles serially. [I3] d7: 25 retries, 1,676.6 s; profile windows put ~1,390 s (83% of d7 elapsed) in its serial portion. #251 uses halving worker counts / doubled shares (`bccac563:integration.rs:286`); [J] refusals at 4.541 and 9.083 GiB. 14 of 25 earlier d7 refusals had already rasterised before failing codec/ledger admission [J README]. | Bounded ownership is inherent. Late refusal, discarded raster work and equal-share-induced serial tails are avoidable. #251's pre-check and graduated retries help; #260 removes retries for this input, so those savings must not be counted again on [K]. |
| **Cancellation lock convoy** (#246) | C fix2→fix3: 2,764→283 CPU-s, 905.9→112.1 s wall; system CPU 712→3 s. Source and colour code also changed, so this is a combined repair, not a perfectly isolated lock benchmark [I README]. `integration.rs:245` records the lock mechanism. | Demonstrated historical waste, largely repaired. Do not extrapolate pre-fix multi-hour failures to current expected throughput. |

The H17 contract is [here](h17-lod-contract.md); the bake contract and integration
record are [here](../../bench/architecture_audit/mesh_large_h27/README.md), with
the source design in `E/tiler-bake-design-20261013/`. H28's unchanged-geometry and
batching guarantees are [here](h28-tile-batching-contract.md). Today's
[mesh guide](../MESH.md) explicitly records original-source sampling above
leaves, the missing leaf terminal ledger, and one-thread-per-proxy execution.

### October 24 CPU-stamped control: #251 is not uniformly faster

All rows are C, 788,425 triangles, balanced, four jobs, CPUs 8–15. Base/head
names follow the saved queue; unlike [J], their exact binary SHA is not in each
run record. Do not silently bind them to the latest PR head. CPU below comes
from `/proc` stamps, not worker timers; `s1` is a sampled run and less controlled.

| Pair [P] | Base → head CPU s | CPU-s/Mtri | Proxy phase CPU s | LOD + handoff CPU s | Wall s | Load start | RSS GiB |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | 435 → 473 | 552 → 600 | 383.97 → 425.67 | 46.71 → 42.31 | 160.1 → 171.1 | 51.9 → 16.1 | 1.76 → 1.80 |
| 2 | 389 → 467 | 493 → 592 | 339.50 → 417.48 | 44.03 → 44.31 | 143.9 → 166.8 | 19.8 → 18.0 | 1.80 → 1.79 |
| s1 | 371 → 490 | 471 → 621 | 322.84 → 431.27 | 42.63 → 53.28 | 137.1 → 193.7 | 12.5 → 10.7 | 1.80 → 1.74 |

Peak scratch was not recorded in these runs. Proxy CPU is 88–90% of base total
and 88–90% of head total. The unsampled pairs locate ~42–78 additional CPU-s in
the bake, while capture/leaves are near-flat. This supports investigating a
real regression; it does not isolate its code cause. Earlier five paired C runs
already failed the 5% CPU gate (median paired ratio 1.221). In contrast, #251's
three paired T runs passed at median ratio **0.630**, byte-identical output.
#252's CPU ratios were 1.019 T / 1.033 C, and #254's 0.996 T / 0.989 C [Q].
Thus compact records reduce space without an established large CPU saving.

## What the release tool actually did

**v0.3.0 already made textured LOD.** Inspecting the tag, not the retired files
at HEAD:

| Capability | v0.3.0 source evidence | Difference from current mesh |
|---|---|---|
| Geometry LOD | `src/hlod.rs:67,106,197`: weld + meshopt simplification of children, target roughly a quarter of child triangles; `src/tile.rs:25` default leaf cap 20,000 | H17 frozen lattice, source provenance, seam consensus and certified decoded bounds; current leaves cap 8,192 |
| Textured parents | `src/hlod.rs:138`; `src/texture.rs:837,1696`: chart unwrap, nearest child surface sampling, child atlas downsampling to parent density, lossless PNG intermediate | H27 supports per-slot material/sampler/normal semantics and terminal evidence; leaf-adjacent proxies currently re-query original source |
| Error estimate | `src/hlod.rs:392,434`: sampled two-way distance using strided vertices and triangle centroids; `src/tile.rs:795` combines accumulated geometry with texel size ×16 | Sampled estimate is not H17's conservative certificate; no equivalent global seam-consensus guarantee |
| Default delivery | `src/main.rs:419`: CLI default **JPEG**; `src/tile.rs:716` dispatches PNG/JPEG/WebP/UASTC; Rust options default to PNG (`Lossless`) | Balanced now uses authored KTX2 mip chains and verification. Legacy enum “Lossless” is not today's leaf/proxy profile contract |
| Verification / escalation | `src/gpu_texture.rs:37,66`: optional external Basis UASTC levels 0 then 1; encoder-reported base RGB/alpha PSNR floors, KTX2 signature, PNG fallback | Not “no verification”, but not today's independently decoded every-mip fidelity, semantic ledger and final-GLB readback guarantees. Default JPEG bypasses that UASTC gate |

The recovered perf-lane release run explicitly logs JPEG, eight leaves and one
parent on a 96,800-triangle synthetic heightfield [L]. It is **not** a 10M-model
measurement and cannot support a 660× real-model slowdown claim. The archived
`perf/legacy-results.tar.zst` was extracted only into `/tmp`; its
`results/baseline-process.json` names **f2238cd / version 0.4.0**, not v0.3.0.
The later lane's `environment.binaries.base` explicitly identifies the v0.3.0
binary and SHA. The October 12 lane records place that tiny baseline at
0.111–0.115 s / 0.143–0.147 CPU-s; they are not independent full-model evidence.
The owner may remember a valid fast legacy model run, but none was located in
the supplied records. Its codec, triangle budget and command remain unknown.

## Ranked opportunities

Ranges below are **engineering estimates**, not new measurements. Unless an
observed result is named, they are conditional savings against [K]'s completed
run, with unchanged guarantees. They overlap and must not be added. Effort is
rough implementation + independent correctness review, excluding scheduling.

| Rank / opportunity | Expected saving and basis | Output / effort / status |
|---|---|---|
| 1. Reduce raster query work: reuse face-local candidates/witnesses, improve BVH pruning and cache locality while preserving exact ties | **15–40% total CPU** if the dominant raster work drops 20–45%; lower confidence because full-model timers are not CPU. Historical nearest-query controls demonstrate large avoidable searches [F profiling], and [K] still spends 95.6% of named proxy worker time here. No promise of another historical 10× gain. | Output-neutral only with differential search/witness checks; **1–3 weeks**. Include the C regression investigation before extending shared-cache machinery. |
| 2. Land/evaluate the tighter proxy atlas and source-order visits, **#260** | Already observed **6% whole-run CPU, 44% wall, 49% proxy wall**, no additional saving to claim against [K]. T page texels 53.7M→7.1M with ~same covered texels; M source decodes 2,354→614 [K README]. | **Output-changing proxies**, leaves byte-identical; implementation done, visual/contract review remains. Density inheritance shifts slightly; not purely a packer speed tweak. |
| 3. Bounded packets + shared images, **#251**, with C regression resolved | Already observed **23–46% CPU saving per paired T run** (median paired 37%); full M previously failed, so no full-run base speedup exists. Recovering C's unsampled excess would save **8–17% of head C total CPU** [P]. | Output-neutral in recorded crop byte-identity checks; implemented, **several days–1 week** to isolate regression. Current PR is open; do not dismiss C's CPU gate failure as wall noise. |
| 4. Precompute exact 8-bit sRGB→linear lookup for source/child taps | **3–15% total CPU**, conditional: old d7 profile has 18% self samples in `pow`, but the post-#260 distribution is unknown [I profile]. `source.rs:1319,1406` and `sampling.rs:58,330` still exponentiate byte-derived values. Keep continuous linear→sRGB conversion separate. | Output-neutral if entries use the identical expression and rounding; **1–3 days**. #251 README lists it as deferred; earlier mip LUTs do not remove every tap conversion. |
| 5. Remove unused leaf pixel retention, raster duplicate arrays; further streamline packet reads/commit | **0–10% CPU**, potentially **5–20% wall on I/O/admission-bound inputs**; smaller or zero wall win on [K], which has no retries. Commit is only 2.7% of its named worker time, so merely deleting fsync cannot explain hours. Larger opportunity is reducing raster reads and future memory refusals. [J] shows packet size reduction; #237 records 84 GB unused leaf pixels on the stopped 100M input, not M. | Output-neutral only if semantic readback/cancellation/atomic publication survive; **3–10 days**. #251 handles whole-child decoding, not every redundant representation. |
| 6. Parallelize within expensive shallow tiles under one ownership lease | **5–15% wall, approximately 0% CPU saving** (CPU may rise). Depths 1–2 alone consume 883 s, 24.4% of [K] wall; a partial 2× reduction could save ~12%. Dependency order prevents arbitrary cross-depth parallelism. | Output-neutral with deterministic reductions/writes; **1–2 weeks**. More jobs alone can shrink shares and hurt other inputs. |
| 7. Compact capture **#254**, following merged #252 | **0–3% whole-run CPU expected**, rather than its much larger space reduction: paired CPU ratios ~1.0 [Q]; capture is only 3.1% of [K] wall. #254 measures 27% C / 32% T logical stage-peak reduction [R]. Larger benefit is admission and large-input I/O. | Output-neutral; implementation done/open, review/landing effort. Does not shrink the dominant proxy raster workload. |
| 8. Tune codec effort/format, preserving all gates; avoid proven redundant verification only | **0–3% total CPU expected on M after #260**, low confidence; proxy encode/delivery is only 1.4% of named worker time, leaves only 3.8% of wall. More relevant to F and high-quality profiles. No measured time win follows from ETC1S archive-size results [U]. | Different codec/effort changes output: **3–7 days** plus quality review. Reusing an existing accepted decode is output-neutral and already implemented. Weakening verification would change the promise and is not recommended. |

**Verdict:** certification and textured, verified proxies justify additional work,
but not all observed expense. The records demonstrate avoidable lock contention,
search amplification, decoded-image churn, enormous mostly-empty atlases, large
duplicated witness representations and repeated admission attempts. #251/#260
address several of these, while residual raster work and the C CPU regression
remain. Full-model CPU is still ~8× the H17 four-job run after #260; elapsed
slowdown alone exaggerates that comparison. There is no defensible exact
“inherent minimum” or additive CPU budget per feature in the existing logs.

## Evidence index and reproduction

`E` = `/home/bend/.cache/rusty-tiles-evidence`; `S` =
`/home/bend/scratch/rusty-tiles-scratch`. JSON selectors below disambiguate
one-line reports. All code line numbers refer to `f97cd930` unless a tag/commit
is written explicitly. Historical files can be read with `git show REF:path`;
no other checkout is needed. PR descriptions were read with `gh pr view` on
2026-10-24; #251, #254 and #260 were open, #209/#234/#237/#244 merged. PR heads
may be newer than measured artifacts; the table uses evidence identities.

| Key | Evidence paths / selectors |
|---|---|
| L | `E/perf/2026-10-13-69aa85e/process.json`: `cases[case=mesh].sides.{base,head}`, `environment`; `meta.json`; `logs-base/mesh.log:1-8`. Fixture definition `bench/perf_lane/corpus.py:98`. Earlier comparable fixture records: `E/perf/2026-10-12-{a86f429,b13a86d,d1874ef,d1874ef.2}/process.json`, same selectors. |
| L0 | `E/perf/2026-10-11-f2238cd/process.json`, same selectors, head side; identical archived baseline in `E/perf/legacy-results.tar.zst::results/baseline-process.json`. |
| A0, A | `E/h8-perf-20261013/runs/{v6,v8}-full2048-balanced/{run.json,stage-timing.json,stdout.json}`; `run.delivery.jobs` in timing; `E/h8-perf-20261013/pr-body.md:43` binds reviewed head. |
| B | `E/h16-20261013/out/balanced.{run.json,stdout}`; input preparation and stage time in `bench/architecture_audit/mesh_large_h16/README.md:23-37`. |
| C0, C, C2 | `E/h17-20261013/real/prev-128321f/default.run.json`; `real/{default,det2}.{run.json,timing.json,stdout}`; `E/h17-20261013/pr-body.md:68-77` load/configuration. |
| D | `E/vs-c-20261014/real/{run.run.json,run.stdout,identity.txt}`; `README.md:10-13` identifies the H16 producer; `resources.json` is the separate validator measurement and is excluded. |
| E0, E1 | `E/mesh-scratch-20261014/{mgal-develop,mgal-B}.measure.json:1`; `README.md:2-8` binds binary/profile; companion `.stdout` and `.split.samples` retain stage/scratch evidence. |
| F0, F | `E/h27-int-20261020/{crop-21815e51-jobs8,crop-bc6b3710-jobs8}/{measure.json,report.json}`; `crop-debug3/measure.json:1`; `prof-timing.log:1`, `prof3-timing.log:1`; [#237](https://github.com/BenDyson-Arch/rusty-tiles/pull/237) evidence and limitations. |
| G | `E/h28-20261021/{measure.json,report.json}:1`; `README.md:3-11`; [#244](https://github.com/BenDyson-Arch/rusty-tiles/pull/244). |
| H | `S/mgal-diag/{measure.json,report.json,run.sh}`; `progress.log:1-4`; identity in `E/proxy-donor-20261022/README.md:10-16`. |
| I3, I4 | `E/proxy-donor-20261022/m2048-{fix3,fix4}/{measure.json,progress.log}`; `README.md:3-6,30-70`; fix3 `progress.log:286,320`; `m2048-fix3/profile-summary.txt` sections `bake_d7_parallel` and `bake_d7_serial_retries`; `crop3-{fix2,prof}/measure.json:1`. |
| J | `E/bake-bounded-20261022/m2048-final/{measure.json,diag.json,spill.peak,rt.sha256}`; `progress.log:4,224-270,286,289-352,360,450,511,534,543,548,550,552-558`; `S/bake-bounded/m2048-final/report.json:1` (`diagnostics`, `tilerReport`); `E/bake-bounded-20261022/{README.md,q10.sh,run.sh}`. [#251](https://github.com/BenDyson-Arch/rusty-tiles/pull/251). |
| K | `E/proxy-density-20261023/m2048/{measure-after.json,report-after.json}:1`; `S/proxy-density/m2048/stderr.log:4,224-281,301,476,885,1183,1294,1337,1358,1364,1367-1373`; `E/proxy-density-20261023/{README.md,scripts/bake.sh}`. [#260](https://github.com/BenDyson-Arch/rusty-tiles/pull/260). |
| P | `E/p251-cpu-20261024/runs/crop3-{base,head}-{1,2,s1}/{measure.json,progress.log}`; each log lines 4,162,214,287 delimit capture/leaf-end/proxy-start/proxy-end; `tools/{phasecpu.py,run.sh,queue1.sh}` describe stamping/sampling. |
| Q | `E/perf-gate-20261023/tables.md`, each named PR/input table; raw paired `runs/*/measure.json` and `report.json` back it. CPU ratios are medians of **paired ratios**, not ratios of medians. |
| R | `E/capture-compact-20261023/README.md`, `E/lod-slim-20261023/README.md`; [#254](https://github.com/BenDyson-Arch/rusty-tiles/pull/254). |
| U | `E/etc1s-proto-20261024/results/*.json` records archive/view bytes, not bake CPU; `E/load-profile-20261023/m2048-cold.json` is browser loading, not conversion. Neither is used as bake timing. |

The audit also inspected `h10-perf-20261013` (legacy fixture lane),
`mlarge-p0-20261012` (early different meshes / profiles), `h27-bake-20261014`
(core acceptance, not a completed M bake), `h27-integration-20261014`,
`mesh-size-20261014` / `scale-findings-20261014` (Pohnpei geometry/size work),
and `scratch-records-20261021` (including copied H16 full-resolution records).
Those do not supply an additional matched legacy M baseline. Copied records,
validator times, browser loads, killed 100M-instance runs and sampler-stalled
diagnostics are not counted as successful comparable M conversions.

Reproduce the arithmetic without running a workload:

```sh
chrt -i 0 nice -n 19 python3 bench/architecture_audit/bake_time_history/summarize.py
```

The script only reads existing small JSON/log files and prints normalized run
totals, full-model phase reports, CPU-stamped crop segments and legacy fixture
records. It intentionally cannot run bakes, extract archives, build or benchmark.
