# H28 per-tile primitive batching: independent comparison tools

> **`cmp_h28.py` is historical — 2026-10-26.** Its proxy-face identity reads H17's
> `proxy_source` feature set (and its `fill` column), which HLOD slice S6 removed: HLOD
> coarse tiles carry only a representative `source_primitive` set (D4), so the script
> stops with `KeyError: 'proxy_source'` on current archives, including its same-build null
> control. It is kept unchanged (its hash below still matches) as evidence for the
> `h28-20261021` runs of H17-era archives; it is not an oracle for current output.
> `overlap.py` reads only leaf `TEXCOORD_0` and remains usable.

Retained oracles for `docs/architecture/h28-tile-batching-contract.md`. They share no
code with the Rust writers: GLBs are decoded with the reference JavaScript meshopt decoder
and read through numpy. Bulk output (reports, archives) stays in
`~/.cache/rusty-tiles-evidence/h28-20261021/`. Neither tool has a self-test (there is no
mutant set); a run is evidence only for the archives and tool hashes it names.

Requirements: `python3` with `numpy`, `node`, and the `meshoptimizer` npm package.
`decode_many.cjs` loads the decoder from the hard-coded `modulePath` at its top; edit that
path (or install the package there) on another host.

| File | sha256 |
| --- | --- |
| `cmp_h28.py` | `20ca06d54094f3fa5e18897e83cbb96d80bd4d55ef2598906122e7ac14d097eb` |
| `overlap.py` | `aa94d9e7fbf99bc0e733feb03c9654e57d36686b45ee3865f6024de49e31108f` |
| `decode_many.cjs` | `bb04ea636dfb81332aebeeb68f04d2da62200139ae4e88df962d85134322bdb0` |

## cmp_h28.py

Checks two mesh archives (`.3tz`) of the same source. It extracts both, decodes every GLB,
and reports per-tile primitive, node, texture, image, material and feature-ID-texture
counts and archive bytes. It then checks, by source identity (node, mesh, primitive,
ordinal, source triangle, fill marker), that every leaf triangle and every proxy face has
bit-identical decoded corners (`u * scale + translation` in f32, or the stored f32) in
both archives, and lists the non-GLB members whose hashes differ. It asserts both archives
hold the same tile members.

    python3 cmp_h28.py BEFORE.3tz AFTER.3tz WORKDIR OUT.json

`WORKDIR` is a new empty directory (it receives `before/` and `after/`). Archives to
compare:

- merged archive (this branch, or `develop` after merge) as `AFTER`, and an archive of
  the same source and flags built by the previous `develop` build as `BEFORE`
  (for example the `rt-develop-*` binary named in the run record). Expect
  `leafTilesDiffering` and `proxyTilesDiffering` empty, `nonGlbMembersDiffering` limited
  to the archive index and `conversion.json`, and fewer primitives and textures after.
- two archives of the same build (merged against merged) must report no differing leaf
  or proxy tiles; use that as the null control.

## overlap.py

Measures texture-space sharing in one leaf GLB: for the first primitive of each mesh it
sweeps the decoded `TEXCOORD_0` triangles, clips each overlapping pair exactly (convex
polygon clipping in f64), and prints the number of overlapping pairs and the largest
overlaps in uv^2 and in 4096^2 texels. It cross-checks the Rust `texel_ownership` record
(`shared` leaves must show overlapping pairs; `unique` leaves none).

    python3 overlap.py WORKDIR/after/leaves/<leaf>.glb

Run it on a leaf of the merged archive; a develop archive keeps one primitive per source
primitive, so only the first source primitive of each mesh is examined there.
