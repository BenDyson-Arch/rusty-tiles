# F1c1 ownership audit and proposed bounded architecture

Static audit against `/home/bend/.cache/rusty-tiles-f1c1-placement`, HEAD `71cd11dcdb226ce7639912dba85adf0b2f35048b` (merged F1b3). Issue #142 was read through `gh issue view 142 --json title,body,comments`. Prior contracts read: F0 foundation/runtime, F1a, F1b2 and F1b3. No Cargo, production edits, test edits, or executable correctness probes were performed. The worktree was clean at inspection. Findings below are source inspection; proposed numerical limits and representation still require independently authored evidence and consumer acceptance.

## Main decision

Extend the existing bounded mesh operation with an explicit placement sum type. One private placement owner resolves it into a rigid tile-local-to-world transform before source binding. Keep the existing bounded glTF decoder's authored-companion/node-transform semantics, resource capture and F0 lifecycle owners. Carry a resolved spatial value into preparation and one common tileset writer. Do not feed F1 through `MeshTo3tzOptions`, heuristic `mesh::bake_to_enu`, `mesh_crs`, unchecked public `georef::root_transform`, or legacy `Scene`.

This is a domain addition and a rewrite of the unproved placement boundary. It is not sufficient merely to add optional cartographic/rotation fields to the old orchestration and branch in `produce`. The new invariant is that every prepared mesh has one finite, admitted rigid map and a report describing that exact map. A producer cannot reach encoding with unresolved coordinate choices. Paths and publication policy do not enter numerical placement, partition, or GLB encoding.

Root transform is the smallest representation candidate for this rigid profile: preserve stored f32 metre Y-up GLB geometry and its baked source-node companions; use f64 tile transform for orientation/ECEF translation. Do not bake Earth-sized coordinates into f32 POSITION, invent per-leaf rebasing, or add another normal/tangent rewrite. A rigid positive-determinant placement preserves the existing conformal restriction and companion relationships. Root transform and local boxes represent transformed oriented world boxes; world bounds need not be ECEF axis-aligned boxes. An independent reader must apply complete tile/content transforms to both geometry and box half axes. Final adoption depends on actual consumer evidence.

## Current ownership and concrete seams

