# F1a executable acceptance evidence

The bounded contract is [f1a-contract.md](../../../docs/architecture/f1a-contract.md). The independent oracle and viewer procedures are described in [tests/f1a_oracle.md](../../../tests/f1a_oracle.md). This directory holds their machine-readable observations and the separate resource sampler; it does not turn sampled results into general correctness or memory proofs.

## Runs and provenance

`implementation-results.json` records final local verification of production
`d444d82`: 325 portable and 350 native tests passed (8/9 existing ignored),
workspace/all-target clippy with warnings denied, formatting and source packaging.
`installed-wheel.json` records 27 tests with no failures/errors/skips in an isolated
Python3.14 environment with an empty PATH. It identifies the rebuilt local abi3
wheel; this is not a manylinux release artifact. Platform CI remains on PR128.

`oracle.json` contains the analytical self-tests, fifteen rejected corruption controls and 33 successful CLI conversions: eleven source variants at leaf limits 1, 3 and 1000. The standard-library oracle reads actual GLB/accessor bytes, checks oriented triangle multiplicity, positions, normal presence and values, material declaration presence, hierarchy, bounds, exact archive closure, report facts and the 3TZ index independently of production loaders and validators.

`viewer.json` records a twelve-triangle/four-leaf archive in Cesium 1.146.0 and Linux Chromium 153.0.8010.52 with SwiftShader. At the recorded near camera all four leaves render twelve triangles and a scene pick succeeds; the far camera selects none, and returning near restores all four. A zero-root-error negative control selects none. Page errors, failed requests and external requests are empty. A display-only ENU placement lets this viewer show local geometry; it does not establish source CRS support.

Both files record the same final portable binary SHA256. `provenance.json` records that hash, exact replay commands, script hashes, hashes of the frozen production source files and the worktree HEAD at recording. The implementation was uncommitted during these runs, so that initial HEAD is explicitly a base revision. The actual tested production manifest subsequently matched committed source `d444d82` byte-for-byte, verified against both the working tree and `git show`; the production diff is empty. Exact compiler/features belong to the integration owner's build log. The viewer also records the loaded Cesium JavaScript hash and browser version. The analytical fixture writer/reader were added independently for F1a and import no production implementation helpers. Previous provisional runs were replaced by these final frozen-binary observations.

## Replay from the repository root

Use the final portable binary built by the integration owner. No native geospatial codec is needed for this static embedded GLB profile.

```sh
python3 -B tests/f1a_oracle.py --binary target/debug/rusty-tiles \
  --json-output bench/architecture_audit/f1a/oracle.json
python3 -B tests/f1a_viewer.py --binary target/debug/rusty-tiles \
  --cesium-dir "$CESIUM_DIR" --node-modules "$NODE_PATH" \
  --chromium /usr/bin/chromium \
  --json-output bench/architecture_audit/f1a/viewer.json
python3 -B bench/architecture_audit/f1a/measure.py \
  --binary target/debug/rusty-tiles \
  --output bench/architecture_audit/f1a/resources.json
```

The first command needs Python's standard library only and fails if the binary is absent. The viewer requires existing Cesium/Playwright packages and Chromium; the runner downloads nothing. Locally, `CESIUM_DIR` is `/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/cesium/Build/Cesium` and `NODE_PATH` is `/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules`. Cesium 1.146.0 and Playwright 1.63.0 were recovered from the existing npm offline cache; the exact setup command and defined camera/SSE conditions are in the oracle README. This changes no repository dependencies. Missing viewer assets fail explicitly rather than silently passing acceptance.

## Resource measurement limits

`measure.py` reuses only the independent source fixture generator, not the quadratic small-fixture geometry matcher. `resources.json` exercises small geometry, 100,000 source triangles, 4,096 leaves, source/JSON byte envelopes, their combined workload and over-limit refusal controls. The combined32MiB source/100,000triangle/4,096leaf case used63,348KiB peak RSS, one sampled thread, six sampled descriptors and10,956,860 sampled workspace bytes. It left no workspace. The geometry-only case used49,768KiB; these are observations, not universal maximum-memory promises. Read the recorded sizes and outcomes rather than treating labels as proof of every admission boundary. Expanded-instance/accessor/source/JSON refusal controls run through the public facade in `tests/mesh_archive.rs`; the resource sampler does not claim to measure every possible combination.

`measure_child.c` first starts as a fresh small C process, then forks/execs the converter and reads Linux `wait4` peak RSS. This avoids contaminating the converter's high-water mark with memory inherited from Python's large fixture generator. Compilation requires `cc`; the helper is built in the temporary workspace and removed afterward. Descriptors, threads and workspace files/bytes are sampled through `/proc` at intervals of at least 2 ms: recorded maxima are observations, not proven peaks. An unsampled fast run can report zero descriptors or scratch despite using them. Serial materialization has bounded admission; these measurements do not claim constant memory, all-platform behavior, filesystem durability or crash recovery.

The independent analytical oracle, corruption controls, real viewer and resource measurements address different obligations. Viewer success does not prove semantic mesh fidelity or GPU numerical accuracy; a Rust round trip or production validator is not the oracle for these cases.

## Embedded-interpreter harness correction

The first official Blender matrix on `636ebe6` failed only the new CWD test: its child Python process did not inherit Blender's injected wheel import directory. The test now exercises the already-loaded API in the sequential unittest harness and restores CWD in `finally`; no Blender-specific branch or production change was needed. The same candidate wheel passes all27 tests in both the isolated CPython runner and local distribution Blender5.2.2. `distribution-blender.json` records that local check; it does not substitute for the rerun of the four pinned official Blender bundles in CI.
