### Changed

- `mesh` coarse tiles: the coarse stage no longer decodes source images. The leaf
  stage's crop pass, which decodes each source image once, also builds the coarse
  stage's source chain and stores only the 64×64 tiles that source triangles can be
  sampled from; coarse pages sample those tiles. The coarse stage's plan reserves
  their index instead of a decode slot and an image cache, so it needs less memory and
  admits more jobs. Archives are byte-identical.
  `diagnostics.memory.hlod.sparse_index_bytes`, `diagnostics.hlod.sparse_tile_decodes`
  and the `coarse_*` fields of `diagnostics.leaf_delivery.crop_store` report it.
