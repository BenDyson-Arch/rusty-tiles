### Fixed

- The `mesh` tiler no longer adds a back-facing duplicate "fill" over the outline of an
  open component (a flat sheet, sign or planar roof). A coarse tile rejects a border loop
  when the summed area vectors of its fan point against the summed area vectors of the
  faces bordering the loop (a strictly negative dot product); loops with a zero or
  positive dot, such as true holes, are filled as before.
- A proxy with many materials is no longer refused with `ResourceLimit` ("prepared adapter
  exceeds complete assembly residual lease") for charging every face the sum of all
  materials' JSON; each face group clones one material, so the charge is faces times the
  largest material.
