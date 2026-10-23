# Scale gate (a): plan for the production mesh run

2026-10-24; design only. **Gate (a) has never passed through `mesh`.** The
[roadmap milestone 4](04-tiles2-roadmap.md#4-accept-resources-fidelity-platforms-and-performance-retire-superseded-code)
and [budgets](../../bench/scale_gates/budgets.toml) remain unchanged. P0's
1,238 s / 1,228 MiB is an untextured prototype, not production acceptance.

Source inspected: `f97cd930fa36df0ca4213131f2e9d1026a5bcda0`;
post-compact-capture model: #254 `fbc84b7ef5cfcb3d02beffcf5102d693e9b7fd79`,
read from git objects without touching its checkout. #251, #254, #260 and #258
were OPEN when checked on 2026-10-24; #252 is already merged. Measurements below
refer to their recorded evidence revisions, not the latest PR heads.

## 1. Input and per-stage model

`generate.py mesh --preset production --large` produces **526,174,760 triangles,
2,008 cells, up to 6,024 primitives, four scalar materials, no images/textures
and no COLOR_0**. Attributes are POSITION/NORMAL/TANGENT/TEXCOORD_0 with u32
indices. Terrain is about 60%; the rest is subdivided buildings and rooftop
filler. It is not gate (b)'s deliberate 10,000 holes/islands per root: terrain
sheets are welded across cells, while buildings/filler introduce separate
components/open boundaries. Do not claim the gate-(b) topology obligation here.

The existing `mesh-1e7/manifest.json` is 394,625,028 bytes / 10,000,000 triangles:
39.4625 B/triangle. Extrapolation gives **20.76 GB (19.34 GiB)** input and
**83.06 GB (77.35 GiB)** at 4×. Use the generated manifest's actual bytes for
acceptance. The production file is glTF with approximately 1 GiB binary chunks.

### Scratch and resident ownership

Let N be source triangles, J source JSON bytes, W workers, A staged member bytes,
H LOD sidecars/handoffs, P live proxy faces, and D live seam/depth bookkeeping.
GB is decimal; GiB is binary. Scratch figures are **logical simultaneous spill
envelopes**, not measured physical allocation or sums of all writes.

Post-#254: full triangle = `106 + 3*(12+16+8) = 214 B`, full corner = 73 B.
The crop evidence's 130 B UV-only and 190 B NORMAL+COLOR layouts are different.
The source geometry estimate is `20 + 3*48 = 164 B/triangle`, not input bytes.
The node upper bound is 16,660,240, not a predicted tile count; at least 64,231
leaves are needed at 8,192 triangles/leaf. See the reproducible, allocation-free
[arithmetic model](../../bench/scale_gates/analysis/gate_a_model.py).

| Stage | Logical scratch model, B/source triangle; at 526M | RSS ownership/model at 526M |
| --- | --- | --- |
| Capture/accessor join | 220.0; 115.77 GB including its sorts | C(W,J): two bounded sorters, windows, metadata, decoder/planning reserves |
| Morton sort → retained sequence | 215.0; 113.14 GB | Same C; records spill, not N resident triangles |
| Tree/weld/corner join | tree 285.1; 150.00 GB; corner join 319.3; 168.01 GB | Same C plus depth stack; node streams remain on disk |
| Leaf construction | retained capture/plans 272.3; 143.28 GB, plus growing A and handoff files | `256J + 256 MiB + cache + page gate + W*leaf closure + ordered window`; host-sized today |
| LOD pre-pass | 363.3; **191.17 GB**, plus already-written A/handoffs | Bounded incidence/corner sort work plus retained metadata; no total-RSS proof |
| LOD simplify | retained sequences about 353.3; 185.90 GB, plus A+H and depth logs | `256J + O(W*P + D)`; per-tile face limits do not bound all depth maps |
| Seam resolution | Same retained base, plus seam sorts/forced-key logs and A+H | Sort buffers plus `done/keep/strict/latest/failed` maps: input/depth dependent |
| LOD finish | Same retained base, plus A+H; release consumed child sidecars | Worker certificate/GLB/sidecar closures plus D |
| Proxy bake/adapter | No texture pages; retained source/LOD and geometry packets still live | Frozen/child packets, source query index, optional resident source range; host-derived lease |
| Encode | Leaf/proxy GLBs included in A, not another independent copy; no image codec work | Geometry/meshopt/assembly within leaf and proxy closures; peak overlaps those stages |
| Forest/archive | Retained capture/LOD state can still coexist with A + new archive; index/spool extra | Streamed forest fragments; archive writer alone roughly 50–64 MiB, plus retained job state |

`C(W,J)` includes **256J**, two 80 MiB sort runs with merge buffers, 64 MiB
runtime, 192 MiB decoder and 128 MiB planning reserves, roughly 31.3 MiB/worker,
and descriptor inventory. Extrapolating the 10M GLB's 110,120-byte JSON to 2,008
cells gives about 5.7 MB JSON (format/offset growth unmeasured), hence about
1.36 GiB of metadata charge alone. C can exceed 2 GiB before leaves, despite
actual RSS being smaller. This needs a tighter justified bound, not deletion
of accounting. Compact on-disk records do not shrink Rust's resident structs.

#254 also trades occupancy for read traffic: three tree/weld/corner passes
read 214 B records instead of 93 B projections, adding `3*(214-93)*N` ≈191.0 GB
of sequential reads. It removes 48.9 GB of projection writes plus projection-sort
traffic. Calibrate the net on the target disk; lower spill is not a speedup claim.

The spill envelope adds 10% and file rounding: about **210.3 GB** before rounding.
Admission also estimates `3*119*N = 187.8 GB` for output copies: **398.1 GB plus
rounding** on one filesystem. Output estimates are not sound upper bounds.
For planning, historical 47–119 B/triangle gives A ≈25–63 GB; actual gate output
is unknown. The 191 GB pre-pass excludes A/H, so whole-run scratch is larger.
Btrfs compression may reduce sampled allocation; it cannot establish a logical
bound. `run_scratch_peak_bytes` omits sidecars/handoff stores; it is not the
runner's whole-directory scratch metric.

### Time model and its limits

No evidence supplies per-stage CPU time for this fixture. `phase_seconds` is
wall time; bake worker seconds are elapsed intervals summed over workers, not
CPU. The 2048-texture run used an **eight-CPU affinity slot with `--jobs 4`**:
3,612 s wall, 9,998 CPU-s, 9.37 GiB RSS, 3,046 s bake. Its non-bake phase sum is
555.4 s. The older H17 untextured 10M run was 396.5 s. Linear wall analogues are
therefore **20,863–29,223 s (5.8–8.1 h)** at gate scale, before any new adapter
cost; neither is a prediction of the generated terrain on an idle server.

The following is an explicit **sensitivity model**, not measured CPU rates.
Take the smaller/larger per-triangle wall rates of compact `crop3-final`
(788,425 triangles) and density `m2048/report-after.json`, and assume 1–4 active
CPU equivalents per phase. Capture's scan/sort/tree split is an unmeasured
25%/50%/25% allocation solely for scheduling; replace it with phase CPU counters.
The wide ranges deliberately expose missing evidence, especially texture-free
leaves and coarse-level work. `c` is CPU microseconds/source triangle;
`CPU_seconds = 526.175*c`. Sorts can add merge passes; LOD depends on topology,
retention and seam retries, so linear scaling must be checked at 100M.

| Stage | Assumed c, µCPU-s/triangle | CPU-s at 526M |
| --- | --- | --- |
| Capture scan/join | 0.86–11.30 | 453–5,946 |
| Sort | 1.71–22.60 | 900–11,892 |
| Tree/weld/corner join | 0.86–11.30 | 453–5,946 |
| Leaves, including leaf encode | 0.49–54.96 | 258–28,919 |
| LOD pre-pass | 1.02–5.82 | 537–3,062 |
| Simplify | 4.12–24.56 | 2,168–12,923 |
| Seams | 5.56–39.01 | 2,926–20,526 |
| Finish, including proxy geometry encode | 9.42–43.54 | 4,957–22,910 |
| Proxy image bake | **0 image texels**; adapter cost β unmeasured | `526.175*β`, not zero |
| Encode separately | Already included above; no defensible separate CPU attribution | Do not double-count |
| Archive | 0.17–9.08 | 89–4,778 |

Excluding β, this sensitivity range is roughly 13–117 thousand CPU-s:
0.9–8.1 h at four perfectly utilized workers, before I/O/serial penalties.
A 7,200 s finish needs **73,080 triangles/s**, or at most 13.68 µwall-s/triangle;
at four effective workers that allows 54.7 µCPU-s/triangle before overhead.

**Untextured does not mean the adapter is free.** `bake::raster_faces` uses
material bindings, so UVs/normals alone create no pages, decoded images or KTX2
work. But `pipeline` still invokes the baker; frozen packets, source residency,
frames, colour queries and assembly remain. `source::color` skips its search
only when a resident range has been established as colourless. At coarse levels
under a small lease the range cannot be resident, potentially restoring streamed
queries. Prove a source-attribute-based no-colour/no-texture fast path before
assuming β small. The COLOR crop's 148.8 s adapter time is not a gate-(a) estimate.
Do not multiply the textured 3,046 s by 52.6 and call the resulting 44.5 hours
this gate's bake cost.

## 2. Gap against every current budget

| Budget | Gap / binding uncertainty |
| --- | --- |
| Producer RSS ≤2 GiB | No declared total cap. Balanced leaf admission alone demands 3.031 GiB page space even without images, **1.031 GiB over**, before metadata/workers. Capture's JSON charge may refuse first. 9.37 GiB photogrammetry is 4.69× the target but is not gate RSS. |
| Producer wall ≤7,200 s | Historical geometry analogues 20.9–29.2 ks are **2.90–4.06×**, +13.7–22.0 ks; loaded-host effects and different topology prevent a pass/fail inference. Capture I/O, LOD/seams and coarse adapter scans are risks. |
| Scratch ≤4× input | ≈83.1 GB allowed versus 191.2 GB spill envelope: **2.30×**, +108.1 GB, before payloads/handoffs. Even retained full triangles alone are 112.6 GB. This predicts failure on uncompressed storage; physical compression and actual node counts must be measured. |
| Validate RSS ≤2 GiB | Unknown, not inherited from VS-A/B/C. Per-content decode/containment plus archive/catalog/traversal state are the risks; require independent final-output measurement. |
| Validate wall ≤3,600 s | Unknown. At least 64,231 leaves implies ≥17.8 leaf contents/s, plus proxies and hierarchy. For a planning 25–63 GB archive, ≥7–17 MB/s end-to-end plus geometry checks is required. |
| Cesium loaded ≤30 s | Unknown. Document/subtree request dependencies and first-view content bytes bind. #258 helps dependencies, not guaranteed latency. |
| Cesium refined ≤300 s | Unknown. If a full-domain view forces all leaves, ≥214 leaf contents/s and 1.75M triangles/s; a selected-view SSE-0 test need not load all 526M. Pin camera/domain; do not claim global refinement from that test. |

Bounded today: sort runs/fan-in/chunks, accessor windows, tree stack, per-tile
format/face limits, stage spill accounting, archive copy/index buffers and
forest fragments. Input-dependent: JSON, spill inventories, staged bytes,
LOD depth maps, sidecars and validator delivered-resource state. Host-dependent:
worker admission, leaf page/cache allowances, bake leases/source residency.
A declared lease is not a measured or allocator-enforced RSS bound.

## 3. Can a declared 2 GiB cap work today?

**No public Rust/CLI/Python total-memory knob exists.** `--jobs` is a maximum
worker count, not bytes. `config::available_memory` does observe the minimum of
Linux MemAvailable and ancestor cgroup-v2 headroom (`memory.max-memory.current`).
The runner can set a 2 GiB `MemoryMax`, but this includes page cache, can cause
an OOM kill, and does not promise successful conversion or a typed refusal.
Balanced's unused page reservation is `4096²*182 + 192 MiB = 3.03125 GiB`;
quality is the same; image-free lossless reserves 2.5625 GiB
(`8192²*40 + 64 MiB`), also too large. Fast lowers that reservation to about
0.898 GiB, but changes the profile and does not establish a whole-job cap.

Bake allowance is half available memory after metadata, tmpfs and a 256 MiB
reserve (1 GiB fallback when unavailable). Workers share it; current source
retries refused tiles alone, #251 introduces graduated retries. An item still
unable to fit returns `resource_limit`; memory exhaustion does **not** authorize
coarser density or weaker fidelity. Density retries follow complete-member
physical-size feedback only. #251 bounds child texel access, not process RSS.

Required contract: resolve one public byte budget before work; charge retained
metadata, allocator/runtime reserve, sort/worker/cache/page/codec/LOD/bake/
archive ownership against it; admit actual page needs (zero for this input),
spill or serialize otherwise, and refuse an indivisible item before allocating.
Report a predeclared bound with stage peaks and the refusing owner. Keep fidelity,
provenance and certified geometry invariant under worker/cache changes. Test
small-cap refusal, cleanup, and successful 2 GiB completion separately from the
cgroup kill backstop. Audit JSON amplification and depth maps explicitly.

## 4. Ranked implementation work

Sizes are engineering estimates including focused acceptance: S 1–3 days,
M 3–7 days, L 1–3 weeks; not commitments. Retain streaming sorts/tree/archive
subject to their existing domain; rework resource ownership and retained
representations; replace unbounded depth inventories where necessary. Do not
remove certificates or lower fidelity to obtain a performance pass.

1. **Declared memory policy and untextured admission (L; small first fix S).**
   Remove the unused 3.031 GiB leaf floor through actual-need admission; tighten
   justified metadata charges and lease LOD/bake/global state. This enables a
   meaningful 2 GiB proving run; it does not prove it will succeed. Add stage
   CPU/RSS counters and a fail-closed whole-run scratch ledger (M).
2. **H17/capture lifetime and indexed attribute storage (L).** Land #254 first:
   post-#252 envelope about 484 →363 B/triangle for this fixture, ≈64 GB less.
   The full 214 B retained record alone exceeds 4× input. To keep 4×, geometry
   must fall by at least 57%, and more after output staging. Share indexed
   vertex attributes, avoid retaining full attributes beside 81 B LodSource,
   and release/spool by consuming stage; model random-read cost and preserve
   source identity. A scratch-budget revision is more defensible than claiming
   #254 alone solves this.
3. **Measure and eliminate unnecessary coarse adapter work (S–M), then tune
   capture/LOD from phase CPU and I/O (M–L).** Decide no-texture/no-colour from
   admitted source flags independent of source-range residency; retain frames,
   material factors, provenance and mandatory assembly audits. Gate target:
   β≤4 µCPU-s/triangle (≤2,105 CPU-s), a proposed allocation to verify, not a
   measured speedup. Reaching 7,200 s needs roughly 3–4× improvement over loaded
   geometry wall analogues or demonstrated equivalent idle-host throughput.
4. **Finish #258 forest layout (in flight, M acceptance).** 2048 first-view
   simulation: 102→87 requests, dependency depth 5→3; crop executions 22→15,
   depth 3→2 (simulated requests on freshly produced crop artifacts).
   GLBs unchanged. Evidence: #258 `e2c1f7d`,
   `bench/architecture_audit/implicit_forest_consolidation/README.md` at that
   revision; bakes `41af7f6e`/`2e149d92`, crop3 branch archive SHA-256
   `d9a978bff2a240aa9c3e9854cec5bdb0614dad0414b57ac143868ec8a20f1b20`.
   Its 2048 calibration uses `load-profile-20261023/m2048-cold.json`.
   Measure gate camera bytes/load/refinement on GPU;
   no 30/300-second guarantee follows from request counts.
5. **Finish #251 bounded bake and #260 density (in flight; separate textured
   acceptance, M).** #251 tex3 paired CPU ratio 0.630, scratch 8.49→3.80 GB;
   COLOR crop ratio **1.221 FAIL** at the measured revision. #260's loaded
   2048 run: bake 5,919→3,046 s, RSS 13.1→9.4 GiB. These address gate (c) and
   photogrammetry, not gate-(a) page cost. Neither is a 2 GiB cap; remeasure
   current combined heads and the no-raster path rather than adding speedups.
