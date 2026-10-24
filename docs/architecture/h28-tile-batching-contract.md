# H28 per-tile primitive batching

> **HLOD amendment (draft, effective when the legacy path is removed).** Coarse
> identity and position assumptions change: a coarse triangle carries one
> representative `source_primitive` ID (the nearest source primitive at its decoded
> centroid) duplicated at all three corners, with no `proxy_source` and no per-triangle
> source table; leaves keep exact `source_primitive`/`source_triangle`. Coarse
> positions are final decoded positions with fresh charts rather than a frozen H17
> glTF with per-texel provenance. Semantic material-class batching, one property
> table per ID set and appendable feature-ID sets are unchanged. References below to
> `proxy_source` and frozen H17 proxies describe the legacy path.
> See [identity and annotation](../MESH.md#identity-annotation-and-editability).


Status: contract for `ml/h28-tile-batching`, light process (tests, one nonauthor
review, real-data correctness run; the coordinator owns the viewer and
performance lanes). Approved by the owner on 2026-10-21. Evidence:
`~/.cache/rusty-tiles-evidence/h28-20261021/`.

Before H28 every leaf emitted one glTF node, mesh and primitive per source
primitive, and every textured proxy one per (source primitive, source material,
page set), plus one PNG baked feature-ID page per primitive. On the 0.79M-triangle
`tex3` crop that is about 2,300 primitives and the root proxy alone carries 58
primitives for 4,696 triangles and 116 textures, so CesiumJS is draw-call bound.
H28 makes each tile emit as few primitives as glTF semantics allow.

## Invariants

1. **Decoded positions are unchanged.** Every triangle's decoded corners
   (client order `u·s + t` in f32, or the stored f32) are bit-identical to the
   pre-H28 output, so certificates, seams and containment on decoded bytes keep
   their meaning. Leaves still encode each source primitive with the frozen
   lattice (`LatticePlan::encode`). A merged primitive rebases those codes onto
   one offset per axis: `o_g = min o_i`, `u' = u + (o_i − o_g)` in exact integer
   arithmetic, `t_g = o_g · s`. The writer recomputes every corner as
   `RN(u'·s + t_g)` unfused and fused and refuses any bit difference
   (`InvalidState`). It never re-quantizes. Float profiles store f32 values without a
   transform. A merged proxy primitive takes its faces from one frozen H17 glTF
   primitive, whose POSITION words and TRS the adapter already copies and
   verifies per face.
2. **Identity and provenance are unchanged.** Leaves keep `_FEATURE_ID_0`
   (`source_primitive`) and `_FEATURE_ID_1` (`source_triangle`) per triangle;
   proxies keep `source_primitive` and `proxy_source`. Each triangle's row
   content is unchanged; the leaf `source_triangle` row order follows the
   published triangle order and is not itself part of the contract. Each
   feature-ID set keeps one property table per model, within 8,192 rows. Further
   feature-ID sets stay appendable.
3. **Merge keys.** Two pieces share one glTF primitive only when all hold:
   - *Material class* (`batching::material_class`): the admitted material
     without texture identity, meaning base-colour, metallic, roughness and
     emissive factors, `doubleSided`, `alphaMode`, `alphaCutoff` (MASK only),
     each slot's presence with `normalTexture.scale` and
     `occlusionTexture.strength`, and each bound slot's source sampler min/mag
     filter. Numbers compare as f64. An absent field equals its glTF default.
     Admission closes the material field set (no extensions, no texture
     transforms), so the list is complete. Wrap modes are excluded because both
     writers resolve wrapping into page content and publish `CLAMP_TO_EDGE`.
     The source `texCoord` is excluded because published UV sets are allocated
     per bound slot.
   - *Texture identity:* every bound slot names the same published texture.
   - *Vertex attribute set:* the same NORMAL, TANGENT, COLOR_0 and TEXCOORD_n
     presence (leaf attribute flags and UV count; proxy no-raster state, the
     leaves' NORMAL/COLOR and the normal-map TANGENT). No attribute is invented.
   - *Extra feature-ID sets* (ID textures) are equal.
   - *Quantization fit:* the same scale, and every rebased code fits
     UNSIGNED_SHORT. Leaves group first-fit in source-primitive order. The
     frozen plan bounds every tile's span to 2^15 codes per axis, so one window
     always fits; the split path is defensive and tested directly. Proxies
     never cross frozen glTF primitives. H17 now groups source primitives by
     flat material *class* (the same canonical comparison, so flat materials
     written differently share one node and one lattice encoding, whose decode
     is unchanged), so frozen glTF primitives split only by a different class or
     by the 8,192-face `proxy_source` chunk.
4. **Shared atlases per class.** The leaf atlas already shares pages across
   source primitives by slot and filter. H28 also lets MASK base-colour bindings
   of one class share a page (before H28, one page per primitive). A pooled
   page's MASK coverage fitting and receipts are aggregate: its surface samples
   are the union of its bindings' samples under the class's one alpha factor and
   cutoff, and one receipt set covers the page (a different cutoff is another
   class and another page). Proxy page packing groups
   charts by material class instead of source material. Charts stay per
   (source primitive, material), and the raster still samples each face's own
   source texture. A page's `material` is the class representative (the lowest
   source material ID of the class), and child sampling checks the donor page
   against the face's class.
5. **Texture-space ownership.** A tile is texture-editable exactly when no two
   triangles' decoded-UV footprints share interior area on any published page.
   For leaves it is computed after atlasing on the published f32 UVs with exact
   predicates. Each coordinate converts exactly to an integer multiple of
   2^-56 (|uv| < 8). A triangle of zero exact area owns no interior. Two
   triangles are disjoint when one of the six edge-normal axes weakly separates
   their projections, evaluated exactly in i128. A coordinate outside that
   domain counts its triangle as shared, which is conservative. Proxies are
   unique by construction: the chart overlap test, disjoint packed chart
   rectangles, and the raster double-coverage check, which still refuses a
   violation. Each leaf row in `leaves.jsonl` records
   `texelOwnership` (`unique`/`shared`) and `sharedTriangles`.
   `conversion.json` records `texel_ownership`: unique and shared leaf counts,
   shared triangles, proxies (all unique), and the first 64 shared leaves by
   `(first, depth)` with an omitted count. Overlapping or wrapping source UVs
   never refuse a tile; such a tile is only vertex-space editable.
   Ownership is all-or-nothing per leaf; `sharedTriangles` over the leaf's
   triangle count gives the affected fraction. `unique_proxies` is vacuous when no
   baker runs (proxies carry no pages then).

## Proxy baked feature-ID textures

Off by default (`BakeLimits::id_textures`). Every frozen proxy feature-ID set
is an attribute set with a constant value per face. The adapter publishes each
one exactly as an integer vertex attribute against the same property table. It
refuses an ID above 2^24 (`Unsupported`), as before. The `baked_feature_id_*` texture sets
only repeated those values, so no proxy face needs a texture to carry a
feature-ID set. The ID/class texture machinery (`leaves/id_texture.rs`,
`bake/ids.rs`, the writer's NEAREST, lossless and null-gutter rules) stays
compiled and tested. The switch keeps it reachable for a 0.4.x annotate pass
that labels texels in the merged atlas UVs.

## Failure semantics

A key mismatch splits; it never refuses. A rebase that cannot reproduce a
corner's bits is `InvalidState`, a defect; the argument above rules it out.
Texel sharing is recorded, never refused. Budget, density, codec, gate, MASK
and refusal behaviour are unchanged.

## Unchanged

The lattice plan; leaf planning, density schedule and byte budget; page codecs,
profiles, appearance floors and mip-drop reporting; the `lossless`
texel-exact leaf requirement; proxy charts, density and certificates; tile
geometric errors and bounds; H16 delivery. The leaf witness and pre-encode
handoff stay per source primitive, because they never index published glTF
primitives.

## Domain and proof limits

- A mipmapped source sampler in a non-lossless profile keeps a dedicated leaf page
  per binding (an original-source mip chain), so its source primitives do not
  merge in leaves. A texture without a sampler resolves to mipmapped sampling,
  so this is the common case for exported scenes; proxies still merge them.
  Sharing mip-chain pages needs its own appearance design (crops aligned per
  level, a truncated chain) and is left open. A class that overflows one page splits by page. Both are recorded
  limits, not refusals.
- "Fewest" is exact for the key above. The quantization grouping is first-fit,
  which is optimal whenever one window fits, as it always does under the frozen
  plan.
- Texel ownership covers decoded-UV footprints, not sampler footprints. Bilinear
  taps, mip levels and gutters spread an edit beyond a footprint; an annotate
  pass must account for its own filter.
- Performance (draw calls, frame rate) belongs to the coordinator's viewer
  lane; this contract claims structure and exactness only.
