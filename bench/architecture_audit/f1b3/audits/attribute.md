# F1b3 attribute and geometry audit

Audit baseline: worktree `/home/bend/.cache/rusty-tiles-f1b3-semantics`, HEAD `ed2ac1537b2878a7b346652bedf134e9bd312007`. The initial audit was read-only. A subsequent root implementation assignment authorized edits limited to `source.rs` and the private Triangle test literals in `partition.rs`; those edits are now complete. No Cargo invocation by this agent. An independent arithmetic receipt is `/home/bend/.cache/rusty-tiles-f1b3-attribute-math.json`.

## Implementation disposition after audit

The root selected fixed owned Triangle fields (positions, optional normals/tangents/colors, two optional UV arrays and optional material), retaining source-module ownership for this small slice. The proposed separate enum/vector IR below is an audit alternative, not the implemented arrangement. Root encoder code owns common channel grouping/emission. Source uses a finite VertexRole iterator for common layout/count/alignment checks and one role/accessor payload cache; UV0/UV1 share cache entries for identical accessor identities. Geometry and Document now carry the material agent's prepared Material type.

Implemented tangent policy requires authored NORMAL whenever authored TANGENT is present; this is explicitly Unsupported for otherwise valid tangent-only input. Normal textures require authored NORMAL+TANGENT and their bound UV. Every admitted tangent primitive requires uniform W signs across its rendered triangles. Selected tangent instances use the scaled accumulated Gram-matrix conformity check at relative 1e-10. The source retains nonorthogonal authored n/t relationships, transforms tangent linearly, adjusts W by determinant sign and permutes every channel with positions.

Finite COLOR_0 outside [0,1] is **Unsupported** as the root-selected bounded admission policy, rather than this audit's earlier InvalidInput proposal. Nonfinite binary channels remain InvalidInput. This classification is deliberately bounded-profile policy; it is not a claim that Khronos universally treats such input as valid. The original audit proposals below remain for provenance. RGB alpha is synthesized as one only after raw extrema checks; all VEC4 raw extrema now use width-four scratch arrays.

Verification by this agent: rustfmt succeeded and source diff reviewed; independent algebra receipt generated. No Cargo/test execution by this agent. The root owns compile, independent-oracle, frontend and viewer acceptance.

### Corrective metadata classification

The root reported an independent F1b3 run that passed 94 positive cases, then rejected an unnormalized integer COLOR_0 with Unsupported when the standard-invalid admitted semantic required InvalidInput. This is a reproduced candidate classification defect, distinct from the initial baseline's deliberate lack of COLOR_0 support.

The final source separates bounded generic accessor metadata from finite storage eligibility. Generic metadata recognizes signed components and matrix shapes solely to classify admitted role contradictions; it calculates matrix column padding/ranges but never decodes matrix/signed payloads. VertexRole validates core encoding and returns InvalidInput for contradictions. Globally impossible normalized FLOAT/u32 is InvalidInput. The original finite accessor profile guard runs after primitive metadata and before returning any dependency-capture request; valid unused signed, matrix, scalar-float and other excluded storage remains Unsupported. Decoder extrema scratch remains four-wide because matrices are excluded before decoding.

Three private source metadata tests cover malformed used COLOR encodings (integer unnormalized, signed, matrix, scalar), valid unused excluded signed/SCALAR-float/MAT4/u32-vector layouts, and invalid normalized FLOAT/u32. Test fixtures declare a missing external dependency and call only pure Document::parse, so classification does not require resource capture. Rustfmt succeeded; root owns execution. source.rs was locked and handed back to root before final build/evidence freezing.

## Findings by evidence class

**Source observations:** `source.rs` owns the prepared triangle/image/geometry types. `encode.rs` and `partition.rs` import those types directly from the source decoder module. The triangle has independent optional NORMAL and TEXCOORD_0 fields. Primitive admission and payload validation repeat channel-specific loops; encoder grouping is `(material, has_normal, has_uv0)` and emits each optional channel in separate branches. This is reasonable for two channels but copying the pattern for UV1, tangent and color will proliferate presence flags, resource predicates and writers. These are design observations, not independently reproduced conversion defects.

Admission explicitly rejects TANGENT, TEXCOORD_1 and COLOR_0. The existing unsupported-attribute tests verify that earlier bounded contract, so these refusals are not defects. Source names are not preservation identities.

