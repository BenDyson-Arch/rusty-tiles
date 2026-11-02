### Changed

- Mesh coarse levels use classic octree HLOD with exact leaves and at most 8,192
  triangles per payload. Shared source-arc contracts have canonical same-depth
  borders; contracted arcs have bounded mixed-depth gaps, with junctions and leaf
  contacts fixed. Border strips retain their interior rings and exact collinear
  subdivisions; bounded sheet reduction and local reconstruction recover stalled
  contractions. Memory admission includes live certificates and contract buffers.
  Per-depth diagnostics report topology, borders and omissions.
- After ordinary reduction, reconstruction and staged strategies fail, constructive
  last resorts run in order: N1 admits new shared-point faces/edges only in their
  deterministic seam owner; N2 admits folds (reorienting reversed faces and removing
  excess coincident copies); N3 clusters each input component with interface points,
  opening pins and border-touching open boundaries fixed. N3 permits border-free
  components and holes to drop or close at any size, measured into geometric error G
  rather than bounded by E. Outside N3, coarse drops require cap pressure and drops
  at or above child G go smallest first. All component drops are declared in
  `asset.extras.rustyTiles.droppedComponents` and measured into G.
- Constructive tiles declare strict `asset.extras.rustyTiles.lastResort`
  `{stage, foldFaces}` metadata, recounted by validation; inherited folds use stage 0.
  Omission remains only when the contract alone exceeds the cap or the last resort
  cannot preserve border-touching open boundaries, and propagates to all ancestors.
  Published descendants form the capped frontier; omitted frontiers perform no
  appearance work. Delivery checks total and per-depth omission counts.
