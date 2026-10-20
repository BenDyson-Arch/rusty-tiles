### Fixed

- Mesh tiler: the per-depth seam audit now catches coarse tiles whose simplified seams
  overlap: a seam edge two tiles traverse in the same direction (each edge is recorded with
  its direction), an edge more than two tiles claim, or a face of seam vertices only that
  two tiles both hold (a fold over the seam, invisible to the edges). The coarse tiles
  involved keep their removed seam vertices and re-run under a guard under which a
  non-seam vertex may not collapse into a new face of seam vertices only; an overlap that
  consensus cannot resolve fails with a typed error. The new leaf-manifest LOD report field
  `seamOverlapTiles` counts the tiles re-run this way (omitted when zero). Leaves on the
  same frontier are never re-run, and their seam records stay in the audit (re-running one
  hid its seam and reported its neighbours as cracked, a false "no partner tile" failure
  on large multi-instance inputs). Source edges whose two triangles disagree in winding,
  and double-sided (coincident) source faces, are treated like non-manifold geometry
  (their vertices are kept), so neither a source's own orientation nor a double-sided face
  split across leaves is taken for an overlap.
