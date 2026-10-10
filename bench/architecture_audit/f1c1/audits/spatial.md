# F1c1 independent spatial audit

Audit target: `/home/bend/.cache/rusty-tiles-f1c1-placement`, baseline `71cd11dcdb226ce7639912dba85adf0b2f35048b`, issue [142](https://github.com/BenDyson-Arch/rusty-tiles/issues/142). Written before production implementation. No production/test edits or Cargo were performed by this audit. Existing source and contracts were inspected as candidates. Independent cache probes import neither production code nor repository oracle helpers.

## Recommended minimal contract

Use one explicitly unplaced local interpretation and one rigid WGS84 placement of that same admitted local metre/Y-up profile. Rigid placement belongs to the existing mesh domain owner and resolves once before staging. Keep GLB geometry, node-bake admission, material/resource closure and local spatial partitioning intact where their existing independent evidence remains applicable. Serialize a single f64 root affine transform. No CRS resolver, native backend, generalized matrix registry, layout migration, world-coordinate f32 bake or geometry repair is required.

Consolidated request shape: one raw typed `MeshPlacement` enum, `Local | Wgs84 { anchor_degrees_metres, orientation_xyzw, scene_offset_metres }`. Local means unplaced local output, with no inferred Earth coordinates. Wgs84 owns the three parameter arrays; only the mesh domain owner resolves them into private `ResolvedPlacement` before binding. Invalid raw placement becomes `InvalidRequest` before source/output work. Local carries no extra offset/orientation. Rotation without an anchor and legacy projected offsets are unrepresentable. A private resolved spatial value carries only a column-major f64 transform and typed report values. Numerical/encoding stages carry no paths or publication policy. F0 still owns run state/publication. Adapters share one tuple-to-enum combination grammar; they do not separately implement normalization or numerical eligibility.

The consolidated design admits scene-local offset as the concrete user-authored metre translation described below. It does not inherit legacy `SourceOffset` names or units. One private `to_tile` B helper is justified by two actual consumers: partition bounds and placement-offset interpretation; no generic matrix framework is needed. Report schema4/profile`f1c1-placed-gltf-v1` records normalized requested parameters and the exact emitted root matrix.

### Frame and composition

Let `M` be the selected source node's accumulated glTF transform (parent * local, local T*R*S), `p` its source vertex, and `p_s` the existing nearest-f32 stored result of computing `M p` in f64. Source quantities remain subject to the F1b3 admission/fidelity contract.

Let `B(x,y,z)=(x,-z,y)`. B is the right-handed +90° rotation about X required when glTF Y-up content enters 3D Tiles Z-up space. Its determinant is +1: +X becomes east, +Y becomes up and −Z becomes north. Do not bake B into GLB vertices while retaining runtime conversion.

Let `o` be scene-local Y-up metre translation applied after source nodes, before B and orientation. Let `R` be the active quaternion rotation in ENU coordinates, and `E` the declared cartographic ENU-to-ECEF basis. Let `C` be the cartographic anchor's WGS84 ECEF point. The logical placement is:

`world(p) = C + E R B(M p + o)`.

The storage representation is:

`world_stored(p_s) = t + L B(p_s)`;

`L = E R`, `t = C + E R B(o)`.

The emitted root transform is `[L,t; 0,0,0,1]`, column-major. GLB nodes emitted by the current encoder remain identity/baked local; children add no transforms. This implements post-node/pre-B offsets entirely in f64 translation and introduces no new local f32 rounding. In the absence of placement the output frame is simply B applied once to stored local GLB content. Every material/UV/color/normal/tangent companion remains attached to its original stored triangle/corner.

A root transform applies to both tile box and content. Local Z-up box center `c` and half-axis columns `H` therefore define the world oriented box `t + L c`, half-axes `L H`. These are already world bounds through the transform chain; emitting an additional geographic region or ECEF AABB is unnecessary. Do not interpret local GLB vertices as Earth coordinates or transform local box extrema without their complete affine matrix.

### Anchor and height

Longitude/latitude are explicit f64 degrees in closed intervals [-180,180], [-90,90]. Height is explicit f64 metres above the WGS84 ellipsoid, including explicit zero and negative heights. No orthometric/MSL/geoid conversion, source vertical datum, epoch transformation or terrain clamp is performed. WGS84 constants are a=6,378,137 metres, f=1/298.257223563.

With λ=longitude and φ=geodetic latitude in radians, e²=f(2−f), N=a/sqrt(1−e²sin²φ):

`C=((N+h)cosφcosλ, (N+h)cosφsinλ, (N(1−e²)+h)sinφ)`.

E's columns are:

`east=(-sinλ,cosλ,0)`;

`north=(-sinφcosλ,-sinφsinλ,cosφ)`;

`up=(cosφcosλ,cosφsinλ,sinφ)`.

Up is the declared ellipsoid-normal direction, never radial Earth-centre up. Longitude remains a declared orientation parameter at either exact pole. No ECEF inverse or default-longitude reconstruction occurs; longitude-zero and longitude-90 poles share their anchor point but intentionally have rotated horizontal frames. ±180 have equivalent positions/frames within numerical tolerance, with input longitude retained. No longitude wrapping or cartographic bounding rectangle enters this path. Forward placement remains deterministic even at an interior/Earth-centre anchor because the caller declares the frame; an inverse's inability to orient the centre does not make this affine transform singular.

### Orientation choice

| Candidate | Advantages | Additional contract burden |
|---|---|---|
| One explicit ENU active quaternion XYZW | Standard right-handed rigid rotation, composable, identity explicit, no Euler ordering singularity, compatible with typed numerical consumers | Four values, near-unit admission/normalization rule, ENU frame must be named because glTF source quaternion axes differ |
| One explicit Euler triple | Three human-readable angles, easy CLI use for simple heading | Must define sign/axis/order/frame/intrinsic vs extrinsic semantics, periodicity and large-angle reduction; common aviation conventions differ from Cesium; noncommuting fixtures mandatory |

Recommend quaternion only. This is not inferred from legacy Cesium HPR. Validate finite XYZW and nonzero norm with `abs(norm−1)<=1e-12`, then normalize this admitted numerical drift. Reject arbitrary nonunit quaternions rather than converting them into an orientation. Identity is [0,0,0,1]; q and −q are equivalent and need not undergo sign canonicalization. Axes of this quaternion are east/north/up; apply it after B. Positive +90° about ENU up maps east to north. A quaternion specified about source Y is a different operation and must be rejected/detected by independent axis fixtures.

An Euler-only contract could instead choose Cesium's `Rz(−heading)*Ry(−pitch)*Rx(roll)` in ENU, applying roll then pitch then heading, but that is a profile decision rather than a glTF/3D Tiles standard. Implementing both forms would reintroduce an unnecessary options bag.

### Magnitude and precision

Keep the already-admitted local source and node-transformed component limit abs<=1,000,000 metres. Root placement preserves the previous local f32 error; it does not establish centimetre accuracy for every large source. Near the local limit the actual nearest-f32 worst error is 0.03125 metres/component, or approximately 0.05413 metres Euclidean. The pre-existing acceptance allowance `max(1e-6m,abs(computed)*2^-24)` is a looser per-component storage tolerance and must be identified separately from actual worst ULP error and node-evaluation accuracy.

For an initial unified placement domain, recommend the numerical budget:

`a + abs(height_m) + sum(abs(scene_offset_yup_m)) + sqrt(3)*1_000_000 <= 2^26 metres`.

All inputs and resolved matrix values must be finite. Evaluate the budget without allowing overflow/cancellation to hide an oversized term. This is an engineering precision admission limit, not a terrestrial-height convention. It supports orbital heights and large shifts, handles both positive and negative heights uniformly, and rejects inputs whose large terms happen to cancel to a small final position. It does not require a hard-coded terrestrial altitude cap or a special-case geographic limit. A smaller derived budget may be chosen if final execution proves the target client requires it; do not silently reduce heights/offsets.

The target for additional f64 placement error against an independently evaluated mathematical placement of decoded stored positions is <=1e-6 metres Euclidean. The generous `64u*2^26=4.7684e-7m` operation-scale budget, with u=2^-53, leaves room beneath that target; it is not a portable libm accuracy theorem. Decimal and frozen-reference boundary fixtures plus supported-platform checks are still required before claiming this error across the implemented admitted profile. Pure rigid basis residual/determinant checks should tolerate floating arithmetic (e.g. Gram/det residual <=1e-14), never normalize columns independently or alter a requested frame as repair.

Against the fully transformed source, measure world deviation separately: `norm(world_stored−world_source) <= norm(p_s−M p) + 1e-6m`, with source-node evaluation compared against analytic/frozen reference fixtures. The root adds no source-dependent f32 bake. A consumer's GPU/camera-relative representation has its own limits; CPU matrix precision does not prove rendering accuracy at every extent/view.

Quaternion/ENU operations preserve lengths, angles and orientation in exact arithmetic. Normals use `E R B(n_s)` and tangent XYZ uses `E R B(t_s)` at runtime, with renderer normalization; tangent W stays unchanged under placement because det(E R B)=+1. Existing negative node determinants still govern source bake corner reversal and tangent W adjustment once. No placement normal Jacobian, inverse CRS, tangent texture rewrite or additional reflection permutation applies. Existing conformal accumulated-node restriction remains necessary for baked tangent semantics and is not relaxed by rigid placement.

## Static candidate observations and disposition

| Candidate | Disposition | Reason and remaining proof |
|---|---|---|
| mesh_archive source node f64 composition, f32 local geometry and supported attribute bake | Retain only for previously admitted profile | Clear separation from paths; placement root cannot change source admission. Replay geometry/PBR and near-limit fixtures. |
| mesh_archive partition membership and local stored-geometry Z-up bounds | Retain membership; verify/extend bounds evidence | Complete triangle indices preserve companions; existing B mapping matches standard. Exact local stored enclosure is necessary before independent transformed enclosure. |
| partition Bounds outward midpoint/extent logic | Retain arithmetic only after analytic/decoded checks | Four independent local probes passed including mixed magnitudes and zero extents; root parent union and many-leaf cases remain execution obligations. |
| mesh_archive root/report construction | Rework | Add one prepared root transform/placement report; coordinates must distinguish unplaced vs Earth-placed. Root transform changes schema/meaning, so report versioning needs explicit decision. |
| georef WGS84 forward and cartographic ENU formula | Retain math after independent implementation comparison; copy/extract only actual reused dependency | Formula independently matches Decimal and primary semantics. Existing public unchecked types/functions are legacy policy, not placement admission. A small private domain helper is sufficient; no generic framework extraction justified. |
| georef Cartographic, RotationDegrees and root_transform public legacy contract | Replace for this path | Unvalidated public fields, Option rotation, HPR policy. No compatibility need justifies importing them into the foundation request. |
| georef ecef_to_cartographic, EnuFrame CRS probes, coordinate guessing and projected offsets | Exclude/defer | No inverse/CRS operation needed for rigid placement. Guessing and portable/native agreement do not establish source CRS eligibility/datum/grid accuracy. |
| bbox.rs legacy helpers | Replace dependency with existing private stored bounds | Accessor bounds/node arithmetic f32 and broader scene readers are not needed; current partition bound has the correct admitted ownership and data. |
| mesh_crs.rs/crs.rs | Defer | General geographic/projected conversion, offset units, height datum and Jacobian normals require separate contracts/reference evidence. |
| geometricError=max(1m,local root-box diagonal), leaves=0 | Retain as declared routing metric | 3D Tiles 1.1 applies largest transform scale to tile error; rigid scale=1 leaves metre interpretation intact. Never recompute from world AABB diagonal, which changes with orientation; no Hausdorff/approximation claim. Near/far Earth viewer acceptance still required. |

### Legacy use-case disposition

- Unplaced local metre/Y-up source: supported as explicit unplaced local.
- Local metre source anchored by longitude/latitude/ellipsoidal height: supported by rigid placement.
- Manual legacy HPR: replaced with one quaternion orientation; no undocumented Euler adapter.
- Author's local metre shift: supported if admitted as scene-local post-node/pre-B offset, represented through root translation. It is not a projected CRS offset.
- Geographic glTF `(longitude,height,−latitude)`, Web Mercator, Metashape offset.txt shifts, arbitrary source axes/projected definitions: deferred/rejected by this new bounded path; no magnitude guessing.
- Orthometric heights/geoid/grid corrections, arbitrary datum/epoch transformations, nonlinear reprojected mesh tangent/normal frame conversion: deferred pending independent operation-specific evidence.
- Scale/shear in placement: excluded. Admitted source-node nonuniform transforms remain governed by previous normal/tangent profile rules.

## Executed design evidence

Artifacts: `/home/bend/.cache/rusty-tiles-f1c1-spatial-probes/probe.py`, `reference.mjs`, `results.json`. Python decimal precision 65 with independently authored Taylor sine provides WGS84 origin arithmetic; quaternion vector identity is used rather than copying the candidate matrix formula. Cached Cesium engine26.4.0 contributes frozen origins and separately recorded frame observations. The reference implementation source hash is included. These are design probes, not candidate implementation passes, Cargo tests, browser validation, or final binary identity evidence.

- Eleven equatorial, mixed-latitude, antimeridian, exact/near-pole and negative/elevated anchors: f64 forward origin maximum component difference vs Decimal was `9.8114e-9m`; ENU Gram residual maximum was `2.2204e-16`.
- Independent Cesium origins differed by at most `1.4901e-8m`. Cesium's elevated ENU convenience builder is not the proposed cartographic frame oracle: its ellipsoid-normal calculation from the elevated Cartesian point differed in basis by `.002236124` at longitude123°, latitude45°, height100,000,000m. Even height42m gave `1.5984e-8` basis difference. Exact poles with ordinary Earth radii retained longitude because Cartesian residuals did not trigger its centre/pole threshold; do not generalize that accidental behavior into a pole-frame contract.
- Direct f32 ECEF bake of a sub-metre point near Brisbane lost `0.0877649m` in one component. Adjacent f32 values at ECEF X=6,378,137 are 0.5m apart. Keeping local f32 plus f64 root avoids this additional loss.
- Four independent local bounds probes passed exact stored-vertex inequalities, including [-1e-38,1e6] mixed magnitudes, ±1e6 corners, point bounds and duplicate degenerate points. Placement affine injectivity preserves mathematical enclosure; these checks do not cover the production parent union or actual viewer culling.
- Proposed numerical-budget boundary with height51,998,675.44243112m, nontrivial quaternion and scene-local offset[4,000,000.125,−2,000,000.25,1,000,000.375] gave `8.6822e-9m` maximum component world difference against Decimal for local-limit corners and a sub-metre point. This single boundary case supports the chosen budget; it is not exhaustive proof.
- Sensitive offset control at anchor(0°,0°,0m), +90° ENU-up rotation, offsetYup[1,2,3]: correct world offset is [2,3,1], whereas adding an unrotated offset yields [2,1,−3]. A nonzero multi-axis offset makes transform-order errors visible.

## Acceptance still required before retention is final

1. Independent complete reader decodes actual archives, source nodes and all emitted GLB/node/tile transforms. Derive expected world positions directly from source + declared placement; never infer correctness solely from an inverse round trip or production-generated matrix. Compare local placement/unplaced GLB content equivalence and triangle/companion/resource multisets across single/many leaves.
2. Pure analytic cardinal anchors: equator λ0 and λ90; latitudes45/−27; exact north/south poles with different declared longitudes; both longitude boundaries; elevated and negative heights. Height+10m must translate by 10 times declared geodetic up. At λ0/φ0, east=(0,1,0), north=(0,0,1), up=(1,0,0).
3. Quaternion identity, ±90° axis rotations, general noncommuting/composite quaternion, q/−q, scene offset and nontrivial source node/reflection. Mutants must catch wrong quaternion axis frame, swapped XYZW, conjugated rotation, B twice/missing, matrix transpose, rotation/offset order, ignored ellipsoidal height and elevated geocentric-normal frame.
4. NaN/infinity, out-of-range longitude/latitude, malformed/nonunit/zero quaternion, individual finite terms overflowing the magnitude budget, and budget-adjacent accepted/rejected requests. Validate before source discovery/staging. Existing source-node singular/sheared/nonconformal refusals remain intact; no placement fallback.
5. Exact stored local bounds inclusion is tested with no epsilon. Independently transform boxes into world half-axis representation or supporting planes and decoded vertices with complete matrix chain. Use a separately justified <=1e-6m numerical evaluation tolerance for world comparisons; that tolerance must not excuse an inward local box. Include degenerate/flat boxes and large translations: inverse normal projections can otherwise report false floating-point exclusions. Check parent bounds enclose all emitted descendant boxes. Avoid a solver requiring invertible half-axes for flat boxes.
6. Actual supported viewer traverses/renders asymmetric placed geometry at non-equatorial anchor, nontrivial orientation and offset, with near/far routing. Compare known world landmarks/camera axes to establish placement; bounds/request success alone proves neither frame nor material appearance. Lit normal-map tangent appearance remains a separate comparison. Test flat/mixed magnitude cases and large permitted extent/height before claiming universal viewer support.
7. Freeze final source/binary/archive/tool references and limits, record native/browser/platform/wheel acceptance separately, and obtain a nonauthor review after implementation. Failed spatial evidence warrants replacing the responsible component or narrowing an explicit admission domain, not adding a heuristic special-case repair.

## Identity follow-on boundary

Static observation: source `Instance` is currently `(mesh, world, det, normal_matrix)` and decode iterates primitive vectors, discarding selected scene/node/primitive origins from the triangle IR. Placement cannot reconstruct identities from coordinates because distinct instances may coincide. Reserve a stable source association based on selected-scene index, node index (instance), mesh index, primitive ordinal and original triangle/corner ordinals/indices. Reflection corner permutation must carry the association. Spatial leaf membership can preserve the index-bearing identity with no geometry deduplication. F1c1 should document this propagation design and avoid transformations that erase associations; F1c2 owns the actual metadata/feature-ID/picking encoding, consumer APIs and acceptance. Names or source indices alone do not establish picking support.

## Primary semantics used

- [Pinned Khronos glTF2.0 specification](https://github.com/KhronosGroup/glTF/blob/8e798b02d254cea97659a333cfcb20875b62bdd4/specification/2.0/Specification.adoc): metre/Y-up frame, XYZW quaternion and T*R*S source semantics, attributes and tangent handedness. Audit paraphrases remain scoped to these points.
- [OGC3D Tiles1.1](https://docs.ogc.org/cs/22-025r4/22-025r4.html): column-major root affine transforms apply to content/boxes, runtime Y-up→Z-up follows glTF nodes, and error is scaled by largest transform factor. 1.0's different error statement must not be copied into1.1 acceptance.
- [ESA Navipedia ENU/ECEF frame derivation](https://gssc.esa.int/navipedia/index.php/Transformations_between_ECEF_and_ENU_coordinates): ENU columns and distinction between ellipsoidal-normal and spherical-radial up.
- [PROJ geodetic→Cartesian operation](https://proj.org/en/stable/operations/conversions/cart.html): ellipsoidal height input and ECEF axis interpretation; explicit WGS84 parameters are needed because its default ellipsoid differs.
- [Cesium Cartographic](https://cesium.com/learn/cesiumjs/ref-doc/Cartographic.html), [HeadingPitchRoll](https://cesium.com/learn/cesiumjs/ref-doc/HeadingPitchRoll.html): height above ellipsoid and the alternative Cesium Euler signs. They do not choose our orientation API or elevated/pole-frame policy.
