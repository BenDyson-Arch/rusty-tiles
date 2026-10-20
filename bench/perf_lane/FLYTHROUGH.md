# Flythrough report

Generated 2026-10-14T00:34:33 by `bench/perf_lane/flythrough.py`. Raw data: `/home/bend/.cache/rusty-tiles-evidence/perf/flythrough-20261013` (not in git).

- Binary: `rusty-tiles 0.4.0` sha256 `fab72bf589e5`, commit `d1818d15ea19`
- Renderer: `ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 4080 SUPER/PCIe/SSE2, OpenGL ES 3.2)` (gpu), 153.0.8010.52, cdp; viewport [1024, 768] at DPR 1; cache disabled; Cesium 1.146.0
- Paths replay at a fixed 60 frames per simulated second; the display presented frames at a median 16.70 ms. GPU timer queries: EXT_disjoint_timer_query_webgl2.
- Repeats: 3, order rotated per repeat. Waypoints hold until full quality (finish rule, 250 ms) or 10 s.
- Never compare across profiles or renderer classes: each table holds one recipe set, one profile and one renderer.
- Cells are medians over clean timed runs (all timed runs when fewer than two are clean); `[k/N]` means only k of the N runs used had a value; `~` marks a metric whose repeats spread beyond max(10% of the median, floor) and `?` one with fewer than two values. Frame times are presented-frame intervals (rAF); hitches are intervals over 33 and 100 ms.
- Basis: frame, CPU and GPU times, hitches, LOD pops and SSE jumps count motion frames only (`motion`); a path without motion (snap-teleport) counts its hold frames except each cut (`hold`). `all frames` adds the holds. Frames after a screenshot are never counted.
- Matrix `validate`: Trimmed validation that the harness works and is stable: every path on the native GPU for four recipes, the constrained profiles on one recipe and two paths. The terrain pair is the fully refined (SSE 0) one, where #201 measured its effect; at SSE 16 the corpus DEM barely refines.
- Screenshots and Chrome traces come only from one extra, untimed run per selected cell; timed runs capture nothing, trace nothing and render without preserveDrawingBuffer.

## Paths

| path | motion frames | waypoints | description |
|---|---|---|---|
| orbit-far | 600 | start, end | One full orbit at 2.5 r, 10 s of frames |
| approach | 420 | overview, close | Overview at 4 r down to a close view at 0.15 r, eased, 7 s of frames |
| fast-pan | 90 | west, east | Lateral pan across 1.2 r at 0.5 r range in 1.5 s |
| close-crawl | 900 | sw, ne | Slow diagonal crawl at 0.12 r range, 15 s of frames |
| snap-teleport | 0 | overview, ne-close, sw-close, centre-close, overview-again | Instant cuts between distant views, each held until full quality (streaming stress) |

## Profiles

| profile | settings | description |
|---|---|---|
| native | {} | The GPU browser as is |
| cpu4x | {"cpuThrottlingRate": 4} | Chrome CPU throttling 4x (Emulation.setCPUThrottlingRate, page main thread) |
| lowcache | {"tilesetOptions": {"cacheBytes": 8388608, "maximumCacheOverflowBytes": 0}} | Tileset cache trimmed to 8 MiB with no overflow headroom |

## Recipes

| set | recipe | conversion | viewer | output |
|---|---|---|---|---|
| terrain-skiplod | terrain-lod-sse0 | `terrain -i {corpus}/dem.tif -o {out} --height-offset 0 --fill-height 0` | `{"maximumScreenSpaceError": 0}` | 8.39 MiB, 420 files |
| terrain-skiplod | terrain-skiplod-sse0 | `terrain -i {corpus}/dem.tif -o {out} --height-offset 0 --fill-height 0` | `{"tilesetOptions": {"skipLevelOfDetail": true}, "maximumScreenSpaceError": 0}` | 8.39 MiB, 420 files |
| vector-leaves | vector-default | `vector -i {corpus}/vector.geojson -o {out}.3tz --jobs 2 --reproducible` | `{}` | 18.14 MiB, archive files |
| vector-leaves | vector-leaf16 | `vector -i {corpus}/vector.geojson -o {out}.3tz --jobs 2 --reproducible --maxFeatures 16` | `{}` | 28.83 MiB, archive files |

