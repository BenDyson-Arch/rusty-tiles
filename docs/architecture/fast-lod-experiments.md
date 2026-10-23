# Fast LOD bake experiments — keep H17 (2026-10-24)

| Experiment | Total CPU-s / CPU-s per Mtri | Wall s | Peak RSS GiB | Sampled scratch GB | Archive MB |
|---|---:|---:|---:|---:|---:|
| Current #260, jobs 4 | 146 / 185.2 | 60.0 | 4.14 | 0.990 | 73.035 |
| Current #260, jobs 8 | 179 / 227.0 | 51.3 | 4.56 | 0.868 | 73.035 |
| v0.3.0 JPEG anchor | 7 / 8.9 | 3.1 | 0.42 | 0.033 | 35.319 |
| One area-centroid sample | 112 / 142.1 | 45.7 | 4.32 | 0.938 | 73.044 |
| Inverse, certified-error padding (stopped) | 412 / 522.6 **failed work** | 317.0 | 4.20 | 0.937 | no output |
| Inverse, two-texel padding + fallback | 205 / 260.0 | 118.0 | 4.21 | 0.990 | 73.065 |
| Plain child pages; no disk ledger/drift repair | 119 / 150.9 | 44.9 | 3.26 | 0.875 | 73.035 |
| Plain child pages + centroid | 105 / 133.2 | 39.0 | 3.02 | 0.841 | 73.044 |
| Current crop3, jobs 4 | 439 / 556.8 | 161.6 | 1.58 | 0.470 | 78.866 |
| Centroid crop3 (negative control) | 467 / 592.3 | 174.5 | 1.80 | 0.470 | 78.866 |

CPU is user+sys for the complete conversion, not worker elapsed timers. Input = 788,425 triangles. The stopped inverse is not throughput. Scratch is the maximum observed logical size of the run directory (spill plus staged output) sampled every ten seconds; short peaks may be missed. `measure.py` rounds CPU to whole seconds, particularly material for the seven-second legacy anchor.

## Recommendation

**Keep certified geometry provisionally; modify appearance sampling, but reject these prototypes as a solution to the 10× goal.** The best combination is only 1.39× faster in the first total-CPU comparison. The phase-stamped repeat below separates the coarse work. One centroid sample is the better first production candidate: it preserves the ledger and needs a much smaller semantic change. Plain-child sampling saves more memory but removes appearance correspondence/drift evidence, and this prototype still allocates transient compatibility records. The measured inverse rasterisation is slower and changes colour more; do not pursue this implementation.

No unchanged-H17 variant meets the mixed-depth seam criterion. Require the separate seam-fix work before reducing published refinement error. At SSE 16 a gap equal to GE can be about 16 px; scaling GE by 0.5 doubles that to about 32 px. The all-pair measurement below is worse than the leaf-neighbour-only ratio.

For the 9,999,999-triangle 2048 model, multiplying the existing #260 **9,998 CPU-s / 3,612 s wall** by the two observed whole-run improvement ratios gives roughly **6,700–7,200 CPU-s (1.9–2.0 CPU-hours), 40–43 minutes wall at the same effective CPU/wall ratio**. This is a conditional engineering estimate, not a full-model run or a prediction under arbitrary host load. Tex3 baseline is only 185 CPU-s/Mtri versus the recorded full-model 1,000, so linear crop throughput extrapolation would be misleading. Neither the estimate nor the trials establishes the requested ~100 CPU-s/Mtri.

The legacy anchor is 20.9× cheaper on this crop and merits architectural study, but is not a drop-in replacement: it has a different tree and triangle budget, JPEG instead of verified KTX2 mip delivery, sampled rather than certified geometry error, and no current H27 terminal ledger. Its root keeps 19,968 triangles versus H17’s 4,696. Its first-view download is larger despite its smaller complete archive. Certification alone was not isolated or shown to cause the regression.

## Phase CPU repeat

