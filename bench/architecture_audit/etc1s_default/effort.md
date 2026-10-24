# ETC1S processing-cost revision

PR #277 review requests a processing-cost sweep before retaining the default.
The owner requires client and processing speed to remain priorities; the
review gives the earlier 91.2 CPU-second UASTC quality result on tex3
as a balanced encode target. The older fast/balanced UASTC pack 0 result is also measured on
the same caller pages, rather than silently treating quality as that baseline.
Meeting 91.2 s is a bounded profile budget, not proof that ETC1S dominates
UASTC pack 0 in processing speed. Any remaining cost against that faster
reference must be reported alongside the byte saving. Final bake CPU includes
failed attempts and stronger re-encodes; raw sweep CPU does not.

Retain: one policy owner in `leaves::delivery::codec_settings`, q50/q90 colour
semantics, authored mips, mandatory codec verification, strict authored sampler
correspondence, alpha/MASK/data contracts, deterministic escalation and output.
Rework: encoder effort from measured CPU/bytes/error tradeoffs; tiny channel
PSNR from 12 to 13 dB plus a 16 dB joint RGB floor; directly test the decoded sampler
selector; pin a complete default-lossy archive. Do not enable an unbounded
native worker pool or bypass a gate to reach the CPU budget. No HLOD edits.

## Independent sweep

`capture_pages.py` adds removable diagnostic-only hooks around the first
profile encode. They dump exact caller mips, expected RGBA and little-endian
u32 support weights. Proxy mip semantics cannot be reconstructed from the
base image, so the complete chains are retained. Encoding and every existing
gate remain active. Original effort 3 archive identities must reproduce.

The ignored Linux release test `basisu_ktx2::tests::effort_sweep` independently
encodes every captured page at each setting, records current-thread CPU around
only encoding, runs mandatory codec verification, independently decodes every
mip, and measures weighted RGB PSNR, per-channel tails and block RMSE. Codec
refusals are recorded without dropping those pages from decoded measurements.
Two page workers are explicit; CPU sums exclude scheduling, IO, verification
and metric computation. `run_sweep.py` compares all seven distinct native
ETC1S levels 0–6 (wrapper efforts 0/1/3/5/6/8/10), at q50 on the fast corpus and
q90 on the balanced corpus. UASTC+Zstd wrapper efforts 0 and 2 are same-page
references. This is an encoder comparison on fixed inputs; final bakes check
the selected settings and actual escalation/processing cost.

