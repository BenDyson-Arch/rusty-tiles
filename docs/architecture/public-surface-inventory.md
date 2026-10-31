# Public surface disposition inventory (#126 A1)

Declaration-audit baseline: develop `4d9a575c5031e45ff5761ec72ac12808e628370f`
(#180 H1), checked 2026-10-13 against `src/lib.rs`, the operation declarations
and CLI/Python adapters. Historical baseline: `5df15e7d21f502fc931e33e89b84ee2a20a16ee1`.
Status/sequencing and retained declaration checks refreshed against develop
`7fad70c` (#179 merged) on 2026-10-13.
This is an audit inventory, not acceptance inferred from visibility, existing
callers, or passing tests. No module is privatized by this document. “Bounded
accepted” refers only to recorded foundation evidence; it does not certify
arbitrary input, geometry, codec, native operations or release readiness.

**Legacy mesh retirement, 2026-10-23.** `mesh-to-3tz`/`meshTo3tz`,
`mesh-local-to-3tz` and the Python `mesh_to_3tz`/`mesh_local_to_3tz` were
removed with their whole private closure: `tile`, `mesh`, `mesh_crs`, `texture`,
`hlod`, `grid`, `glb_write`, `report` (`Reporter`, `Event`, `EventSink`,
`ConversionResult`, `ndjson`), `jpeg`, `gpu_texture`, `lossless`, `tileset`, the
mesh_archive FullDetail/RootProxy producer (`mesh_to_archive`, `MeshRequest`,
`MeshApproximation`, `MeshApproximationReport`, `MeshCertificateReport`,
`MeshReport`, `MeshResult`), `output::Job`, the legacy `georef` CRS adapters and
`parse_metashape_offset`, the `bbox` glTF reader and AABB helpers,
`fixtures::geographic_glb` and the `Error::{TextureProjection, InputNotFound,
NoContent}` variants. Rows and declarations below that name them are historical.
`MeshPlacement`/`MeshPlacementReport`, W1 (`model_to_archive`,
`model_to_manifest`), `bbox::{BoundingBox, Obb}` and the `mesh` tiler remain.

The 2026-10-13 H6/H14 decision supersedes H9's older vector-only retirement and
point-explicit retention statements: **all explicit production hierarchies
retire**. H6 removes vector AND point-cloud CLI `--explicit`, Python `explicit`
parameters, `VectorOptions`/point options plumbing and dead production writers;
legacy mesh explicit output retires with its replaced writer. This does not
remove W1's bounded single-model explicit manifest/wrapping envelope, tiny
explicit fixtures, shared external-document/skeleton machinery, or validation
of third-party explicit tilesets. W1 and general packaging remain retained.
0.4 emits Tiles 1.1/glTF 2.0; the official format transition belongs to 0.6.

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
| `mesh_to_archive,MeshRequest,MeshApproximation,MeshApproximationReport,MeshCertificateReport,MeshPlacement,MeshPlacementReport,MeshReport,MeshResult` / mesh_archive.rs | `mesh-local-to-3tz` / `mesh_local_to_3tz` | Bounded local metre/Y-up core PBR source, rigid placement, identity/picking; F1d2 schema 7/profile f1d2-adaptive-root-proxy-gltf-v1 full-detail/root-proxy archive | Mesh #121, placement #120, codec #125; [F1c2 evidence](f1c2-evidence.md), [F1d2 evidence](../../bench/architecture_audit/mesh_approximation_f1d2/README.md) | F1c1 #143, F1c2 #147 and certified approximation #149/#150 merged within their finite contracts. Retire this old archive writer, FullDetail/RootProxy modes, approximation facade and leaf/proxy flags after the complete production tiler is independently accepted. Do not migrate this writer to M1. H1 capture/tree and H8 leaves alone do not satisfy that gate. |
| `mesh_to_tileset,TilerRequest,TilerResult,TilerReport,TilerDiagnostics,TilerJobs,TilerProfile,MeshSourceCrs,MeshSourceCrsReport` (root exports; private mesh_large) | `mesh` / `mesh_to_tileset` | Streaming large-mesh tiler: local metre/Y-up static glTF/GLB or declared source CRS, rigid placement, certified LOD, textured proxies, KTX2/Basis with meshopt, implicit Tiles 1.1 `.3tz` | Large-mesh tiler (roadmap milestone 3): H1, H8, H16, H17, H26, H27 and source CRS #218 | The one public mesh entry point (owner decision 2026-10-14). Retain. Profiles stay provisional; milestone-4 scale, consumer and validator acceptance is pending. |
| `raster_to_directory,RasterDirectoryRequest,RasterDirectoryReport,RasterDirectoryResult` / raster_directory.rs | `raster-tile-to-directory` / `raster_tile_to_directory` | Single aligned RGB GeoTIFF tile, completed directory publication, native feature | Raster producer + D1/D2; #124/#120/#125 | Bounded accepted D1/D2 slice. Retain; separate from the full raster pyramid below. |
| `vector::{vector_to_archive,VectorRequest,VectorOptions,VectorLodOptions,VectorResult}` / vector.rs | `vector` / `vector_to_3tz` | Typed vector run, source/worker drain, finite acceptance and finalized report | Vector producer + F0, #131 V1/V2; #123 fidelity and #120/#125 | Bounded accepted V1/V2 lifecycle, not complete vector fidelity. Retain bounded implicit facade and source CRS. H6 (#188) retired CLI/Python/options explicit production. Each native/portable source policy remains constrained. |
| Baseline `point_cloud::{PointCloudOptions,point_cloud_to_3tz,point_cloud_to_3tz_reported}` / point_cloud.rs | `point-cloud` / `point_cloud_to_3tz` | Baseline uses output::Job, Reporter, force and string coordinates; preparation after staging | #132 producer + F0, #120 coordinates, #125 attributes/POSITION, #122 source/LOD | Historical; replaced by merged #134 (issue #132): both Rust entry points and producer-owned force/report paths removed after all callers migrated. No legacy acceptance inherited. |
| Current `point_cloud::{PointCloudRequest,PointCloudCoordinates,PointCloudCrs,PointCloudOptions,PointCloudResult,PointCloudReport,point_cloud_to_archive}` / point_cloud.rs | Same CLI / single Python spelling with required source_crs | Typed required coordinate intent, options defaults implicit/metadata false/attributes false/8192 (maximum)/100000; request owns publication policy; result owns required report | Point producer + F0; #132/#120/#125 | Bounded P1 migration merged #134; [independent evidence](../../bench/architecture_audit/point_acceptance/README.md) covers the declared LAS/LAZ profile, not all source/LOD fidelity. Retain required source_crs and coordinate intent. H6 (#188) retired CLI/Python/options explicit production. |
| `terrain::{TerrainRequest,TerrainHeights,TerrainOptions,TerrainReport,TerrainResult,terrain_to_directory}` / terrain.rs | `terrain` / — | Bounded DEM→3D Tiles 1.1 GLB directory; native source decoding, owned sampling and one F0 Attempt | Terrain producer + accepted D1/D2; #124, #120/#125 | Bounded T1 merged #136 (issue #135); LOD/skirts/implicit forest merged #169. [Independent terrain evidence](../../bench/architecture_audit/terrain_acceptance/README.md) scopes acceptance; broader source profiles remain under #124. Retain terrain GLB directory, queries, clamping and draping. |
| `raster_to_pyramid,RasterRequest,RasterDisplay,RasterLimits,RasterReport,RasterResult` / private raster.rs | `raster` / — | Standalone COG and bounded direct-nearest PNG pyramid, typed F0/D1/D2 operation | Raster producer + D1/D2; #124/#120/#125/#126 | Bounded R2 implementation merged #155, executed contract regressions #156 and imagery/tooling correction #159. Retain COG/PNG pyramid and display modes within the [R2 contract](r2-raster-contract.md); source/resource/publication/pixel claims are finite. Separate from accepted single-tile raster_to_directory; mesh texture retirement does not retire raster PNG. |
| `tile::{MeshTo3tzOptions,TextureFormat,mesh_to_3tz,mesh_to_3tz_reported,DEFAULT_*}` / tile.rs | `mesh-to-3tz` / `mesh_to_3tz` | Legacy broader mesh splitting/textures/CRS/HLOD; Job/report owner | Mesh producer #121; #120 kernels/#125 codecs/#126 lifecycle | Retire mesh-to-3tz, aliases, all old options and implicit/explicit writers after complete accepted production tiler; the removal is a 0.4.0 gate (owner decision 2026-10-21). Automatic CRS detection and the legacy georeferencing paths retire with it; declared-CRS input (EPSG/WKT/PROJ with Metashape E/N/A offsets) moved to `mesh --source-crs` (#218, owner decision 2026-10-14). Keep new local metre/rigid placement intent and W1 wrapping. |
| `model_to_archive,model_to_manifest,ModelWrapRequest,ModelWrapResult,ModelManifestRequest,ModelManifestResult,ModelReport` (root exports; private mesh_archive/model.rs) | `createTilesetJson`, `glb-to-3tz` / `model_to_manifest`, `glb_to_3tz` | W1 bounded static local metre/Y-up source, original-node conservative bounds, exact captured archive bytes or fixed sibling reference manifest; one F0 Attempt | #145/#126; source/placement owners #120/#125 | Replaces public legacy wrapping/manifest APIs. Directory/multimodel, arbitrary manifest output, animation/skin/morph/imported extensions, data URI and oversized sources deliberately retired at this boundary. Bounded accepted W1 merged #147 with [final source/consumer/wheel/platform evidence](f1c2-evidence.md). Retain its single-model explicit envelope and exact source bytes; H6 hierarchy retirement does not delete it. Private tileset.rs remains only for legacy broader mesh shortcuts; it is not public or W1 accepted. |
| `convert_implicit::{ConvertToImplicitOptions,convert_to_implicit,convert_to_implicit_reported}` / convert_implicit.rs | `convert-to-implicit` / `convert_to_implicit` | Rewrite eligible explicit point/vector archive; legacy Job/report | Dedicated operation #126 + implicit/archive codecs #125/#133 | **Removed 2026-10-11 (user decision for 0.4.0).** The [A2 replacement contract](implicit-rewrite-contract.md#disposition-2026-10-11) stopped at an unresolved 3D Tiles gate: its terminal external template would make an unavailable root slot name JSON. The point-cloud, vector and mesh producers already emit implicit tiling by default, so callers re-run from source instead. The CLI subcommand, Rust module and root exports, and the Python function were deleted together; no wrapper remains. `validate` still accepts single-node archives written by earlier runs (`conversion.json` `retainedContentUris`); multi-node ones carry JSON content templates and are Unsupported under I1 (X-1). |
| `compress_vector_file,VectorCompressionRequest,CompressionLimits,VectorCompressionReport,VectorCompressionResult` / private vector_compression.rs; private vector_encoding.rs | Hidden `encode-vector-content` / — | One captured regular non-symlink GLB, finite lossless physical-view codec, one consumed RunControl and F0 Replace | Dedicated operation #126 + codec #125; [foundation record](vector-compression-foundation.md) | Accepted through #154 with separate final source, actual consumer/replacement and platform/wheel/official Blender evidence. Replaces legacy public `vector_encoding::compress_file`; producer calls the same private pure codec within its existing Attempt. Remaining content/rendering, full A2 and broader legacy removal gates remain open. |
| `pack::{list_zip_names,validate_3tz,TZ_INDEX_NAME}` / pack.rs | CLI convert / Python convert_to_3tz both use package | Read-only archive utilities | package F0 facade owns mutations; archive codec #125/#133; #146/#126 export disposition | Removed PackOptions, convert_to_3tz, convert_to_3tz_reported and pack_named_files after Rust caller migration. The private pack::tree_members alias was removed with its last caller, convert_implicit; legacy mesh publication uses output::tree_members directly. Read utilities do not gain broader codec acceptance. |
| `validate::inspect(ValidationRequest)` / validate.rs; types.rs reexports | `validate` / `validate` | Read-only bounded archive/payload inspection; typed report and InvalidInput/Unsupported/ResourceLimit/Io/InvalidState failures | C1 #133/#125, C2 and I1; #126 tooling | Bounded C1 merged #137/#153 (meshopt header correction #176), C2 slice 1 #157, I1 #161 and work-bound correction #164. [C2 evidence](../../bench/architecture_audit/c2_decoded_containment/README.md) and [I1 evidence](../../bench/architecture_audit/implicit_read/README.md) establish their finite scopes. I1 reads implicit roots on explicit tiles/external documents and rejects JSON content templates; C2 slice 3 #179 adds scoped accessor-backed TILE_BOUNDING_BOX containment for implicit references. Derived implicit cells, RTC, regions and rotated boxes remain unclaimed. Broader metadata/material/source fidelity remains unclaimed. Third-party explicit reading is retained. H7 classifies all b3dm/i3dm/pnts/cmpt content as Unsupported by member suffix before any read, by the first four bytes of the existing read, and by implicit content template ([amendment](c1-legacy-content-unsupported-amendment.md)); b3dm inspection and the `rtcCenter` reason are removed. |
| `doctor::{Command,Status,Probe,Capability,NativeGeospatial,Readiness,ProjInventory,Report,report}` / doctor.rs | `doctor` / — | Read-only readiness: ready/unavailable/unsupported per selected command, named bounded probes | Tooling owner #126; native operation owners supply actual capabilities | T1 ([contract](t1-tooling-contract.md)) replaced `COMMANDS`/`canonical` with `Command`, removed `DEFAULT_CESIUM` (the CLI owns that default) and `display` (now `Display`), and typed the report. Readiness and refusal categories come from execution's own checks (`vector::native_available` with `job_error`, `raster::output_capability` = `init` + `output_drivers`); terrain/D1 compose GTiff from the raster driver inventory until their owners export resolvers (their GTiff `invalid_input` category is a recorded divergence). Read-only authority is retained. Bounded tooling contract merged #159; retain read-only authority, with future command/readiness narrowing tied to actual producer retirement. |
| `preview::{Preview,MANIFESTS}` / preview.rs | `preview` / — | Selected local roots and Cesium files served read-only; `Preview::new` validates roots, `serve(SocketAddr, json)` owns the listener until the process is signalled | Tooling owner #126; resource authority and network lifetime | T1 ([contract](t1-tooling-contract.md)) retained the transport and reworked the fence to OS-independent lexical admission plus exact canonical names (no symlinks below a root), after live traversal/symlink/alias/unselected-layer attacks. It added the loopback `Host` fence (421) and an IP-only explicit non-loopback opt-in, and proved SIGTERM/SIGINT lifetime. `MANIFESTS` remains provisional data. There are no converter defaults or publication authority. Bounded tooling contract merged #159; retain read-only authority, with future command/readiness narrowing tied to actual producer retirement. |
| `error::Error` / error.rs; `report::{ConversionResult,Event,EventSink,Reporter,ndjson}` / report.rs | Remaining legacy CLI operations / legacy Python run_conversion | Legacy error and infallible observer transport; still shared by old operations/utilities | New operation errors/results use F0 JobFailure and domain results; #126 | Replace per operation, then narrow/remove legacy exports. Point removal after #132 alone cannot remove global report/output utilities still used by legacy mesh. |

## Bounded placement and legacy use dispositions

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
specified in the [contract](f1c1-contract.md). F1c1 merged #143; its retained placement proof was replayed with final F1c2/W1
acceptance in #147. These finite contracts do not admit arbitrary source CRS.

| Existing use case | F1c1 disposition / remaining gate |
| --- | --- |
| Local metre/Y-up static core PBR with admitted node transforms | Retained bounded source profile; Local explicitly keeps unplaced output. |
| Manual WGS84 anchor, orientation and local scene translation | Retain explicit anchor/normalized XYZW/post-node Y-up metre offset under bounded accepted placement proof. Explicit height zero means ellipsoidal zero. |
| Legacy HPR/Euler options | Replaced at this boundary by one quaternion; existing legacy API remains until its caller/support migration. |
| Alternate source axes or geographic/projected source coordinates | The legacy mesh admission retires with it. The new tiler admits a declared 2D horizontal source CRS with `--source-axes y-up\|xyz` (#218, owner decision 2026-10-14); callers may still author glTF node rotations for local axes. No CRS inference from position magnitudes. |
| Metashape E/N/A shifts or offset files; auto+offset selecting WebMercator | Legacy offset guessing and auto+offset WebMercator selection retire with legacy mesh. `mesh --source-offset` / `--source-offset-file` take explicit E/N/A values with a declared CRS (#218); these are not the retained scene offset. |
| General EPSG/WKT/PROJ with source_axes and source height_offset | Legacy admission retires with legacy mesh. `mesh --source-crs` admits EPSG/WKT/PROJ 2D horizontal operations through the shared conservative resolver, with `--height-offset` (#218); vector/point CRS remains. Manual anchor height is not source height conversion. |
| Compound/geocentric/vertical CRS, geoid grids or epochs | No new admission; separate capability/data/accuracy evidence required. |
| Source node features/picking, approximation/atlases/implicit delivery | F1c2 identity/picking and F1d2 approximation have bounded merged proof; old approximation/atlas/archive modes retire with the complete tiler. New tiler picking, LOD, baking and implicit delivery require their own proof. |

The legacy `tile`, `mesh`, `mesh_crs` and old Job/report families and the legacy
`georef` adapters were removed on 2026-10-23 (see the note at the top). The
legacy-only dispositions in the table above are historical.

## Public implementation building blocks

These are transitively public implementation families, not accepted stable
facades. Methods and public fields belong to their containing type's disposition.
Proposed privatization is not executed here; external callers must migrate and
required domain replacements must exist before hiding a family.

| Module / symbols (source) | Adapter route and current contract / owner | Target, proof and disposition gate |
|---|---|---|
| `bbox::{BoundingBox,bounding_box_from_gltf_path,bounding_box_from_gltf,aabb_center,aabb_diagonal,aabb_to_box,union_aabb,box_to_aabb,Obb}` (bbox.rs) | No direct route; bounds parser/math consumed by mesh/wrapping; utility owner | #120 bounds + #125 glTF parsing + #126 facade. Rework/retain only proved finite functions; otherwise private after callers migrate. OBB methods/fields included. |
| `georef::{Cartographic,RotationDegrees,SourceAxes,SourceCrs,SourceOffset,CrsKind,EnuFrame,root_transform,geodetic_to_ecef,ecef_to_cartographic,mercator_*,looks_*,geographic_*,parse_metashape_offset,geog_yup_to_enu_yup,east_north_up,translation,apply_mercator_offset}` (georef.rs) | Placement/source values reach legacy mesh CLI/Python; math and heuristic CRS detection incidental | #120/#121/#126. Retire mesh auto-CRS/heuristic/EPSG/Metashape admission after complete accepted tiler. Preserve numerical/placement functions used by W1 and vector/point CRS; remove old-only helpers only after retained-call-site proof. Root reexports are aliases, not broader proof. |
| Native `geospatial::{Versions,versions,Crs,StrictTransform,EcefTransform}` (geospatial.rs) | No direct Python/CLI route; native source/transform infrastructure and capability probes | #120/#126. Rework/retain strict native contract with operation selection/grids/axis/height/resource evidence; avoid stable FFI wrapper promise merely because module is public. |
| `mesh::{Vertex,Triangle,EncodedImage,Scene,load,load_with_node_features,GeographicBake,BakeToEnu,position_aabb_yup,bake_to_enu,bake_geographic,centroid,triangle_aabb_yup}` (mesh.rs) | No direct route; mutable IR, loader and baking implementation under legacy mesh | #121/#120/#125/#126. Retire old-only IR/loader/geographic baking after complete accepted tiler and retained-call-site proof; do not assume new capture needs this legacy IR. Any retained public IR requires its own contract. |
| `glb_write::{TilePrimitive,write_glb,image_mime}` (glb_write.rs) | No direct route; legacy mesh primitive/codec writing | #125 codec/#121 producer/#126 exports. Retain only finite codec/MIME dependencies proved for retained producers or new tiler; remove old-only writer after complete tiler and caller proof. No output publication policy in codec. |
| `metadata::{MetadataGlb,MeshFeatures,FeatureId,StructuralMetadata,Schema,Class,ClassProperty,PropertyTable,PropertyTableProperty,PropertyAttribute,PropertyAttributeProperty,tile_schema,encode_property_table,MESH_FEATURES,STRUCTURAL_METADATA,TILE_BOUNDING_BOX,TILE_GEOMETRIC_ERROR}` (metadata.rs; MetadataGlb defined glb.rs) | No direct route; public schema/IR and mutable glTF builder, consumed by point/vector codecs | #125 attributes/features/alignment/type semantics; #132 source attributes; #126 facade. Rework/private builder/IR after producer migration; a supported codec facade needs independent decoder negatives and exact scalar semantics. |
| `implicit::{SubdivisionScheme,Coordinates,Subtree,TileMetadata,expand_tileset,write_forest,ForestOutput,WrittenTile,WrittenContent,TileRole,RebaseRecord,rebase_payload,restore_payload}` (implicit.rs, implicit/tileset.rs) | No direct route; consumed by point/vector/mesh producers and vector reuse; Subtree methods new/set_tile/set_child_subtree/set_metadata/to_bytes included. P (2026-10-11, [layout contract](p-impl-layout-contract.md)): `write_forest` writes the bounded implicit forest and replaces the crate-private `write_tileset`/`write_tileset_recorded` and chunked external-JSON roots; `rebase_payload`/`restore_payload` are the exact reversible scene-root rebase used by vector reuse; `expand_tileset` follows explicit children, link tiles and documents and no longer follows JSON templates. The validator no longer uses `expand_tileset` (I1 reads independently); it is not validation truth | #125/#133 format model; #126 export lifecycle; #132 producer support does not stabilize utility API. Retain implicit output and shared explicit skeleton/external-document dependencies for W1, tiny controls and third-party reads. H6 (#188) removed vector/point explicit production branches; no blanket utility deletion. Private exports require caller migration or a proved codec facade. |
| `grid::{Hit,TriGrid,IdHasher,IdMap,point_tri_dist2_bary,sub,add,scl,dot,dist2,cross,normalize,face_normal_area}` (grid.rs) | No direct route; spatial lookup/hash and f32 geometry kernels | #120/#121/#126. Prove domain/degenerate/finite policy then retain internal kernel; proposed private incidental types/math after external caller migration. |
| `hlod::{Timing,ParentResult,parent_triangle_budget,parent_atlas_size,build_parent,simplify_primitive,two_sided_error}` (hlod.rs) | No direct route; legacy mesh simplification/error/atlas ownership | #120/#121/#125/#126. Retire old-only HLOD after complete tiler LOD/baking acceptance and retained-call-site proof; keep only dependencies independently proved for the new tiler. Counts/budgets do not prove geometric error. |
| `texture::{CHART_GUTTER_PX,MAX_LEAF_ATLAS,LeafGroup,Blit,LeafPlan,SceneSampler,image_dimensions,decode_rgb_max,plan_leaf_atlas,plan_leaf_atlas_limit,pack_rects,pack_rects_wh,needed_decode_size,blit_chart,paste_chart,resample_chart,finish_leaf_atlas,texel_density,bake_simplified,encode_texture,encode_perceptual,encode_texture_rgb,sampled_child}` (texture.rs) | No direct route; image codec, atlas planning/IR/resampling, legacy mesh dependency | #121 textured input/HLOD + #125 codecs/#120 sampling/#126 exports. Retire old JPEG/WebP/general-PNG re-encoding and external basisu selectors with the replacement. H8 must land the hardened in-process KTX2 encoder (#163) with its first real caller, all PBR slots, profile floors and PNG only for lossless. Retain source image decoding, W1 exact image bytes and raster PNG. Remove old-only planning/baking code only after retained-call-site proof. |
| `fixtures::{triangle_glb,geographic_glb}` (fixtures.rs), root `ORACLE_NPM` (lib.rs) | No advertised adapter route; test fixture generator and historical oracle pin | #126. Remove/private/test-only after external fixture callers migrate. Test oracle pin is provenance, not production source authority or format acceptance. |
| `vector::{SPEC_ISSUE,SPEC_PR,SPEC_BLOG,TILESET_EXTENSION,GLTF_RESTART,GLTF_POLYGON,GLTF_FEATURES,GLTF_METADATA,INPUT_ORDER}` (vector.rs) | No direct route; draft/provenance/extension constants and input ordering | #131/#123/#125/#126. Retain documentation constants only if intentionally supported; private incidental source preference constant after callers migrate. Draft provenance does not certify format support. |

## Root export coverage

At the current source, `pub mod` declares bbox, error, fixtures, georef,
feature-gated geospatial, glb_write, grid, hlod, implicit, mesh, metadata, pack,
package, point_cloud, report, terrain, texture, tile, vector, doctor, preview and
validate. `convert_implicit` is absent; tileset/vector_encoding are private.

Root aliases expose Error; georef placement/source values and offset parser;
mesh archive's three operations, mesh/model request/result/report values,
MeshApproximation/MeshApproximationReport/MeshCertificateReport and placement;
pack's validator/index constant; raster pyramid and single-directory facades;
report/runtime values; legacy tile operation/options/defaults; and vector
compression facade. `ORACLE_NPM` is a public provenance constant. Private
mesh_archive, raster, raster_directory, runtime, vector_compression and glb
appear below only where aliases expose values; MetadataGlb is reexported by
metadata. Public fields/methods inherit their containing family's disposition.
`mesh_large` and shared `spill` are private modules. The tiler's public surface is
the `mesh_to_tileset` family of root exports, CLI `mesh` and Python
`mesh_to_tileset` (table row above); the H8 leaf-directory and H16 delivery APIs
stay internal.

## Acceptance and removal sequence

Merged implementation and finite accepted scope are distinct. The identities
below bind this inventory's status corrections to merged PRs and recorded
contracts/evidence; no acceptance was rerun for this docs-only update.

| Slice | Merged identity / proof reference | Scope or remaining gate |
|---|---|---|
| P1 point migration | #134 `c4912f8c2689cfdfc7fec5d69387734ad6c55590`; [P1 evidence](../../bench/architecture_audit/point_acceptance/README.md) | Bounded source/lifecycle/POSITION/attributes; no full point LOD/source fidelity. |
| T1 terrain | #136 `00a9d1c34c3017a9475e6099c2805d6d2366d70a`; #169 `d3ae2e8f46c10730067a98524cf5867cd01b5999`; [terrain evidence](../../bench/architecture_audit/terrain_acceptance/README.md) | Finite T1 and later LOD/skirts scope; not arbitrary DEM acceptance. |
| F1c2/W1/package aliases | #147 `e3d222a4c27a86e1f06e1e4b47db4e6a4fa5e24e`; [final evidence](f1c2-evidence.md) | Bounded identity, exact wrap/manifest and placement; W1 remains. |
| F1d2 approximation | #150 `34a76152d18b02553691f472207225f116307a67`; [evidence](../../bench/architecture_audit/mesh_approximation_f1d2/README.md) | Schema 7 finite root-proxy certificates; old writer retires after complete tiler. |
| R2 raster | #155 `f2238cd0c718a076024f844d120b3a819527852e`; #156 `4fcc07709e04a54dd52a048dd96c4f6ff7e75ecb`; [contract](r2-raster-contract.md) | Merged bounded implementation and executed regressions; contract preparation alone is not proof. |
| Doctor/preview and rewrite deletion | #159 `e1bda75fa4f006feced14a68190db634d89e93cc`; [tooling contract](t1-tooling-contract.md) | Bounded read-only capabilities/transport; includes R2 imagery correction, not general network authority. |
| C2/I1/work bound | #157 `f2ed077eecf369375620c875b454d9df127c4b5f`; #161 `f23a4f8f771ea818a7a956c262bdca2c83e5ba83`; #164 `e29cd19d2f236dc688e91a80a45170d4a573e20b` | Finite explicit containment, independent implicit addressing and bounded exact work; accessor-backed TILE_BOUNDING_BOX implicit reference containment. C2 slice 3 #179 merged at `7fad70c` (accepted branch `daea592`); derived cells/RTC/regions/rotated boxes remain unclaimed. H7 is future work. |
| H1 capture/sort/tree | #180 `4d9a575c5031e45ff5761ec72ac12808e628370f`; [independent evidence](../../bench/architecture_audit/mesh_large_h1/README.md) | Private bounded streamed foundation only: no payloads, textures, LOD, tileset delivery or certification. H8 is future work. |

H6 is unblocked: #185 (#75 GLB fills), #186 (outline runs) and #183
(point outward bounds) are merged. It removes all vector/point explicit
production before the M1 serializer. C2 slice 3 #179 is merged; H7 (legacy
content Unsupported) carries its contract amendments, independent oracle and
controls, and goes through the coordinator's nonauthor audit.
Tiny explicit controls and third-party explicit reads remain. Neither removal
is complete at this source. Claude owns H6, H7 and H11; H14 records
status and decisions only.

Mesh retirement is done. The owner waived the remaining acceptance gates on
2026-10-23 and the legacy commands, their leaf-triangles/root-proxy-triangles/
max-proxy-error-metres parameters, FullDetail/RootProxy modes and approximation
types were deleted; the 0.4.0 tag ships without `mesh-to-3tz` and
`mesh-local-to-3tz`. Declared georeferenced mesh input uses `mesh --source-crs`
(#218); automatic CRS detection was not migrated. Local metres and rigid
placement live on in `mesh`/`mesh_to_tileset` and W1. The
[retirement record](legacy-mesh-retirement.md) lists what was removed.

For each removal, enumerate retained Rust/adapter callers, migrate admitted
uses or record the deliberate lost domain, prove the replacement, record the
breaking change and delete the superseded path. Remove old-only
mesh/texture/HLOD/georef/output/Reporter/native-JPEG/external-basisu routes only
after concrete retained-call-site proof. Preserve W1 model wrapping/manifest,
packaging, source image decoding, raster PNG, vector/point CRS and tooling.
Numerical/format utility privatization is not a blanket pub-to-private edit.
No status here closes #113/#121/#122/#123/#125/#126 or authorizes release.

## Declaration cross-check

This current-source lexical cross-check records declaration names, including methods, to make
family coverage reviewable. It is a lexical inventory, not a claim that every
method is an independently supported operation. Rows for removed/private legacy islands are explicitly historical. This lists
the named files, not every nested module declaration; visibility reachability must
be checked before final API narrowing.

| Source | Public declaration names |
|---|---|
| `src/bbox.rs` | `BoundingBox`, `bounding_box_from_gltf_path`, `bounding_box_from_gltf`, `aabb_center`, `aabb_diagonal`, `aabb_to_box`, `union_aabb`, `box_to_aabb`, `Obb`, `from_aabb`, `from_box`, `fit`, `volume`, `corners`, `contains`, `aabb`, `for_tile`, `padded_flat`, `expanded`, `to_box` |
| `src/convert_implicit.rs` (removed 2026-10-11) | `ConvertToImplicitOptions`, `convert_to_implicit`, `convert_to_implicit_reported` |
| `src/error.rs` | `Error`, `category`, `msg` |
| `src/fixtures.rs` | `triangle_glb`, `geographic_glb` |
| `src/georef.rs` | `Cartographic`, `new`, `RotationDegrees`, `root_transform`, `geodetic_to_ecef`, `ecef_to_cartographic`, `EnuFrame`, `to_enu`, `normal_to_enu_yup`, `normal_to_enu_yup_offset`, `geog_yup_to_enu_yup`, `mercator_yup_to_enu_yup`, `mercator_yup_offset_to_enu_yup`, `mercator_to_geodetic`, `looks_geographic_yup`, `looks_web_mercator_yup`, `geographic_origin_yup`, `geographic_bbox_wgs84`, `mercator_origin_yup`, `mercator_shift_origin`, `mercator_bbox_wgs84`, `CrsKind`, `SourceCrs`, `SourceAxes`, `parse_cli`, `SourceOffset`, `apply_mercator_offset`, `mercator_origin_yup_offset`, `mercator_bbox_wgs84_offset`, `parse_metashape_offset`, `east_north_up`, `translation` |
| `src/geospatial.rs` | `Versions`, `versions`, `Crs`, `from_definition`, `coordinate_epoch`, `set_coordinate_epoch`, `has_native_height`, `is_horizontal`, `StrictTransform`, `new`, `transform`, `EcefTransform` |
| `src/glb_write.rs` (removed 2026-10-23) | `TilePrimitive`, `write_glb`, `image_mime` |
| `src/grid.rs` (removed 2026-10-23) | `Hit`, `TriGrid`, `new`, `tri`, `len`, `is_empty`, `cell_size`, `nearest_by`, `nearest_by_seeded`, `nearest_dist2`, `IdHasher`, `IdMap`, `point_tri_dist2_bary`, `sub`, `add`, `scl`, `dot`, `dist2`, `cross`, `normalize`, `face_normal_area` |
| `src/hlod.rs` (removed 2026-10-23) | `Timing`, `add`, `summary`, `parent_triangle_budget`, `parent_atlas_size`, `ParentResult`, `build_parent`, `simplify_primitive`, `two_sided_error` |
| `src/implicit.rs` | `SubdivisionScheme`, `as_str`, `Coordinates`, `Subtree`, `TileMetadata`, `new`, `set_tile`, `set_child_subtree`, `set_metadata`, `to_bytes` |
| `src/mesh.rs` (removed 2026-10-23) | `Vertex`, `Triangle`, `EncodedImage`, `is_empty`, `len`, `load`, `load_prefix`, `Scene`, `triangle_count`, `under_budget`, `load_with_node_features`, `GeographicBake`, `position_aabb_yup`, `BakeToEnu`, `bake_to_enu`, `bake_geographic`, `centroid`, `triangle_aabb_yup` |
| `src/metadata.rs` | `MESH_FEATURES`, `STRUCTURAL_METADATA`, `TILE_BOUNDING_BOX`, `TILE_GEOMETRIC_ERROR`, `MeshFeatures`, `FeatureId`, `attribute`, `StructuralMetadata`, `attach`, `attach_gltf`, `Schema`, `Class`, `ClassProperty`, `PropertyTable`, `PropertyTableProperty`, `PropertyAttribute`, `PropertyAttributeProperty`, `tile_schema`, `encode_property_table`; MetadataGlb re-exported from private glb |
| `src/pack.rs` (current #146) | `list_zip_names`, `validate_3tz`; `TZ_INDEX_NAME` re-exported |
| `src/package.rs` | `PackageMember`, `new`, `name`, `source`, `PackageRequest`, `directory`, `members`, `with_policy`, `PackageReceipt`, `PackageResult`, `package` |
| `src/point_cloud.rs` | `PointCloudCrs`, `PointCloudCoordinates`, `PointCloudOptions`, `PointCloudRequest`, `new`, `with_policy`, `PointCloudReport`, `PointCloudResult`, `point_cloud_to_archive` |
| `src/report.rs` (removed 2026-10-23) | `Event`, `EventSink`, `Reporter`, `silent`, `human_stderr`, `ndjson_stderr`, `custom`, `wants_progress`, `emit`, `progress`, `warn`, `note`, `ndjson`, `ConversionResult` |
| `src/terrain.rs` | `TerrainOptions`, `new`, `TerrainHeights`, `TerrainRequest`, `with_policy`, `TerrainReport`, `TerrainResult`, `terrain_to_directory` |
| `src/texture.rs` (removed 2026-10-23) | `CHART_GUTTER_PX`, `MAX_LEAF_ATLAS`, `image_dimensions`, `decode_rgb_max`, `LeafGroup`, `Blit`, `LeafPlan`, `plan_leaf_atlas`, `plan_leaf_atlas_limit`, `pack_rects`, `pack_rects_wh`, `needed_decode_size`, `blit_chart`, `paste_chart`, `resample_chart`, `finish_leaf_atlas`, `texel_density`, `bake_simplified`, `encode_texture`, `encode_perceptual`, `encode_texture_rgb`, `sampled_child`, `SceneSampler`, `from_prims`, `sample`, `sample_seeded` |
| `src/tile.rs` (removed 2026-10-23) | `DEFAULT_MAX_TRIANGLES`, `DEFAULT_MAX_BYTES`, `DEFAULT_TILE_SIZE`, `DEFAULT_MAX_TEXEL_DENSITY`, `MeshTo3tzOptions`, `set_source_crs`, `TextureFormat`, `mesh_to_3tz`, `mesh_to_3tz_reported` |
| `src/tileset.rs` (historical baseline; removed 2026-10-23) | `LEAF_GEOMETRIC_ERROR`, `TILESET_GEOMETRIC_ERROR`, `CreateTilesetOptions`, `create_tileset_json`, `glb_to_3tz`, `glb_to_3tz_reported` |
| `src/vector.rs` | `SPEC_ISSUE`, `SPEC_PR`, `SPEC_BLOG`, `TILESET_EXTENSION`, `GLTF_RESTART`, `GLTF_POLYGON`, `GLTF_FEATURES`, `GLTF_METADATA`, `INPUT_ORDER`, `VectorLodOptions`, `VectorOptions`, `VectorRequest`, `new`, `with_policy`, `VectorResult`, `vector_to_archive` |
| `src/raster.rs` | `RasterDisplay`, `RasterLimits`, `RasterRequest`, `image`, `gray`, `with_alpha_band`, `with_policy`, `with_limits`, `RasterBandReport`, `RasterBands`, `as_slice`, `RasterSourceReport`, `RasterGridReport`, `RasterReport`, `RasterResult`, `raster_to_pyramid` |
| `src/vector_encoding.rs` (historical baseline; now private codec) | Legacy `compress_file` removed; no public mutation helper |
| `src/vector_compression.rs` (current private module, root exports) | `VectorCompressionRequest`, `new`, `with_limits`, `VectorCompressionReport`, `VectorCompressionResult`, `report_json`, `compress_vector_file`; `CompressionLimits` re-exported |
| `src/doctor.rs` | `Command`, `ALL`, `name`, `aliases`, `from_name`, `Status`, `Probe`, `Capability`, `NativeGeospatial`, `Readiness`, `ProjInventory`, `Report`, `refusal`, `report` |
| `src/preview.rs` | `MANIFESTS`, `Preview`, `new`, `serve` |
| `src/validate.rs` | `inspect`, `ValidationRequest`, `ValidationFailure`, `ValidationLimits`, `PayloadReport`, `ValidationReport`; types.rs reexports, new/category methods; ValidationLimits includes all five containment fields (span_bits/work_units/bytes_read/decoded_bytes/node_depth) |
| `src/runtime.rs` | `JobErrorKind`, `JobError`, `new`, `io`, `kind`, `message`, `path`, `JobFailure`, `DirectoryRecovery`, `CleanupDiagnostic`, `OutputPolicy`, `RunEvent`, `Observer`, `RunControl`, `cancellation_handle`, `CancellationHandle`, `cancel` |
| `src/mesh_archive.rs` (producer removed 2026-10-23; placement and W1 plumbing remain) | `MeshApproximation`, `MeshCertificateReport`, `MeshApproximationReport`, `MeshRequest`, `local_gltf`, `with_policy`, `with_placement`, `with_approximation`, `MeshReport`, `MeshResult`, `mesh_to_archive`; model and placement values re-exported |
| `src/mesh_archive/model.rs` (current W1) | `ModelWrapRequest`, `local_gltf`, `with_policy`, `with_placement`, `ModelManifestRequest`, `ModelReport`, `ModelWrapResult`, `ModelManifestResult`, `model_to_archive`, `model_to_manifest` |
| `src/mesh_archive/placement.rs` | `MeshPlacement`, `from_parameters`, `MeshPlacementReport` |
| `src/raster_directory.rs` | `RasterDirectoryRequest`, `web_mercator_rgb`, `with_policy`, `RasterDirectoryReport`, `RasterDirectoryResult`, `raster_to_directory` |
| `src/glb.rs` | `MetadataGlb`, `new`, `from_parts`, `view`, `accessor`, `replace_view`, `compact_views`, `encoded_len`, `into_parts`, `finish` |

## A2 next operation after #147

Superseded on 2026-10-11: `convert-to-implicit` was removed without a
replacement; see the [disposition](implicit-rewrite-contract.md#disposition-2026-10-11).
The plan below is historical.

F1c2/W1 and mutating packaging facade retirement merged in #147 at
`e3d222a4c27a86e1f06e1e4b47db4e6a4fa5e24e`, with final-head platform wheel
and official Blender checks passing. The next #126 operation is the
[explicit-to-implicit rewrite foundation](implicit-rewrite-contract.md).
Its ownership audit and baseline probes do not accept the current converter.

The replacement must delete ConvertToImplicitOptions, the two legacy Rust
entry points, this operation's Reporter/ConversionResult adaptation, outer
output::Job and the nested package run/candidate repack after all retained
uses migrate. The CLI/Python spellings may select the new operation directly;
no compatibility mutation facade is required. Shared report/output definitions
still serve broader mesh and other helpers, so this operation
alone cannot delete them.

Parallel evidence compaction changes only storage: verbose F1c2 receipts become
verbatim gzip archives with an original-byte identity index. It does not rerun
or broaden their domain acceptance. Next #126 gates remain in-place vector
replacement/compression, read-only tooling contracts and intentional utility
export disposition; mesh/raster migrations retain their separate owners.
