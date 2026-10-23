### Deprecated

- `mesh-to-3tz` and `mesh-local-to-3tz` now print a deprecation warning on stderr on every run, naming `rusty-tiles mesh` as the replacement. Under `--progress json` it is an NDJSON `warning` event with `"code":"deprecated"`. Their `--help` lines and the Python `mesh_to_3tz` / `mesh_local_to_3tz` docstrings say they are removed before the 0.4.0 release. [docs/architecture/legacy-mesh-retirement.md](docs/architecture/legacy-mesh-retirement.md) lists exactly what that removal deletes and the conditions it waits for.

### Changed

- Vector reuse: existing `--reuse-tileset` caches are invalidated once, so the first reuse run after upgrading rebuilds every tile. The native and portable encoder fingerprints no longer hash the legacy mesh writer (`glb_write.rs`), so removing it later leaves reuse identity unchanged. They now cover sources they previously omitted: `vector.rs`, `vector/pipeline/outline.rs`, `vec3.rs`, `vector_compression/source.rs`, the `content_integrity` payload checks, `implicit/tileset/stream.rs` and, in native builds, `geospatial/native.rs`. A test keeps the list closed over the vector modules and their crate dependencies.
- Release binary smoke tests, `scripts/release_acceptance.sh`, the preview-layer fixture script, the runtime benchmark, the perf lane's `mesh` case and the scale-gate examples now run `rusty-tiles mesh` instead of the deprecated commands. The perf lane drops its `mesh-local` and `mesh-explicit` cases. The committed output digests replace the two `mesh-to-3tz` recipes with one `mesh` recipe (lossless profile, one worker); the other recipes' digests are unchanged.
- The source-CRS Jacobian helpers that the `mesh` tiler shares with the legacy bake moved out of the legacy module, with their analytic tests, so the legacy module can be deleted whole.

### Fixed

- `bench/benchmark_vector.py` no longer fails on the removed `src/vector/native.rs`. It now records the hashes of the encoder fingerprint's own source list.