| Run | Total CPU s | Capture + leaves CPU s | H17 geometry/handoff CPU s | Proxy bake CPU s | Coarse total CPU s / per Mtri | Wall s |
|---|---:|---:|---:|---:|---:|---:|
| cpu-baseline-tex3 | 159 | 21.55 | 56.27 | 80.62 | 136.89 / 173.62 | 57.6 |
| cpu-plain-centroid-tex3 | 107 | 21.56 | 48.67 | 36.71 | 85.38 / 108.29 | 41.9 |

These phase CPU stamps read `/proc/<child>/stat` as progress events arrive; they are approximate boundaries, not multiplication of elapsed time by jobs. Geometry is unchanged, so the geometry-time difference is run noise, not a claimed optimisation. Remaining total CPU is archive/orchestration. The repeat binaries are bound by SHA-256 in each `command.json`.

## Per-experiment shared metrics

### Current #260, jobs 4

| Metric | Measured result |
|---|---|
| Triangles / tiles at each depth | d0: 4,696/1; d1: 4,702/3; d2: 4,702/1; d3: 7,680/1; d4: 56,375/8; d5: 243,980/43; d6: 520,511/96; d7: 305,404/58; d8: 39,517/8 |
| Geometry vs source (root, mean / p95 / max m) | Source→proxy 0.169111 / 0.602723 / 2.429; proxy→source 0.123331 / 0.452163 / 1.20831. All-depth values below/in evidence. |
| Proxy colour vs source, PSNR dB by depth | d0 19.53; d1 19.75; d2 19.98; d3 20.96; d4 23.59; d5 25.59; d6 26.49; d7 29.37 |
| GLB bytes per depth (MB) | d0 0.310; d1 0.334; d2 0.325; d3 0.479; d4 2.991; d5 13.724; d6 31.608; d7 20.942; d8 2.220 |
| Simulated first view (far / close) | 1600-3: 4.463 MB / 22 requests; 2560-0.75: 12.289 MB / 57 requests |
| Mixed-depth / same-depth cracks | Same H17 geometry: 778 mixed-depth pairs, max 0.377111 m, worst depth-pair p99 0.035589 m; max 1.71569×GE, worst depth-pair p99 0.348295×GE. Same-depth census has an unresolved 0.207103 m outlier (1.04764×GE, six samples in one d5 pair); do not claim this census proves watertightness. |

### Current #260, jobs 8

| Metric | Measured result |
|---|---|
| Triangles / tiles at each depth | d0: 4,696/1; d1: 4,702/3; d2: 4,702/1; d3: 7,680/1; d4: 56,375/8; d5: 243,980/43; d6: 520,511/96; d7: 305,404/58; d8: 39,517/8 |
| Geometry vs source (root, mean / p95 / max m) | Source→proxy 0.169111 / 0.602723 / 2.429; proxy→source 0.123331 / 0.452163 / 1.20831. All-depth values below/in evidence. |
| Proxy colour vs source, PSNR dB by depth | d0 19.53; d1 19.75; d2 19.98; d3 20.96; d4 23.59; d5 25.59; d6 26.49; d7 29.37 |
| Published identity | Entire archive SHA-256 identical to its four-job/input control. |
| GLB bytes per depth (MB) | d0 0.310; d1 0.334; d2 0.325; d3 0.479; d4 2.991; d5 13.724; d6 31.608; d7 20.942; d8 2.220 |
| Simulated first view (far / close) | 1600-3: 4.463 MB / 22 requests; 2560-0.75: 12.289 MB / 57 requests |
| Mixed-depth / same-depth cracks | Same H17 geometry: 778 mixed-depth pairs, max 0.377111 m, worst depth-pair p99 0.035589 m; max 1.71569×GE, worst depth-pair p99 0.348295×GE. Same-depth census has an unresolved 0.207103 m outlier (1.04764×GE, six samples in one d5 pair); do not claim this census proves watertightness. |

