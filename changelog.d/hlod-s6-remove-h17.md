### Removed

- `mesh` builds its coarse tiles only with the 0.4 classic octree HLOD stage. H17
  certified coarse geometry, the H27 proxy texture bake and the hidden `--lod`
  switch are removed, with no legacy mode. Coarse tiles carry one representative
  `source_primitive` feature ID per triangle, and no `proxy_source`,
  `lod_vanished` or `lod_filled` tables or certificates. Leaves and the HLOD
  output are byte-identical to the previous `--lod hlod` output.
- Run diagnostics drop `proxy_bake` and the `lod_prepass`, `lod_simplify`,
  `seam_resolution`, `lod_finish` and `proxy_bake` phase seconds; `memory` and
  `hlod` are always present. A memory budget is always resolved for `mesh`.
