### Fixed

- `mesh` and vector content no longer write `"filter": "NONE"` on `EXT_meshopt_compression`
  buffer views. NONE is the extension's default, but Blender 5.2's glTF importer refuses the
  explicit value, so untextured mesh tiles did not open in Blender. `OCTAHEDRAL` is still
  written. Only the glTF JSON chunk changes; compressed and fallback payload bytes are
  unchanged. Archive bytes and the vector encoder fingerprint change, so `--reuse-tileset`
  rebuilds vector archives made before this change. `validate` accepts both forms, as before.
