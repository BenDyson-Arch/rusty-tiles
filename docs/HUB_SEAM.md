# Hub seam: `tinyowl-tiles` ↔ `tinyowl-server`

`createTilesetJson` and `convert` accept the same argv as `3d-tiles-tools@0.5.4`. That is enough for `TINYOWL_TILES_CMD=tinyowl-tiles` **without** cartographic flags: Go still writes `root.transform` via `eastNorthUpMatrix`.

Do **not** pass `--cartographicPositionDegrees` / `--rotationDegrees` from `model-worker` until a later ticket drops Go placement. Those flags match Cesium HPR (the tools oracle), not Go heading.

`TINYOWL_TILES_CMD` is the swap valve. Default remains `npx --yes 3d-tiles-tools@0.5.4`.

## Contract

Drop-in for `tools.go` today:

```text
tinyowl-tiles createTilesetJson -i <dir-with-glb> -o <dir>/tileset.json
tinyowl-tiles convert -i <dir>/tileset.json -o <out.3tz>
```

Convenience (not used by the worker):

```text
tinyowl-tiles glb-to-3tz -i <src.glb> -o <out.3tz> [--cartographicPositionDegrees LON LAT [H]] [--rotationDegrees H P R] -f
```

`.3tz` is a stored ZIP with `"@3dtilesIndex1@"` last (Cesium 3TZ). `tileset.ExtractZip` only needs a ZIP that contains `tileset.json`.

## Planes

This binary never talks to S3, Postgres, or `media_index`. The worker hydrates a workdir from `upload`, runs the CLI, then promotes:

- source GLB → `tinyowl`
- `.3tz` (and extract) → `tinyowl-web`

## Docker

When swapping: copy the Rust binary into the `model-worker` image (or `cargo install` in the image). Keep Node in the image until LookPath succeeds in CI.

## Spine

After this repo exists, list `tinyowl-tiles` on the CONTEXT.md attention/spine table (hub docs).
