# Mesh archive size: meshopt provenance tables and octahedral leaf normals

This slice makes two changes to the large-mesh tiler.

- **Provenance tables (lossless).** The `source_primitive`/`source_triangle` columns of
  every leaf, and the proxy tables, are written as `EXT_meshopt_compression`
  ATTRIBUTES/NONE streams. Each 4- or 8-byte column is its own stream, with the value
  width as its stride. UINT8 columns stay raw, because the extension needs a stride
  divisible by 4. The schema (`rusty_tiles_large_source_v1`), the column types and
  the decoded values are unchanged.
- **Leaf normals (lossy, profile-gated).** In `fast`, `balanced` and `quality`, leaf
  `NORMAL` holds 12-bit octahedral codes behind the OCTAHEDRAL filter. Clients decode
  them to normalized SHORT VEC3, which KHR_mesh_quantization allows. `lossless` keeps
  f32.

The spec citations and the validator rules are in
[c1-meshopt-octahedral-amendment.md](../../../docs/architecture/c1-meshopt-octahedral-amendment.md).
The 4-byte feature-ID attributes are unchanged. glTF core forbids UNSIGNED_INT vertex
attributes; UNSIGNED_SHORT would need a 4-byte stride and a per-leaf range argument, for
about 50 MB on pohnpei.

## Tooling kept here

`mutants.py` holds the production mutants. Each one patches a single site; the run passes
only if the named tests fail, and it restores the source by hash.

## Evidence (outside git)

The evidence is in `~/.cache/rusty-tiles-evidence/mesh-size-20261014/`:

- `spec/`: the pinned extension texts.
- `nerr/`: the archive comparison tool.
- `nerr-pohnpei.json`: pohnpei compared leaf by leaf. Tables, positions and feature IDs
  are byte-identical. It holds the normal error and the per-tier bytes.
- `mutants-report.json`.
- `pick/`: the Cesium 1.146 pick proof over CDP, with leaf and proxy picks and their
  checks.
- `h8/` and `scripts/oracle_geometry_subset.py`: leaf-oracle decode, provenance and
  normal checks on balanced and lossless leaf payloads.
- `c1gen*`: the regenerated C1 corpus and its CLI parity report.