* `src/mesh_archive.rs:21`: `MeshRequest` owns private input/output, positive explicit leaf limit, policy. Its constructor explicitly names local source interpretation. `validate` currently checks paths/output suffix/leaf limit, then `prepare` calls `binding::load`, decodes owned geometry, drops snapshots, partitions and determines selected image closure before any workspace. Keep this direction; add pure placement resolution immediately after request validation and before `binding::load`.
* `src/mesh_archive.rs:74`: `PreparedMesh` still owns the entire raw `MeshRequest`, geometry/leaves/bounds/counters. Rework this to own resolved output/policy/leaf limit and resolved placement. Do not retain raw placement alongside resolved placement as a second source of truth. An additional generic validated/resolved request framework would not improve this concrete operation.
* `src/mesh_archive/binding.rs:495`: filesystem owner resolves absolute source/output, captures root and external bytes, verifies native identities, disallows source/output aliases, and returns owned bounded snapshots. `Document::parse` at line 552 precedes dependency opens. Metadata refusals already happen before dependency I/O where declarations permit. Keep all placement request failures before even root I/O; retain source metadata refusal before dependency I/O; payload/magnitude-dependent refusals occur after capture but before workspace/staging. Do not claim malformed payload can be proved before reading it.
* `src/mesh_archive/source.rs:1072`: `Document` owns pure semantic/layout/source transform plans. `scene_plan` composes parent*local in f64, checks accumulated linear maps and tangent conformality from metadata, then `decode` bakes selected triangles to f32 Y-up. No paths or publication policy enter it. Retain this concrete responsibility against F1 evidence; placement metadata does not require reworking its material/accessor admission.
* `src/mesh_archive/partition.rs`: triangles are partitioned by their stored Y-up centroids; bounds independently convert positions to tile Z-up `[x,-z,y]`. This is consistent with existing admitted frame semantics, but currently implicit in a separate implementation. Make the frame conversion a single named private operation owned by the spatial module if it acquires a second actual production consumer. Do not create a generic coordinate utility package for tests alone.
* `src/mesh_archive/encode.rs`: pure GLB writer receives geometry and exact leaf triangle indices, groups by material/attribute presence and uses one channel writer for all companions. Root placement must not create a second Earth-only GLB encoder. Material/image resource identity and closure are already carried by source indices; preserve that path.
* `src/mesh_archive.rs:produce`: combines workspace writes, progress, direct tileset JSON construction and report construction. Extract a pure small manifest encoding function accepting leaves/root bounds/resolved transform/routing error. Build root/children once for all modes. A pure encoder can return bytes or a JSON value; it must not own publication or files. The orchestrator stays the only mesh workspace/inventory/report/archive owner.
* `src/mesh_archive.rs:mesh_to_archive_with_operations`: one `control.begin()` encloses validation, capture, preparation, workspace, candidate archive, cleanup, event closure, seal and publication. Retain exactly one Attempt. Never call public package conversion with this used control; never make placement resolution invoke runtime or observers.
* `src/runtime.rs:Staging::create`: Unix/Windows capability refusal happens only here, after `Workspace::create` and mesh production. This is a static existing contract gap for excluded publication platforms: scratch/output parents can precede Unsupported. No audit-only new runtime defect reproduction was performed. A search of runtime/package/point/terrain/directory owners found no shared file-publication preflight; directory `ResolvedDirectoryOutput` uses directory-filesystem `platform::preflight`, which must not be reused for file archives. Track this invariant separately from F1c1 placement correctness. If corrected in this slice, one crate-private pure file-capability check reused by Staging and pre-workspace preparation is sufficient; no duplicated producer `cfg!` branch or generic runtime redesign.

## Smallest request and private resolved representation

One possible exact public surface is:

```rust,ignore
pub enum MeshPlacement {
    Local,
    Wgs84 {
        anchor_degrees_metres: [f64; 3], // lon, lat, ellipsoidal h
        orientation_xyzw: [f64; 4],
        scene_offset_metres: [f64; 3],  // post-node glTF Y-up x,y,z
    },
}

MeshRequest::local_gltf(input, output, leaf_triangles)
    .with_placement(MeshPlacement::Wgs84 { /* all values explicit */ })
    .with_policy(OutputPolicy::CreateNew)
```

`local_gltf` continues to declare the source metre/Y-up profile; it must be documented as source semantics, not a guarantee that the output is unplaced. A rename is permitted if that distinction proves confusing, but do not keep synonymous constructors as compatibility baggage. Default placement can be Local because the named constructor already explicitly establishes the unchanged local interpretation; Earth placement requires an explicit enum variant. Typed arrays with units in field names are sufficient here; do not export arbitrary matrices, a coordinate-mode string, legacy `Cartographic`, legacy `RotationDegrees` or a CRS selection type for the new operation. A dedicated finite anchor struct may improve readability but is not necessary for invariants: constructors/builders collect values and validation occurs once in core.

Prefer a quaternion over Euler/HPR for the new boundary: the profile has no compatibility obligation to `--rotationDegrees`; XYZW matches familiar glTF ordering and represents one right-handed active rotation about tile-local ENU axes. Require finite components, norm within a declared near-unit tolerance (the existing source quaternion tolerance 1e-6 is a reasonable consistent candidate), then normalize once in f64. Report the normalized quaternion actually used. Do not accept arbitrary magnitudes by normalizing every nonzero input. Quaternion q and -q produce the same rotation; canonical sign normalization is unnecessary unless reproducible equivalent-request bytes are promised. No Euler order branch or enormous-angle trigonometric domain is required. If convenience HPR is eventually required, derive it in an adapter or a separate explicitly scoped conversion; the core resolved map remains one representation. A quaternion defines orientation frame/handedness directly; contract tests establish positive rotations about each ENU axis and noncommuting composition.

