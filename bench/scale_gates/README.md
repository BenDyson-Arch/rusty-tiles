# Milestone-4 scale-gate harness

Bench-only tooling for the pass/fail scale obligations of
[roadmap milestone 4](../../docs/architecture/04-tiles2-roadmap.md#4-accept-resources-fidelity-platforms-and-performance-retire-superseded-code).
It does not need the large-mesh tiler: the generators, synthetic archives and runner are
ready for whichever binary and command line the tiler ends up with.

| Gate | Obligation | Input | Budget set |
| --- | --- | --- | --- |
| (a) | spilled 5.26×10^8-triangle production octree | `generate.py mesh --preset production --large` | `gate-a` |
| (b) | 10^4 primitives and 10^4 holes/islands per root | `generate.py density` | `gate-b` |
| (c) | procedural-texture bake | `generate.py texture --texture-size 1024` | `gate-c` |
| (d) | ONE archive: ≥ 2^22 available tiles AND > 4 GiB | `gen_archive.py combined` | `gate-d-combined` |

Each run records peak RSS, scratch, wall time, output size and tile count, a
`rusty-tiles validate` run with its own time and RSS, and optionally Cesium load and
SSE-0 refinement, and compares them with declared budgets in [`budgets.toml`](budgets.toml).

Files: `generate.py` (input generators), `gen_archive.py` (synthetic 3TZ archives),
`run_case.py` (runner), `launch.py` (measurement launcher), `collect.py` (report table),
`budgets.toml`, `validation_pass.sh` (this pass's runs), `scalecommon.py` (shared writer,
PRNG, disk checks) and `test_scale_gates.py` (self-tests).

## Data location and disk

Generated data and run directories go under `~/.cache/rusty-tiles-scale/`
(`RUSTY_TILES_SCALE_ROOT`); relative `--out`/`--work` paths resolve there. The scripts refuse
`/tmp`, `/dev/shm`, `/run`, `/var/tmp` and the git tree. Every generator checks free space
before writing anything. It needs the estimated size plus `--reserve-gib` (default 20 GiB).
`--force` replaces only an earlier generator output (one with `manifest.json`) strictly inside
that root. Mesh inputs above `--large-threshold` (5×10^7 triangles) need `--large`. The production
case is 526,174,760 triangles (the P0 grid count), about 20 GiB as glTF + 1 GiB `.bin`
files. `/home` is btrfs with zstd compression, so on-disk use is lower than logical size.

## Generators (`generate.py`)

All generators stream. Generation runs one cell (`--cell-triangles`, default 2^18) or one
region at a time. Only accessor metadata stays in memory, and the self-test checks that
generator memory does not grow with N. Output is GLB below 3 GiB and glTF with rolled-over
`.bin` files (`--max-buffer-mib`) above that, or as chosen with `--format`. GLB assembly
streams a part file after the JSON chunk. Each output directory has a `manifest.json`
holding the parameters, environment, generator and shared-module sha256, every file's size
and sha256, the disk preflight and a `truth` record.

Bytes depend only on the seed and writer version. Geometry and textures use integer-hash
value noise with polynomial interpolation and IEEE-exact NumPy arithmetic. Rotations use
rational parameterisation, and bell-shaped offsets use Irwin–Hall sums, so no libm
trigonometry is involved. PNGs use stored deflate blocks unless `--compress-png` is given;
zlib-compressed bytes depend on the zlib build.

- **`mesh`** (a). This is a grid of terrain cells. Each cell is one node with a terrain sheet
  (fractal height field with analytic normals, sheets welded across cells) and clustered
  buildings. Facades are subdivided into floor/bay quads, and each building has a roof. There
  are four materials (grass, rock, facade, roof) and up to three primitives per cell.
  Attributes are POSITION/NORMAL/TANGENT/TEXCOORD_0, with uint32 indices and exact accessor
  min/max. Triangles are wound counter-clockwise about their normals, and tangent `w` follows
  dP/dv. Rooftop filler triangles make the total exactly `--triangles`. The truth record holds
  the terrain/building/filler counts, building count and bounds.
- **`density`** (b). Each region (`--regions`, one node per root) is a gently undulating sheet
  of unit cells with `--holes` interior one-cell holes on a 2-cell lattice. No hole touches
  another hole or the border, so each is one interior boundary loop. There are `--islands`
  small quads: the first sit coplanar inside the holes with a ring gap, and the rest float 2 m
  above the sheet. The islands are spread round-robin over `--primitives − 1` primitives (16
  materials), with the sheet as primitive 0. Each attribute shares one strided bufferView per
  region. The defaults are 10^4 / 10^4 / 10^4.
- **`texture`** (c). This is the `mesh` geometry with a baseColor (sRGB colour), normal
  (linear, unit tangent-space, blue-dominant) and metallicRoughness (linear; R=0, G=roughness
  noise, B=metallic panels on facade/roof) texture per material. All 12 images are distinct,
  each slot uses a different pattern, and they tile under the REPEAT sampler. The truth record
  stores each image's sha256, colour class, pattern, channel means and 16 probe texels.

## Synthetic archives (`gen_archive.py`)

`combined` is the single-archive gate (d) case. Defaults use QUADTREE levels12,
subtreeLevels6 and contentLevels4: exactly 5,592,405 available tiles, 4,097
subtrees and 85 triangle GLBs padded to 56 MiB each. The contents alone total
4,991,221,760 bytes; ZIP headers, subtrees and index add to this. Padding is sparse
zero-filled BIN space, with valid CRCs; logical bytes and allocated bytes are
reported separately. All contents are referenced and each is below 128 MiB.
The inherited validator/browser budget remains provisional, not an executed pass.

```sh
python3 -I bench/scale_gates/gen_archive.py combined --out archives/combined-d
python3 -I bench/scale_gates/run_case.py --case combined-d --budget gate-d-combined \
  --binary target/release/rusty-tiles --input ~/.cache/rusty-tiles-scale/archives/combined-d
```

Manifest scale checks conjunctively require `truth.tiles >= 4194304` and
`truth.archive_bytes >= 4294967297` on this same input. Small overrides remain
BELOW-SCALE. The separate `implicit` and `large` cases are diagnostic controls,
not joint satisfaction of gate (d). Combined manifests always stream-hash the
actual archive, including sparse zero padding, and report its logical byte size.

The archives are stored ZIPs. ZIP64 records are written only where a field overflows. Every
member is referenced. The `@3dtilesIndex1@` index is the last central entry: MD5 of the name
plus the u64 offset, sorted by the two little-endian u64 halves.

- **`implicit`**: a QUADTREE tileset with `availableLevels` 12 and `subtreeLevels` 6, using
  constant availability. It has 5,592,405 tiles (≥ 2^22) in 4,097 subtree members, and the
  archive is about 1.2 MB. Content (one in-box triangle GLB per tile) is on levels below
  `--content-levels` (default: the root only). There is an ENU root transform at
  `--anchor` (`--no-anchor` for none).
- **`large`**: an explicit tileset with ⌈target / member⌉ child contents of `--member-mib`
  (default 120 MiB, under the validator's 128 MiB member limit). Each child is a valid
  triangle GLB whose BIN chunk is padded with unreferenced bytes. The default target of
  4.5 GiB gives 39 contents and a 4,907,348,173-byte archive with ZIP64 offsets. With
  `--fill zeros` the padding is file holes (229 KB allocated, generated in about 3 s);
  `--fill pattern` writes deterministic dense bytes.

## Runner (`run_case.py`)

```sh
python3 -I bench/scale_gates/run_case.py --case NAME --budget SET --binary BIN \
  --input ~/.cache/rusty-tiles-scale/inputs/DIR \
  --command '{binary} --json TILER-SUBCOMMAND -i {input} -o {output} ...' \
  [--output-name out.3tz] [--env KEY=VALUE] [--producer-report '{output}/report.json'] \
  [--cesium [--cesium-arg=--cdp-url=...]] [--memory cgroup|wait4] [--enforce-memory] [--timeout S]
```

- The command is a template, because the tiler CLI is not final. The placeholders are
  `{binary} {input} {input_dir} {output} {output_dir} {scratch} {work} {jobs} {cpus}`.
  `{input}` is the manifest's `truth.input_file`. Without `--command` the case is
  validate-only and the input is the archive under test.
- The producer gets `PATH=/usr/bin:/bin`, `RAYON_NUM_THREADS`/`RUST_TEST_THREADS` = `--jobs`
  (default 2), `TMPDIR={scratch}` and `PROJ_NETWORK=OFF`.
- **CPUs.** Without `--cpus`, the runner picks the `--jobs` idlest CPUs of `--cpu-pool`
  (default `4-7,12-15`) that are each at least 90% idle over a 3 s `/proc/stat` sample. It
  waits up to `--idle-wait` for them. Commands run under `taskset` at `nice -n 10`.
- **Peak RSS.** `launch.py` forks and execs the command and records its `wait4` `ru_maxrss`.
  This is the largest single process; children are not summed. Linux carries the launcher's
  resident high-water mark across exec, so each value includes a floor of about 6.5 MiB. Each
  run measures that floor with `/bin/true` and records it as `rss_floor_bytes`. Launching
  straight from the Python runner would put the runner's ~35–50 MiB under every number.
- **cgroup** (`--memory cgroup`, the default; falls back to `wait4`). The command runs in a
  transient `systemd-run --user --scope`. The scope's exact `memory.peak` covers the whole
  process tree including page cache. `memory.stat` anon is sampled.
  `--enforce-memory` sets `MemoryMax` to the resolved `max.peak_rss_bytes`.
- **Scratch.** The runner samples allocated bytes (`st_blocks`) under `{scratch}`, plus staging
  entries in `{output_dir}` other than the final output, every `--sample-seconds` (0.2 s). A
  periodic sample is a lower bound, not an atomic peak; sub-second producers can show 0. The
  free-space drop of the filesystem is a coarse cross-check that other writers pollute.
  Leftover scratch entries are listed.
- **Timeout.** With `--timeout`, the runner kills the command; the launcher still reports.
  A timed-out run always FAILs.
- **Output.** The runner records bytes, sha256 (up to `--hash-limit-gib`) and zip members,
  plus `output_exists`, so refusal budgets can require that nothing was published.
  The output is deleted after the run unless `--keep-output` is given.
- **Validate.** `{binary} --json validate {output}` (`--validate-command`) runs with the same
  measurement. `tiles` and `contentReferences` come from its report, and a refusal records its
  error code and message.
- **Cesium.** `--cesium` runs `bench/perf_lane/cesium.py` against the user's GPU Chromium
  over CDP and records `secondsToLoaded`, the SSE-0 refined pass, requests and problems.
  cesium.py takes `~/.cache/rusty-tiles-chromium.lock` itself around each browser case, the
  same lock as `flock ~/.cache/rusty-tiles-chromium.lock`. Do not wrap the runner in another
  `flock` on that file: a second open file description would wait on the first and
  self-deadlock.

Budgets (`budgets.toml`) use `max`, `min`, `expect` and `scale` tables, with `extends`
inheritance. A value is either a literal or a reference of the form
`producer:|manifest:|validate:<dotted.path>[*factor]`. For example, the `h1-capture` set
compares RSS and scratch against the producer's own declared bounds. `scale` checks the input
manifest against the obligation's size. The result is one of:

- `PASS`: every budget holds and the input is at gate scale.
- `BELOW-SCALE`: every budget holds on a smaller input.
- `FAIL`: a budget failed, or a required metric is missing.

Stages that did not run (Cesium without `--cesium`, validate without an output file) are
`skipped`. Each run writes `report.json` and `SUMMARY.md` under `--work`.
`collect.py` tabulates several reports.

The gate (a)–(d) budgets are **provisional** planning values. They come from the P0 prototype
(`mlarge-p0-20261012`: 526,174,760-triangle grid, balanced 1,238 s and 1,228 MiB high-water)
and the H1 run. The roadmap freezes constants only after the final checks pass. Replace them
with the tiler's declared configuration bounds (`producer:` references) once its report names
them.

## Self-tests

```sh
python3 -I bench/scale_gates/test_scale_gates.py
```

These 15 tests run in about 4 s. The GLB/glTF, PNG and ZIP checks use their own decoders and
Python's `zipfile`, not the writers.

- **Mesh.** Exact triangle counts, attribute lengths, accessor min/max, unit orthogonal
  tangent frames, and winding agreeing with the normals. Seeded determinism, `.bin` rollover,
  and generator memory that stays bounded as N grows.
- **Refusals.** The `--large` flag and the disk preflight refuse as expected, and the refused
  output roots are rejected.
- **Density.** Boundary-loop counts (1 + holes) and island components per region.
- **Textures.** Texture classes and distinctness, and seamless wrap.
- **Archives.**
  - Implicit: subtree walk and tile count, index order and offsets, and in-box content.
  - Large: sparse holes and CRCs; a sparse > 4 GiB ZIP64 archive read back through `zipfile`.
- **Runner.** Budget resolution and inheritance; skipped/missing/unresolved statuses; RSS,
  launcher floor and scratch sampling; exit codes; process-group timeout.

## Running the gates

```sh
G=bench/scale_gates
python3 -I $G/generate.py mesh --preset production --large --out inputs/mesh-526m   # ~20 GiB, ~9 min
python3 -I $G/generate.py density --out inputs/density-1e4
python3 -I $G/generate.py texture --triangles 1000000 --texture-size 1024 --out inputs/texture-1e6
python3 -I $G/gen_archive.py implicit --out archives/implicit-2p22
python3 -I $G/gen_archive.py large --out archives/large-4p5g
python3 -I $G/run_case.py --case gate-a --budget gate-a --binary BIN --input ~/.cache/rusty-tiles-scale/inputs/mesh-526m \
  --command '{binary} --json TILER ... -i {input} -o {output} ...' --cesium
python3 -I $G/run_case.py --case gate-d-implicit --budget gate-d-implicit --binary BIN \
  --input ~/.cache/rusty-tiles-scale/archives/implicit-2p22
```

## Harness validation pass, 2026-10-13

Source `develop` at `87b7078a`. The release CLI has sha256 `a48a929c…4b43c849`. The lib test
binary (H1 `mesh_large::tests::external_driver`) has sha256 `1016cc4e…20683dab`.
`validation_pass.sh` reproduces the runs. They used CPUs from 4–7/12–15 and cgroup memory mode.

| case | status | input | wall s | peak RSS MiB | cgroup peak MiB | scratch peak MiB | output MiB | tiles | validate s | validate RSS MiB | Cesium load / SSE0 s | note |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| density-1e4-mesh-local | FAIL | 80,802 tris | 0.03 | 17.1 | 14.8 | 0.0 | - | - | - | - | - | unsupported: GLB JSON exceeds 1 MiB F1a ceiling |
| gate-d-implicit | FAIL | 5,592,405 tiles | - | - | - | - | 1.1 | - | 0.16 | 158.4 | - | resource_limit: implicit: subtree tile availability exceeds remaining hierarchy visits |
| gate-d-large | FAIL | 40 tiles | - | - | - | - | 4,680.0 | - | 0.03 | 7.8 | - | resource_limit: source archive exceeds byte limit |
| h1-capture-1e6 | PASS | 1,000,000 tris | 1.97 | 67.6 | 248.1 | 163.1 | 95.1 | - | - | - | - |  |
| h1-capture-1e7 | PASS | 10,000,000 tris | 24.60 | 69.2 | 1,979.9 | 1,888.5 | 951.0 | - | - | - | - |  |
| h1-capture-density-1e4 | PASS | 80,802 tris | 0.55 | 104.0 | 97.1 | 0.0 | 8.2 | - | - | - | - |  |
| mesh-local-1e5 | PASS | 100,000 tris | 0.16 | 43.7 | 73.7 | 0.0 | 17.6 | 9 | 0.26 | 67.9 | 0.33 / 0.25 |  |
| mesh-local-1e6 | PASS | 1,000,000 tris | 0.03 | 7.9 | 5.7 | 0.0 | - | - | - | - | - | unsupported: mesh source exceeds 32 MiB file ceiling |
| mesh-local-1e7 | PASS | 10,000,000 tris | 0.03 | 8.1 | 5.8 | 0.0 | - | - | - | - | - | unsupported: mesh source exceeds 32 MiB file ceiling |
| texture-1e5-mesh-local | PASS | 100,000 tris | 0.20 | 62.8 | 99.3 | 0.0 | 25.7 | 9 | 0.28 | 70.5 | 0.43 / 0.37 |  |

Cesium figures are seconds to loaded at SSE 16 and in the fully refined SSE-0 pass. The final
pass used a private headless GPU Chromium (`CESIUM_ARG=--launch`, RTX 4080 SUPER through ANGLE),
because the CDP browser on :9222 had closed. An earlier run of the same 10^5 cases over CDP
loaded in 0.30 / 0.26 s with no problems. H1 declared bounds at 10^7 triangles were 242 MiB of
memory (69.2 MiB measured) and 14,106 MiB of scratch (1,888 MiB sampled; 1,917 MiB logical peak
reported by the producer). The tree had 1,293 nodes. Sub-second producers show 0 scratch
because the 0.2 s sampler missed their staging.

Findings:

- **`mesh-local-to-3tz` cannot run 10^6–10^7.** It refuses typed and early, publishing
  nothing, at its F1a ceilings: a 32 MiB source file (10^6 and 10^7 triangles), 1 MiB of GLB
  JSON (the 10^4-primitive density scene) and 32 MiB of owned image bytes. The stored-PNG
  1024² texture set is 36 MiB; `--compress-png` brings the case to 13.9 MB, which it accepts.
  Its 100,000-triangle limit applies below that. Its 10^5 runs pass converter, validator and
  Cesium budgets. The 10^6/10^7 runs record the typed refusal against the
  `mesh-local-ceiling` budget.
- **The streamed H1 capture/sort/tree stage covers 10^6–10^7.** The same runner drove it
  through a different command template and `--env` placeholders. On the 10^7 input it stayed
  well inside its own declared memory and scratch bounds (`producer:` budgets), and it also
  accepted the density scene. H1 writes diagnostics, not tiles, so validate is skipped.
- **The current validator refuses both (d) archives.** It refuses with `resource_limit` and
  exit 3 under its fixed default limits, which have no CLI override. The implicit archive
  needs about 5.59 M hierarchy visits against the default 2^18. The large archive's
  4,907,348,173 bytes exceed the 1 GiB source/stored limits, and the 2 GiB total-read limit
  applies too. The archives pass the validator at small size (self-check: 85-tile implicit,
  12-tile padded explicit). Closing (d) needs validator work and memory that scale with the
  delivered-resource domain, as the roadmap requires.

Evidence (reports, logs, Cesium JSON, input manifests) is in
`~/.cache/rusty-tiles-evidence/scale-gates-harness-20261013/`. `INDEX.json` there has sha256
`09837e17e0538dd47e297114c6460b3637ba00303c3c5aafd892da8ab0494864`. Generated inputs stay under `~/.cache/rusty-tiles-scale/`.

Proof limits:

- These runs validate the harness, not any gate. No case reached gate (a) scale, and
  mesh-local's 10^5 runs and H1's diagnostics are not the production tiler.
- Scratch samples are lower bounds. RSS is single-process, and the cgroup peak includes page
  cache.
- Cesium ran only on the 10^5 mesh-local outputs. The (d) archives were not loaded, because
  the validator refuses them and a 5.6 M-tile SSE-0 traversal in the shared browser needs its
  own bounded plan.
- Generator RAM is bounded per cell or region for geometry. The JSON metadata (accessors,
  bufferViews, zip directory rows) still grows with the total number of primitives and
  subtrees: about 40k accessor records per 10^4-primitive region.
- Byte determinism is per writer version and NumPy integer/IEEE semantics. Only same-machine
  regeneration is tested.

## H18 combined fixture execution

The default combined fixture was generated and independently inspected on
2026-10-13: 5,592,405 available tiles, 4,992,428,223 logical bytes, 4,097
subtrees, 85 contents and 4,184 ZIP members. Sparse physical allocation is
1,597,440 bytes. The independent reader checks every subtree, all content
bounds and CRCs, strict index associations and ZIP64 offsets. All 16 harness
self-tests pass, including controls missing either scale requirement. The retained
`check_combined_archive.py --input <generator-directory> --report-json <receipt>`
reader reruns the default full-archive check without trusting manifest counts.

[combined-acceptance.json](combined-acceptance.json) binds exact source, archive,
execution and independent review identities. The archive and bulk evidence are
retained under `~/.cache/rusty-tiles-evidence/h18-combined-scale-20261013/`.

The current validator returns exit 3, `resource_limit: source archive exceeds
byte limit`, before traversal. Both scale predicates pass, but harness validation
fails. H18 supplies the requested combined fixture; roadmap gate (d) remains
unmet until successful validator/resource and consumer acceptance. No production
limits were raised and no browser acceptance is claimed.
