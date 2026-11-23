### Fixed

- HLOD: P1 no longer makes a collapse that changes a boundary reaching a contract point
  (other than shortening it by merging two of its vertices). The topology floor refuses
  any such change (`touchingBoundaryChanged`), so a pass that made one proposed nothing
  and its cell was omitted; P1's link and pinch tests let collapses through that cover or
  drop such an edge on rough meshes (holes, fins, pinched loops). The refusal is P1's new
  collapse reason 13 (`refusedBy`); passes that built before are unchanged.
