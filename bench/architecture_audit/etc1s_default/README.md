# ETC1S colour default — 2026-10-24

Owner decision brings D7 forward for both leaves and H17 coarse pages.
Scope: fast q50, balanced q90, ETC1S effort 3; quality UASTC and lossless PNG.
Codec/profile policy is owned by `leaves::delivery::codec_settings`.
Encoder wiring and quality settings were harvested from `proto/etc1s-colour`
at `b1c23897`; its unrelated proxy-bake changes were excluded.

Before implementation: retain authored colour-correct mips, explicit sRGB DFD,
independent decoding, alpha/MASK and normal/data contracts, and the deterministic
UASTC 2 / UASTC 3 / sampler-compatible PNG escalation ladder. Rework RGB floors
only where ETC1S measurements establish a need. Remove prototype soft acceptance:
no environment codec override or gate bypass is shipped. Structural codec faults
remain hard errors; decoded gate misses retain typed escalation semantics.

Acceptance: measure every decoded mip/channel on existing codec fixture families
and actual tex3 pages at q50/q90; include wrong transfer, channel swap, alpha,
MASK, flat-block damage and contrast-loss controls. Compare tex3/crop3 profile
archives, textures, encode cost and finite simulated first views against develop
60a76db11da00551c0a9f18839095ff678675be1. These are bounded measurements, not
a 2048 campaign, GPU upload, browser or release acceptance.

## Floor calibration

The existing `basisu_ktx2::tests::measure_caller_mip_floors` now includes the
exact q50/q90, effort-3 colour settings alongside its historical configurations.
It independently transcodes photo, alpha-photo, odd-sized photo, checker,
full/low noise and chroma-noise chains and emits per-channel PSNR, p99.9,
maximum error and worst 4×4-block RMSE. The additional ignored
`delivery::tests::measure_etc1s_smooth_photo_floors` measures the existing
smooth-photo family at 32², 64², 128² and 256² with authored colour-correct mips.
Together these contribute 704 level/channel observations before real pages.
`RT_COLOUR_FIXTURE_DUMP` exports the original base witnesses; the independent
`prepare_semantic_fixtures.py` edge-extends them into valid POT pages with zero
padding support, then the existing leaf calibration test authors colour-correct
mips. Its measurements are in `semantic-fixtures.jsonl`; only these semantic
chains and the smooth-photo/tex3 controls define the class floor. Historical
byte-domain chains remain codec stress evidence. Exact channels are recorded
separately from finite PSNR observations.

Candidate tex3 runs use a diagnostic-only build made with
`capture_calibration.py`: it records all channels of every attempted mip and
source-sampler PSNR/max errors without disabling any gate or modifying any
encoded texel. Its logging hooks are removed from production source before shipping;
its source patch, file hashes and immutable binary hash are retained in the
external evidence directory. `summarize_calibration.py` reports min/p50/p90/p99/
max over level/channel observations (the individual production statistics are
support-weighted). It separates the first profile attempt from stronger steps.

RGB acceptance is 20/16/12 dB at ≥1024 / 64–1023 / <64 supported texels, for
both ETC1S profiles. Large-level p99.9 caps are 112/80 and worst-block RMSE caps
96/80 for fast/balanced. The codec's relative error/detail corruption checks
remain mandatory on every level. Alpha, normal, metallic-roughness, occlusion,
MASK coverage and the sampler-compatible final PNG restriction are retained.
Decoded source colour sampling uses the same size bands and max errors 160/144;
pre-encode source correspondence retains its original stricter limits.

Derivation: the valid photo/alpha/odd-size pages reach 20.38/20.72 dB
(q50/q90) on large levels, 18.64/18.61 dB on middle levels and 13.30 dB on tiny
levels. The smooth 64² photo reaches 17.22/17.16 dB in its 8² mip. Shared
palettes and joint-channel reconstruction explain those coarse/tiny channel
tails; hence additional size bands rather than relaxed alpha or disabled
verification. The 256² smooth-photo stress case still misses (q90 17.46 dB,
p99.9 94 on a 1024-texel mip) and must escalate. Noise/chroma-noise remain poor
ETC1S fits, not a reason to lower the floor. The local caps cover accepted
tex3/fixture tails while rejecting damaged blocks.

