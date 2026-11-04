### Changed

- Vector reuse: upgrading from 0.3 requires one fresh `--reuse-tileset` cache/baseline. Encoder fingerprints cover the vector modules and their dependencies, including source files previously omitted; the first reuse run rebuilds every tile.

- Release binary smoke tests, `scripts/release_acceptance.sh`, the preview-layer fixture script, the runtime benchmark, the perf lane's `mesh` case and the scale-gate examples now run `rusty-tiles mesh` instead of the deprecated commands. The perf lane drops its `mesh-local` and `mesh-explicit` cases. The committed output digests replace the two `mesh-to-3tz` recipes with one `mesh` recipe (lossless profile, one worker); the other recipes' digests are unchanged.

### Fixed

- `bench/benchmark_vector.py` no longer fails on the removed `src/vector/native.rs`. It now records the hashes of the encoder fingerprint's own source list.
