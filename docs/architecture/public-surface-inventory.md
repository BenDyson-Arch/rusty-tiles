# Public surface disposition inventory (#126 A1)

Baseline: develop `5df15e7d21f502fc931e33e89b84ee2a20a16ee1`; point migration changes
are identified separately. This is an audit inventory, not acceptance inferred
from visibility, existing callers, or passing tests. No module is privatized by
this document. “Bounded accepted” refers only to the recorded foundation slice;
it does not certify arbitrary input, geometry, codec, or native operations.

Ownership/proof references: #113 release completeness; #120 numerical and
coordinate kernels; #121 broader mesh; #122 spatial/source pipelines; #123
remaining vector fidelity; #124 terrain/directory migration; #125 format
correctness; #126 operation and export lifecycle; #131 vector V1/V2; #132 point
foundation migration; #133 bounded validation. F0/F1a/D1/D2/V1/V2 evidence is in
this directory's contracts and evidence ledger.

## Advertised operations and foundation facade

Every entry below includes its module-qualified spelling; root aliases listed
later refer to the same operation, not a second owner. Python routes refer to
`bindings/python/src/lib.rs`, CLI routes to `src/main.rs`. A dash means no
advertised route in that adapter.

| Rust symbols / source | CLI / Python routes | Contract and actual owner | Target owner, dependencies and proof | Status, disposition and removal gate |
|---|---|---|---|---|
| `runtime::{CancellationHandle,CleanupDiagnostic,DirectoryRecovery,JobError,JobErrorKind,JobFailure,Observer,OutputPolicy,RunControl,RunEvent}` (private module, root exports) / runtime.rs + runtime/directory*.rs | Shared new CLI observers / Python run_job + typed exceptions | One run, causal failure, fallible observer, cancellation; file and completed directory publication controlled by Attempt | Runtime F0/D1/D2; domain producers own drain, reports, inventory; #113/#126 | Bounded accepted foundation facade. Retain. This does not authorize legacy operations to claim F0 acceptance. |
| `package::{package,PackageMember,PackageRequest,PackageReceipt,PackageResult}` / package.rs | `convert` / `convert_to_3tz` | Pack declared members/source tree through F0 package run; accepted inventory and receipt | Package producer + runtime, archive3tz codec; #125/#126 | Bounded accepted F0 packaging contract. Retain typed facade. Direct codec acceptance remains finite. |
| `mesh_to_archive,MeshRequest,MeshPlacement,MeshPlacementReport,MeshReport,MeshResult` / mesh_archive.rs + mesh_archive/placement.rs | `mesh-local-to-3tz` / `mesh_local_to_3tz` | Local metre/Y-up static core PBR GLB/glTF with captured confined dependencies; explicit full-detail archive under F0. F1c1 adds explicit Local/Wgs84 rigid placement | Mesh producer owns pure placement resolution before source I/O; format owner consumes one resolved map; #120/#121/#125 | F1b3 merged through #141. F1c1 #142 is an implementation candidate under its contract, pending independent review and final consumer/platform checks; schema 4/profile f1c1-placed-gltf-v1. Source identity/picking, arbitrary source CRS and approximation remain separate gates. No broad legacy deletion. |
| `raster_to_directory,RasterDirectoryRequest,RasterDirectoryReport,RasterDirectoryResult` / raster_directory.rs | `raster-tile-to-directory` / `raster_tile_to_directory` | Single aligned RGB GeoTIFF tile, completed directory publication, native feature | Raster producer + D1/D2; #124/#120/#125 | Bounded accepted D1/D2 slice. Retain; separate from legacy raster pyramid below. |
| `vector::{vector_to_archive,VectorRequest,VectorOptions,VectorLodOptions,VectorResult}` / vector.rs | `vector` / `vector_to_3tz` | Typed vector run, source/worker drain, finite acceptance and finalized report | Vector producer + F0, #131 V1/V2; #123 fidelity and #120/#125 | Bounded accepted V1/V2 lifecycle, not complete vector fidelity. Retain facade; each native/portable source policy remains constrained. |
| Baseline `point_cloud::{PointCloudOptions,point_cloud_to_3tz,point_cloud_to_3tz_reported}` / point_cloud.rs | `point-cloud` / `point_cloud_to_3tz` | Baseline uses output::Job, Reporter, force and string coordinates; preparation after staging | #132 producer + F0, #120 coordinates, #125 attributes/POSITION, #122 source/LOD | Replaced in #132 candidate: both Rust entry points and producer-owned force/report paths removed after all callers migrated. No legacy acceptance inherited. |
| Candidate `point_cloud::{PointCloudRequest,PointCloudCoordinates,PointCloudCrs,PointCloudOptions,PointCloudResult,PointCloudReport,point_cloud_to_archive}` / point_cloud.rs | Same CLI / single Python spelling with required source_crs | Typed required coordinate intent, options defaults implicit/false/50000/100000; request owns publication policy; result owns required report | Point producer + F0; #132/#120/#125 | Implemented #132 candidate; independent source/POSITION/multiplicity, lifecycle, native/portable, installed-package and resource evidence recorded in point_acceptance. Final CI remains a gate. Retain only within the declared LAS/LAZ profile. |
| `terrain::{TerrainRequest,TerrainHeights,TerrainOptions,TerrainReport,TerrainResult,terrain_to_directory}` / terrain.rs | `terrain` / — | Bounded DEM→3D Tiles 1.1 GLB directory; native source decoding, owned sampling and one F0 Attempt | Terrain producer + accepted D1/D2; #124, #120/#125 | T1 #135 removes the legacy family and migrates CLI/preview. Queries, clamping and draping are included; broader source profiles remain under #124. Final CI/review evidence is required before acceptance. |
| `raster::{RasterOptions,raster_to_directory,raster_with_options,raster_reported}` / raster.rs | `raster` / — | COG and styled PNG pyramid directory, native backend; legacy Job/report | Raster producer + D1/D2; #124/#120/#125/#126 | Rework individually bounded pyramid contract; distinguish from root raster_to_directory new facade. Remove old module family only after caller migration and independent raster equivalence/publication proof. |
| `tile::{MeshTo3tzOptions,TextureFormat,mesh_to_3tz,mesh_to_3tz_reported,DEFAULT_*}` / tile.rs | `mesh-to-3tz` / `mesh_to_3tz` | Legacy broader mesh splitting/textures/CRS/HLOD; Job/report owner | Mesh producer #121; #120 kernels/#125 codecs/#126 lifecycle | Rework with finite input/LOD/source policies. F1a does not replace all exposed use cases. Remove old family after supported uses have typed replacements and proof. |
| `tileset::{CreateTilesetOptions,create_tileset_json,glb_to_3tz,glb_to_3tz_reported,LEAF_GEOMETRIC_ERROR,TILESET_GEOMETRIC_ERROR}` / tileset.rs | `createTilesetJson`, `glb-to-3tz` / `glb_to_3tz` | Direct JSON creation and opaque GLB wrapping; legacy publication and fixed error constants | Bounded wrapping operation #126; #125 resource closure/#120 bounds | Rework each operation, required reports/error meaning and source closure first. Remove legacy functions/options after typed replacements and external consumers migrate. No inference of arbitrary GLB correctness. |
| `convert_implicit::{ConvertToImplicitOptions,convert_to_implicit,convert_to_implicit_reported}` / convert_implicit.rs | `convert-to-implicit` / `convert_to_implicit` | Rewrite eligible explicit point/vector archive; legacy Job/report | Dedicated operation #126 + implicit/archive codecs #125/#133 | Rework typed request/result, resource inventory, finite hierarchy semantics, source preservation. Remove old family after equivalence and failure/cancellation/publication proof. |
| `vector_encoding::compress_file` / vector_encoding.rs | `encode-vector-content` / — | In-place content mutation/compression, independent output policy bypass | Dedicated replacement operation #126 + codec #125 | Rework replacement/source-preservation contract before public facade acceptance. Gate removal on independent decoded equivalence and failed replacement checks. |
| `pack::{PackOptions,convert_to_3tz,convert_to_3tz_reported,pack_named_files,list_zip_names,validate_3tz,TZ_INDEX_NAME}` / pack.rs | CLI convert now package / Python convert now package | Superseded packaging entry points plus raw archive utilities; direct codec delegates remain exposed | package F0 facade for mutations; archive codec #125/#133; #126 export disposition | Replace old converters after all Rust callers migrate. Rework/private raw named-file writer unless an independently proved supported operation needs it. Read utilities need their own finite codec contract. |
| `validate::inspect(ValidationRequest)` / validate.rs | `validate` / `validate` | Read-only bounded archive/payload inspection; typed report, InvalidInput/Unsupported/ResourceLimit/Io failures; no external subprocess | C1 #133/#125, #126 tool capability | Core payload ranges/values/indices, explicit extension dispositions and honest checks/notInspected; acceptance requires independent corruption/resource controls and adapter parity. Broader metadata/material/source-fidelity and decoded world-content bounds remain unclaimed. |
| `doctor::{COMMANDS,DEFAULT_CESIUM,canonical,report,display}` / doctor.rs | `doctor` / — | Read-only readiness/capability report; backend/process probes | Tooling owner #126; native operation owners supply actual capabilities | Rework capability reporting parity with execution; retain read-only authority. Bound probes/cache side effects; no ownership of conversion defaults. |
| `preview::{Preview,MANIFESTS}` / preview.rs | `preview` / — | Selected local roots and Cesium files served read-only; Preview::new/serve own server lifetime | Tooling owner #126; resource authority and network lifetime | Retain bounded operation after root/traversal/symlink/network/lifetime proof. Public constants/object methods are provisional; no converter defaults or publication authority. |
| `error::Error` / error.rs; `report::{ConversionResult,Event,EventSink,Reporter,ndjson}` / report.rs | Remaining legacy CLI operations / legacy Python run_conversion | Legacy error and infallible observer transport; still shared by old operations/utilities | New operation errors/results use F0 JobFailure and domain results; #126 | Replace per operation, then narrow/remove legacy exports. Point removal after #132 alone cannot remove global report/output utilities still used by raster/mesh/wrapping/implicit. |