Sensitivity is exercised through real decoding plus wrong colour transfer,
red/blue swaps, broken alpha, flat-block damage and erased saturated checker
detail, including each small-level band. Existing independent MASK coverage,
normal/data, every-step corruption, cancellation and exact PNG fallback controls
remain active. Textured atlas byte determinism now runs both ETC1S profiles
with one and three workers. The five-slot proxy control independently reads
the emitted DFD model/transfer and every mip's metric/limit receipt.

## Evidence and scope

Raw commands, binary/source identities, decoded distributions, timings and first
view selections are under `~/.cache/rusty-tiles-evidence/etc1s-default-20261024/`.
Only tex3/crop3 are baked, at idle priority through measurement slot 2, with two
workers. Slot 2 uses CPUs 0–3,16–19 and is contention-sensitive; elapsed time is
not a clean performance claim. Candidate encode CPU uses Linux thread CPU time
summed across encoder calls, including retries, excluding verification. Develop
has the original summed worker-wall metric; no baseline CPU value is invented.
Crop3 has no source images and controls geometry-only behaviour.

First views use `/tmp/simview/sim.py --json` at 1600×1000/range 3 and
2560×1600/range 0.75. These are simulated transferred bytes and request counts,
not GPU transcode/upload or actual first-paint observations. No browser, 2048
bake or HLOD implementation change is included. The later integrated D7/S8
acceptance remains open.

## Bounded tex3 results

All byte counts below include the same geometry, provenance and archive layout.
Develop is `60a76db11da00551c0a9f18839095ff678675be1`; candidate uses the
profile policies above. Texture bytes sum embedded image buffer views across
both leaf and proxy GLBs. First-view totals include the selected geometry and
metadata as well as textures; requests remain 15 / 42 in every profile.

| Profile | Archive bytes, develop → candidate | Texture bytes, develop → candidate | Candidate encode CPU seconds |
| --- | --- | --- | --- |
| fast | 82,287,053 → 55,554,440 | 37,105,816 → 10,373,367 | 175.575 |
| balanced | 84,344,677 → 60,170,748 | 39,083,023 → 14,909,217 | 195.595 |
| quality | 92,568,655 → identical | 40,838,041 → identical | 91.214 |
| lossless | 94,500,728 → identical | 41,807,840 → identical | 0.289 |

The size reduction costs encoder work. Summed encode worker-wall seconds are
16.599 → 183.986 (fast), 20.992 → 212.070 (balanced), 106.336 → 101.194
(quality) and 0.377 → 0.308 (lossless). These wall sums include scheduling
delays and are not interchangeable with candidate thread CPU. Whole-process
user+system CPU seconds are 258 → 414, 266 → 437, 358 → 352 and 288 → 303,
respectively; they include geometry, verification and archive work. ETC1S at
effort 3 therefore trades substantial encode work for fewer transferred bytes.

| Profile | First-view bytes, 1600×1000/range 3 | First-view bytes, 2560×1600/range 0.75 |
| --- | --- | --- |
| fast | 6,902,996 → 3,963,232 | 17,678,668 → 10,985,496 |
| balanced | 7,820,116 → 4,742,824 | 19,142,420 → 12,493,884 |
| quality | 8,677,712 → identical | 21,401,660 → identical |
| lossless | 7,037,636 → identical | 18,791,716 → identical |

Fast reduces texture bytes by 72.04% and total archive bytes by 32.49%; balanced
reduces them by 61.85% and 28.66%. Every fast/balanced tex3 image is ETC1S:
**0/217 pages escalate**, covering 154 leaves and 63 proxies. The conversion
report separately records zero leaf stronger-step/PNG attempts; independently
reading every image's actual codec establishes the proxy result too. Quality
and lossless archives have exactly the same SHA-256 as develop.

The 27,824,128 leaf base texels use 21,343,303 bytes (0.7671 B/texel) on
develop fast/balanced, 5,118,211 (0.1839) on fast ETC1S and 7,535,439 (0.2708)
on balanced ETC1S. Those ETC1S leaf byte counts reproduce the accepted prototype.
Proxy totals differ from that prototype's unrelated bake implementation;
only its encoder wiring and quality settings were harvested.

