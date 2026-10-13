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
