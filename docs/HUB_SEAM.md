# Hub seam: `tinyowl-tiles` ↔ `tinyowl-server`

Do **not** wire this into `model-worker` until:

1. v0 golden tests pass against `3d-tiles-tools@0.5.4`
2. Heading-only ENU 4×4 matches `modeltiles.eastNorthUpMatrix` (clockwise from north in the EN plane). Cesium HPR (`−heading`, pitch, roll) is a **later** flag — swapping now would rotate placed models.
3. A later ticket explicitly asks for the swap (not implied by P2).

`TINYOWL_TILES_CMD` is the swap valve. Default remains `npx --yes 3d-tiles-tools@0.5.4`.

## Contract

`model-worker` (and later raster/vector jobs) should treat this CLI as a local filter:

```text
tinyowl-tiles glb-to-3tz -i <src.glb> -o <out.3tz> [--cartographic-position-degrees LON LAT [H]] [--rotation-degrees H P R] -f
```

Suggested Go (when the gates above pass — not in P2):

```go
func tilesBin() string {
    if p, err := exec.LookPath("tinyowl-tiles"); err == nil {
        return p
    }
    return "" // fall back to npx 3d-tiles-tools@0.5.4
}
```

Fallback recipe today:

```text
npx --yes 3d-tiles-tools@0.5.4 createTilesetJson -i <dir-with-glb> -o <dir>/tileset.json -f
npx --yes 3d-tiles-tools@0.5.4 convert -i <dir> -o <out.3tz> -f
```

`--cartographic-position-degrees` must not replace `modeltiles.eastNorthUpMatrix` until heading-only matrices match. Go applies heading as an EN-plane rotation (clockwise from north). This crate’s `--rotation-degrees` uses Cesium HPR (`−heading`, pitch, roll) — different 4×4 when heading ≠ 0.

## Planes

This binary never talks to S3, Postgres, or `media_index`. The worker hydrates a workdir from `upload`, runs the CLI, then promotes:

- source GLB → `tinyowl`
- `.3tz` (and extract) → `tinyowl-web`

## Docker

When swapping: copy the Rust binary into the `model-worker` image (or `cargo install` in the image). Keep Node in the image until LookPath succeeds in CI.

## Spine

After this repo exists, list `tinyowl-tiles` on the CONTEXT.md attention/spine table (hub docs).