Actual tex3 profile attempts have large-level minimum channel PSNR 22.07 dB
(fast) / 23.31 dB (balanced), worst p99.9 98 / 77, and worst block RMSE
79.31 / 70.20. Middle-band minima are 21.52 / 23.34 dB and tiny-band minima
16.94 / 21.73 dB. These are observations, not additional acceptance floors.
Maximum individual RGB errors are 146 / 121, motivating decoded sampler caps
160 / 144 with margin; the sampler still checks its own sampled errors and
authored correspondence independently. These tex3 unique-image deliveries do
not exercise the finite source-sampler mip check, so there are no real-page
sampler observations in `distribution.json`. Its retained independent sampler
tests cover that path; the tex3 archive replay cannot establish broader sampler
acceptance on its own.

## Verification

Final shipping source passes `cargo clippy --all-targets -- -D warnings`,
`cargo test --lib mesh_large` (297 passed, six ignored measurement tests),
`cargo test --lib basisu_ktx2` (18 passed, two ignored), and integration targets
`h26_mesh_core` (seven), `h26_mesh_frontends` (eight), `golden` (seven) and
`output_digests` (two, one ignored export helper). Ignored calibration probes
were executed separately as described above. Formatting, changelog validation
and the indexed tree-budget guard pass. No golden or committed output digest
is changed: the digest mesh recipe explicitly uses lossless.

Source-only independent review found no defects and checked the frozen
production file hashes. Final evidence acceptance additionally checks the
completed comparisons and final-source artifact binding.

## Final source and determinism

The diagnostic binary is `05df72fc7a47020b081ef3c13852068240f5bdfeb8be250dec7e7ac75d987980`.
It predates the corrected actual-mip sampler band selector and restoration of
the historical lossless helper limits; it must not stand in for final-source
execution. Its logging hooks were removed, the reviewed source was checked
and rebuilt in release, and `final-source.json` binds all ten relevant source/
dependency hashes to final binary
`6f422d172d2caf7f9f3f05f0bdca5210e677e38dfee786e70e45445424a4ba6b`.

Both changed profiles were then baked on tex3 with that final binary and
`--jobs 1`, under the same slot/priority protocol. The entire archives match
the diagnostic two-worker campaign byte for byte:

- fast: `703ab71131e75a2b21df6d7e938515426e1757462c40376917bdaec0b0903a60`
- balanced: `6937d25f645c582016cacff42fbed10cf356c65862900015a6deacc344d209d4`

Thus diagnostic logging and the final source corrections do not alter these
bounded outputs. This supplements the one/three-worker textured atlas controls;
it does not extend tex3's sampler-path coverage. Develop balanced archives also
match its baseline helper's four-worker results for both crop families.

## Crop3 geometry control

All four candidate archives are byte-identical to their develop counterparts.
There are no images, texture bytes, encoder CPU or escalation attempts in this
input. Requests remain 15 / 42. Profile differences below are existing geometry
policy differences, not codec effects.

| Profile | Archive bytes (unchanged) | Wide first-view bytes (unchanged) | Near first-view bytes (unchanged) |
| --- | --- | --- | --- |
| fast | 78,863,869 | 6,217,148 | 18,010,800 |
| balanced | 78,863,873 | 6,217,148 | 18,010,800 |
| quality | 85,351,531 | 6,763,388 | 19,556,032 |
| lossless | 102,061,768 | 6,763,388 | 20,084,780 |

## Retained identities

[evidence.json](evidence.json) pins the SHA-256 of the external
`evidence-manifest.json`, plus every case's archive identity/bytes/worker count.
That manifest binds the actual retained archives, raw calibration and view
records, review responses, helper/tool copies, fixture inputs, source hashes,
binary hashes and toolchain record. Bulk evidence is new and has never been
tracked in Git; its source baseline is the commit above, rather than a prior
commit containing the raw data. `seal_evidence.py` in the external cache
independently re-hashes all 18 archives and checks the declared comparisons.
All owned scratch output is removed after retaining these records in the
evidence cache. The repository contains only the small acceptance record,
hash receipt and witness generators.

Separate final nonauthor review accepted this bounded implementation and
evidence with no concrete defects or unresolved identity gaps. It independently
verified the sealed raw files, all 18 archive hashes, final source/binary
identities, 29,504 distribution observations and the retained final check logs.
Its response is retained as `review-final.json` and included in the final seal.
This establishes no CI, foundation, release or broader GPU/viewer acceptance;
the scope limits above remain active.
