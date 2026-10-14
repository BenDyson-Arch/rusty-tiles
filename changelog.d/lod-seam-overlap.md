### Fixed

- Mesh tiler: coarse tiles whose independently simplified seams overlapped (the same
  seam-vertex face built on both sides, or a seam edge claimed by more than two tiles) are
  now detected by the per-depth seam audit and resolved by keeping those tiles' seam
  vertices and re-running them under a seam-span guard; an overlap consensus cannot
  resolve fails with a typed error instead of surfacing one depth later as a crack.