## terrain-skiplod: native

| path | recipe | basis | frame p50/p95/p99 ms | hitch >33/>100 | all frames p95 / >33 | cpu p95 ms | gpu p95 ms | first render s | full quality s | requests / MiB | peak req/s | draws p50 | tris p50 k | selected mean | GPU MiB peak | load/evict/reload | pops / SSE jumps | runs used/total (clean) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| orbit-far | terrain-lod-sse0 | motion | 16.7 / 18.0 / 18.4 | 0 / 0 | 18.0 / 3 | 2.1 | 7.30 | 0.21 ~ | 0.83 (0.82-1.32) ~ | 419 / 8.39 | 419 | 294 | 2,232.5 | 278.2 | 28.8 | 418 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| orbit-far | terrain-skiplod-sse0 | motion | 16.7 / 18.1 / 18.4 | 0 ~ / 0 | 18.1 / 3 ~ | 2.2 | 7.57 ~ | 0.21 | 0.67 (0.66-0.67) | 300 / 6.28 | 300 | 294 | 2,232.5 | 280.3 | 21.6 | 299 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| approach | terrain-lod-sse0 | motion | 16.7 / 18.0 / 18.1 | 0 / 0 | 18.0 / 3 | 1.9 | 6.52 | 0.20 | 0.77 (0.77-0.81) | 419 / 8.39 | 419 | 170 | 1,329.6 | 165.1 | 28.8 | 418 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| approach | terrain-skiplod-sse0 | motion | 16.7 / 18.0 / 18.2 | 0 / 0 | 18.0 / 3 | 2.1 | 6.25 | 0.21 | 0.67 (0.65-0.75) ~ | 322 / 6.72 | 300 | 171 | 1,333.7 | 165.4 | 23.1 | 321 / 0 / 0 | 0 / 12 | 3/3 (3 clean) |
| fast-pan | terrain-lod-sse0 | motion | 16.7 / 18.1 / 18.1 | 0 / 0 | 18.1 / 3 | 1.4 | 5.28 ~ | 0.20 | 0.59 (0.57-0.65) ~ | 214 / 4.46 | 110 | 53 | 400.0 | 37.0 | 15.3 | 213 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| fast-pan | terrain-skiplod-sse0 | motion | 16.7 / 17.8 / 18.1 | 0 / 0 | 18.1 / 3 | 1.4 | 5.22 | 0.20 | 0.46 (0.45-0.54) ~ | 118 / 2.51 | 62 | 45 | 334.7 | 32.4 | 8.5 | 117 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| close-crawl | terrain-lod-sse0 | motion | 16.7 / 18.3 / 18.5 | 0 / 0 | 18.3 / 3 | 1.2 | 6.05 ~ | 0.20 | 0.33 (0.32-0.37) | 192 / 4.23 | 86 | 28 | 191.9 | 22.8 | 14.6 | 191 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| close-crawl | terrain-skiplod-sse0 | motion | 16.7 / 17.7 / 18.2 | 0 / 0 | 18.0 / 2 | 1.1 | 5.99 | 0.21 ~ | 0.26 (0.25-1.29) ~ | 123 / 2.73 | 30 | 28 | 200.1 | 22.9 | 9.4 | 122 / 0 / 0 | 0 / 34 | 3/3 (3 clean) |
| snap-teleport | terrain-lod-sse0 | hold | 16.7 / 18.1 / 46.8 | 4 / 0 | 18.1 / 4 | 8.3 ~ | 8.44 ~ | 0.22 | 0.87 (0.86-0.88) | 419 / 8.39 | 419 | 48 | 347.7 | 121.4 | 28.8 | 418 / 0 / 0 | 2 / 0 | 3/3 (3 clean) |
| snap-teleport | terrain-skiplod-sse0 | hold | 16.7 / 18.1 / 47.7 ~ | 3 / 0 | 18.0 / 3 | 8.9 | 8.26 ~ | 0.21 | 0.81 (0.78-0.82) | 301 / 6.28 | 301 | 19 | 91.9 | 114.1 | 21.6 | 300 / 0 / 0 | 3 / 1 | 3/3 (3 clean) |