**Latent extension hazard:** the raw accessor extrema arrays in `decode_accessors` have three elements (`source.rs:440-441`). Simply allowing VEC4 in layout admission would make the component loop index beyond those arrays. Extend extrema scratch to four or accessor width as part of introducing VEC4; do not regard a widened shape whitelist as sufficient implementation. This is proven from control flow, not a run of modified production code.

**Reproduced mathematical counterexample:** the attached independent Python calculation shows that naively baking a general nonuniform matrix and reconstructing bitangent from normalized transformed normal/tangent does not preserve the linearly transformed authored tangent frame. This is an arithmetic limitation of a proposed algorithm; current production rejects tangents and therefore has no reproduced tangent conversion defect.

No newly reproduced production defect was established in this audit. Existing source behavior must still pass independent F1b3 acceptance before being retained.

## Primary semantics verified

Source: [Khronos glTF specification at registry commit 8e798b02d254cea97659a333cfcb20875b62bdd4](https://github.com/KhronosGroup/glTF/blob/8e798b02d254cea97659a333cfcb20875b62bdd4/specification/2.0/Specification.adoc), especially accessors-bounds and meshes-overview. A local raw copy is `/home/bend/.cache/rusty-tiles-f1b3-gltf-spec.adoc`.

- UV sets accept FLOAT or normalized unsigned byte/short VEC2. Set indices begin at zero and are consecutive.
- TANGENT uses FLOAT VEC4, unit XYZ and W exactly ±1. Bitangent derives from cross(normal,tangent) times W. Mixed W signs within a triangle leave tangent space undefined.
- COLOR_0 uses FLOAT or normalized unsigned byte/short VEC3/VEC4. RGB implies alpha one; color is a linear base-color multiplier. Components must lie in the clamped unit interval.
- Every primitive attribute count agrees. POSITION requires extrema. Bounds concern raw stored components, regardless of normalized decoding; FLOAT bounds round to f32 before comparison.
- Missing normals imply flat normals and ignored authored tangents. Missing tangents may require MikkTSpace generation.

The pinned spec does not establish one universal nonuniform tangent-baking algorithm. Normal-map frame preservation under converter baking is a separate implementation claim requiring evidence.

## Proposed finite source contract

Admit exactly optional NORMAL, TEXCOORD_0, TEXCOORD_1, TANGENT and COLOR_0 with POSITION required. Unknown/custom channels and higher sets remain Unsupported. TEXCOORD_1 without TEXCOORD_0 is InvalidInput because the supported consecutive set sequence is broken. Preserve both UV sets even if no material samples them. UV sets are not renamed, flipped, rescaled or collapsed when their values happen to agree.

Use existing UV finite magnitude ceiling (absolute components <=1,000,000), normalized-integer decode and nearest-f32 storage tolerance for both sets. Reflection permutes all attribute corners using the exact same vertex-index triple as positions. Index expansion preserves every corner association and duplicate/degenerate triangle multiplicity.

COLOR_0 is decoded to one canonical RGBA f32 channel; VEC3 appends alpha 1, VEC4 retains authored alpha. Integer channels decode c/255 or c/65535 in f64 before nearest-f32 storage. Do not apply sRGB transfer, premultiply alpha, bake material factors, or omit colors from an absent/default-material primitive. Retain absence of COLOR_0. The renderer interpolates the retained per-vertex linear multipliers; the converter performs no interpolation or subdivision.

For FLOAT color payloads, admit only finite values in [0,1], with InvalidInput outside that range. This is an explicit authored-input validation choice that preserves a conforming input's values and avoids silent repair. The pinned normative text uses clamping language; Khronos Validator's independent `ACCESSOR_NON_CLAMPED` diagnostic treats out-of-range elements as errors. A different contract that deliberately accepts then clamps such data would need to say so and test raw extrema before clamping. Do not silently mix those two policies.

Generalize raw accessor handling separately from semantic roles. A byte-layout decoder may understand supported SCALAR/VEC2/VEC3/VEC4 storage, but each primitive binding checks its own shape, component type and normalization. In particular, adding normalized uint VEC3 for colors must not admit uint POSITION/NORMAL. Keep raw unsigned components available for extrema verification. Optional provided min/max use raw integers even for normalized colors/UVs, with exact observed extrema across all four channels. Do not calculate extrema from normalized or synthesized RGBA data.

Keep actual range/stride/vertex alignment/count checks across every newly admitted attribute, including unused primitive declarations. A tightly packed three-byte RGB or six-byte RGB vertex array cannot satisfy the existing per-vertex four-byte stride rule: valid packed normalized colors need explicit stride/padding (4 or 8). VEC4 byte/short may use tight stride 4/8. FLOAT VEC3/VEC4 uses 12/16. Last-element range is `(count-1)*stride + component_size*source_width`, not `count*stride`, so legal omitted trailing pad still works. Missing sparse/extension features remain Unsupported; do not inherit them through broader component support.

## Deliberate tangent disposition

Recommended bounded slice: support authored tangents under **conformal accumulated world transforms**, including reflections. For a tangent-bearing selected mesh instance, let M be the full accumulated 3x3 matrix. Require `MᵀM` to equal a positive scalar times identity within a declared tight f64 relative tolerance; one candidate is 1e-10. Check accumulated matrices, not just local TRS or local matrix columns. Nonuniform transforms remain admitted for geometry without tangents under the existing position/normal contract.

Choose and document the numerical conformity tolerance alongside tangent storage tolerance. A permissive 1e-6 conformity threshold is not automatically compatible with 2e-7 component preservation. Depth-bounded composition noise of valid uniform-scale rotations should be tested independently. Compute the positive scalar robustly or remain within the existing finite matrix arithmetic domain; do not add an unbounded generic matrix library.

Validate tangent XYZ finite and unit length within the existing normal tolerance (candidate 1e-4), then normalize according to a declared policy. W is exactly +1 or -1; a magnitude near one does not authorize rounding handedness. There is no explicit core-spec orthogonality requirement for n·t, so do not invent an InvalidInput rule that silently orthogonalizes authored directions. Any deliberate narrower condition must be named Unsupported and justified separately.

Bake tangent XYZ with normalized `M*t` in f64 and store nearest f32. Preserve existing normal bake `normalize(M⁻ᵀ*n)`. Set baked W to `source_W * sign(det(M))`, and still swap reflected triangle corners. Changing W and changing winding serve separate purposes and both are needed. Require a tangent component storage bound (candidate <=2e-7 relative to the computed unit direction), and exact W preservation after determinant sign adjustment.

Coordinate with material audit: for a normal-textured primitive, require authored NORMAL + TANGENT and the exact UV set selected by that texture. Missing authored normals/tangents on otherwise valid glTF are Unsupported in this deliberate subset, rather than generated or silently ignored. Require one W sign across each rendered triangle whose tangent space is used; mixed signs describe undefined source tangent space and should be Unsupported. TANGENT without NORMAL is valid core input whose tangent is ignored by the renderer; it can be retained under the same conformal attribute restriction without inventing normals. The contract may restrict all tangent-bearing selected instances uniformly for simplicity. Unselected instances do not get baked, but all declared tangent payloads still receive finite/unit/W validation.

If this restriction is disproportionate to the intended first useful slice, defer TANGENT and normalTexture together while implementing the other material slots, UV1 and colors. Do not claim general normal-map preservation using a convenient normal/tangent transform without an independent contract.

## Independent arithmetic evidence

For n=(0,0,1), t=(1,1,0)/sqrt(2), w=+1 and M=diag(2,1,1):

- original b=(-1,1,0)/sqrt(2)
- normalized M*t=(2,1,0)/sqrt(5)
- normalized M*b=(-2,1,0)/sqrt(5)
- cross(normalized M⁻ᵀ*n, normalized M*t)=(-1,2,0)/sqrt(5)

The last two directions differ by Euclidean distance 0.6324555320. That rejects a blanket assertion that the reconstructed baked frame is the transformed authored frame. It does not define all renderer normal-map behavior, or prove that every nonuniform input is incorrect. A conformal restriction avoids making that broad claim.

For M=diag(-1,1,1), multiplying W by -1 restores the transformed original bitangent direction in the independent receipt. For nested parent scale (2,1,1) and child Z rotation 45 degrees, accumulated Gram matrix has off-diagonal -1.5 even though both local nodes individually admit glTF TRS. This fixture distinguishes accumulated-world checks from superficial local checks.

## Clean owned IR and encoder arrangement

Move prepared Geometry, Triangle and Image types out of `source.rs` into a small private `ir.rs` module. Source produces this owned representation; partition and encoder consume it. No source document, borrowed accessor bytes, serde JSON geometry accessor, filesystem capability or legacy mesh IR should escape preparation. This is an internal ownership correction, not a public schema or compatibility layer.

A practical small representation keeps `Triangle.positions` explicit for partition and bounds, and replaces the expanding optional-channel fields with a canonically ordered vector of typed `CornerAttribute` variants:

```rust
Normal([[f32; 3]; 3]),
Tangent([[f32; 4]; 3]),
Texcoord { set: TexcoordSet, values: [[f32; 2]; 3] },
Color([[f32; 4]; 3]),
```

TexcoordSet is a finite enum with exactly Zero/One. Attribute kind is an ordered finite enum; each payload variant provides kind, arity and a flattened f32 slice. Order and uniqueness are checked once during preparation. This avoids invalid semantic/shape pairs, channel options on every corner, unknown extensible payload types, and a growing boolean tuple. A source Primitive can retain canonical attribute/accessor bindings so count/alignment and semantic validation traverse the same binding list. Irreducible normal/tangent/UV/color semantic operations live in one decode dispatch; common range and emission logic is shared.

Encoder grouping key is material identity plus the canonical attribute layout signature. The signature may be a tiny finite mask wrapper or ordered kind sequence; avoid adding an extensible registry or interning framework before measured need. Every admitted layout has homogeneous triangle payloads. Emit POSITION with stored-f32 extrema, then loop its optional kinds through one common channel writer/accessor builder. Canonicalize output colors to FLOAT VEC4 and UVs to FLOAT VEC2; normal/tangent remain FLOAT VEC3/VEC4. Do not recreate six separate encoder branches and a five-boolean grouping tuple. One preparation invariant check per group can establish each triangle's layout; emit an Invalid prepared-IR error for impossible mixtures.

This representation has bounded additional allocation/storage cost and must be measured at the 100,000 expanded-triangle boundary. It makes no constant-memory/RSS claim. Do not select a more complex pooled or source-indexed mesh design solely to avoid a few finite allocations without measurement.

All material bindings should expose one iterator of typed texture references including their UV sets. Use that same prepared semantic representation to compute resource closure and leaf remapping, so the encoder does not rediscover source JSON fields. The material agent owns the concrete material type design. Tangent/normal-map prerequisites are checked during preparation, not inferred by the encoder from one specific JSON slot.

## Acceptance needed before retention

Extend an independent literal fixture author/reader, without importing production IR, source decoder, encoder or production transform helper. Existing Python oracle helpers are independent candidates, but its current attribute whitelist, tuple shape and base-color-only resource traversal are incomplete for F1b3 and must themselves prove the new checks with corruption controls.

Positive cases: both genuinely different UV sets; each uint width and RGB/RGBA FLOAT color; raw color/UV extrema; valid padded/interleaved layouts with omitted final pad; absent attributes/default material; same geometry repeated with differing colors/UVs; conformal nested rotations and uniform signed scales; negative determinant tangent W and complete corner associations; standalone untextured attributes. Include positions and tangent directions whose coordinates prevent a wrong transform from accidentally passing axis-aligned examples.

Negative cases: UV1 gap, wrong role/shape/normalization, mismatch counts, bad fourth-channel extrema, out-of-range color, NaN/Inf in any channel, invalid tangent XYZ/W, bound texture UV absence, missing authored tangent/normal, mixed triangle W, reflected corner corruption, general nonuniform and nested accumulated nonconformal tangent instances. Distinguish valid excluded inputs (Unsupported) from malformed inputs (InvalidInput).

Sensitive controls independently change UV0 vs UV1 binding, tangent W alone, tangent direction alone, color corner/alpha/gamma, a fourth-channel bound, and channel-group presence while retaining plausible valid GLB output. Match the complete oriented triangle multiset including every channel and each material slot's resource identity. A parser roundtrip does not establish interpolation, texture-role appearance or tangent-frame semantics.

Pinned viewer evidence must show color multiplier/alpha MASK, different slot-selected UVs and a directional normal texture under positive and reflected conformal instances. Include a normal-map scale control and a deliberately wrong W/corner control. State limits: arithmetic/structural checks establish the declared representation, while appearance is evidenced only for the viewer/version/render setup actually exercised.