Recommend including `scene_offset_metres` because issue #142 explicitly requires source-offset ordering acceptance, and this makes the rigid local-shift use case concrete. It is **not** legacy E/N/A `SourceOffset`, not source accessor coordinates, not geographic degrees, and not a projected source CRS offset. Source-authored node translation can test source/node/placement ordering but would not prove an external shift API. If the agreed bounded use-case disposition instead explicitly defers external shift, omit the term and clearly narrow what that acceptance proves. If supported, use this exact placement chain (column vectors):

```
p_glb = f32(parent * node * p_source)     // existing source bake contract
p_tile = C * p_glb                      // C(x,y,z)=(x,-z,y)
p_world = A + B(lon,lat) * R(q_xyzw) * (p_tile + C * scene_offset_metres)
root_transform = [B*R, A + B*R*C*scene_offset_metres]
```

Offset is after the full source-node chain, before placement rotation; it is not rotated by source node transforms. All offset/anchor arithmetic stays f64; no large translation is added to f32 GLB positions. The exact chain, including the existing f32 bake boundary, must be the contract/oracle rather than silently moving the rounding point.

Use one private concrete type such as:

```rust,ignore
struct ResolvedPlacement {
    tile_to_world: [f64; 16],
    report: MeshPlacementReport,
}
```

Fields and constructors are private to placement owner. Its `resolve(MeshPlacement)` performs all finite/domain checks and constructs only a rigid map with finite f64 components, determinant +1 and metre scale. Local resolves to identity. Other modules receive the value through narrow read-only methods for transform/report. Do not return `(Option<anchor>, Option<rotation>, Option<transform>)`; that recreates invalid combinations. Do not introduce `Resolved<T>`, general transform traits, registries, contexts, backend handles, or per-mode writers. Always emitting the resolved root matrix, including identity in Local, is a simple common writer; omission of identity is also possible as a single serialization detail owned by that type. Do not branch throughout partition/encoding/orchestration on modes.

The profile's numerical contract must bound finite height/offset magnitude and quaternion norm as well as checking NaN/Infinity. Merely accepting every finite f64 risks Earth translation overflow. Longitude [-180,180], latitude [-90,90], explicit ellipsoidal metre height, near-unit quaternion and bounded local translation are engineering admission choices, not automatic claims of arbitrary CRS/datum support. The mathematics/oracle work should settle actual height/translation ceilings and tolerance budget before implementation. Include both poles and both longitude endpoints with a stated longitude-based pole orientation; a manual rigid tangent frame need not inherit singular geographic-CRS Jacobian behavior. Do not infer a geoid correction, ground altitude, epoch, grid availability, or ellipsoidal height from an omitted height.

## Report/frontends and error ownership

Changing report fields/meaning requires schema 4 (currently 3), with a new profile identifying this slice. Replace the ambiguous `coordinates="local-gltf"` output interpretation with unambiguous source coordinates and a typed placement report. An illustrative concrete schema is `source_coordinates="local-gltf-metres-y-up"`, `placement={mode:"local"|"wgs84", ...}`, and the exact emitted `root_transform` in that placement report. Earth mode reports explicit input anchor and scene offset, normalized `orientation_xyzw` actually used, and frame/ellipsoidal-height units; Local carries identity/frame interpretation. Original near-unit quaternion need not be retained as a second report parameter: this is a receipt for the operation actually performed. Preserve input anchor endpoints/pole longitude unless a documented canonicalization decision changes them. Do not expose two independent hand-built report dictionaries. Produce typed report once, serialize the same report into conversion.json, and return it through Rust/CLI/Python. Keep existing counters, bytes, exact resource closure and routing omission-error meaning. A rigid transform keeps routing error in metres; placement is not measured approximation.