6. **Independent validator/consumer scale acceptance (M plus runs).** Keep
   VS-A/B/C; do not reuse the harness README's obsolete 2026-10-13 validator
   refusal as current behavior. VS-D, all profiles and final artifacts remain
   obligations even after a balanced gate run succeeds.

## 5. Run plan, hardware and stop conditions

No conversion, generation, build or browser was run for this plan. Use an idle
Linux server with **8 dedicated physical cores initially, 32 GiB RAM (64 GiB
preferred for diagnostic headroom), local NVMe and ≥600 GB genuinely free in the
shared scratch/output pool**. Memory capacity is not permission for the tiler to
use it: retain independent 2 GiB acceptance. Require sustained ≥0.5 GB/s mixed
sequential spill throughput, adequate filesystem metadata performance and no
competing heavy writer. The 20-thread loaded desktop is unsuitable for wall
acceptance; if used for diagnostics, `chrt -i 0 nice -n 19`, two workers and no
performance claim. RAM-backed scratch is unsuitable.

Consumer acceptance needs a separate quiet GPU workstation (the evidenced RTX
4080 SUPER-class setup, pinned Chromium/Cesium), fixed viewport/camera/SSE,
server path, bandwidth and cold/warm cache conditions. The current harness
serves extracted files; it does not measure native 3TZ range-request delivery.
An idle headless server does not replace that proof. GPU memory and full-domain camera
feasibility must be established on the 100M output. **Harness prerequisite:**
`bench/perf_lane/cesium.py::mount` hashes `path.read_bytes()`, allocating the
entire archive (planning 25–63 GB). Replace this with streaming hashing before
the scale consumer run, bound member extraction, and reserve another archive-sized
amount of disk for extracted members. More server RAM is not the fix.

