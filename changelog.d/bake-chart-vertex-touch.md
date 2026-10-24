### Fixed

- `mesh`: proxy texture charts now grow across faces that share only a corner.
  The chart overlap test treated every such pair as overlapping, so each chart held
  at most two faces, with a UV seam and gutter on almost every proxy edge. A pair
  sharing one corner is now separate when their corner wedges are disjoint by a
  margin that survives 16-bit UV storage, and near-misses keep the same margin.
  Charts stop growing before they could outgrow a page after repacking, and their
  normal cone narrows from 60° to about 26° so a chart's shared density costs at
  most 1.23× texel area. On a 0.79M-triangle photogrammetry crop, proxy texture
  bytes fall 20% with unchanged leaves and run time.
