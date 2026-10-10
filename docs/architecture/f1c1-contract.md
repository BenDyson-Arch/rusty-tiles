# F1c1: explicit mesh placement and world bounds

Implementation [#142](https://github.com/BenDyson-Arch/rusty-tiles/issues/142), under
#120/#121/#113. Baseline is merged F1b3 #141 at
`71cd11dcdb226ce7639912dba85adf0b2f35048b`. This contract precedes implementation.
The existing local-metre/Y-up source profile and immutable resource capture
remain bounded by [F1b3](f1b3-contract.md). This slice does not accept arbitrary
CRS conversion, identity/picking, approximation or implicit delivery.

## Invariants and ownership

A raw request declares Local or Wgs84 placement. Before source I/O, one pure
placement owner validates it and resolves exactly one finite rigid transform
and the normalized parameters used. Prepared mesh owns that resolved value;
encoding never inspects raw placement choices. Source interpretation, placement,
format serialization and F0 publication are separate owners. No unchecked
legacy Cartographic, Euler/HPR, MeshTo3tzOptions or heuristic CRS path participates.

Public `MeshPlacement` is a request enum: `Local`, or `Wgs84` with
`anchor_degrees_metres: [longitude, latitude, ellipsoidal_height]`,
`orientation_xyzw: [x,y,z,w]`, and `scene_offset_metres: [x,y,z]`.
`MeshRequest::local_gltf` declares source semantics and defaults to Local;
`with_placement` chooses the output placement. A core `from_parameters` helper
owns the CLI/Python combination grammar: anchor omission selects Local, but
orientation/offset without an anchor is InvalidRequest. With an anchor, omitted
orientation is identity `[0,0,0,1]` and omitted offset is zero. These are declared
domain defaults, not compatibility modes. Raw numeric request fields are
validated once by the mesh consumer before binding the source.

Invalid/nonfinite components, longitude outside [-180,180], latitude outside
[-90,90], or quaternion norm outside `abs(norm-1) <= 1e-12` are InvalidRequest.
Require a finite near-unit quaternion; normalize the admitted quaternion once
in f64, report it, and apply its right-handed active Hamilton rotation to ENU
vectors. Quaternion q and -q are equivalent rotations; canonical sign and
equivalent-request byte identity are not promised. There is one orientation
representation, with no Euler order or arbitrary nonzero-quaternion repair.

## Coordinate representation

Let N be the admitted accumulated source node transform. Source decoding retains
its established node-baked f32 Y-up geometry. The glTF-to-tile basis is
`B(x,y,z) = (x,-z,y)`. Scene offset d is a post-node translation in glTF Y-up
metres; it precedes B and orientation. It is neither a projected SourceOffset
nor an ECEF displacement, vertical datum correction or automatically inferred
Metashape offset.

For cartographic longitude lambda, latitude phi and ellipsoidal height h, use
WGS84 `a=6378137 m`, `f=1/298.257223563`, `e²=f(2-f)` and
`v=a/sqrt(1-e² sin²(phi))`. The ECEF anchor is
`C=((v+h)cos(phi)cos(lambda),(v+h)cos(phi)sin(lambda),(v(1-e²)+h)sin(phi))`.
The cartographic ENU columns are
`E=(-sin(lambda),cos(lambda),0)`,
`N=(-sin(phi)cos(lambda),-sin(phi)sin(lambda),cos(phi))`, and
`U=(cos(phi)cos(lambda),cos(phi)sin(lambda),sin(phi))`.
Supplied longitude defines the pole meridian. These formulas define the frame
even at elevated, negative or centre-crossing forward coordinates; no ECEF
inverse, surface-normal approximation or pole/centre repair is performed.

If Q is the admitted ENU quaternion rotation and F the ENU basis, the serialized
column-major root matrix has linear part `F Q` and translation `C + F Q B(d)`.
The bottom row is `[0,0,0,1]`. World geometry is therefore
`C + F Q B(Np + d)`. Local resolves to the identity root matrix. All output modes
use one manifest writer and emit the resolved root matrix explicitly; children
have no additional transforms. POSITION and every companion/resource retain
the same local leaf bytes across placement choices. glTF runtime applies B
once; root bounds are already in B-space. Placement must not apply B twice.

This is a proper rigid transform with determinant +1. Source reflection and
authored tangent restrictions remain in the source owner. Root placement does
not bake Earth-scale values into f32 POSITION, rebase each leaf, rewrite normals
or tangents, repair frames, or change materials. Root and leaf boxes remain
local conservative boxes, transformed as oriented boxes in world coordinates.
An independent reader applies the complete transform chain to centre and half
axes and checks stored geometry containment, including degenerate extents.

## Precision and limits

The existing node-baked component bound is 1e6 metres. Its existing f32 storage
tolerance remains separate from placement accuracy. Rigid f64 placement adds at
most 1e-6 metre coordinate evaluation error against independent high-precision
forward arithmetic on the decoded local coordinates. The finite numerical
profile admits
`a + abs(h) + sum(abs(d_i)) + sqrt(3)*1e6 <= 2^26 metres`.
Admission compares the exact sum of the four nonnegative f64 request components
against `58998676.19243112 m`, the greatest f64 no larger than
`2^26 - a - sqrt(3)*1e6`. A bounded error-free sum preserves rounding residuals;
rounding a total down to the cap cannot admit a request above it. This deliberately
conservative representable boundary is the operative limit. Above it is
Unsupported before source I/O; nonfinite parameters remain InvalidRequest. This bound controls intermediate magnitudes and leaves a
conservative floating-operation allowance (`64u * 2^26 < 1e-6 m`, `u=2^-53`).
It is an engineering profile choice, not a universal geodetic accuracy proof.
Trig/reference and boundary probes plus final consumer evidence must test it.
Local has no additional placement parameters and retains the source limits.

Local box containment is tested without an arbitrary epsilon. World OBB
evaluation uses the separately stated 1e-6 metre f64 allowance. Source storage
error is assessed before placement, rather than hidden in a large ECEF-relative
tolerance. Rendering/picking precision is a separate, measured consumer limit.
Rigid root scale is one; 3D Tiles 1.1's transform scaling leaves routing
geometricError in metres unchanged. The empty-root routing metric remains the
local conservative root diagonal with minimum one metre; leaf error stays zero.
This metric is not a promised coarse approximation error.

## Report, adapters and future identity

Report schema advances to 4 and profile to `f1c1-placed-gltf-v1` for the one
operation. Existing counts retain their meanings. `source_coordinates` is
`local-gltf`; output `coordinates` is `local-gltf` or `wgs84-ecef`. A typed
`placement` report records Local or the actual anchor, normalized quaternion
and source offset used; `root_transform` records exactly the serialized matrix.
All values come from the resolved owner. Rust, CLI, installed Python and
`conversion.json` agree without compatibility serialization branches.

CLI uses `--anchor LON LAT HEIGHT`, `--orientation-xyzw X Y Z W` and
`--scene-offset X Y Z`; Python uses optional `anchor`, `orientation_xyzw` and
`scene_offset` triples/quaternion. No format extension is inferred from them.

F1c2 will define selected source node/instance/primitive/triangle identities
before partition, preserve those associations through regrouping/approximation,
and prove metadata/picking. Names are labels, not unique IDs. Source discovery
owns document identities; producer and format codecs carry them without deriving
identity from coordinates/material grouping. This slice records that design;
it adds no unused identity fields or claim of feature support.

## Retention decisions and acceptance gate

Retain source/capture, authored companions, partition membership/local outward
bounds, image closure and F0 only within their independently accepted profiles
and replayed regressions. Rework prepared-mesh ownership and direct manifest
construction into resolved spatial state and one pure writer. Replace legacy
placement interpretation at this boundary with the defined frame above. Keep
broader legacy routes advertised pending their separate migrations; this slice
cannot delete them. No generic spatial package or policy framework is justified.

Independent analytical/Decimal and frozen-reference fixtures cover cardinals,
nonzero-meridian poles, antimeridian boundaries, elevated/negative/centre
anchors, noncommuting quaternion/basis order, offsets, source node transforms,
precision limits and actual typed refusals before I/O/staging. Sensitive controls
must detect axis, order, height, matrix/report and world-bound errors. Local PBR
truth remains independent of world-placement truth; a strict expectation seam
in the independent resource inspector may reuse local channel evidence without
rewriting or weakening the original archive checks. Consumer tests load the
emitted matrix unchanged and compare to independently anchored world targets.

Implementation acceptance additionally requires final source/artifact identity,
an independent nonauthor review, full lifecycle/resource/frontend/package
regressions, and platform/native/browser/wheel/official Blender CI. Static audit,
executed defects, representation choices and remaining proof limits are recorded
separately. #142 closes only for this bounded acceptance; #120/#121/#113 stay open.