Time to full quality per waypoint (s, median), terrain-skiplod, native:

| path | waypoint | terrain-lod-sse0 | terrain-skiplod-sse0 |
|---|---|---|---|
| orbit-far | start | 0.81 | 0.65 |
| orbit-far | end | 0.02 | 0.02 |
| approach | overview | 0.75 | 0.65 |
| approach | close | 0.02 | 0.02 |
| fast-pan | west | 0.35 | 0.27 |
| fast-pan | east | 0.23 | 0.18 |
| close-crawl | sw | 0.32 | 0.24 |
| close-crawl | ne | 0.02 | 0.02 |
| snap-teleport | overview | 0.81 | 0.64 |
| snap-teleport | ne-close | 0.02 | 0.12 |
| snap-teleport | sw-close | 0.02 | 0.02 |
| snap-teleport | centre-close | 0.02 | 0.02 |
| snap-teleport | overview-again | 0.02 | 0.02 |

## terrain-skiplod: cpu4x

| path | recipe | basis | frame p50/p95/p99 ms | hitch >33/>100 | all frames p95 / >33 | cpu p95 ms | gpu p95 ms | first render s | full quality s | requests / MiB | peak req/s | draws p50 | tris p50 k | selected mean | GPU MiB peak | load/evict/reload | pops / SSE jumps | runs used/total (clean) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| approach | terrain-skiplod-sse0 | motion | 16.7 / 18.1 / 18.4 | 0 / 0 | 33.4 / 35 ~ | 9.2 | 9.61 ~ | 0.52 | 1.89 (1.81-1.92) | 322 / 6.72 | 216 | 171 | 1,351.4 | 165.8 | 23.1 | 321 / 0 / 0 | 0 / 12 | 3/3 (3 clean) |
| snap-teleport | terrain-skiplod-sse0 | hold | 16.8 / 66.7 / 105.8 ~ | 32 / 2 | 66.7 / 32 | 31.0 | 26.07 ~ | 0.52 | 2.03 (2.00-2.07) | 301 / 6.28 | 216 | 25 ~ | 161.0 ~ | 118.9 | 21.6 | 300 / 0 / 0 | 2 / 1 | 3/3 (3 clean) |

Time to full quality per waypoint (s, median), terrain-skiplod, cpu4x:

| path | waypoint | terrain-lod-sse0 | terrain-skiplod-sse0 |
|---|---|---|---|
| approach | overview | n/a | 1.87 |
| approach | close | n/a | 0.02 |
| snap-teleport | overview | n/a | 1.86 |
| snap-teleport | ne-close | n/a | 0.11 |
| snap-teleport | sw-close | n/a | 0.02 |
| snap-teleport | centre-close | n/a | 0.02 |
| snap-teleport | overview-again | n/a | 0.03 |

## terrain-skiplod: lowcache

| path | recipe | basis | frame p50/p95/p99 ms | hitch >33/>100 | all frames p95 / >33 | cpu p95 ms | gpu p95 ms | first render s | full quality s | requests / MiB | peak req/s | draws p50 | tris p50 k | selected mean | GPU MiB peak | load/evict/reload | pops / SSE jumps | runs used/total (clean) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| approach | terrain-skiplod-sse0 | motion | 16.7 / 18.0 / 18.4 | 0 / 0 | 18.0 / 3 | 1.9 | 6.46 | 0.22 | n/a | 322 / 6.72 | 300 | 102 | 831.2 | 86.5 | 8.0 | 321 / 214 / 0 | 0 / 8 | 3/3 (3 clean) |
| snap-teleport | terrain-skiplod-sse0 | hold | 16.7 / 18.1 / 18.5 | 4 ~ / 0 | 18.1 / 4 ~ | 1.6 | 5.82 | 0.21 | n/a | 471 / 10.28 | 300 | 102 | 831.2 | 93.9 | 8.1 | 305 / 170 / 5 | 2 / 1 | 3/3 (3 clean) |

