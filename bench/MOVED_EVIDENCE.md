# Evidence moved out of the tree (2026-10-14 tree trim)

These generated run results were removed from git to stay inside the tree budget
(see AGENTS.md, "Evidence and tree size"). Nothing in CI, the tests or the build
reads them. Each file is kept unchanged, at the same relative path, under
`~/.cache/rusty-tiles-evidence/tree-trim-20261014/`, and was last held by commit
`df3f7a5`. Recover one with `git show df3f7a5:<path>` and check it against the SHA-256.

| Path | Bytes | SHA-256 |
| --- | ---: | --- |
| `bench/demodata_benchmark_results.json` | 155924 | `163f0cc7f8bdcfc9877483ec39af72a0277a8cce2d7c6c826baa5c74d3271eec` |
| `bench/demodata_baseline_results.json` | 65015 | `17597d951151042df9eb510913a7b3c54768a5342e3b5e01c941ffeafd9b44e0` |
| `bench/vector_benchmark_results.json` | 51888 | `7f2442516debbd69a93b4b3b48108635fd158b8c0581aaa476d5acb93d79ecdc` |
| `bench/vector_fill_glb_results.json` | 42713 | `eca34af724271f5b79724d5c68ba9241374a4b5cb3837d124e5023a4036b0c1e` |
| `bench/convert_implicit_browser_results.json` | 30107 | `e57778eacbe0ccb5dc3075f1e4003032a0762afad5245335581ae06ec6457855` |
| `bench/demodata_gltf_wrap_results.json` | 29139 | `939b999cccce130f7f0c95c2dd4acf137e52574c44df12c3fb516f82192ed884` |
| `bench/release_04_blockers_results.json` | 19354 | `2dce67db9f6db425809b77797bbf55619f00f4e7946040265a706d2524a08251` |
| `bench/terrain_performance_results.json` | 17432 | `9d9a9a620325665dbde919f487d39f331ce22ee2da043cd54e7df849b0b5e1f2` |
| `bench/raster_benchmark_results.json` | 16020 | `9351b0e910a81e00dddf39e37eb3474f9e2f66eb0120e038822bf5d6c8c99c8c` |
| `bench/terrain_benchmark_results.json` | 14871 | `2b3ded4c1b1f7fb1cb31d514d611bb34f4c18331f0702616485c96550ab5327e` |
| `bench/architecture_audit/foundation/results.json` | 100877 | `41cedf375d6d585850bb856ad3be3b2924964d4bd3c8103173fca4996f154489` |
| `bench/architecture_audit/f1b1/resource-initial.json` | 75371 | `9d05cc245567df997e071c6a49f629c0564b6d57dc7dcfd09dc9db7008cc6d8b` |

## S5 control hash manifests (2026-10-25)

Per-file SHA-256 dumps of the generated S5 controls. `tests/hlod_acceptance.rs` now
checks one digest per family (`controls-digest.json`) over the same sorted
`path<TAB>sha256` lines. The dumps are kept under
`~/.cache/rusty-tiles-evidence/tree-trim-s5-20261025/` and were last held by commit
`f1e841c0`.

| Path | Bytes | SHA-256 |
| --- | ---: | --- |
| `bench/architecture_audit/mesh_hlod/s5/controls-sha256.json` | 50276 | `d8f04095256266611909891bdd3309e0f710d91c4f26c49b6ad6d7b474089da9` |
| `bench/architecture_audit/mesh_hlod/s5/revision-controls-sha256.json` | 30316 | `e3d6d3cd51d4e68b5711b1c10fa2d783942f756b12ddd84a0eb9363cfd5e88a0` |

## Mutation-run reports and 0.4 review results (2026-10-29)

Generated outputs that nothing in CI, the tests or the build reads (the mutant scripts rewrite
the three reports when run; the two `bench/*.json` files are dated 2026-10-09 run results). Kept
unchanged under `~/.cache/rusty-tiles-evidence/tree-trim-ci-time-20261029/` at the same relative
path and last held by commit `e5c475f3`.

| Path | Bytes | SHA-256 |
| --- | ---: | --- |
| `bench/architecture_audit/c1_payload_integrity/production_controls/ktx2_basisu_amendment/2026-10-13/mutants/production-report.json` | 34282 | `aa96959e35cf908a4360db15314bee940f9264f14f8c3f1e8206841c3bf4c46d` |
| `bench/architecture_audit/c1_payload_integrity/production_controls/ktx2_basisu_amendment/2026-10-13/mutants/oracle-report.json` | 11554 | `4430c89561222c184198a976524f4a82361ceb8974c3e7fbf9435eb45ab83f64` |
| `bench/architecture_audit/c2_decoded_containment/oracle/mutants/slice3-report.json` | 12274 | `19b0937499aa82014e0546bfb365efeb0613c28cc212885018f2b40f90cfdbf7` |
| `bench/release_04_review_results.json` | 6259 | `cf37f6aa9801b867c698869c28a1a71ee06563ca0b5dd1bd4435c0acaccfcfdd` |
| `bench/release_readiness_results.json` | 3205 | `2a6dfd036d5e1493a53d221c79a88b8e94210165a1f101e4667e6cbc7d13c972` |