## F1c1 placement candidate and legacy use dispositions

The new public values describe one bounded local-source operation:

```rust,ignore
use rusty_tiles::{mesh_to_archive, MeshPlacement, MeshRequest, RunControl};

let request = MeshRequest::local_gltf("local.glb", "placed.3tz", 1000)
    .with_placement(MeshPlacement::Wgs84 {
        anchor_degrees_metres: [153.02, -27.47, 25.0],
        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        scene_offset_metres: [10.0, 2.0, -5.0],
    });
let result = mesh_to_archive(request, &RunControl::default())?;
```

`local_gltf` declares source metres/Y-up and defaults to `MeshPlacement::Local`.
`from_parameters` supplies the shared adapter combination grammar; numerical
admission belongs to private resolution before source I/O. No public arbitrary
matrix, legacy Euler/CRS value, generic context or source IR is added. The exact
cartographic frame, quaternion norm, forward magnitude and report fields are
specified in the [contract](f1c1-contract.md). The candidate has not completed
its independent final review or platform/consumer acceptance at this writing.

| Existing use case | F1c1 disposition / remaining gate |
| --- | --- |
| Local metre/Y-up static core PBR with admitted node transforms | Retained bounded source profile; Local explicitly keeps unplaced output. |
| Manual WGS84 anchor, orientation and local scene translation | Candidate explicit anchor/normalized XYZW/post-node Y-up metre offset; actual world-placement proof required. Explicit height zero means ellipsoidal zero. |
| Legacy HPR/Euler options | Replaced at this boundary by one quaternion; existing legacy API remains until its caller/support migration. |
| Alternate source axes or geographic/projected source coordinates | Deferred; callers may author admitted glTF node rotations for local axes. No CRS inference from position magnitudes. |
| Metashape E/N/A shifts or offset files; auto+offset selecting WebMercator | Deferred source-coordinate operation, not the new scene offset; do not import offset-file I/O or offset-driven guessing into F1. |
| General EPSG/WKT/PROJ with source_axes and source height_offset | Deferred numerical/source admission; manual anchor height is not a source height-reference conversion. |
| Compound/geocentric/vertical CRS, geoid grids or epochs | No new admission; separate capability/data/accuracy evidence required. |
| Source node features/picking, approximation/atlases/implicit delivery | Separate F1c2 and later #121 work; names alone are not picking support. |

