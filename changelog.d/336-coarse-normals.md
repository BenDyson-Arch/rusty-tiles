### Fixed

- Mesh HLOD coarse tiles no longer shade surfaces with starburst gradients or
  inverted facets when the source carries normals. Coarse `NORMAL` is now
  crease-aware: a vertex splits its normal across edges sharper than 60°, and
  every corner normal lies within 60° of its face, so no face is shaded against
  its own winding. Leaves are unchanged; coarse tiles of sources with normals
  have more vertices (13–15% more coarse bytes on the measured meshes).