CLI `LocalMeshArgs` currently has only I/O/leaf limit; Python `mesh_local_to_3tz` likewise constructs `MeshRequest`. Extend these adapters with explicit anchor plus orientation-XYZW/scene-offset only when anchor is present, mapping into the same enum. Preserve typed InvalidRequest for invalid option algebra and InvalidInput/Unsupported for source failures. Clap/Python parsing can reject wrong shape/type as transport errors, but shared domain range/finite decisions must be core-owned; do not reuse legacy Python `placement()` returning PyValueError or legacy CLI `placement_opts` as a competing numerical owner. An Earth-specific new frontend command is optional and unnecessary; the existing function is a source profile name.

Python `MeshResult.report` already converts serialized core report to Python dynamically; its conversion needs no placement logic. CLI `mesh_summary` already embeds serialized report directly. Inspect Python stub/docstrings: the current docstring incorrectly describes embedded base-color GLB only despite F1b3 external/core-PBR support. Fix the advertised current bounded profile during adapter work. Installed-wheel parity must exercise the actual installed adapter/report, not merely Rust-created objects.

One Begin/Attempt remains even for invalid placement (invalid requests consume control as F1a specifies). Pure finite/options checks precede filesystem work. Source metadata-driven invalid/unsupported combinations still refuse before dependency opens. Source payload transformed-magnitude and output precision/bounds failures refuse before workspace/staging. Errors enter Attempt.fail/F0 arbitration once; callbacks and publication remain exactly their current owners.

## Source identity architecture and F1c2 boundary

The current `type Instance = (usize, Matrix, f64, [[f64;3];3])` records mesh and transform but drops source node index at traversal. Triangle retains material/companions but drops source mesh/primitive and original triangle ordinal. Encoder regrouping subsequently combines triangles from different source nodes/primitives sharing materials/layout. Names are admitted descriptive labels and discarded. These are explicit F1 identity absences, not picking support.

F1c2 should use source document indices as stable authored identity, not leaf ID/output primitive index/name/accessor identity. In this admitted strict forest, a selected source node index uniquely identifies each mesh instance; distinct nodes referencing the same mesh remain distinct. Planned provenance key is `(source_node_index, source_mesh_index, source_primitive_index, source_triangle_ordinal)` within one captured source document. Source triangle ordinal is the original primitive TRIANGLES ordinal before reflection corner permutation and partition/regroup. If a future source profile permits DAG instances/repeated traversal, introduce a document-rooted instance path/id then; do not invent it now for the strict tree profile.

When implementing identity, replace tuple Instance with a named record including node/mesh and use enumerated primitives/triangle ordinals at decode. Carry provenance on each accepted triangle or a same-length provenance vector with a single owner; the former makes synchronization errors harder. Partition carries existing geometry indices; no identity is allocated during partition. Encoder derives feature association from triangle provenance while retaining common corner/channel selection and resource closure. Do not generate identity from material grouping or output content indices. Later simplification needs a concrete mapping of each approximation primitive to source association sets, with merge/refusal semantics proved in that approximation slice; a single fabricated node ID cannot describe a many-source approximation.

This F1c1 slice records the design and the source-loss seams, but does not add unused provenance fields, metadata schema writers, extension declarations, feature-ID accessors, name maps, pick adapters or speculative lineage containers. Placement uses none of them. Independent geometry/resource oracles can still establish source instance multiplicity/association by known fixture geometry; this is not exported picking. If a small named Instance extraction is needed by placement work for a real consumer, retain it, but do not claim indices alone implement stable identity. The F1c2 follow-on explicitly owns implementation, public metadata and actual browser/Blender picking proof.

## Retain/rework/replace/remove ledger