Time to full quality per waypoint (s, median), terrain-skiplod, lowcache:

| path | waypoint | terrain-lod-sse0 | terrain-skiplod-sse0 |
|---|---|---|---|
| approach | overview | n/a | n/a [0/3] (3 unreached) |
| approach | close | n/a | 0.02 |
| snap-teleport | overview | n/a | n/a [0/3] (3 unreached) |
| snap-teleport | ne-close | n/a | 2.73 |
| snap-teleport | sw-close | n/a | 0.02 |
| snap-teleport | centre-close | n/a | 0.17 |
| snap-teleport | overview-again | n/a | n/a [0/3] (3 unreached) |

## vector-leaves: native

| path | recipe | basis | frame p50/p95/p99 ms | hitch >33/>100 | all frames p95 / >33 | cpu p95 ms | gpu p95 ms | first render s | full quality s | requests / MiB | peak req/s | draws p50 | tris p50 k | selected mean | GPU MiB peak | load/evict/reload | pops / SSE jumps | runs used/total (clean) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| orbit-far | vector-default | motion | 16.7 / 18.2 / 18.4 | 0 / 0 | 18.3 / 4 | 0.7 | 5.06 | 0.23 | 0.24 (0.23-0.25) | 5 / 1.45 | 5 | 16 | 18.8 | 4.0 | 0.8 | 4 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| orbit-far | vector-leaf16 | motion | 16.7 / 18.1 / 18.4 | 0 / 0 | 18.1 / 4 | 0.6 | 5.60 | 0.25 | 0.24 (0.23-0.25) | 5 / 1.66 | 5 | 16 | 15.9 | 4.0 | 0.9 | 4 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| approach | vector-default | motion | 16.7 / 18.1 / 18.4 | 0 / 0 | 18.2 / 3 | 0.8 | 5.80 ~ | 0.23 | 0.23 (0.22-0.23) | 17 / 3.47 | 8 | 16 | 18.8 | 4.2 | 2.1 | 16 / 0 / 0 | 1 / 0 | 3/3 (3 clean) |
| approach | vector-leaf16 | motion | 16.7 / 18.2 / 18.5 | 0 / 0 | 18.3 / 4 | 2.0 | 4.80 ~ | 0.24 | 0.23 (0.22-0.23) | 39 / 4.83 | 18 | 17 | 15.9 | 6.7 | 2.8 | 38 / 0 / 0 | 2 / 0 | 3/3 (3 clean) |
| fast-pan | vector-default | motion | 16.7 / 17.9 / 18.1 ~ | 0 / 0 | 18.3 / 4 | 0.7 | 4.26 | 0.23 | 0.23 (0.23-0.23) | 5 / 1.45 | 5 | 14 | 16.4 | 3.8 | 0.8 | 4 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| fast-pan | vector-leaf16 | motion | 16.6 / 18.2 / 18.3 | 0 / 0 | 18.2 / 5 | 0.6 | 4.76 | 0.28 | 0.27 (0.26-0.28) | 13 / 2.99 | 13 | 20 | 14.8 | 6.1 | 1.7 | 12 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| close-crawl | vector-default | motion | 16.7 / 18.2 / 18.5 | 0 / 0 | 18.2 / 5 | 0.6 | 6.00 ~ | 0.26 | 0.26 (0.26-0.29) | 21 / 3.95 | 13 | 18 | 26.3 | 5.8 | 2.4 | 20 / 0 / 0 | 0 / 0 | 3/3 (3 clean) |
| close-crawl | vector-leaf16 | motion | 16.7 / 18.3 / 18.5 | 0 / 0 | 18.3 / 5 | 0.7 | 5.56 ~ | 0.30 | 0.34 (0.33-0.34) | 53 / 5.51 | 35 | 28 | 23.0 | 10.5 | 3.3 | 52 / 0 / 0 | 0 / 8 | 3/3 (3 clean) |
| snap-teleport | vector-default | hold | 16.7 / 18.4 / 50.8 | 4 / 0 | 18.4 / 4 | 18.9 ~ | 6.99 ~ | 0.24 | 0.50 (0.50-0.51) | 21 / 3.95 | 21 | 16 | 15.5 | 3.2 | 2.4 | 20 / 0 / 0 | 2 / 2 | 3/3 (3 clean) |
| snap-teleport | vector-leaf16 | hold | 16.7 / 18.4 / 49.7 ~ | 4 ~ / 0 | 18.1 / 4 ~ | 17.0 | 12.24 ~ | 0.24 | 0.56 (0.54-0.57) | 45 / 5.14 | 45 | 16 | 13.6 | 4.5 | 3.0 | 44 / 0 / 0 | 4 / 4 | 3/3 (3 clean) |

