### Changed

- Large-mesh tiler: when a proxy bake pass runs, each accepted leaf also keeps a compact witness for it. In published order, it holds:
  - the decoded POSITION, NORMAL and TANGENT as a renderer reads them (NORMAL after the octahedral decode in lossy profiles);
  - source primitive, expanded instance, triangle and global rank;
  - each literal texture binding (page, TEXCOORD set, sampler, `normalTexture.scale`);
  - the measured post-layout, post-drop texel spacing in local metres.

  Faces that do not rasterise for a binding are kept and marked. Published leaves are unchanged.