1. Pin source SHA, binary SHA, compiler/features/profile and PR combination;
   audit the existing `mesh-1e5`, `mesh-1e6`, `mesh-1e7` manifests and hashes
   without regenerating them. Smoke 1e5/1e6 first. Use balanced, jobs 2 then 4;
   record requested and actual workers. Repeat only after changed code/failures
   or to establish idle-host variance. Test caps at small scale first.
2. Run **1e7 → 1e8 → 526,174,760**, same cell/seed/attribute recipe. Generate
   only the missing inputs, on the measurement server:

   ```sh
   python3 -I bench/scale_gates/generate.py mesh --triangles 100000000 --large --out inputs/mesh-1e8
   python3 -I bench/scale_gates/generate.py mesh --preset production --large --out inputs/mesh-526m
   ```

   Allow about 2–5 min / 10–20 min generation respectively (README's production
   estimate is ~9 min); verify exact scale, textures=false, attributes and hashes.
   Do not increase cell size to hide JSON growth without declaring a new fixture.
3. Diagnostic execution uses the existing runner; set an absolute pinned BIN
   and an idle server-specific CPU list rather than its desktop default pool:

   ```sh
   python3 -I bench/scale_gates/run_case.py --case gate-a-1e7 --budget gate-a \
     --binary /absolute/pinned/rusty-tiles --input "$HOME/.cache/rusty-tiles-scale/inputs/mesh-1e7" \
     --jobs 4 --cpus 0-3 --memory cgroup --keep-output --hash-limit-gib 128 --timeout 7200 \
     --command '{binary} --json mesh -i {input} -o {output} --profile balanced --jobs {jobs} --scratch-dir {scratch} --progress json'
   ```

   Repeat with new case/input names at each scale. Add `--enforce-memory` only
   for the explicit cap proof after admission fixes; verify cgroup setup did
   not fall back to wait4. The current flag caps only the producer; validator
   has neither memory_max nor timeout passed by the runner. Supply a separate
   measured validator wrapper/cgroup/watchdog for 2 GiB and 3,600 s. With future
   `producer:` RSS budgets, enforcement also needs a pre-run literal cap because
   the runner currently cannot resolve producer references before execution.