### v0.3.0 JPEG anchor

| Metric | Measured result |
|---|---|
| Triangles / tiles at each depth | d0: 19,968/1; d1: 159,975/8; d2: 788,425/64 |
| Geometry vs source (root, mean / p95 / max m) | Source→proxy 0.0116011 / 0.0317703 / 0.386385; proxy→source 0.0111482 / 0.0305101 / 0.0956136. All-depth values below/in evidence. |
| Proxy colour vs source, PSNR dB by depth | d0 29.06; d1 30.06 |
| GLB bytes per depth (MB) | d0 2.658; d1 7.761; d2 24.874 |
| Simulated first view (far / close) | 1600-3: 13.924 MB / 18 requests; 2560-0.75: 25.982 MB / 50 requests |
| Mixed-depth / same-depth cracks | 119 d1↔d2 pairs, max 1.07e−14 m, p99 0, max 7.36e−15×GE. 12 proxy same-depth pairs: same numerical residual; 223 leaf pairs: exact zero. 42,480 mixed-depth samples. |

### One area-centroid sample

| Metric | Measured result |
|---|---|
| Triangles / tiles at each depth | d0: 4,696/1; d1: 4,702/3; d2: 4,702/1; d3: 7,680/1; d4: 56,375/8; d5: 243,980/43; d6: 520,511/96; d7: 305,404/58; d8: 39,517/8 |
| Geometry vs source (root, mean / p95 / max m) | Source→proxy 0.169111 / 0.602723 / 2.429; proxy→source 0.123331 / 0.452163 / 1.20831. All-depth values below/in evidence. |
| Proxy colour vs source, PSNR dB by depth | d0 19.82; d1 19.95; d2 20.06; d3 20.98; d4 23.63; d5 25.63; d6 26.53; d7 29.65 |
| Proxy colour vs current proxies, PSNR dB | d0 31.69; d1 32.06; d2 33.20; d3 32.86; d4 34.28; d5 36.66; d6 37.83; d7 37.59 |
| Published identity | All 156 leaf GLBs byte-identical; all decoded proxy positions and face order identical. |
| GLB bytes per depth (MB) | d0 0.311; d1 0.335; d2 0.325; d3 0.480; d4 2.992; d5 13.727; d6 31.610; d7 20.942; d8 2.220 |
| Simulated first view (far / close) | 1600-3: 4.467 MB / 22 requests; 2560-0.75: 12.295 MB / 57 requests |
| Mixed-depth / same-depth cracks | Same H17 geometry: 778 mixed-depth pairs, max 0.377111 m, worst depth-pair p99 0.035589 m; max 1.71569×GE, worst depth-pair p99 0.348295×GE. Same-depth census has an unresolved 0.207103 m outlier (1.04764×GE, six samples in one d5 pair); do not claim this census proves watertightness. |

### Inverse, certified-error padding (stopped)

| Metric | Measured result |
|---|---|
| Geometry, texture, bytes and cracks | No completed artifact; stopped at 59/63 proxies after the broad search regressed. No quality acceptance. |

### Inverse, two-texel padding + fallback