The public effort range is 0–10. ETC1S maps those settings only through
`round(6*effort/10)`: 0→0, 1/2→1, 3/4→2, 5→3, 6/7→4, 8/9→5, 10→6.
Aliased settings do not need duplicate measurements; every distinct native
level is included. UASTC wrapper effort 2 maps to pack 1 (quality's existing
setting), while efforts 0/1 map to pack 0 (develop's fast/balanced setting).

From the new mandatory resource rule onward, every build, test, bake and sweep
runs under `systemd-run --user --scope -q -p MemoryMax=10G -p MemorySwapMax=0`,
then `chrt -i 0 nice -n 19`. Cap exhaustion is a defect, never a reason to raise
the cap. Measurement slot 2 serializes heavy runs. Raw evidence is separate
from the original campaign at
`~/.cache/rusty-tiles-evidence/etc1s-effort-20261024/`.

The initial fast capture and release measurement-test binary were built before
the new cap instruction. Their later sweep executions are capped; the
`memory_max` field in `sweep-source.json` describes execution, not that earlier
build. All subsequent builds/tests/bakes/sweeps use the mandatory scope.

Stock H17 capped balanced captures stop at 205/217 pages with a controlled
`child packet decoded ownership lease` refusal, including after flushing
scratch cache and bounding glibc arenas. A shipping fast bake also refuses
that lease. These are incomplete runs, not OOMs or complete-bake evidence.
The pre-codec child packets retain large image/coverage/witness owners; the
automatic stage budget takes only half available headroom, then reserves part
of that for child loading. Lower codec effort does not remove those owners.

`capture_budget.py` therefore gives the **measurement binary only** an explicit
8 GiB stage allowance, retaining all ownership charges and every codec,
appearance, alpha/MASK and escalation check. It asserts that its actual cgroup
hard cap is exactly 10 GiB and swap is 0. The hard cap is never raised. The
balanced capture completes 217 pages at 6,696,419,328 bytes recorded peak cgroup usage,
with zero max/OOM events. Stock automatic-budget acceptance under 10 GiB is
**not established**; the full tex3 bake tables are labelled diagnostic.

Capture hooks fsync/advice their completed dump files; an optional campaign
guard does the same for stable files beneath only that case's owned scratch.
This controls reclaimable cache at an IO/wall-time cost, not file contents or
memory charges. Allocator settings are retained in every result. These runs
are contention-bound correctness/cost observations, not clean-performance
acceptance. The raw two-worker sweep is the matched encoder comparison;
whole-bake CPU uses a different workload/scheduling/allocator context and is
reported separately rather than substituted for the sweep.

Both diagnostic hooks are removed after building its immutable binary.
Removal restores the shipping source byte-for-byte to `shipping-lib-source.json`;
`shipping-source.json` binds the ordinary release binary. Diagnostic source
hashes/patch and binary, refused runs, cgroup samples and raw caller pages are
retained independently. `partial-input-match.json` verifies all 6,886 files
(1,070,215,797 bytes) of the first 205 balanced inputs are identical at effort 3
and effort 0. Complete-input codec replays must also reproduce the original
effort 3 ETC1S and develop pack 0 UASTC byte totals for both profiles.

## Native settings audit

The audited native source is the pinned `basisu_c_sys 0.9.1` registry crate's
`vendored/basis_universal/encoder/` directory. `sweep-source.json` uses
`vendor/basis_universal/` as a portable label for those exact hashed files.
`native-source.json` also pins retained copies of the eight audited encoder,
frontend, backend and compression source/header files. The seal rechecks
every sweep source hash, including the actual registry paths and Cargo inputs.

The shim calls `basis_compress2` with sRGB/KTX2 flags and explicit caller mips.
ETC1S endpoint/selector RDO defaults are enabled (`m_no_*_rdo=false`), with
thresholds 1.5/1.25; high-quality codebook policy scales those thresholds down.
At q90 they become 0.375/0.3125: backend remapping clamps the error multiplier
to at least 1, and the endpoint branch requiring a multiplier above 1 is skipped.
The flags therefore mean "enabled", not permission for extra q90 error.
Quality selects endpoint and selector clustering budgets automatically; no
custom cluster counts or external/global codebooks are supplied. Native
levels 0–2 use hierarchical endpoint/selector codebooks. Level 0 disables
endpoint refinement and expensive selector-codebook ordering; higher levels
change hierarchy/refinement iterations. These are real algorithm changes,
not merely a thread-count adjustment.

Each native encode has a one-thread pool (`cFlagThreaded` is unset), while
tiler jobs parallelize independent pages. Enabling that flag would request all
hardware threads per page, oversubscribe the operation pool, invalidate the
thread-CPU measurement and require new memory/fault-containment proofs. It
would target wall time rather than reduce CPU work; it is not enabled here.

## Corruption controls and proof limits

Black/blank RGB, inverted red, and a coarser checker mip upsampled to the right
dimensions must fail at every size band. Light flat grey 200 wrongly decoded
as linear bytes 147 must also fail; its error is about 13.65 dB, which the old
12 dB tiny floor admitted. A 14 dB channel floor would also reject the measured
13.30 dB semantic photo/odd-size tails. Their minimum combined RGB PSNR is
16.85 dB, while the grey control has 13.65 dB in every channel. The revised
13 dB/channel plus 16 dB combined RGB guard separates these cases. Combined
PSNR uses total RGB SSE and weight, not an average of channel dB values.
The captured q50 tex3 tiny minima are 14.44 dB/channel and 16.17 dB combined;
thus this strengthening adds no tiny refusals on that real corpus. The
semantic alpha-photo retains only its independent alpha refusals; photo,
odd-size, flat/low-noise and checker RGB remain admitted.
A separate final-source effort-0 smooth-photo run retains all 240 channel
observations at 32²/64²/128²/256². The 32² and 64² RGB controls pass; 64²
middle-band minima are 17.047/17.156 dB, and its tiny combined minimum is
22.282 dB. The 128² and 256² stress pages retain large-level refusals; the
256² q90 32² mip reaches 17.523 dB and p99.9 95. No tiny guard causes those
refusals. Smooth-photo measurements are retained as `smooth-final.jsonl`.
The sampler tests distinguish positive-support counts from level area, test
fractional blends/nearest rounding/non-mip selection and clamping, and retain
the separate strict authored check and decoded maximum-error caps.

A smooth wrong-mip control deliberately demonstrates a remaining blind spot:
upsampling the next coarser mip of a slow ramp differs by only about one byte,
so its PSNR exceeds 40 dB and the 20 dB gate passes. The gross checker wrong-mip
control fails, but a PSNR gate is not a mip-provenance theorem. A proposed
additional check compares reference/decoded spatial derivatives or band-pass
correlation at each meaningful mip, measuring both attenuation and phase/
aliasing error. Its thresholds must first be calibrated on the accepted ETC1S
corpus; flat regions need a defined zero-detail case. That additional spatial
contract is not claimed by this revision.

## Selection and complete results

Both colour profiles select **public/native effort 0**: fast q50 and balanced
q90. On the matched caller corpus, their raw encode CPU is 47.089 / 56.538 s,
versus 154.790 / 179.611 s at the initial effort 3. Balanced effort 1 already
costs 98.291 s for only 0.17 dB more base RGB PSNR; fast effort 1 costs 63%
more CPU for 1.1% fewer bytes and 0.13 dB more base RGB PSNR. Effort 0 is the
smallest supported native level. It retains 99.68% / 94.64% of the original
raw texture-byte saving against the same profile on develop UASTC.

This meets the earlier 91.214 s quality-profile encode budget, while the
matched UASTC pack-1 reference takes 96.980 / 84.417 s on these two corpora.
It **does not preserve processing speed** against
develop's faster pack-0 setting: that takes only 16.148 / 18.183 s, so the
selected ETC1S remains 2.92× / 3.11× its CPU cost. No supported effort below
zero exists; more native threads would change latency rather than this CPU
work. The implementation follows the requested ETC1S default and balanced CPU
target, with this remaining owner-policy tradeoff exposed for review.
The approximately 23 CPU-second new HLOD geometry result is external context;
this branch's H17 campaign does not remeasure that pipeline.

<!-- sweep-table-start -->
| Codec / public effort (native) | Fast corpus: CPU-s / MB / base dB / all-mip dB | Balanced corpus: CPU-s / MB / base dB / all-mip dB |
| --- | --- | --- |
| ETC1S / 0 (0) | 47.089 / 10.458381 / 31.521 / 29.172 | 56.538 / 16.205284 / 34.923 / 31.472 |
| ETC1S / 1 (1) | 76.679 / 10.343923 / 31.653 / 29.178 | 98.291 / 14.748015 / 35.090 / 31.872 |
| ETC1S / 3 (2) | 154.790 / 10.373367 / 31.668 / 29.045 | 179.611 / 14.909217 / 35.260 / 32.063 |
| ETC1S / 5 (3) | 327.710 / 10.436981 / 31.873 / 29.233 | 446.621 / 14.918610 / 35.250 / 32.079 |
| ETC1S / 6 (4) | 396.570 / 10.397710 / 31.755 / 29.283 | 463.494 / 14.923305 / 35.269 / 32.076 |
| ETC1S / 8 (5) | 1062.467 / 10.464142 / 31.979 / 29.512 | 1858.852 / 14.941301 / 35.256 / 32.081 |
| ETC1S / 10 (6) | 2810.481 / 10.477430 / 32.007 / 29.432 | 4157.458 / 14.958809 / 35.265 / 32.076 |
| UASTC / 0 (0) | 16.148 / 37.105816 / 44.749 / 39.605 | 18.183 / 39.083023 / 44.923 / 40.010 |
| UASTC / 2 (1) | 96.980 / 38.702021 / 45.327 / 40.237 | 84.417 / 40.840507 / 45.470 / 40.779 |
<!-- sweep-table-end -->

Each row includes all 217 pages, before escalation; MB is decimal. ETC1S uses
q50 on fast and q90 on balanced; UASTC has no corresponding quality
parameter. PSNR is support-weighted combined RGB, computed from aggregate SSE, with base and all
mips reported separately. Separate per-channel/local/alpha guards still apply.
No failed page is omitted. Public efforts 0/1/3/5/6/8/10 cover every distinct
native ETC1S level 0–6; the other public values are aliases. The effort-3 rows
reproduce the original texture totals exactly (10,373,367 / 14,909,217 bytes),
and UASTC pack-0 reproduces develop exactly (37,105,816 / 39,083,023 bytes).
These four independent byte checks bind captured mips to both earlier outputs.
All 18 configurations pass mandatory codec verification. Their 3,906 page
encodes produce 126,981 RGB level/channel observations; refreshed quantiles
are in `distributions.json`, alongside 456 semantic and 240 smooth-photo
RGBA channel observations. Exact channels remain distinguishable from finite
PSNR entries. The two highest native-6 runs cost 2,810.481/4,157.458 CPU-s
for only 32.007/35.265 dB base RGB PSNR, versus effort 0’s 31.521/34.923 dB.

## Whole archives and simulated views

The following tex3 outputs use final shipping codec/gates plus the explicitly
labelled diagnostic 8 GiB stage allowance, inside the unchanged 10 GiB hard cap.
They establish the selected output and escalation behaviour, **not stock H17
completion under that cap**. CPU includes failed encodes and escalation but
excludes mandatory verification, decoding, IO and geometry. These observations
have a different allocator/scheduling context from the matched raw sweep.
Do not substitute their lower CPU observations into the matched comparison.

| Profile | Archive bytes: develop → revision | Texture bytes: develop → revision | Revision encode CPU-s | Escalated pages |
| --- | --- | --- | --- | --- |
| fast | 82,287,053 → 56,546,087 | 37,105,816 → 11,364,524 | 42.712 | 2/217 (0.92%) |
| balanced | 84,344,677 → 61,466,844 | 39,083,023 → 16,205,284 | 38.332 | 0/217 |
| quality (historical unchanged-policy control) | 92,568,655 → identical | 40,838,041 → identical | 91.214 (historical) | unchanged |
| lossless (historical unchanged-policy control) | 94,500,728 → identical | 41,807,840 → identical | 0.289 (historical) | unchanged |

Fast's first attempts fail page 0149's block RMSE and page 0208's p99.9 cap;
one is a leaf, one a proxy. Both deliver UASTC, leaving 215 ETC1S images;
there are no level-3 or PNG fallbacks. Balanced delivers all 217 as ETC1S.
The stricter tiny guard causes no additional real-page escalation. Texture
savings after actual escalation are 69.37% / 58.54%, retaining 96.29% /
94.64% of the initial effort-3 byte saving. The actual KTX2 SGD records
contain nonzero endpoint/selector codebooks: fast ranges 58–2,140 / 138–2,731,
balanced 58–3,584 / 186–12,330. Thus native clustering is exercised, rather
than merely enabled in an unused configuration.

| Profile | Wide bytes: develop → revision | Close bytes: develop → revision |
| --- | --- | --- |
| fast | 6,902,996 → 4,179,684 | 17,678,668 → 11,204,712 |
| balanced | 7,820,116 → 4,806,720 | 19,142,420 → 12,652,564 |
| quality (historical unchanged-policy control) | 8,677,712 → identical | 21,401,660 → identical |
| lossless (historical unchanged-policy control) | 7,037,636 → identical | 18,791,716 → identical |

Wide means 1600×1000 at range 3; close means 2560×1600 at range 0.75.
Requests remain 15 / 42. Simulator, measurement and baseline helper hashes
match the original campaign and are retained as exact helper snapshots.
These are selected transferred bytes, not observed first-paint/GPU results.

Both fast and balanced diagnostic archives are byte-identical across jobs 1
and 2, including all failed attempts' resulting codec choices. Their SHA-256s:

- fast: `ab089e2df2596d0dcd363675eae7501b7ba16a3a6958ba0a22ad65d5e34a42d3`
- balanced: `2ea4bdd185001d7795338cf948971b465621232239eb508f56ef9c9f0dbdec08`

A fresh **stock shipping** balanced crop3 bake completes under the hard cap:
78,863,873 archive bytes, no textures/encode calls, wide/close 6,217,148 /
18,010,800 bytes and 15 / 42 requests. Its complete archive is identical to
develop (`c94cbbf690f04d499c409f6e734f973e30642e7940c01a2f047bbd4e15afeb16`).
The original campaign's remaining crop3 controls are historical unchanged-policy
results: fast 78,863,869 bytes; quality 85,351,531; lossless 102,061,768.
They are not new execution acceptance under the later cap instruction.

## Verification and retained identities

The final shipping library passes `cargo test --lib mesh_large` (301 passed,
six ignored) and `basisu_ktx2` (18 passed, three ignored). Clippy all-targets
with warnings denied, `h26_mesh_core` (seven), `h26_mesh_frontends` (eight),
`golden` (seven), and `output_digests` (two, one ignored) pass. The ignored
semantic calibration and raw release sweep run separately; they are not
silently counted as normal regression tests. All these follow-up checks use
the mandatory hard scope, with bounded workers.

The sole intentional committed digest addition is the new `mesh-balanced`
recipe, which omits `--profile`, checks actual ETC1S DFD model 163, and pins
the **entire** small archive as well as its entries:
`57065671d8a7878f9f3d638e98edf714d43834a2cdc48610aca8858ef639bf9e`.
Its nonconstant low-contrast fixture exercises the default lossy codec;
all pre-existing recipe digests and input bytes remain unchanged. Existing
lossless goldens therefore remain controls rather than being rewritten.

Final source and immutable ordinary release/library binaries are bound by
`shipping-source.json` and `shipping-lib-source.json`. The removable diagnostic
patch and its complete modified-file hashes bind `diagnostic-rusty-tiles`;
the frozen sweep binary has its own `sweep-source.json`.
`shipping-complete-source.patch/json` retain the complete final source/test diff
against the base commit, supplementing the build-time production snapshots.
`integration-source.json` pins the final test sources, logs, four integration
executables and their debug CLI child. Their executions
are distinct and are never treated as interchangeable production binaries.
The source-only nonauthor round reproduced selected CPU, semantic minima,
source hashes and binary identities with no defects; it did not accept the
then-incomplete final evidence.

The external execution manifest and final independent review are pinned by
`effort-evidence.json`. Bulk raw inputs, distributions, all distinct-level
reports, commands/logs, source patches, binary witnesses, archives and helper
snapshots live only in the separate effort evidence cache. The historical
campaign and its manifest remain immutable. Final acceptance is bounded to
this source/output evidence: stock tex3 automatic-budget completion under
10 GiB remains unestablished; tex3 does not exercise the finite sampler gate;
smooth wrong-mip provenance remains open. No CI, GPU/browser, integrated
D7/S8/HLOD, clean-performance, foundation or release acceptance is claimed.

Separate final nonauthor review accepted this bounded implementation and
complete retained execution evidence with no defects or identity gaps. It
independently verified every one of the 21,685 manifest entries (4,322,736,084
bytes), all six archives, all 18 configurations and 126,981 RGB observations,
semantic/smooth controls, source/artifact bindings, passing checks and actual
view selections. Removing only the diagnostic hooks restores shipping source
byte-for-byte. The post-seal review response is pinned separately in the
receipt; the execution manifest remains immutable. The explicit CPU tradeoff,
stock-admission limitation and remaining sampler/spatial/GPU/integration/CI
limits above remain open; this grants no owner-policy or merge authorization.

Post-seal cleanup completed under the unchanged 10 GiB / zero-swap hard scope: `cargo clean` removed the dedicated target and the owned scratch directory was deleted. Source hashes remained unchanged. The tracked receipt pins the separate cleanup result and log.