4. Record stage process CPU deltas, wall and RSS samples alongside lifetime
   wait4 RSS; page/cache/worker leases; logical and allocated scratch per owner
   (capture, leaves, LOD, handoff, archive), total bytes read/written, disk queue,
   CPU utilization/load, retries, proxy faces/depths, output bytes/members and
   cleanup. The runner defaults to hashing only ≤8 GiB: the example raises this
   to 128 GiB; if exceeded, explicitly stream-hash separately. Samples are lower
   bounds: use ownership high-water accounting as the independent cross-check. Cgroup memory.peak includes page cache and is
   a separate metric. Record archive hash and source/generator identities.
5. At 100M require no near-limit resource trend, no typed refusal, stable
   CPU/triangle after merge-pass adjustments, a calibrated forecast with ≥20%
   timing margin, and physical free space exceeding declared admission plus
   output/handoff requirements. For the unchanged 7,200 s target, require
   roughly ≤1,095 s at 100M for that 20% margin; investigate superlinear seam or
   adapter work before escalating. Do not launch 526M merely because 100M ends.
6. Validate and independently audit retained output before the browser pass.
   For later consumer work, **omit `--command`**, pass the retained archive
   path as `--input`, and use `--cesium --keep-output --hash-limit-gib 128`.
   This avoids reconversion and preserves the exact artifact and camera record.
   That supplemental report lacks the original producer/scale manifest and
   cannot itself pass `gate-a`: join receipts by archive SHA-256, retaining the
   original input manifest and producer report. Final acceptance needs a
   complete joined record (or one combined run after the harness fixes).
   Check every required metric: the harness permits skipped
   validate/Cesium checks, so a reported PASS without those stages is not gate
   acceptance. Smaller cases remain BELOW-SCALE when all checks pass. No CI
   polling is part of these runs.

