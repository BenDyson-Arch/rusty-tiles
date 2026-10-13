# H8 throughput and real-data calibration

This slice makes the H8 leaf producer finish the user's real photogrammetry
mesh. The mesh has 9,999,999 triangles, 89 meshes, and 89 JPEG base-colour atlases.
Every profile now completes on the 2048-texel copy. The original
8192-texel textures are admitted. Bulk evidence is kept in
`~/.cache/rusty-tiles-evidence/h8-perf-20261013/`: binaries, `run.json`,
stage timing, calibration JSONL and page dumps.

## Where the time went

`RUSTY_TILES_STAGE_TIMING=<file>` writes summed per-thread stage times, call
counts and texel counters. These are diagnostics only and never touch output.
The first profile used the old code on a real 39,456-triangle crop with 27
atlases. It ran `balanced`, one job, with the fidelity abort replaced by
logging (preview only):

| stage | seconds | share |
|---|---:|---:|
| Basis encode | 44.0 | 69% |
| sparse mips | 6.5 | 10% |
| codec verification | 5.3 | 8% |
| source decode (79 decodes of 27 images) | 2.7 | 4% |
| decode-for-check + fidelity check | 2.3 | 4% |

The encoder was not slow in itself. It was given far too many texels.
Metashape atlases scatter each mesh's charts over the whole texture. A leaf
crop was the bounding box of its triangles' UVs, so it covered almost the
whole atlas. The crop encoded 60.5 M interior texels (92 M page texels) for
geometry that touches about 0.4 M texels. Across leaves this duplicated each
source texel 25 to 60 times. The texture budget was spent re-encoding the same
atlases in every leaf.

## Changes

- **Island crops** (`crops::plan_islands`). Triangle footprints are clustered on an
  8-texel grid. Each cluster becomes one crop with the same one-texel LINEAR
  halo and gutter as before. Lossy islands are aligned to the 4x4 codec block.
  The result is never larger than the old whole-hull crop, which remains the fallback.
  All islands of one page bucket are shelf-packed directly into a
  power-of-two page at the smallest page area. On the full mesh the encoded
  interior texels are 534 M against 373 M source texels, a 1.43x duplication.
  Pages are 1.07 G texels, so power-of-two and shelf padding cost 2.0x.
- **Parallel delivery.** Leaves are read in frozen order and encoded by a
  `--jobs` rayon pool. Pages within a leaf run in parallel too. Workers write
  their own members (named by leaf order). The manifest row and the first
  reported failure are taken strictly in leaf order, so output bytes do not
  depend on jobs. Two runs, jobs 1 and jobs 3, on a scattered-atlas fixture
  (balanced) are byte-identical in CI. The real crop gave the same result
  with jobs 1 and 4.
- **Memory-aware auto jobs.** The hidden CLI now defaults to `--jobs auto`
  (it was fixed at 2). Each page leases its actual codec, assembly and decode
  workspace from one byte gate before it allocates. The gate is sized from
  available memory minus the decoded-source cache and the in-flight leaf
  window. Auto jobs is `min(cores, memory / per-worker leaf ownership)`.
  The old serial stage reserved a whole 4096² page per job.
- **Basis encoder.** There is no global lock: the shim's only shared state is
  `call_once` init plus thread-local diagnostics. Each call stays
  single-threaded (`cFlagThreaded` unset), and parallelism comes from
  concurrent calls. `concurrent_encodes_match_sequential_bytes` already pins
  byte identity.
- **Cheaper checks, none weaker.** The codec's mandatory verification already
  decodes every level with the vendored transcoder, and delivery now reuses
  those bytes. The base-level block-mean SSE is computed once instead of
  twice. Decoded sources are cached once per run: one decode per image
  against 22,186 uses. Admission decodes at capture run in parallel, and
  sparse mips read sRGB from a table built from the same expression.
- **Lossless.** Island crops remove the 96 MiB member refusal: the largest
  2048 lossless leaf is 7.0 MB. PNG islands sampled with viewer-generated
  mips (`minFilter` 9984 to 9987) carry 8 texels of real source context,
  aligned to 8. The first three box-filtered levels of an island therefore
  never read a neighbouring island. Non-mip PNG samplers keep the 2-texel
  context.
- **Admission.** `DecodeAdmission::LARGE_MESH` admits edges up to 16384,
  the GPU and KTX2 encoder maximum. The decoder may allocate 8 bytes per
  texel. Whether a decoded owner fits is decided by the stage plan from
  available memory, not by the decoder. The finite-source consumer keeps its
  4096 / 4 Mi limits.

## Gates

A per-texel maximum cap was decided by a single outlier. On the real mesh,
UASTC level 0 has a maximum error up to 156 at PSNR above 27 dB, while the old
cap was 24 to 32. Each colour channel of each level now gets three statistics,
all support-weighted:

- **PSNR** bounds mean quality.
- **99.9th-percentile absolute error** bounds the tail without depending on
  one texel.
- **Worst 4x4-block RMSE** catches one corrupted codec block, which a
  percentile over a large page cannot see.

| colour floors | PSNR | p99.9 | block RMSE |
|---|---:|---:|---:|
| fast, balanced (UASTC pack level 0) | 24 dB | 80 | 80 |
| quality (pack level 1) | 27 dB | 64 | 48 |
| levels under 1024 supported texels | floor - 4 dB | none | as above |

The floors were set against every level of the full real mesh, about 6,000 to
21,000 colour levels per profile, keeping roughly 3 dB and 13 levels of
margin:

| real mesh, levels of >=1024 texels | worst PSNR | worst p99.9 | worst block RMSE |
|---|---:|---:|---:|
| balanced | 27.2 dB | 67 | 63.4 |
| fast | 29.0 dB | 61 | 63.4 |
| quality | 30.3 dB | 45 | 26.6 |

Small levels need their own rule. Below 32x32 supported texels a 99.9th
percentile is just the maximum of a few hundred samples. Packed islands also
share codec blocks at those levels. The worst small-level colour PSNR was
24.8 dB.

Alpha, data and normal channels are unchanged: they keep their previous
PSNR floors and maximum caps. The real mesh has none of them, so the robust
statistics stay limited to calibrated colour.

The old balanced colour PSNR floor was 22 dB, so the new 24 dB floor is
stricter. Quality's old 34 dB floor was unattainable on real blue channels:
level 3 encoding (38x the cost) reached only 34.5 dB on the worst crop page.

Sensitivity controls still fail in CI, for every profile:
- on a procedural photo-like page: a corrupted 4x4 block (the block guard
  alone), swapped red and blue channels, and damage spread over 10% of texels
  (the percentile);
- on a one-texel checker: a mip level shifted by one;
- on a smooth normal page: a flipped green channel;
- the 1-texel radial normal error, the colour-transfer data mutant, and the
  small-level colour, normal and garbage cases.

A single full-range outlier texel, which the old cap refused, is accepted.
The ignored `calibration_tests` reads page dumps and runs the same mutants on
the four real crop pages; there the gate refuses each of these at mip 0:

- a corrupted base block;
- corrupted payload bytes (the decode fails);
- swapped red and blue;
- mip levels shifted by one;
- a non-renormalized normal chain;
- a flipped normal green channel.

A wrong-colour-space chain is not refused on these real pages. Gamma and
linear averaging of natural content differ by less than codec error, and the
gate compares the decode with the authored chain. Mip-construction correctness
is pinned separately by the exact oracle tests, which still fail their
wrong-gamma mutants.

## Encoder settings

These figures come from a single thread on real pages, measured on the
contended machine:

| pack level (effort) | s/Mpx | bytes/texel | worst blue PSNR (crop) |
|---|---:|---:|---:|
| 0 (fast 0, balanced 1) | 0.20 | 0.381 | 33.0 |
| 1 (quality 2) | 0.82 | 0.392 | 33.8 |
| 2 (effort 4) | 3.4 | 0.398 | 33.9 |
| 3 (effort 7) | 7.7 | 0.398 | 34.5 |

The settings are kept. Level 0 meets the floors at the lowest cost. Quality's
level 1 buys +0.3 to 0.8 dB at 4x the cost. Higher levels cost 17 to 38x for
under 1 dB. The shim refuses UASTC RDO by design. An RDO setting for `fast`
would shrink files at a quality cost and is a separate policy decision.
Fast and balanced therefore differ only in leaf and page size.

## Real-mesh results (2048 textures, auto jobs, 20 cores)

| profile | wall | CPU s | peak RSS | scratch peak | leaves | GLB bytes |
|---|---:|---:|---:|---:|---:|---:|
| balanced | 88.8 s | 639 | 7.4 GB | 5.1 GB | 971 | 1,705,533,244 |
| fast | 132.8 s* | 592 | 6.7 GB | 5.7 GB | 486 | 1,698,444,316 |
| quality | 482.9 s* | 1,862 | 4.7 GB | 5.0 GB | 1,959 | 1,797,907,992 |
| lossless | 75.0 s | 90 | 2.9 GB | 6.1 GB | 1,959 | 2,158,325,948 |

\* Fast and quality ran before members were written by workers, while the user
was copying data between drives (30% iowait). Their wall times are
upper bounds.

Every leaf was published, with no fidelity refusals. The 15-minute target
holds for all profiles. Before this slice, `balanced` had run for 1 h 50 m
without publishing.

The same crop went from 54.2 s to 2.6 s on one job (21x), and to 1.4 s with auto
jobs. In the balanced run, capture is 48 s of the 89 s wall. Of the summed
delivery thread time, Basis encode is 72%, and codec verification plus the
fidelity check are 9.6%.

Metashape shared the machine throughout, using 1 to 8 cores; the machine was
39 to 90% busy during runs. `run.json` records the load for each run.

A 39,456-triangle crop with the original 8192-texel atlases ran at
`balanced` in 13.1 s and `lossless` in 12.1 s. Of the CPU time, 9.8 s is
decoding the 27 JPEGs. The full original-size run is pending, because the
source is being moved to another drive.

## Checks are not a sampling candidate

The independent checks are under 10% of CPU, so no sampling scheme is
adopted. If they were to matter, block-mean and detail statistics of the
codec's corruption floors could be evaluated on every second 4x4-block row.
Any change like that would need its sensitivity controls re-run and
coordinator approval.

## Known limits

- Binding receipts record every island as
  `[sourceX, sourceY, width, height, pageX, pageY]` (interior origin), plus
  `islandGutter`. Only dedicated MIP pages keep `cropOrigin` and `cropSize`,
  so `leaf_oracle.py` needs island support before it can re-check non-MIP
  pages.
- Capture charges each admission decode from the image's header dimensions
  (12 bytes per texel plus the encoded bytes). The budget is the decoder
  allowance plus the memory left by capture's resident bound. An image that
  cannot fit is a typed resource refusal.
- MIP-sampled lossy bindings still use dedicated, aligned whole-hull pages.
  Their source-sampler checks keep the maximum caps. This mesh does not
  exercise them.
- Power-of-two pages cost 2.0x padding over island interiors.
- With original-size atlases the decoded-source cache (half of usable memory)
  may not hold all 89 sources. Least-recently-used reloads then trade decode
  time for memory.