| Metric | Measured result |
|---|---|
| Triangles / tiles at each depth | d0: 4,696/1; d1: 4,702/3; d2: 4,702/1; d3: 7,680/1; d4: 56,375/8; d5: 243,980/43; d6: 520,511/96; d7: 305,404/58; d8: 39,517/8 |
| Geometry vs source (root, mean / p95 / max m) | Source→proxy 0.169111 / 0.602723 / 2.429; proxy→source 0.123331 / 0.452163 / 1.20831. All-depth values below/in evidence. |
| Proxy colour vs source, PSNR dB by depth | d0 19.32; d1 19.37; d2 19.37; d3 20.19; d4 23.24; d5 25.70; d6 26.52; d7 29.36 |
| Proxy colour vs current proxies, PSNR dB | d0 23.80; d1 24.10; d2 24.50; d3 25.69; d4 29.49; d5 34.35; d6 37.39; d7 37.52 |
| Published identity | All 156 leaf GLBs byte-identical; all decoded proxy positions and face order identical. |
| GLB bytes per depth (MB) | d0 0.313; d1 0.337; d2 0.326; d3 0.481; d4 3.000; d5 13.731; d6 31.611; d7 20.942; d8 2.220 |
| Simulated first view (far / close) | 1600-3: 4.482 MB / 22 requests; 2560-0.75: 12.314 MB / 57 requests |
| Mixed-depth / same-depth cracks | Same H17 geometry: 778 mixed-depth pairs, max 0.377111 m, worst depth-pair p99 0.035589 m; max 1.71569×GE, worst depth-pair p99 0.348295×GE. Same-depth census has an unresolved 0.207103 m outlier (1.04764×GE, six samples in one d5 pair); do not claim this census proves watertightness. |

### Plain child pages; no disk ledger/drift repair

| Metric | Measured result |
|---|---|
| Triangles / tiles at each depth | d0: 4,696/1; d1: 4,702/3; d2: 4,702/1; d3: 7,680/1; d4: 56,375/8; d5: 243,980/43; d6: 520,511/96; d7: 305,404/58; d8: 39,517/8 |
| Geometry vs source (root, mean / p95 / max m) | Source→proxy 0.169111 / 0.602723 / 2.429; proxy→source 0.123331 / 0.452163 / 1.20831. All-depth values below/in evidence. |
| Proxy colour vs source, PSNR dB by depth | d0 19.53; d1 19.75; d2 19.98; d3 20.96; d4 23.59; d5 25.59; d6 26.48; d7 29.37 |
| Proxy colour vs current proxies, PSNR dB | d0 59.81; d1 59.91; d2 58.59; d3 58.52; d4 55.94; d5 56.26; d6 51.73; d7 54.73 |
| Published identity | All 156 leaf GLBs byte-identical; all decoded proxy positions and face order identical. |
| GLB bytes per depth (MB) | d0 0.310; d1 0.334; d2 0.325; d3 0.479; d4 2.991; d5 13.724; d6 31.608; d7 20.942; d8 2.220 |
| Simulated first view (far / close) | 1600-3: 4.463 MB / 22 requests; 2560-0.75: 12.289 MB / 57 requests |
| Mixed-depth / same-depth cracks | Same H17 geometry: 778 mixed-depth pairs, max 0.377111 m, worst depth-pair p99 0.035589 m; max 1.71569×GE, worst depth-pair p99 0.348295×GE. Same-depth census has an unresolved 0.207103 m outlier (1.04764×GE, six samples in one d5 pair); do not claim this census proves watertightness. |

### Plain child pages + centroid

| Metric | Measured result |
|---|---|
| Triangles / tiles at each depth | d0: 4,696/1; d1: 4,702/3; d2: 4,702/1; d3: 7,680/1; d4: 56,375/8; d5: 243,980/43; d6: 520,511/96; d7: 305,404/58; d8: 39,517/8 |
| Geometry vs source (root, mean / p95 / max m) | Source→proxy 0.169111 / 0.602723 / 2.429; proxy→source 0.123331 / 0.452163 / 1.20831. All-depth values below/in evidence. |
| Proxy colour vs source, PSNR dB by depth | d0 19.82; d1 19.95; d2 20.06; d3 20.97; d4 23.63; d5 25.63; d6 26.53; d7 29.66 |
| Proxy colour vs current proxies, PSNR dB | d0 31.69; d1 32.06; d2 33.19; d3 32.86; d4 34.27; d5 36.64; d6 37.75; d7 37.52 |
| Published identity | All 156 leaf GLBs byte-identical; all decoded proxy positions and face order identical. |
| GLB bytes per depth (MB) | d0 0.311; d1 0.335; d2 0.325; d3 0.479; d4 2.992; d5 13.726; d6 31.610; d7 20.942; d8 2.220 |
| Simulated first view (far / close) | 1600-3: 4.467 MB / 22 requests; 2560-0.75: 12.295 MB / 57 requests |
| Mixed-depth / same-depth cracks | Same H17 geometry: 778 mixed-depth pairs, max 0.377111 m, worst depth-pair p99 0.035589 m; max 1.71569×GE, worst depth-pair p99 0.348295×GE. Same-depth census has an unresolved 0.207103 m outlier (1.04764×GE, six samples in one d5 pair); do not claim this census proves watertightness. |

