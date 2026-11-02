### Changed

- HLOD local closures are measured on changed oriented faces touching a closing
  boundary cluster, its original edges and projected cycle-centroid controls.
  Other changes in that component contribute through S2 and moved-boundary
  witnesses. Retained faces remain boundary-to-surface witnesses; bounded interior
  remeshing cannot hide an unchecked closure. N3 retains its broader drop/closure
  domain, with loss measured into G.
