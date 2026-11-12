### Added

- Python `list_tiles(archive)` lists the content tiles of a `.3tz` archive or
  unpacked tileset directory with their depth, `leaf`/`coarse` role, geometric
  error and accumulated transform.
- Python `decode_tile(archive, member, *, format="glb")` returns one tile as a
  plain glTF 2.0 GLB (meshopt decoded, KTX2/Basis pages transcoded to PNG at
  level 0, feature IDs and property tables kept) or, with `format="arrays"`,
  as NumPy arrays in the tile frame. It is a view for display and inspection,
  such as in Blender's bundled glTF importer, not a reversible round trip.