Time to full quality per waypoint (s, median), vector-leaves, native:

| path | waypoint | vector-default | vector-leaf16 |
|---|---|---|---|
| orbit-far | start | 0.22 | 0.22 |
| orbit-far | end | 0.02 | 0.02 |
| approach | overview | 0.22 | 0.21 |
| approach | close | 0.01 | 0.02 |
| fast-pan | west | 0.21 | 0.25 |
| fast-pan | east | 0.02 | 0.02 |
| close-crawl | sw | 0.24 | 0.32 |
| close-crawl | ne | 0.02 | 0.02 |
| snap-teleport | overview | 0.22 | 0.22 |
| snap-teleport | ne-close | 0.13 | 0.17 |
| snap-teleport | sw-close | 0.12 | 0.15 |
| snap-teleport | centre-close | 0.02 | 0.02 |
| snap-teleport | overview-again | 0.02 | 0.02 |

## Screenshots

Canvas captures at full quality, from the untimed capture run of each recipe, path and profile.

terrain-skiplod, native (files under `/home/bend/.cache/rusty-tiles-evidence/perf/flythrough-20261013/screenshots/`):

| path | waypoint | terrain-lod-sse0 | terrain-skiplod-sse0 | identical |
|---|---|---|---|---|
| orbit-far | start | `e0a1b92f02` | `e0a1b92f02` | yes |
| orbit-far | end | `e0a1b92f02` | `e0a1b92f02` | yes |
| approach | overview | `565b4ba6b5` | `565b4ba6b5` | yes |
| approach | close | `e1f418f4e5` | `e1f418f4e5` | yes |
| fast-pan | west | `41739fa0ef` | `41739fa0ef` | yes |
| fast-pan | east | `1b03f08b64` | `1b03f08b64` | yes |
| close-crawl | sw | `9f5fb14bc0` | `9f5fb14bc0` | yes |
| close-crawl | ne | `2577ff0661` | `2577ff0661` | yes |
| snap-teleport | overview | `79b40e19e1` | `79b40e19e1` | yes |
| snap-teleport | ne-close | `45e53711f2` | `45e53711f2` | yes |
| snap-teleport | sw-close | `840a950491` | `840a950491` | yes |
| snap-teleport | centre-close | `fb1e1158bf` | `fb1e1158bf` | yes |
| snap-teleport | overview-again | `79b40e19e1` | `79b40e19e1` | yes |

vector-leaves, native (files under `/home/bend/.cache/rusty-tiles-evidence/perf/flythrough-20261013/screenshots/`):

