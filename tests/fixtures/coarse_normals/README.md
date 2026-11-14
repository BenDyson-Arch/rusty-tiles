# Pre-#336 coarse normals fixture

`prefix.3tz` (224,711 bytes, sha256
`86d7f27b417edc428cdbb6c563e66d6665107a7762de4563b99abca3ac55e61f`) is the
`mesh` output of `rough_glb` in `tests/coarse_normals_validate.rs`, baked by
the producer before #336, which averaged every face at a vertex for coarse
`NORMAL`. `tests/coarse_normals_validate.rs` validates it: `ok: true` with the
`coarseNormalInversion` warning. Its coarse root has 340 of 6,143 faces with a
corner normal more than 90° from the face.

Regenerate:

```sh
RT_COARSE_NORMALS_INPUT=rough.glb cargo test --release --locked \
  --test coarse_normals_validate write_rough_input -- --ignored
# input sha256 5f77177f82e200e7020692310eccdb360fb2599909003435dfe99fd602ff7e2f
git checkout 85157619   # origin/develop before #336
cargo build --release --locked
target/release/rusty-tiles mesh -i rough.glb -o prefix.3tz --jobs 2
```

The bake is deterministic: two runs gave the same bytes. Size is above the
100 KB fixture aim because a coarse tile needs more than 8,192 source
triangles (two capped leaves). The leaves, with full provenance, are most of
the bytes. Integer relief already keeps them as small as the
measured alternatives allowed (224–256 KB).
