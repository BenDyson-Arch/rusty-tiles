### Changed

- `mesh`: HLOD coarse tiles are indexed. Bit-identical corners share one vertex, so each tile decodes to the same triangles in fewer bytes: tex3 coarse tiles are 0.44–0.76× their former size and crop3's 0.25–0.45×. Leaves and geometric errors are unchanged.