| path | waypoint | vector-default | vector-leaf16 | identical |
|---|---|---|---|---|
| orbit-far | start | `388fc6c3ad` | `61686e99bf` | no |
| orbit-far | end | `388fc6c3ad` | `61686e99bf` | no |
| approach | overview | `7e37361f79` | `a23ad71ad3` | no |
| approach | close | `18e44a8bc3` | `94008dc0de` | no |
| fast-pan | west | `e990c39b6f` | `0eb183a1ae` | no |
| fast-pan | east | `0d649e6f97` | `ee787f78aa` | no |
| close-crawl | sw | `778cb88e53` | `aeab26b5bb` | no |
| close-crawl | ne | `cd82ef6a43` | `7c941f9e6b` | no |
| snap-teleport | overview | `6db0d31fff` | `c24c04e221` | no |
| snap-teleport | ne-close | `4295a3fd77` | `34fdaecccd` | no |
| snap-teleport | sw-close | `90e47c9b65` | `b9620db46b` | no |
| snap-teleport | centre-close | `2bdc1cce51` | `9e6c91a405` | no |
| snap-teleport | overview-again | `6db0d31fff` | `c24c04e221` | no |

## Chrome traces

One untimed, traced run per selected cell (tracing slows it). Busy ms over the flythrough, overlapping events counted once: the page renderer main thread in total, its JS, GC and GPU command issue and waits; the renderer's other worker threads (Chrome's thread pool: background compile, GC, decode; and any web workers); image decode; and the GPU process main thread, which other tabs share. Traces are under `traces/` in the evidence directory.

| run | mainBusyMs | jsMs | gcMs | gpuCommandsMs | gpuWaitMs | workerBusyMs | imageDecodeMs | gpuProcessBusyMs |
|---|---|---|---|---|---|---|---|---|
| terrain-lod-sse0.approach.native.untimed | 1,527 | 1,228 | 21 | 159 | 137 | 564 | 1 | 803 |
| terrain-skiplod-sse0.approach.native.untimed | 1,449 | 1,195 | 20 | 141 | 119 | 539 | 1 | 753 |
| vector-default.approach.native.untimed | 791 | 618 | 21 | 178 | 159 | 367 | 1 | 602 |
| vector-leaf16.approach.native.untimed | 924 | 726 | 32 | 199 | 180 | 509 | 1 | 649 |

## Host load and noise

Per run: PSI stall share of the run wall time (cpu, memory, io), lowest MemAvailable, GPU utilisation before the run (another process) and mean CPU busy. Noisy when: cpuPressure 0.25, memoryPressure 0.02, ioPressure 0.1, memAvailableMinGiB 4, gpuUtilStart 30 (MemAvailable below, others at or above).

| profile | runs | noisy | cpu PSI (median) | mem PSI | io PSI | min MemAvail GiB | GPU util % (median) | CPU busy (median) |
|---|---|---|---|---|---|---|---|---|
| native | 60 | 0 | 0% | 0.0% | 0.2% | 53.2 | 3 | 14% |
| cpu4x | 6 | 0 | 0% | 0.0% | 0.1% | 53.4 | 8 | 18% |
| lowcache | 6 | 0 | 0% | 0.0% | 0.8% | 52.8 | 6 | 14% |

## Stability across repeats

Spread (max - min) of each metric over the runs used, as a share of the median. `~` marks a spread beyond max(10%, floor: Ms 1, Seconds 0.05, MiB 0.25, counts 2).

16 unstable metric cells of 288.

