### Added

- Mesh tiler (`rusty-tiles mesh` / `mesh_to_tileset`): every internal
  tile now carries a certified coarse proxy (H17). Proxies are simplified onto existing
  source vertices, quantized on the frozen H8 lattice, and certified in both directions
  against the baked source (composed and direct bounds, outward-rounded) after decoding
  their own bytes; their certified error replaces the empty-parent placeholder. Seams
  between same-depth tiles are bit-identical on decoded bytes. Small isolated islands
  vanish and small interior holes are filled at coarse depths within the certified error,
  with published `lod_vanished` / `lod_filled` records, and both reopen deeper. Proxies
  hold one glTF primitive per flat material and at most 8,192 faces; each proxy triangle
  names the source triangle it descends from (one `proxy_source` table per proxy, keyed
  with the `source_primitive` feature ID). H17 on its own publishes untextured proxies
  (flat material factors); the H27 integration bakes their textures (see the textured
  coarse tiles entry).
