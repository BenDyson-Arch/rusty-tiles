### Added

- Internal proxy texture baking core with immutable geometry packets, source
  correspondence, class-specific PBR and feature-ID mips, normal reprojection,
  and reported appearance displacement. Adapters consume the H17 finishing
  handoff and translate complete-member appearance and retained child packets.
  The core is activated in `mesh` / `mesh_to_tileset` by the H27 integration
  (see the textured coarse tiles entry).
