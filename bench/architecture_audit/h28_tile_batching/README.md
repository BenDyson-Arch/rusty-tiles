# H28 per-tile primitive batching: independent comparison tools

> `cmp_h28.py`, the H17-era archive comparison, was removed on 2026-11-04 (it read the
> `proxy_source` feature set that HLOD slice S6 removed); its last version is at
> [`11204466`](https://github.com/BenDyson-Arch/rusty-tiles/blob/112044668e8a39394a96832833a0ebaa17c4c0db/bench/architecture_audit/h28_tile_batching/cmp_h28.py).

Retained oracles for `docs/architecture/h28-tile-batching-contract.md`. They share no
code with the Rust writers: GLBs are decoded with the reference JavaScript meshopt decoder
and read through numpy. Bulk output (reports, archives) stays in
`~/.cache/rusty-tiles-evidence/h28-20261021/`. `overlap.py` has no self-test (there is no
mutant set); a run is evidence only for the archives and tool hashes it names.

Requirements: `python3` with `numpy`, `node`, and the `meshoptimizer` npm package.
`decode_many.cjs` loads the decoder from the hard-coded `modulePath` at its top; edit that
path (or install the package there) on another host.

| File | sha256 |
| --- | --- |
| `overlap.py` | `aa94d9e7fbf99bc0e733feb03c9654e57d36686b45ee3865f6024de49e31108f` |
| `decode_many.cjs` | `bb04ea636dfb81332aebeeb68f04d2da62200139ae4e88df962d85134322bdb0` |

## overlap.py

Measures texture-space sharing in one leaf GLB: for the first primitive of each mesh it
sweeps the decoded `TEXCOORD_0` triangles, clips each overlapping pair exactly (convex
polygon clipping in f64), and prints the number of overlapping pairs and the largest
overlaps in uv^2 and in 4096^2 texels. It cross-checks the Rust `texel_ownership` record
(`shared` leaves must show overlapping pairs; `unique` leaves none).

    python3 overlap.py EXTRACTED/leaves/<leaf>.glb

Run it on a leaf of the merged archive; a develop archive keeps one primitive per source
primitive, so only the first source primitive of each mesh is examined there.