### Current crop3, jobs 4

| Metric | Measured result |
|---|---|
| Triangles / tiles at each depth | d0: 4,696/1; d1: 4,702/3; d2: 4,702/1; d3: 7,680/1; d4: 56,375/8; d5: 243,980/43; d6: 520,511/96; d7: 305,404/58; d8: 39,517/8 |
| Geometry vs source (root, mean / p95 / max m) | Source→proxy 0.169111 / 0.602723 / 2.429; proxy→source 0.123331 / 0.452163 / 1.20831. All-depth values below/in evidence. |
| Proxy colour vs source, PSNR dB by depth | d0 17.94; d1 17.95; d2 17.93; d3 18.62; d4 22.42; d5 28.70; d6 31.48; d7 37.88 |
| GLB bytes per depth (MB) | d0 0.370; d1 0.394; d2 0.383; d3 0.621; d4 4.453; d5 17.648; d6 33.607; d7 18.919; d8 2.375 |
| Simulated first view (far / close) | 1600-3: 6.246 MB / 22 requests; 2560-0.75: 18.059 MB / 57 requests |
| Mixed-depth / same-depth cracks | All 219 decoded triangle sets exactly match tex3 (seven material-group ordering changes). Both crop3 archives are identical, so the same 778 mixed-depth pairs and H17 gaps apply. |

### Centroid crop3 (negative control)

| Metric | Measured result |
|---|---|
| Triangles / tiles at each depth | d0: 4,696/1; d1: 4,702/3; d2: 4,702/1; d3: 7,680/1; d4: 56,375/8; d5: 243,980/43; d6: 520,511/96; d7: 305,404/58; d8: 39,517/8 |
| Geometry vs source (root, mean / p95 / max m) | Source→proxy 0.169111 / 0.602723 / 2.429; proxy→source 0.123331 / 0.452163 / 1.20831. All-depth values below/in evidence. |
| Proxy colour vs source, PSNR dB by depth | d0 17.94; d1 17.95; d2 17.93; d3 18.62; d4 22.42; d5 28.70; d6 31.48; d7 37.88 |
| Published identity | Entire archive SHA-256 identical to its four-job/input control. |
| GLB bytes per depth (MB) | d0 0.370; d1 0.394; d2 0.383; d3 0.621; d4 4.453; d5 17.648; d6 33.607; d7 18.919; d8 2.375 |
| Simulated first view (far / close) | 1600-3: 6.246 MB / 22 requests; 2560-0.75: 18.059 MB / 57 requests |
| Mixed-depth / same-depth cracks | All 219 decoded triangle sets exactly match tex3 (seven material-group ordering changes). Both crop3 archives are identical, so the same 778 mixed-depth pairs and H17 gaps apply. |

## Per-level baseline geometry and errors

Samples are area-weighted, fixed seed, **200,000 in each direction per level**, including shallower leaves to make a complete frontier. Values are sampled deviations, not a Hausdorff certificate. All successful texture variants reproduce the baseline decoded positions and therefore the same geometry measurements.