| recipe | path | profile | runs | frameP50Ms | frameP95Ms | frameP99Ms | hitches33 | firstRenderSeconds | fullQualitySeconds | requests | requestMiB | drawCallsP50 | trianglesP50 | gpuMemoryPeakMiB | lodPops |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| terrain-lod-sse0 | orbit-far | native | 3 | 0% | 2% | 1% | 0% | 77% ~ | 60% ~ | 0% | 0% | 0% | 0% | 0% | 0% |
| terrain-lod-sse0 | approach | native | 3 | 0% | 0% | 1% | 0% | 16% | 5% | 0% | 0% | 0% | 1% | 0% | 0% |
| terrain-lod-sse0 | fast-pan | native | 3 | 0% | 3% | 1% | 0% | 3% | 13% ~ | 0% | 0% | 2% | 2% | 0% | 0% |
| terrain-lod-sse0 | close-crawl | native | 3 | 0% | 3% | 1% | 0% | 5% | 13% | 0% | 2% | 0% | 0% | 2% | 0% |
| terrain-lod-sse0 | snap-teleport | native | 3 | 0% | 1% | 5% | 50% | 6% | 2% | 0% | 0% | 0% | 0% | 0% | 0% |
| terrain-skiplod-sse0 | orbit-far | native | 3 | 0% | 3% | 2% | 0% ~ | 6% | 1% | 0% | 0% | 0% | 0% | 0% | 0% |
| terrain-skiplod-sse0 | approach | native | 3 | 0% | 7% | 6% | 0% | 4% | 15% ~ | 0% | 0% | 2% | 1% | 0% | 0% |
| terrain-skiplod-sse0 | approach | cpu4x | 3 | 0% | 1% | 0% | 0% | 6% | 6% | 0% | 0% | 0% | 0% | 0% | 0% |
| terrain-skiplod-sse0 | approach | lowcache | 3 | 0% | 2% | 2% | 0% | 6% | n/a | 0% | 0% | 0% | 0% | 0% | 0% |
| terrain-skiplod-sse0 | fast-pan | native | 3 | 0% | 2% | 0% | 0% | 5% | 18% ~ | 0% | 0% | 2% | 0% | 0% | 0% |
| terrain-skiplod-sse0 | close-crawl | native | 3 | 0% | 4% | 2% | 0% | 238% ~ | 400% ~ | 0% | 0% | 0% | 2% | 0% | 0% |
| terrain-skiplod-sse0 | snap-teleport | native | 3 | 0% | 2% | 30% ~ | 0% | 4% | 5% | 0% | 0% | 0% | 0% | 0% | 0% |
| terrain-skiplod-sse0 | snap-teleport | cpu4x | 3 | 1% | 5% | 12% ~ | 6% | 6% | 4% | 0% | 0% | 72% ~ | 97% ~ | 0% | 0% |
| terrain-skiplod-sse0 | snap-teleport | lowcache | 3 | 0% | 2% | 2% | 100% ~ | 6% | n/a | 0% | 0% | 0% | 0% | 0% | 50% |
| vector-default | orbit-far | native | 3 | 0% | 2% | 2% | 0% | 11% | 8% | 0% | 0% | 0% | 0% | 0% | 0% |
| vector-default | approach | native | 3 | 0% | 3% | 2% | 0% | 15% | 7% | 0% | 0% | 0% | 0% | 0% | 0% |
| vector-default | fast-pan | native | 3 | 0% | 5% | 13% ~ | 0% | 2% | 2% | 0% | 0% | 0% | 0% | 0% | 0% |
| vector-default | close-crawl | native | 3 | 0% | 2% | 1% | 0% | 14% | 12% | 0% | 0% | 0% | 0% | 0% | 0% |
| vector-default | snap-teleport | native | 3 | 1% | 2% | 1% | 0% | 4% | 2% | 0% | 0% | 0% | 0% | 0% | 0% |
| vector-leaf16 | orbit-far | native | 3 | 0% | 2% | 2% | 0% | 10% | 10% | 0% | 0% | 0% | 0% | 0% | 0% |
| vector-leaf16 | approach | native | 3 | 0% | 1% | 2% | 0% | 5% | 4% | 0% | 0% | 9% | 0% | 0% | 0% |
| vector-leaf16 | fast-pan | native | 3 | 1% | 1% | 1% | 0% | 6% | 7% | 0% | 0% | 0% | 0% | 0% | 0% |
| vector-leaf16 | close-crawl | native | 3 | 0% | 2% | 1% | 0% | 4% | 2% | 0% | 0% | 0% | 0% | 0% | 0% |
| vector-leaf16 | snap-teleport | native | 3 | 0% | 5% | 32% ~ | 75% ~ | 2% | 5% | 0% | 0% | 0% | 0% | 0% | 0% |

## Failures

Cancelled tile requests (Cesium aborts requests for tiles that leave the view; normal while moving, not failures): 4 over 1 runs.

None.
