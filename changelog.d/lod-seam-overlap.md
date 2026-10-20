### Fixed

- Mesh tiler: the per-depth seam audit now catches coarse tiles whose simplified seams
  overlap: a seam edge two tiles traverse in the same direction (each edge is recorded with
  its direction), an edge more than two tiles claim, or a face of seam vertices only that
  two tiles both hold (a fold over the seam, invisible to the edges). The coarse tiles
  involved keep their removed seam vertices and re-run under a guard that refuses to build
  faces of seam vertices only; an overlap that consensus cannot resolve fails with a typed
  error. Leaves on the same frontier are never re-run, and their seam records stay in the
  audit (re-running one hid its seam and reported its neighbours as cracked, a false
  "no partner tile" failure on large multi-instance inputs). Source edges whose two
  triangles disagree in winding are treated like non-manifold edges (their vertices are
  kept), so a source's own orientation is never taken for an overlap.
