### Fixed

- The `mesh` tiler no longer adds a back-facing duplicate "fill" over the outline of an
  open component (a flat sheet, sign or planar roof). A coarse tile only fills a border
  loop whose fan continues the faces around it, so true holes are filled as before and
  outlines are not.
- A proxy with many materials is no longer refused with `ResourceLimit` ("prepared adapter
  exceeds complete assembly residual lease") for charging every face the sum of all
  materials' JSON; each face group clones one material, so the charge is faces times the
  largest material.
