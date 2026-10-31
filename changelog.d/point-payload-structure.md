### Changed

- Breaking point-cloud format: tiles hold only position and colour by default.
  POSITION is 16-bit per tile axis (`KHR_mesh_quantization`), `COLOR_0` 8-bit;
  the per-point property table (all source fields, 16-bit RGB) and feature IDs
  need `--metadata` / `metadata=True`. About 12 instead of 98 bytes per point.
  A leaf whose 16-bit step would be coarser than the source coordinates keeps
  float32 positions.
- Point parents fill to `--maxPoints`: the first point per cubic voxel on the
  finest grid that fits, instead of a fixed 20×20×20 grid over the tile extent.

### Fixed

- Point parents no longer have an identical one-child duplicate with the same
  geometric error when all their points lie in one octant. A child holding
  nearly all of its parent's points can still repeat its parent's error.
