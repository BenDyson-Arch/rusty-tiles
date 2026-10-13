### Added

- Large-mesh tiler (hidden `mesh-leaf-payloads` / `mesh-large-to-3tz`): every internal
  tile now carries a certified coarse proxy (H17). Proxies are simplified onto existing
  source vertices, quantized on the frozen H8 lattice, and certified in both directions
  against the baked source (composed and direct bounds, outward-rounded) after decoding
  their own bytes; their certified error replaces the empty-parent placeholder. Seams
  between same-depth tiles are bit-identical on decoded bytes. Small isolated islands
  vanish and small interior holes are filled at coarse depths within the certified error,
  with published `lod_vanished` / `lod_filled` records, and both reopen deeper. Each proxy
  triangle names the source triangle it descends from (`proxy_source` tables of at most
  8,192 rows). Proxies are untextured (flat material factors) until H13 bakes textures.