| Candidate | Disposition | Reason / required evidence |
| --- | --- | --- |
| F0 Attempt/event arbitration/file seal/publisher | Retain within this slice | Existing lifecycle owner; replay source capture/callback/cancel/cleanup/replacement failures. Placement must not duplicate it. |
| Runtime platform publication check | Track existing static gap separately; optionally rework narrowly | Excluded platform currently discovered after workspace; no defect reproduction was run. One runtime-owned file check if corrected, no runtime registry. |
| F1 private source admission/node bake/material plan | Retain conditionally | Placement preserves the admitted source profile; independent F1 geometry/PBR regressions remain required. No inferred correctness from type names. |
| F1 binding/captured snapshots/output aliases | Retain conditionally | Real source stability/path ownership, not placement policy. Replay capture/alias/CWD and URI failures. |
| Raw Request retained in PreparedMesh | Rework | Prepared owner should carry resolved destination/limits/policy and invariant-carrying placement rather than raw placement decisions. |
| Inline tileset JSON in filesystem producer | Replace with small pure writer | One resolved transform, one exact hierarchy/bounds serialization for both modes; easy independent reader verification. |
| Bounds conversion `[x,-z,y]` | Rework ownership if shared consumer exists | Name/pin tile frame once; require transformed box/geometry enclosure evidence and sensitive axis controls. Do not rewrite deterministic membership needlessly. |
| GLB common attribute/resource encoder | Retain conditionally | Preserve f32 source bake/material/UV/color/normal/tangent associations; no placement-mode codec forks. |
| `coordinates` report as local-only interpretation | Replace in schema 4 | Source and output frames differ for Earth; typed report derived from exact resolved map. No legacy alias fields. |
| Legacy `georef::root_transform` and unchecked HPR helpers | Exclude from new boundary | No finite/domain invariant; tied to historical Cesium convention. Implement independently justified bounded placement owner. Generic shared math extraction only with actual migration consumers. |
| Legacy `mesh::bake_to_enu`, guessing and general `mesh_crs` | Keep isolated; defer removal | Different nonlinear/source-coordinate operation, advertised route not discharged by rigid placement. Do not import into new request. |
| Legacy SourceCrs/SourceAxes/SourceOffset/offset parser exports | Keep isolated; defer deletion | Used by advertised old mesh route and other consumers; no compatibility exports in F1 placement. Removal gate must enumerate unsupported/deferred use cases. |
| Current tuple Instance/source identity loss | Document now; replace in F1c2 | Adding unused identity/picking code now violates bounded scope. Follow-on must own real propagation and public proof. |
| Legacy Job/reporting APIs and wrapping/implicit operations | Deferred under #126 | No claim that F1c1 authorizes deleting still-advertised routes, closing #121/#120, or a release. |

## Legacy placement/axes/height use-case disposition

