# Draft vector compatibility audit

Audited on 2026-10-09 for CesiumJS 1.146.0, release commit
[`b8d3a36fe98a3e432eb89253c95d5f20e605e0f1`](https://github.com/CesiumGS/cesium/tree/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1).
These are experimental glTF 2.0 vector extensions delivered in 3D Tiles 1.1
archives. This audit does not establish 3D Tiles 2.0 RFC or glTF 2.1 conformance.
[Issue #87](https://github.com/BenDyson-Arch/rusty-tiles/issues/87) tracks the
ratification recheck.

| Extension | Exact draft source | Contract used by the encoder |
| --- | --- | --- |
| `KHR_mesh_primitive_restart` | [CesiumGS/glTF `9811e8407d4533500cfc6b10e3bc408345035a6f`](https://github.com/CesiumGS/glTF/blob/9811e8407d4533500cfc6b10e3bc408345035a6f/extensions/2.0/Khronos/KHR_mesh_primitive_restart/README.md) | Root `extensionsUsed` and `extensionsRequired`; no primitive extension object. `LINE_STRIP` batches use the maximum unsigned index as restart. |
| `EXT_mesh_polygon` | [CesiumGS/glTF `c1a035499b70aeb5d8281470101423e5e285dfe3`](https://github.com/CesiumGS/glTF/blob/c1a035499b70aeb5d8281470101423e5e285dfe3/extensions/2.0/Vendor/EXT_mesh_polygon/README.md) | `TRIANGLES` primitive with `count`, `indicesOffsets`, `loopIndices`, `loopIndicesOffsets`; one offset per polygon, exterior followed by holes, restart between rings. Optional for triangle rendering. |
| `3DTILES_content_gltf_vector` | [CesiumGS/3d-tiles `c48ebdc8db43dc00917b4f200eff5e2131d7e493`](https://github.com/CesiumGS/3d-tiles/blob/c48ebdc8db43dc00917b4f200eff5e2131d7e493/extensions/3DTILES_content_gltf_vector/README.md) | Optional tileset declaration; vector content has `{ "vector": true }`. Optional boolean `clip` defaults to false; the encoder omits it. |

All three previously documented SHAs still equal the heads of
[PR 2569](https://github.com/KhronosGroup/glTF/pull/2569),
[PR 2570](https://github.com/KhronosGroup/glTF/pull/2570) and
[PR 838](https://github.com/CesiumGS/3d-tiles/pull/838) on the audit date.
There are no draft schema changes between the old pins and this target.
The glTF source links now identify the Cesium fork that holds those commits.
No encoder bytes or converter identity change is needed for this audit.

The runtime evidence is its
[`GltfLoader.loadMeshPolygonExtension`](https://github.com/CesiumGS/cesium/blob/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1/packages/engine/Source/Scene/GltfLoader.js),
which reads the four polygon fields above, and
[`createVectorTileBuffersFromModelComponents`](https://github.com/CesiumGS/cesium/blob/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1/packages/engine/Source/Scene/Model/createVectorTileBuffersFromModelComponents.js),
which splits unsigned indices at restart values and reads polygon offset ranges.
The loader also retains an older `LINE_LOOP` polygon path; the encoder uses the
pinned draft's `TRIANGLES` path.
[`Cesium3DTileContentFactory`](https://github.com/CesiumGS/cesium/blob/b8d3a36fe98a3e432eb89253c95d5f20e605e0f1/packages/engine/Source/Scene/Cesium3DTileContentFactory.js)
chooses the vector GLB reader from the tileset-wide extension declaration.
Consequently plain triangle fill GLBs cannot be mixed into that tileset on
1.146.0; fragmented fills retain their b3dm wrapper.

## Schema provenance and verification

[`3DTILES_content_gltf_vector.schema.json`](3DTILES_content_gltf_vector.schema.json)
is an unmodified copy from
[the pinned draft](https://github.com/CesiumGS/3d-tiles/blob/c48ebdc8db43dc00917b4f200eff5e2131d7e493/extensions/3DTILES_content_gltf_vector/schema/3DTILES_content_gltf_vector.schema.json).
Its SHA-256 is `7ab77592be01c10ce854a6518300c13ec3031b4230f5fbbc12aa2a17ff712cdd`.
Attribution: Copyright 2016–2022 Cesium GS, Inc., under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/), as specified by the
[upstream licence](https://github.com/CesiumGS/3d-tiles/blob/c48ebdc8db43dc00917b4f200eff5e2131d7e493/specification/LICENSE.adoc).
The two glTF drafts define their JSON requirements in their README files and
do not supply JSON Schema files at these revisions. We do not invent upstream
schemas for them. The separate bundled 3D Tiles 1.1 validator schema in
`docs/schema/` is unchanged; this draft schema is reference material, not a new
runtime validation claim.

`tests/test_vector_batching.py` checks declarations, restart separation,
polygon offset counts, unique loop indices, triangulated boundary membership
and feature IDs. The optional `tests/fixtures/vector_compat.cjs` probe checks
native rendering, holes, boundaries, LOD and picking after initial load and
hard refresh. Upstream validator 0.6.1 reports unknown-extension warnings and
rejects valid primitive restart indices in batched lines and repaired outlines
because its glTF validator does not implement `KHR_mesh_primitive_restart`.
The polygon, point and aggregate cases have zero errors; the restart cases
cannot satisfy a zero-error stock-validator gate. Neither result establishes
full draft conformance. The [recorded check](../../bench/vector_draft_compatibility.json)
keeps this distinction explicit.