Legacy `tile`, `mesh`, `georef`, `mesh_crs` and old Job/report families still have
advertised consumers. Their deletion requires replacing or explicitly removing
those actual uses with independent evidence; F1c1's bounded success would not
authorize a blanket export removal or close #120/#121/#126/#113.

## Public implementation building blocks

These are transitively public implementation families, not accepted stable
facades. Methods and public fields belong to their containing type's disposition.
Proposed privatization is not executed here; external callers must migrate and
required domain replacements must exist before hiding a family.

| Module / symbols (source) | Adapter route and current contract / owner | Target, proof and disposition gate |
|---|---|---|
| `bbox::{BoundingBox,bounding_box_from_gltf_path,bounding_box_from_gltf,aabb_center,aabb_diagonal,aabb_to_box,union_aabb,box_to_aabb,Obb}` (bbox.rs) | No direct route; bounds parser/math consumed by mesh/wrapping; utility owner | #120 bounds + #125 glTF parsing + #126 facade. Rework/retain only proved finite functions; otherwise private after callers migrate. OBB methods/fields included. |
| `georef::{Cartographic,RotationDegrees,SourceAxes,SourceCrs,SourceOffset,CrsKind,EnuFrame,root_transform,geodetic_to_ecef,ecef_to_cartographic,mercator_*,looks_*,geographic_*,parse_metashape_offset,geog_yup_to_enu_yup,east_north_up,translation,apply_mercator_offset}` (georef.rs) | Placement/source values reach legacy mesh CLI/Python; math and heuristic CRS detection incidental | #120/#121/#126. Retain explicit domain values only where proved; replace heuristic/default inference in scoped producer; private numerical helpers after caller migration. Root reexports are aliases, not broader proof. |
| Native `geospatial::{Versions,versions,Crs,StrictTransform,EcefTransform}` (geospatial.rs) | No direct Python/CLI route; native source/transform infrastructure and capability probes | #120/#126. Rework/retain strict native contract with operation selection/grids/axis/height/resource evidence; avoid stable FFI wrapper promise merely because module is public. |
| `mesh::{Vertex,Triangle,EncodedImage,Scene,load,load_with_node_features,GeographicBake,BakeToEnu,position_aabb_yup,bake_to_enu,bake_geographic,centroid,triangle_aabb_yup}` (mesh.rs) | No direct route; mutable IR, loader and baking implementation under legacy mesh | #121/#120/#125/#126. Private IR/loader/baking after typed mesh domain facade has admitted policies and consumers migrate; retaining a public IR requires its own contract. |
| `glb_write::{TilePrimitive,write_glb,image_mime}` (glb_write.rs) | No direct route; legacy mesh primitive/codec writing | #125 codec/#121 producer/#126 exports. Rework finite codec and MIME behavior; private implementation after callers migrate; no output publication policy in codec. |
| `metadata::{MetadataGlb,MeshFeatures,FeatureId,StructuralMetadata,Schema,Class,ClassProperty,PropertyTable,PropertyTableProperty,PropertyAttribute,PropertyAttributeProperty,tile_schema,encode_property_table,MESH_FEATURES,STRUCTURAL_METADATA,TILE_BOUNDING_BOX,TILE_GEOMETRIC_ERROR}` (metadata.rs; MetadataGlb defined glb.rs) | No direct route; public schema/IR and mutable glTF builder, consumed by point/vector codecs | #125 attributes/features/alignment/type semantics; #132 source attributes; #126 facade. Rework/private builder/IR after producer migration; a supported codec facade needs independent decoder negatives and exact scalar semantics. |
| `implicit::{SubdivisionScheme,Coordinates,Subtree,TileMetadata,expand_tileset}` (implicit.rs, implicit/tileset.rs) | No direct route; consumed by implicit converters and point/vector producers; Subtree methods new/set_tile/set_child_subtree/set_metadata/to_bytes included | #125/#133 format model; #126 export lifecycle; #132 producer support does not stabilize utility API. Retain/rework internal dependency; private exports only after callers migrate or narrow proved codec facade supplied. |
| `grid::{Hit,TriGrid,IdHasher,IdMap,point_tri_dist2_bary,sub,add,scl,dot,dist2,cross,normalize,face_normal_area}` (grid.rs) | No direct route; spatial lookup/hash and f32 geometry kernels | #120/#121/#126. Prove domain/degenerate/finite policy then retain internal kernel; proposed private incidental types/math after external caller migration. |
| `hlod::{Timing,ParentResult,parent_triangle_budget,parent_atlas_size,build_parent,simplify_primitive,two_sided_error}` (hlod.rs) | No direct route; legacy mesh simplification/error/atlas ownership | #120/#121/#125/#126. Rework error and parent semantics; private implementation after facade migration; counts/budgets do not prove geometric error. |
| `texture::{CHART_GUTTER_PX,MAX_LEAF_ATLAS,LeafGroup,Blit,LeafPlan,SceneSampler,image_dimensions,decode_rgb_max,plan_leaf_atlas,plan_leaf_atlas_limit,pack_rects,pack_rects_wh,needed_decode_size,blit_chart,paste_chart,resample_chart,finish_leaf_atlas,texel_density,bake_simplified,encode_texture,encode_perceptual,encode_texture_rgb,sampled_child}` (texture.rs) | No direct route; image codec, atlas planning/IR/resampling, legacy mesh dependency | #121 textured input/HLOD + #125 codecs/#120 sampling/#126 exports. Rework finite fidelity/limits, private planning/IR after callers migrate; no accepted broad texture facade yet. |
| `fixtures::{triangle_glb,geographic_glb}` (fixtures.rs), root `ORACLE_NPM` (lib.rs) | No advertised adapter route; test fixture generator and historical oracle pin | #126. Remove/private/test-only after external fixture callers migrate. Test oracle pin is provenance, not production source authority or format acceptance. |
| `vector::{SPEC_ISSUE,SPEC_PR,SPEC_BLOG,TILESET_EXTENSION,GLTF_RESTART,GLTF_POLYGON,GLTF_FEATURES,GLTF_METADATA,INPUT_ORDER}` (vector.rs) | No direct route; draft/provenance/extension constants and input ordering | #131/#123/#125/#126. Retain documentation constants only if intentionally supported; private incidental source preference constant after callers migrate. Draft provenance does not certify format support. |