| Depth | Source→frontier mean / p95 / max m | Frontier→source mean / p95 / max m |
|---:|---|---|
| 0 | 0.169111 / 0.602723 / 2.429 | 0.123331 / 0.452163 / 1.20831 |
| 1 | 0.168337 / 0.598625 / 2.42842 | 0.123987 / 0.455209 / 1.20384 |
| 2 | 0.168216 / 0.597669 / 2.42842 | 0.123811 / 0.454638 / 1.19065 |
| 3 | 0.11533 / 0.415612 / 2.44153 | 0.093589 / 0.350091 / 1.19176 |
| 4 | 0.0265206 / 0.1367 / 0.738499 | 0.0213649 / 0.107366 / 0.744155 |
| 5 | 0.00117068 / 0.0061471 / 0.0835213 | 0.00110468 / 0.00581575 / 0.092582 |
| 6 | 0.000134592 / 0.000601564 / 0.0177515 | 0.000130402 / 0.000588489 / 0.0174067 |
| 7 | 2.89338e-05 / 8.00225e-05 / 0.002622 | 2.87823e-05 / 8.04901e-05 / 0.00295787 |
| 8 | 2.79042e-05 / 7.9161e-05 / 0.000196636 | 2.7837e-05 / 7.90492e-05 / 0.000174922 |

## Method, interpretation and limits

- Base: `8971c9c3e4021f893d3dde5d8fc457767f2b0809` (`fix/proxy-texel-density`). No changes to H17 simplification, certification, seam thinning, tile tree or leaf writer. No develop baseline was built; the required current-branch baseline was built and run here.
- All bakes used the supplied `measure.py`, `RT_MEASURE_SLOT=2`, CPUs `0-3,16-19`, SCHED_IDLE and nice 19. Its docstring calls slot 2 a correctness slot, not a clean timing lane; these are requested-slot exploratory CPU comparisons under host contention. Initial build used the original four-worker request; later builds used the shared three-worker rule. Builds were serialized in the dedicated target.
- v0.3.0 is tag `99b9605b`; its source was extracted inside this worktree, rather than editing another checkout. Only an empty `[workspace]` stanza was added for the nested build. Rayon was capped at four; defaults were 20,000 triangles per leaf, JPEG quality 95/full chroma, parent page cap 1024. Output: 64 leaves, 8 depth-1 parents and a root. The initial anchor invocation used the modern placement spelling and failed argument parsing; `legacy-tex3` is that zero-work failure, excluded from speed comparisons. `legacy-anchor-tex3` uses the legacy spelling.
- Inverse rasterisation projects same-primitive, facing **source** triangles orthographically into each fixed proxy face’s UV rectangle, rasterises one coverage-centroid sample and chooses minimum absolute surface displacement. Fills and empty projected pixels use ordinary lookup. It is not a full parameterisation from simplification, not a global one-pass source bake, and not mathematically equivalent to nearest-surface sampling. The first padding choice admitted too many candidates; limiting the box to two world texels completed but remained slower and reduced shallow-level source PSNR. No claim that all inverse methods are disproved.
- Centroid keeps clipped positive coverage and replaces the up-to-4×4 sub-sample grid with one area-centroid sample. Existing mip/codec gates remain. Its maximum local colour changes exceed 100 byte values despite good mean PSNR; highlights, thin markings, normal/MR/alpha slots, oblique viewing and temporal aliasing remain untested. No browser or lit-render acceptance was performed.
- Plain mode reads leaf base-colour pre-encode pages, builds ordinary linear-light box mips where needed, and samples child pages through the existing BVH and bilinear/trilinear filter. Non-base-colour leaf slots keep the direct path. It removes the **109-byte terminal record from each stored packet texel** (113→4 wire bytes) and bypasses displacement repair, deep drift checks and the witness-distance source reread. It does **not** remove every transient sample/frame vector: placeholders keep the unmodified raster/mip interfaces working, and those receipts are not valid terminal evidence. Normal-map/alpha/MASK/ID acceptance is not established. Do not ship the flag or interpret synthetic terminal fields as valid correspondence.
- Geometry/colour evaluation independently decodes meshopt, quantisation and node transforms. KTX2 is decoded with the locally installed Three/Basis WASM transcoder; PNG/JPEG with Pillow. Colour is unlit base colour at random proxy surface points, bilinear decoded level 0 in linear light, compared with source colour at the nearest source point. This measures surface colour, not a lit image or viewer mip selection; a close parallel surface can be a different donor. Full decoder paths and hashes are retained.
- H17 mixed-depth all-pair results reuse the parallel UV experiment’s baseline census, bound here by **219/219 complete baseline GLB members byte-identical** to its input. Each successful texture variant separately verifies decoded positions/face order against this baseline. The census tests every adjacent non-overlapping tile pair at all depths using shared source-edge endpoints/midpoints projected to each decoded open border, then symmetric distance to the other surface. Its Open3D float computations have small numerical residuals, but also an unresolved same-depth d5↔d5 outlier: 0.207103 m (1.04764×GE), six samples for `f344092-d4-1-0-1-0.glb` / `f344092-d4-1-1-0-0.glb`. The existing same-depth frontier audit finds no unmatched edges far beyond GE from the source boundary, which does not resolve a gap below GE. This could be a real defect or a source-anchor-to-border projection ambiguity; no watertight conclusion is drawn here. Our independent leaf-neighbour census agrees at 0.377111 m, but only reaches ~1×GE; the symmetric all-depth census reaches 1.71569×GE. Neither finite probe proves a continuous crack-width maximum.
- Legacy pair census uses the independent double BVH on decoded bytes, all 354 adjacent pairs (119 mixed), with the same endpoint/midpoint probe. It recovers all 788,425 source triangles exactly from leaves and compares lexicographically sorted complete XYZ triples and normalises cyclic corner order before source-edge lookup. A sensitive control rejects different triangles that share coordinate-wise sorted values; the earlier weaker check was replaced and the identity/seam probes rerun. An earlier corner-order mapping produced false gaps; the `invalid-corner-order-*` files are retained and explicitly excluded. The corrected probes are essentially zero, but this crop is not a universal seam guarantee.
- Crop3 is a negative control: no proxy textures means the centroid flag changes no bytes. 439→467 CPU-s is noise/regression, not a speed gain. Texture sampling improvements do not address its expensive source-colour queries.
- Jobs 8 versus 4 is 60.0→51.3 s wall but 146→179 CPU-s, with an identical archive. This is a wall-time knob, not CPU savings. Intra-proxy parallelism was not prototyped; it cannot by itself establish the required CPU reduction.

