### Changed

- `mesh` leaf tiles: the leaf stage now decodes each source image once, before any
  leaf is built, and cuts every leaf's texture crops from it into a compressed scratch
  store that leaf pages read. Pages it covers no longer decode or cache whole source images,
  and their memory leases no longer include a source decode. Archives are
  byte-identical. `diagnostics.leaf_delivery.crop_store` reports the store; it is
  `null` when the stage runs without it (memory-backed scratch, or too little memory
  or scratch for the store).