## Root export coverage

All `pub mod` declarations in baseline lib.rs are represented above: bbox,
convert_implicit, error, fixtures, georef, geospatial (feature-gated), glb_write,
grid, hlod, implicit, mesh, metadata, pack, package, point_cloud, report, terrain,
texture, tile, tileset, vector, raster, vector_encoding, doctor, preview, validate.

Root reexports are covered by their module rows: convert_implicit's two functions
and options; Error; georef's parse_metashape_offset and five placement/source
values; mesh archive's request/result/report, placement enum/report and operation; pack's convert_to_3tz,
pack_named_files, validate_3tz, TZ_INDEX_NAME; raster directory's function and
three values; report's four values; the runtime facade values; tile's
mesh_to_3tz, options and four DEFAULT constants; tileset's two functions/options.
The private mesh_archive, raster_directory, runtime and archive3tz modules are
included because root aliases expose their values. MetadataGlb is included
because metadata reexports it from private glb. Public fields and methods of
all listed public types remain transitively exposed.

## Acceptance and removal sequence

Accepted F0/F1a/D1/D2/V1/V2 scopes remain bounded by their contracts. #132 is
pending until its acceptance ledger records the independent final review;
#133 is separately pending format evidence. Neither implementation completion
nor this inventory closes #113, #122, #123, #125 or #126.

