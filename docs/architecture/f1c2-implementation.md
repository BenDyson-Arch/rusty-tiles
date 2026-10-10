# F1c2, W1 and packaging API removal

Expanded slice: #144/#145/#146, based on develop
`29b96e501b1426e08cf7d3f01ba93964c6618a57`. Contracts, independent feasibility
probes and ownership decisions preceded production changes. Source commit
`4e1bda6de8d9f8be9ee4f776d51671420ff8faf5` implements the slice; subsequent
evidence/documentation commits retain the same production input identity.

## Concrete ownership and disposition

| Area | Implemented decision | Boundary retained |
| --- | --- | --- |
| Source identity | Source discovery records authored node/mesh/primitive/original triangle indices before winding or grouping. | Selected admitted scene and document-local identity; labels are optional exact strings. |
| Generated picking | Private leaf codec emits two labeled optional feature sets and two finite property tables. | Primitive/name and original-triangle queries; no imported business metadata or arbitrary extensions. |
| Unchanged model bounds | Source owner evaluates outward original-node intervals before f32 baking. | Raw and normalized admitted quaternion interpretations, decoded selected positions, finite local bounds. |
| Model archive | Typed W1 captures exact source/resource bytes and prepares one member inventory/report before output work. | One F0 Attempt; distinct resource aliases retain distinct archive members and emitted-byte charges. |
| Sibling manifest | Typed W1 publishes fixed sibling tileset.json and returns an inline required report. | Live source/resource stability throughout the call and subsequent use is an explicit caller precondition. |
| Placement | W1 and F1c2 consume the accepted pure F1c1 placement owner. | Local or explicit Wgs84 rigid placement; no source CRS inference. |
| URI validation | C1 decodes each percent-escaped UTF-8 segment once and preserves confinement. | No semantic metadata certification or broader resource authority is inferred. |
| Packaging mutations | Remove obsolete pack options/functions and migrate Rust fixtures plus implicit candidate packing to typed package. | F0 packaging stays opaque; the implicit outer Job/report operation remains legacy. |

The private identity Plan borrows the same immutable Geometry used to establish
its row maps. Callers cannot supply a different geometry while writing feature
attributes or metadata. One leaf's metadata is materialized at a time. The
source-name budget is admitted after partition and before workspace creation.

Independent review reproduced three defects in the first candidate: a manifest
summary claiming an impossible report sidecar, a synthetic source ancestor
collision classified as Io after output-parent creation, and top-level zero
geometric error preventing ordinary Cesium traversal. Their owners were fixed,
and independent sensitive controls replayed against the corrected artifact.
The model root remains full detail with error zero and explicit REPLACE; the
separate top-level box-diameter/floor metric is a declared omission/visibility
policy. It is not a new approximation bound.

## Breaking public and adapter changes

Removed Rust wrapping/manifest exports: CreateTilesetOptions, create_tileset_json,
glb_to_3tz and glb_to_3tz_reported. The tileset module is now private. Replacements
are ModelWrapRequest/ModelWrapResult/model_to_archive and
ModelManifestRequest/ModelManifestResult/model_to_manifest, each with a required
ModelReport and F0 JobFailure. CLI glb-to-3tz and createTilesetJson spellings
select these operations; Python glb_to_3tz selects W1 and model_to_manifest adds
the sibling product. Placement uses anchor/quaternion/scene-offset instead of
old HPR options. Directory/multimodel manifests and arbitrary output bases are
retired. The old broader wrapping escape hatch is deliberately narrowed to the
documented static source profile.

Removed mutating pack exports: PackOptions, convert_to_3tz,
convert_to_3tz_reported and pack_named_files. Rust callers use package,
PackageRequest and PackageMember; Python retains its packaging spelling over
the accepted package owner. Read-only pack utilities remain provisionally public.
There are no compatibility mutation wrappers. Historical baseline audit probes
are preserved as evidence rather than runnable new-API callers.

Mesh reports advance to schema 5/profile f1c2-source-identity-gltf-v1. W1 reports
use schema 1/profile w1-static-model-v1 with separate
root_geometric_error_metres and tileset_geometric_error_metres. Published archive
conversion.json and the returned report agree; a sibling manifest publishes no
report sidecar. See the three contracts for exact admitted domains and limits.

## Remaining removal and foundation gates

Three producer families still create legacy output::Job: broader mesh, raster
pyramid and explicit-to-implicit rewrite. Private tileset wrapping shortcuts
serve broader mesh; they are part of that family's remaining gate. Python
run_conversion still serves mesh_to_3tz and convert_to_implicit. Legacy CLI
result/report adapters still serve the remaining families. Vector's finalized
report writer still calls output::write_report independently of Job creation.
Shared Error, report types, helper calls and provisional public codecs require
caller disposition before global Job/Reporter/ConversionResult deletion. In-place
vector compression remains its own operation/failure contract.

The next mesh foundation decision is approximation/error: independently define
what proxies mean, how a proxy covering several source identities exposes those
associations, and how bounds/error are justified before choosing simplification
or atlas algorithms. Implicit delivery follows that proof. Broader source CRS,
height/datum/epoch, raster-pyramid and implicit-rewrite migrations remain scoped
work under their parent issues. This slice does not close #113/#120/#121/#125/#126
or authorize release.

[Evidence](f1c2-evidence.md) ·
[Separate nonauthor review](../../bench/architecture_audit/f1c2/audits/fresh-review.md) ·
[Current public surface](public-surface-inventory.md).
