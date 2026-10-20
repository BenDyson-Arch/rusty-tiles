### Fixed

- Mesh tiler: the per-depth seam audit now records each seam edge with the direction its
  tile's face traverses it, so two coarse tiles covering the same side of a seam edge are
  caught as overlapping coverage even when only two tiles claim the edge. The tiles involved
  keep their removed seam vertices and re-run under a seam-span guard; an overlap that
  consensus cannot resolve fails with a typed error. Only coarse tiles are ever re-run: a
  leaf on the same frontier keeps its seam records, so its neighbours are no longer reported
  as cracked (the cause of a false "no partner tile" failure on large multi-instance inputs).
  Source edges whose two triangles disagree in winding are treated like non-manifold edges
  (their vertices are kept), and double-sided faces split across tiles are not seam defects.
