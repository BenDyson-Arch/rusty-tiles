### Added

- Large-mesh tiler internals: H17 can hand each final coarse tile to a texture-bake pass
  (H27) as a frozen, crate-private per-node packet (published positions, weld keys,
  provenance, fills, removals, children and tables), with a bake hook after the LOD pass
  and retained H8 pre-encode pages. No public API or output changes without a baker.
