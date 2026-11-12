### Fixed

- `mesh` and vector content no longer write `"filter": "NONE"` on `EXT_meshopt_compression`
  buffer views. NONE is the extension's default, but Blender 5.2's glTF importer refuses the
  explicit value, so untextured mesh tiles did not open in Blender. `OCTAHEDRAL` is still
  written. Only the glTF JSON changes; compressed and fallback payload bytes are the same, but
  `mesh` and vector content hashes and archive bytes change. `validate` accepts both forms.

### Changed

- The vector encoder fingerprint changes, so `--reuse-tileset` refuses every vector archive
  made before this release, with or without `--meshopt` ("previous encoder, CRS or conversion
  settings differ"). Run one fresh conversion without `--reuse-tileset`.