Reserve an **8–12-hour idle slot** for the first successful final-scale candidate:
conversion planning range 2–8 h, validator ≤1 h target, generation and consumer
setup/inspection. At 10M allow 5–15 min conversion, at 100M 40–90 min on the
unoptimized path; these are reservations, not pass budgets. The pass trajectory
is ≤137 s / ≤1,368 s / ≤7,200 s for 10M / 100M / 526M before adding margin.
If measurements do not support it, stop and optimize or explicitly revise the
provisional wall budget; do not run a loaded desktop for days to discover that.

## 6. Recommended budget revisions

Do not silently edit `gate-a` or describe a revised-budget pass as satisfying the
old values. Publish the resolved pre-run contract and hardware tier first.

- **RSS:** retain 2 GiB as the target; after whole-job accounting exists, compare
  RSS with proposed `producer:diagnostics.resources.memory.bound_bytes`, and
  separately require that declared bound ≤2,147,483,648. It must come from an
  explicit requested cap, never measured peak or host free memory. Stage-only
  `capture.memory_bound_bytes`/`leaf_delivery.memory_bound_bytes` are insufficient.
  Until then, use a separate 4–8 GiB diagnostic lane, never a gate-(a) pass.
- **Scratch:** replace the arbitrary input multiplier with proposed
  `producer:diagnostics.resources.scratch.bound_bytes`: max concurrent owned
  spill + sidecars + handoffs + leaf/forest/archive staging, filesystem rounding
  and specified margin. Report logical and physical peaks separately. For the
  post-#254 architecture, **400 GB** is a provisional engineering envelope to
  test, not a proven bound; 600 GB free is hardware provisioning. The current
  ~398 GB admission estimate excludes a sound sidecar/output bound. Freeze a
  real value only after owner accounting and runs, or retain 4× and fund the
  larger indexed/lifetime redesign. Do not point this at capture-only scratch.