| Advertised or actual use case | F1c1 disposition | Concrete consequence |
| --- | --- | --- |
| Static core-PBR local metre/Y-up source with authored node transforms | Supported | Existing source profile; Local identity output remains explicit. |
| Rigid manual WGS84 cartographic placement of that local source | Supported after proof | Explicit ellipsoidal anchor, tangent basis and rotation convention; f64 root map. |
| Manual HPR of that local source | Replaced by explicit XYZW orientation | Rotation use case remains supported after proof; Euler/HPR argument convention is not carried as compatibility API. Exact orientation frame/positive rotations/noncommuting composition in contract. |
| Local post-node metre scene translation | Supported only if selected | Distinct `scene_offset_metres`; folded into root in f64 before placement rotation. If omitted, explicitly deferred and use node-authored translation. |
| Local Z-up/XYZ source interpreted differently without node transforms | Deferred | F1 source semantics stay glTF metre/Y-up; caller may author the appropriate glTF node transform. No SourceAxes switch. |
| Geographic Y-up `(lon°, h m, -lat°)` source and heuristic auto-detection | Replaced as F1 policy / not admitted | Explicit local source only; no geometric guessing. The legacy route remains until support/removal decision. |
| WebMercator source `(E m, h m, -N m)` and geographic bounds-derived origin | Deferred | Nonlinear source conversion not equivalent to rigid local ENU placement, even at small extents. |
| Metashape offset file / E,N,A absolute projected shift | Deferred | Must resolve CRS/units before addition; not interchangeable with local scene offset. No offset-file I/O in new placement owner. |
| Legacy auto+source_offset implicitly chooses WebMercator | Replaced/refused in new boundary | No offset-driven CRS inference; record that current legacy behavior exists, not that it is proved. |
| Legacy Geographic+source_offset | Replaced/refused in new boundary | Current legacy geographic bake ignores offset; no equivalent new promise or silent ignoring. |
| Manual cartographic zero-height behavior | Replaced | Legacy `mesh::bake_to_enu` only overrides derived height if manual height !=0; new explicit anchor h=0 is exactly ellipsoidal zero. |
| General EPSG/WKT/PROJ horizontal CRS, source_axes XYZ/YUp | Deferred | Separate source-conversion evidence needed; native/portable agreement does not establish correctness. |
| Horizontal CRS height_offset after A, including explicit zero | Deferred | Source height-reference conversion is not manual anchor height. Do not advertise arbitrary vertical datum correction. |
| Declared 3D/compound/geocentric CRS, grid/epoch/datum transforms | Deferred | Requires separate resolver capability/data/accuracy contract. No automatic admission from WGS84 rigid placement. |
| Reflection/nonuniform node transforms and tangent limit | Supported as existing admitted source semantics | Placement itself is rigid determinant+1; keep source inverse-transpose/corner/w sign and accumulated conformal restrictions. |
| Legacy source node features/picking | Deferred to F1c2 | Names/source indices alone do not prove public picking. |
| Legacy atlases/LOD/lossy codecs/meshopt/implicit delivery/wrapping | Deferred | Unrelated to this placement slice and retained removal gates. |

## Bounded implementation ownership/fan-out

1. Numerical placement owner: new `src/mesh_archive/placement.rs` only; exports private resolved type, bounded pure resolve and narrow matrix/report access. Public enum/report types can live here and be re-exported at mesh facade/root. This person does not edit runtime/frontends/fixtures.
2. Integration owner: `src/mesh_archive.rs`, pure manifest portion of `src/mesh_archive/encode.rs`, and any actually needed named frame conversion in `partition.rs`; integrate PreparedMesh resolved data, common writer/report and one lifecycle. A separate manifest module is justified only if it keeps encode's actual format owner coherent, not to create a module per helper.
3. Adapter owner: `src/main.rs`, `bindings/python/src/lib.rs`, relevant stub/API docs, corresponding adapter tests; same enum/core errors/report. Root alone handles `src/lib.rs` exports to avoid overlap.
4. Independent acceptance owner: new frozen fixtures/oracle/output-reader/consumer controls and contracts/evidence. Does not edit production numerical placement. Root owns Cargo/build/shared target and runtime capability preflight (if necessary), plus final integration tests.

Audit precedes fan-out. Freeze exact API/report/chain/domains before assigning production work. No generic transform package, identity framework, job framework, registry, or compatibility aliases. A new private spatial owner and a common manifest encoder should need a few hundred production lines, plus concise request/report/frontend wiring. That is an estimate, not a size certification: independently auditable geometry/bounds/PBR/consumer controls may legitimately add substantial test code. The new producer cannot delete the ~2,640 lines of legacy `georef`/`mesh_crs`/`tile` merely because this bounded slice passes. Existing F1 source/binding/encoder/partition files total ~3,860 lines including private tests; placement does not justify indiscriminate rewriting of already independently accepted PBR/resource admission. Record actual added/removed production/test lines after implementation, and reject unexplained growth or repeated per-mode policy branches.

Acceptance must prove transformed conservative bounds, rigid world positions/orientation and normal/tangent behavior, exact multileaf triangle/resource association, geographic variation/poles/endpoints, invalid/nonfinite/magnitude/excluded combinations before side effects, one F0 attempt/lifecycle regressions, exact schema/archive parity across Rust/CLI/installed Python, and actual viewer world placement. Frozen independent reference and sensitive controls are mandatory; no test execution or correctness pass is asserted by this audit.
