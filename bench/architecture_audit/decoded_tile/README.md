# Decoded tile views (#329)

Independent acceptance drivers for `rusty_tiles.list_tiles` and
`rusty_tiles.decode_tile` (`src/tile_view.rs`). Neither driver runs in CI; both
need an installed wheel. The Rust tests in `src/tile_view/tests.rs` run in the
library suite.

| Driver | Reference | What it checks |
| --- | --- | --- |
| `parity.py ARCHIVE` | `../mesh_hlod/s5/decode.mjs` (independent published-byte decoder, spec meshopt in `tests/ext_meshopt_spec.py`) on the member read with `zipfile` | `format="arrays"` corners equal decode.mjs's positions exactly after `(x, -z, y)`; corner order, source addresses and per-face property rows equal; UVs within 1e-7; normals within decode.mjs's own `normalTolerance`; the plain GLB decodes to the same faces |
| `blender_check.py` (inside Blender) | Blender's bundled glTF importer | plain and published GLBs import; polygons, placement against the arrays, base-colour image, UV layers, colour attributes, `_FEATURE_ID_n` attributes and per-face ID fidelity |
| `cargo test --lib tile_view -- --include-ignored` with `TILE_VIEW_ARCHIVE` | the vendored transcoder used by codec verification and the independent dev-only `basisu` crate | every PNG page of the plain GLB equals level 0 of the published KTX2 page from both |

Normals are compared within a tolerance, not exactly: for OCTAHEDRAL normals
the spec reference decoder and the linked meshopt filter can round one code
unit differently. Both are inside decode.mjs's declared bound.

## Results (2026-11-11)

Inputs: `crop3.glb` (vertex colours, normals) and `tex3.glb` (89 JPEG pages)
from the H17 crop, tiled with `--anchor 133.0584598837835 -12.33625061681309 100`
by the release CLI built from this branch. Each archive has 217 tiles (156 leaves).

RESULTS_TABLE

Evidence is external at `~/.cache/rusty-tiles-evidence/decoded-tile-329/`
(never held in git); `manifest.json` there lists every file's SHA-256 and binds
the source commit, wheel and archives.