- **Producer wall:** keep 7,200 s as an aspirational fixed benchmark on declared
  idle hardware. A separate diagnostic timeout up to 28,800 s is reasonable
  after extrapolation, but is not a revised pass. Time is not a sound
  `producer:` resource bound; decide any new SLO from 10M/100M measured CPU/I/O.
- **Validate:** retain 2 GiB / 3,600 s pending VS-D. A future independently
  justified `validate:` memory bound may complement the hard ceiling; do not
  accept a producer's claim about its own validator's resource use. Keep time
  fixed to hardware and delivered-resource counts, with no automatic relaxation.
- **Cesium:** retain 30 s loaded / 300 s refined for a pinned camera/route,
  include network/GPU prerequisites and define what SSE-0 loaded means. These
  remain externally measured SLOs, never producer-declared bounds. A separate
  full-domain stress run must not be substituted for, or confused with, the
  existing selected-view obligation.

## Evidence and proof limits

Read the six evidence READMEs under `~/.cache/rusty-tiles-evidence/`:
`bake-bounded-20261022`, `proxy-density-20261023`, `lod-slim-20261023`,
`capture-compact-20261023`, `perf-gate-20261023`, `proxy-donor-20261022`.
The phase/CPU extraction additionally uses density `m2048/report-after.json`
and `measure-after.json`, and compact `crop3-final/report.json` and
`measure.json`. Their READMEs bind input/binary revisions and artifact hashes.
The 118 GB bake spill is #251's pre-density 2048 run; it is not a measured
post-#260 peak. Timing is loaded-host evidence, and paired CPU evidence applies
only to its stated crops/revisions.

Source owners: `mesh_large/{config,full,attributes,pipeline}.rs`,
`leaves/{producer,schedule}.rs`, `lod/{mod,incidence}.rs`,
`bake/{integration,mod,source,raster}.rs`, `delivery/{plan,archive}.rs`.
Harness semantics were checked in `generate.py`, `run_case.py`, `launch.py` and
its README. The arithmetic script is a planning model, not an independent
acceptance oracle. No current production, validator or consumer scale pass,
profile freeze, #113 closure or release authorization follows from this plan.
