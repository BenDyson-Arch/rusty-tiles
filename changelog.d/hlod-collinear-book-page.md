### Fixed

- Validation: a coarse tile that keeps a source book page collinear with its edge (three
  distinct points on a line, a surface under the published contract) unchanged is no
  longer refused as an `interfaceWinding` overlap. A coarse page whose third corner is the
  same point as a descendant source page's, on the same source edge and direction, now
  corresponds to it; before, correspondence required a strictly acute wedge, which a
  collinear page has with no page, its own copy included. A collinear page moved off its
  source position is still refused (#442).