For each replace/remove decision: enumerate external Rust and adapter callers,
migrate admitted uses to the domain replacement, prove the operation's source,
resource, failure and installed-package contract, record the break, then delete
the superseded entry points. Remaining legacy output::Job/report ownership is
explicitly retained pending its operation migrations; do not hide legacy paths
while advertised adapters still call them. Numerical and format utility
privatization follows dependency/caller proof, not a blanket pub-to-private edit.

## Declaration cross-check

This source cross-check records declaration names, including methods, to make
family coverage reviewable. It is a lexical inventory, not a claim that every
method is an independently supported operation. Private module methods captured
in a public module still inherit the family review; visibility reachability must
be checked before final API narrowing.

| Source | Public declaration names |
|---|---|
| `src/bbox.rs` | `BoundingBox`, `bounding_box_from_gltf_path`, `bounding_box_from_gltf`, `aabb_center`, `aabb_diagonal`, `aabb_to_box`, `union_aabb`, `box_to_aabb`, `Obb`, `from_aabb`, `from_box`, `fit`, `volume`, `corners`, `contains`, `aabb`, `for_tile`, `padded_flat`, `expanded`, `to_box` |
| `src/convert_implicit.rs` | `ConvertToImplicitOptions`, `convert_to_implicit`, `convert_to_implicit_reported` |
| `src/error.rs` | `Error`, `category`, `msg` |
| `src/fixtures.rs` | `triangle_glb`, `geographic_glb` |
| `src/georef.rs` | `Cartographic`, `new`, `RotationDegrees`, `root_transform`, `geodetic_to_ecef`, `ecef_to_cartographic`, `EnuFrame`, `to_enu`, `normal_to_enu_yup`, `normal_to_enu_yup_offset`, `geog_yup_to_enu_yup`, `mercator_yup_to_enu_yup`, `mercator_yup_offset_to_enu_yup`, `mercator_to_geodetic`, `looks_geographic_yup`, `looks_web_mercator_yup`, `geographic_origin_yup`, `geographic_bbox_wgs84`, `mercator_origin_yup`, `mercator_shift_origin`, `mercator_bbox_wgs84`, `CrsKind`, `SourceCrs`, `SourceAxes`, `parse_cli`, `SourceOffset`, `apply_mercator_offset`, `mercator_origin_yup_offset`, `mercator_bbox_wgs84_offset`, `parse_metashape_offset`, `east_north_up`, `translation` |
| `src/geospatial.rs` | `Versions`, `versions`, `Crs`, `from_definition`, `coordinate_epoch`, `set_coordinate_epoch`, `has_native_height`, `is_horizontal`, `StrictTransform`, `new`, `transform`, `EcefTransform` |
| `src/glb_write.rs` | `TilePrimitive`, `write_glb`, `image_mime` |
| `src/grid.rs` | `Hit`, `TriGrid`, `new`, `tri`, `len`, `is_empty`, `cell_size`, `nearest_by`, `nearest_by_seeded`, `nearest_dist2`, `IdHasher`, `IdMap`, `point_tri_dist2_bary`, `sub`, `add`, `scl`, `dot`, `dist2`, `cross`, `normalize`, `face_normal_area` |
| `src/hlod.rs` | `Timing`, `add`, `summary`, `parent_triangle_budget`, `parent_atlas_size`, `ParentResult`, `build_parent`, `simplify_primitive`, `two_sided_error` |
| `src/implicit.rs` | `SubdivisionScheme`, `as_str`, `Coordinates`, `Subtree`, `TileMetadata`, `new`, `set_tile`, `set_child_subtree`, `set_metadata`, `to_bytes` |
| `src/mesh.rs` | `Vertex`, `Triangle`, `EncodedImage`, `is_empty`, `len`, `load`, `load_prefix`, `Scene`, `triangle_count`, `under_budget`, `load_with_node_features`, `GeographicBake`, `position_aabb_yup`, `BakeToEnu`, `bake_to_enu`, `bake_geographic`, `centroid`, `triangle_aabb_yup` |
| `src/metadata.rs` | `MESH_FEATURES`, `STRUCTURAL_METADATA`, `TILE_BOUNDING_BOX`, `TILE_GEOMETRIC_ERROR`, `MeshFeatures`, `FeatureId`, `attribute`, `StructuralMetadata`, `attach`, `attach_gltf`, `Schema`, `Class`, `ClassProperty`, `PropertyTable`, `PropertyTableProperty`, `PropertyAttribute`, `PropertyAttributeProperty`, `tile_schema`, `encode_property_table` |
| `src/pack.rs` | `PackOptions`, `convert_to_3tz`, `convert_to_3tz_reported`, `pack_named_files`, `list_zip_names`, `validate_3tz` |
| `src/package.rs` | `PackageMember`, `new`, `name`, `source`, `PackageRequest`, `directory`, `members`, `with_policy`, `PackageReceipt`, `PackageResult`, `package` |
| `src/point_cloud.rs` | `PointCloudCrs`, `PointCloudCoordinates`, `PointCloudOptions`, `PointCloudRequest`, `new`, `with_policy`, `PointCloudReport`, `PointCloudResult`, `point_cloud_to_archive` |
| `src/report.rs` | `Event`, `EventSink`, `Reporter`, `silent`, `human_stderr`, `ndjson_stderr`, `custom`, `wants_progress`, `emit`, `progress`, `warn`, `note`, `ndjson`, `ConversionResult` |
| `src/terrain.rs` | `TerrainRequest`, `TerrainHeights`, `TerrainOptions`, `TerrainReport`, `TerrainResult`, `terrain_to_directory` |
| `src/texture.rs` | `CHART_GUTTER_PX`, `MAX_LEAF_ATLAS`, `image_dimensions`, `decode_rgb_max`, `LeafGroup`, `Blit`, `LeafPlan`, `plan_leaf_atlas`, `plan_leaf_atlas_limit`, `pack_rects`, `pack_rects_wh`, `needed_decode_size`, `blit_chart`, `paste_chart`, `resample_chart`, `finish_leaf_atlas`, `texel_density`, `bake_simplified`, `encode_texture`, `encode_perceptual`, `encode_texture_rgb`, `sampled_child`, `SceneSampler`, `from_prims`, `sample`, `sample_seeded` |
| `src/tile.rs` | `DEFAULT_MAX_TRIANGLES`, `DEFAULT_MAX_BYTES`, `DEFAULT_TILE_SIZE`, `DEFAULT_MAX_TEXEL_DENSITY`, `MeshTo3tzOptions`, `set_source_crs`, `TextureFormat`, `mesh_to_3tz`, `mesh_to_3tz_reported` |
| `src/tileset.rs` | `LEAF_GEOMETRIC_ERROR`, `TILESET_GEOMETRIC_ERROR`, `CreateTilesetOptions`, `create_tileset_json`, `glb_to_3tz`, `glb_to_3tz_reported` |
| `src/vector.rs` | `SPEC_ISSUE`, `SPEC_PR`, `SPEC_BLOG`, `TILESET_EXTENSION`, `GLTF_RESTART`, `GLTF_POLYGON`, `GLTF_FEATURES`, `GLTF_METADATA`, `INPUT_ORDER`, `VectorLodOptions`, `VectorOptions`, `VectorRequest`, `new`, `with_policy`, `VectorResult`, `vector_to_archive` |
| `src/raster.rs` | `RasterOptions`, `raster_to_directory`, `raster_with_options`, `raster_reported` |
| `src/vector_encoding.rs` | `compress_file` |
| `src/doctor.rs` | `COMMANDS`, `DEFAULT_CESIUM`, `canonical`, `report`, `display` |
| `src/preview.rs` | `MANIFESTS`, `Preview`, `new`, `serve` |
| `src/validate.rs` | `archive` |
| `src/runtime.rs` | `JobErrorKind`, `JobError`, `new`, `io`, `kind`, `message`, `path`, `JobFailure`, `DirectoryRecovery`, `CleanupDiagnostic`, `OutputPolicy`, `RunEvent`, `Observer`, `RunControl`, `cancellation_handle`, `CancellationHandle`, `cancel` |
| `src/mesh_archive.rs` | `MeshRequest`, `local_gltf`, `with_policy`, `with_placement`, `MeshReport`, `MeshResult`, `mesh_to_archive`; placement enums re-exported |
| `src/mesh_archive/placement.rs` | `MeshPlacement`, `from_parameters`, `MeshPlacementReport`; resolved representation and frame math remain private |
| `src/raster_directory.rs` | `RasterDirectoryRequest`, `web_mercator_rgb`, `with_policy`, `RasterDirectoryReport`, `RasterDirectoryResult`, `raster_to_directory` |
| `src/glb.rs` | `MetadataGlb`, `new`, `from_parts`, `view`, `accessor`, `replace_view`, `compact_views`, `encoded_len`, `into_parts`, `finish` |
