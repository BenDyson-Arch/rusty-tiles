### Added

- Add literal native imagery display checks for constant RGB, ramps, impulses,
  selected numeric gray, NoData, masks and partial alpha, with independent PNG
  decoding and raw COG readback; record the finite 0.4.0 imagery resource receipt.

### Changed

- Clarify and test that R2 imagery pyramids are native Rust/CLI capabilities;
  the standard Python wheel retains its explicit unsupported D1 boundary.
