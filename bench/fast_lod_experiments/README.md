# H17 bake research probes

Results: [fast-lod-experiments.md](../../docs/architecture/fast-lod-experiments.md).
These are machine-local research probes, not a production validation suite.

The measured Rust source and reviewed report are held by commit `fd55c6ef9c00c75e780bcf920e0539a254e008ee`.
Bulk evidence is `/home/bend/.cache/rusty-tiles-evidence/fast-lod-exp-20261024/`.
Its frozen manifest `SHA256SUMS.final-reviewed` has SHA-256
`bc81519e7ca4565dd02d8a87a888fe3a19ec18354c2595afc12cde56711f4ca5`. It binds 219 evidence files; the separate binary binding
file records all four executable hashes. `source-final.patch` reproduces the
reviewed commit from `8971c9c3`; `source-v1.patch` changes only inverse padding.
This README is a later documentation-only index, outside that frozen snapshot.

`decode.mjs` uses the local meshoptimizer and Three/Basis WASM packages;
`quality.py` uses NumPy/Pillow and the independently compiled `nearest.cpp`.
Legacy seam probes and controls additionally use SciPy from the existing
`leaf-gaps-20261024/.venv`. All paths deliberately identify the measured host.
The evidence contains exact commands, build/run queues, CPU phase stamps,
source and artifact identities, failed attempts, two independent reviews and
responses. No 2048/full-model bake, browser or server was run.
