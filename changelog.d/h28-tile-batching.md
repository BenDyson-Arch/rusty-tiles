### Changed

- `mesh`: each leaf and coarse tile now publishes one glTF primitive (node, mesh,
  material and texture page set) per material class, instead of one per source
  primitive. Source materials with equal factors, alpha mode, sidedness, bound
  slots and source filters merge, even when they read different images. Proxy
  pages are packed per material class, and MASK leaf pages of one class share a
  page. A primitive still splits for a different vertex attribute set, or for a
  position window that `UNSIGNED_SHORT` cannot hold. Decoded positions and
  every triangle's `source_primitive`/`source_triangle`/`proxy_source` feature
  IDs are unchanged. On a 0.79M-triangle photogrammetry crop, glTF primitives
  fall from 1,439 to 156 over the leaves and from 1,124 to 109 over the coarse
  tiles (the root from 58 primitives and 110 textures to 3 and 1). A mipmapped
  source sampler (also the default without a sampler) still keeps one leaf page,
  and so one leaf primitive, per source binding in lossy profiles.
- `mesh`: coarse tiles no longer carry `baked_feature_id_*` texture feature-ID
  sets. They repeated the exact per-face feature-ID attributes against the same
  property tables. The ID-texture machinery stays available internally for a
  later annotate pass.
- `preview` uses CesiumJS's default maximum screen-space error of 16 (was 8).

### Added

- `mesh`: `conversion.json` reports `texel_ownership`: whether each leaf's
  decoded UVs give every page texel a single owning triangle (`unique`), checked
  exactly, or whether overlapping source UVs share texels (`shared`, editable
  in vertex space only). Coarse tiles are unique by construction.