## Evidence and reproduction

Bulk evidence: `/home/bend/.cache/rusty-tiles-evidence/fast-lod-exp-20261024/`; shared results: `/home/bend/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/astra-keep-h17/README.md`. `SHA256SUMS` binds scripts, logs, measures and reports; `source-final.patch` and `source-files.sha256` bind prototype source; each run records the binary SHA and command. Archives and decoded meshes remain in `/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/astra-keep-h17/`; run spill is deleted.

Build the prototype with `CARGO_TARGET_DIR=/home/bend/.cache/rt-wt/target-fast-lod-exp CARGO_BUILD_JOBS=3 cargo build --release --locked --features fast-lod-experiments --bin rusty-tiles`, prefixed with `chrt -i 0 nice -n 19`. The explicit environment switches are `RT_EXP_CENTROID`, `RT_EXP_INVERSE` (also enables centroid), and `RT_EXP_PLAIN_CHILD`. They are ignored without the feature. `run.py`, queue scripts and binary manifests in evidence reproduce the measured invocations; `bench/fast_lod_experiments/` contains the executed decoders, independent BVH, quality and crack probes.

Validation: 70 existing bake tests passed with the experiment feature compiled and flags off; independent analytic distance controls passed. Experimental paths are supported only by these crop executions, not production tests or broader acceptance. Draft research only; nothing here is ready to merge.
