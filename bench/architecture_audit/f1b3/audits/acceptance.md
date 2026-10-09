# F1b3 independent acceptance audit — proposal before implementation

Audited worktree `/home/bend/.cache/rusty-tiles-f1b3-semantics`, head `ed2ac1537b2878a7b346652bedf134e9bd312007`, on 2026-10-10. No production edits or Cargo invocations. This note proposes scope and evidence; it does not accept F1b3 or discharge #121/#113/release gates.

## Recommended finite scope and chunks

1. **UV1, COLOR_0, non-normal PBR channels:** two consecutive texture coordinate sets; f32 / normalized u8/u16 UVs; VEC3/VEC4 f32 / normalized u8/u16 colors; metallicRoughness, occlusion and emissive texture-info forwarding with per-channel coordinate selection and exact material field presence. Keep image/resource ownership and pre-callback capture in F1b2. No new image transcoding, atlas, texture transforms, extensions, sparse/quantized attributes, BLEND, skins or morphs.
2. **Explicit tangents and normal textures:** VEC4 f32 tangent attributes, component/unit/sign validation, reflection and source/output appearance. Prefer a deliberately bounded normal-texture profile initially requiring explicit NORMAL and TANGENT and a proved transform subset. Missing tangents/normals are generally legitimate glTF and should be classified Unsupported by that profile, rather than asserted malformed. Do not silently synthesize a tangent frame using historical helper code.
3. **Broader normal-map transforms or missing tangent support:** a separate proof chunk if chunk 2 cannot show appearance equivalence. Node-transform preservation is a potential architecture solution; MikkTSpace generation and general nonuniform baking need their own independent evidence. Do not promise all nonuniform normal-mapped scenes merely because POSITION/NORMAL flattening was proved in F1a.

The first two chunks may land together only after the tangent convention decision and viewer evidence are concrete. Keep the accepted image/source/resource ceilings, failure ordering, partition/multiplicity and publication semantics; document any report/profile/schema changes explicitly rather than inferring them from existing code.

## Independently consulted primary evidence

Use the already pinned [glTF specification source at `8e798b02…`](https://raw.githubusercontent.com/KhronosGroup/glTF/8e798b02d254cea97659a333cfcb20875b62bdd4/specification/2.0/Specification.adoc) for the contract; the [published Khronos specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html) was independently consulted. Attribute rules specify VEC4 f32 TANGENT with normalized XYZ and sign W; color is a linear multiplier, with VEC3 alpha equal to one; texture sets are consecutive. The tangent basis uses `cross(N,T)*w`. Missing NORMAL ignores supplied TANGENT; missing tangents have a recommended MikkTSpace path.

The [PBR schema](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/schema/material.pbrMetallicRoughness.schema.json) independently identifies roughness G and metallic B as linear, ignoring R/A. The [material schema](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/schema/material.schema.json) specifies normal RGB in tangent space, XY scale, occlusion R/strength and emissive RGB/factor. [Texture info](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/schema/textureInfo.schema.json) requires the selected coordinate accessor. Base-color/emissive RGB use sRGB; normal/MR/occlusion use linear interpretation. Image colorspace metadata does not decide a glTF slot's transfer function.

Independent implementation evidence: [Khronos Sample Renderer vertex shader](https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Renderer/main/source/Renderer/shaders/primitive.vert) transforms T with the model matrix, N with the normal matrix and derives B from their cross product. Its [material shader](https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Renderer/main/source/Renderer/shaders/material_info.glsl) applies normal scale and the channel selections. These are implementation evidence, not additional normative constraints; fetch/pin a commit and hashes before retaining them as acceptance provenance.

## Tangent/normal-map scope decision

The retained pinned Cesium source at `/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/@cesium/engine/Source/Shaders/Model/GeometryStageVS.glsl` transforms and normalizes N, T and precomputed B with its normal matrix. That differs from the Khronos Sample Renderer for nonuniform transforms. Under conformal transforms their directions coincide up to handedness conventions. Under nonuniform transforms an oblique tangent produces different directions from `A*T` and `A^-T*T`; separately transformed B need not equal `cross(N',T')`.

This is a demonstrated convention difference in inspected independent renderer sources, **not an observed rusty-tiles defect** and **not proof that no valid nonuniform baker exists**. Core prose alone does not establish an exact universal bake/render formula. A new oracle using the same chosen tangent equation as production could certify its own convention without proving appearance preservation.

Required resolution: document the mathematical contract and compare an original independently authored source with its converted output in a renderer, using oblique normals/tangents, both tangent signs, anisotropic scale, reflection and normal texels with nonzero X/Y. Include a sensitivity control that deliberately applies the alternate tangent transform. If this fails or is unresolved, return Unsupported for the excluded normal-map transform cases and state that boundary. Keep existing nonuniform support for non-normal-map materials.

For conformal reflection, the expected W change and triangle winding must be derived with an explicit oriented bitangent convention; a corner permutation alone does not prove tangent-handedness preservation. The glTF-to-tile up-axis rotation is proper and must rotate T alongside N. Normal transformation, tangent transformation, W and corner ordering need distinct literal self-tests.

## Existing independent helper blind spots

`tests/f1b_oracle.py` has a valuable independent GLB/PNG/JPEG/corner/archive reader, and `tests/f1b2_oracle.py` independently binds original logical buffers/files before building an oracle-only virtual GLB. Both self-test commands passed at the audited head, including the existing sensitive controls. Their reuse is reasonable for already proved container/image/resource mechanics, but is not new material/attribute evidence:

| Existing code | New blind spot / required change |
|---|---|
| `accessor` | Width table omits VEC4; normalized decoding permits only VEC2. Add independently checked normalized VEC3/VEC4 color decoding and VEC4 tangent handling, with literal u8/u16 expected fractions and raw integer accessor-bound interpretation. |
| `resources` | Canonicalizes only baseColorTexture. Every admitted slot must canonicalize its reference and retain texture-info field presence, per-slot UV set, scale/strength and sampler/image meaning. |
| `used_resource_ids` | Traverses only baseColorTexture. Exact closure must follow all five texture slots, even emissive with default zero factor or normal scale zero. |
| `scene_triangles` / `match_triangles` | Allow only UV0 and compare `(positions,normals,uv0,material)`. Extend each oriented corner with UV1, tangent and color; preserve absence versus presence, and rotate all corner fields together. |
| Material image canonicalization | Decoded image/hash/sampler equality can hide reassignment between two logical image IDs with identical bytes. Attach original image identity to expected channel bindings and verify each leaf reference resolves to the promised original-ID filename. Exact global image inventory alone cannot detect a binding swap when both equal images remain used. |
| F1b2 virtual GLB bridge | Independent resource binding is retained, but new attribute shape/normalization interpretation passes through F1b's reader. Test normalized color/UV1 and interleaved VEC4 bytes with a second literal reader/control rather than treating bridge reuse as a second oracle. |
| `synthetic_archive` and fixture writer | Writer and reader share helper formulas. Require hardcoded expected decoded corners and tangent frames before comparing a helper-authored synthetic archive to itself. |
| `tests/f1b_viewer.py` / `fixtures/f1b_viewer.cjs` | CustomShader UNLIT proves base-color/UV/alpha sampling only. It supplies no MR, normal or occlusion appearance acceptance. |

Keep source/oracle hashes, authoring literals and tool versions in provenance; do not use historical candidate archive hashes as expected geometry or expected pixels. Keep all new expected values independent of conversion reports and the production validator.

## Finite positive matrix

Use 24 authored cases below. Run all 24 as embedded GLB with leaf ceilings 1, 3 and 1000: 72 conversions. Run eight representative cases (P04, P05, P08, P12, P14, P18, P22, P24) again as external JSON glTF at ceilings 1 and 1000: 16 additional conversions. P22's external replay should use mixed multi-buffer GLB instead of JSON, giving both F1b2 transport paths. Total initial acceptance: **88 conversions**, not an exhaustive Cartesian product. Retain the previous F1b1/F1b2 suite, updating only exclusions that became valid.

| ID | Independently authored positive case | Exact expected invariant |
|---|---|---|
| P01 | MR only; asymmetric R/G/B/A pixels and non-default factors | Correct slot, G/B channel meaning; other channels unchanged bytes; factors/presence preserved. |
| P02 | Occlusion only; strength omitted and explicit 1 on different materials | Default/explicit presence; image used without a base-color texture. |
| P03 | Occlusion strength 0 and 0.375 | Preserve zero and fractional value; still publish referenced image. |
| P04 | Emissive only; zero/default and asymmetric factor variants | Correct closure even visually dark; emissive factor RGB/presence. |
| P05 | All five slots, five different images and scrambled texture indices | Per-slot resource binding, exact closure, per-leaf remapping. |
| P06 | Shared ORM image for MR and occlusion, different texture samplers | One image dependency; distinct sampler/texture semantics retained. |
| P07 | One image shared by base-color/emissive and linear slots | Preserve each slot's role; do not impose an image-level colorspace or copy/recode bytes. |
| P08 | UV0 and UV1 visibly different; each slot alternates set | Per-slot texCoord chooses its authored UV; no defaulting all slots to zero. |
| P09 | UV1 f32 outside [0,1], including negative values | UV preserved with current tolerance/domain ceiling; wrapping preserved. |
| P10 | UV1 normalized u8, padded four-byte vertex stride | Literal fractions and valid alignment. |
| P11 | UV1 normalized u16, separate/interleaved layout | Literal fractions, byte stride/offset interpretation. |
| P12 | COLOR_0 f32 VEC3; asymmetric per-corner color, absent texture | RGB retained and alpha equivalent to one. |
| P13 | COLOR_0 f32 VEC4 with non-unit alpha on OPAQUE/MASK | RGBA associated with correct corner; alpha affects MASK coverage. |
| P14 | COLOR_0 normalized u8 VEC3 with padding | Exact normalized fractions; distinguish missing alpha from explicit RGBA. |
| P15 | COLOR_0 normalized u8 VEC4 | All components decoded/forwarded; nontrivial alpha. |
| P16 | COLOR_0 normalized u16 VEC3 and VEC4 on two primitives | Both shapes, endian/stride/padding, literal fractions. |
| P17 | UV1 + color + tangent present on an otherwise untextured material | Attribute presence retained despite no consuming slot; policy for tangent/NORMAL documented. |
| P18 | Explicit NORMAL/TANGENT, both W signs, normalTexture scale omitted | Literal tangent basis, correct image and UV choice. |
| P19 | Normal scale 0, 0.5 and 2; asymmetric normal pixels | Scale/presence preserved; component scaling affects XY only. |
| P20 | Proper rotation plus positive uniform scale, oblique T/N | Analytic normal/tangent and viewer source/output agreement. |
| P21 | Reflected uniform transform; both signs and distinct corners | W convention, winding and all corner associations remain correct. |
| P22 | Interleaved P/N/T/UV0/UV1/COLOR and indexed/unindexed primitives | Correct offsets/stride; all six supported attributes survive regrouping. |
| P23 | Nested/instanced nonuniform transforms with non-normal channels | Preserve retained F1a behavior and new UV/color associations. Add normal-map version only if explicitly proved. |
| P24 | Selected scene uses nonzero image IDs; unused resources, repeated/alias image files including equal encoded bytes | Selected closure, original logical-ID mapping, physical-byte accounting and leaf closure independently calculated. |

The fixture count should be fixed after the final contract; any split normal-map slice may defer P18–P21 while still exercising P17/P22 attributes where admitted. Cases must not be marked passed by skipping a rejected feature.

## Finite refusal/failure matrix

For each new source feature, test malformed metadata before dependency I/O: bad slot/texture/UV references and wrong JSON types; missing chosen UV accessor; mismatched counts; COLOR shape/type/unnormalized integer/nonfinite/out-of-range values; TANGENT shape/type/normalized flag/nonfinite/zero XYZ/nonunit XYZ/invalid W; UV1 finite domain, range, alignment and stride; bad accessor raw min/max/extrema. Include used and unused declarations and an earlier missing dependency to prove declaration/admission precedence. Repeat every refusal at leaf ceilings 1 and 1000, asserting typed kind, exit code and no output/workspace creation.

Unsupported controls: texCoord >=2, TEXCOORD_2/COLOR_1, texture transforms and material extensions, missing tangent/normal under the selected normal-map profile, excluded normal-map transforms, plus retained skins/morphs/BLEND/sparse/quantization exclusions. Distinguish legitimate out-of-profile documents from malformed references or encodings. Validate numeric ranges from the pinned schemas: occlusion strength [0,1], color channels [0,1], emissive factors [0,1], metallic/roughness [0,1]; normal scale is a finite number with **no schema minimum** (negative scale is not automatically malformed). Apply an explicit profile ceiling if needed.

Resource/failure cases should reuse the proved ownership tests but activate each additional image path: missing used normal/occlusion/emissive dependency; invalid unused new-slot image; same physical alias versus separate logical images; output aliases to new selected and unused resources; mutation/deletion after preparation; image-write fault/cancellation; peak encoded leaf bytes and attribute allocation for fully populated P22. Measure resource use separately from the small quadratic corner oracle.

## Required sensitive oracle controls

At least these 20 mutations must fail the same final acceptance predicate: omit MR; omit normal; omit occlusion; omit emissive; swap two slot bindings; rewrite a nonzero image-ID binding to an equal-byte different logical image; omit explicit texCoord; change UV1 to UV0; swap UV1 corners; omit COLOR; swap color corners; change only color alpha; force VEC3 default alpha to zero; omit tangent; change tangent XYZ to the alternate transform convention; flip tangent W; reflect positions/winding without tangent handedness; change normal scale; change occlusion strength; omit or orphan a non-base-color image. Supplement with no-base-color fixtures so an accidentally unchanged baseColor-only closure walker cannot pass.

Mutation sensitivity must include material scalars set to zero, omitted-versus-explicit defaults, same-byte aliases and CRC-valid archive rebuilds. Do not accept mutation failures caused solely by malformed archive framing when the intended checker is semantic fidelity.

## Viewer proof and official validation

Retain the existing literal nearest/clamp UV/OPAQUE/MASK unlit panels. Add UV1 and COLOR RGB/alpha panels with literal expected colors/coverage. Add a separate **lit** source/output comparison under fixed camera/exposure/directional light and a controlled local environment: MR panels with distinct metallic/roughness responses; normal panels with nonzero XY samples, both signs, scale and transforms; occlusion panels under indirect illumination (a direct-only light cannot prove occlusion); emissive panels with low midtone RGB/factors to expose sRGB interpretation. Source and candidate load through the same pinned renderer, but source is independently authored and never generated by conversion. Require nonzero expected effects as well as closeness, and rerun wrong-slot/UV/tangent-sign/scale/strength/color controls through the same assertion. Record framebuffer samples, camera/lighting, actual render commands/selected leaves and request/error logs. Exact shader/BRDF pixels are not universal glTF requirements; claims must name the pinned renderer and tolerance.

The local official Khronos package is `/home/bend/.cache/rusty-tiles-135-validator/node_modules/gltf-validator`, version `2.0.0-dev.3.10`. The existing `tests/fixtures/t1_validate_glb.cjs` calls `validateBytes` without an external resource callback: this is insufficient for F1b shared images. The [official npm API documentation](https://github.com/KhronosGroup/glTF-Validator/tree/main/npm) states external resources are not validated when the callback is omitted. New validation must use a separately written confined archive-map `externalResourceFunction`, validate every source and every leaf, record callback URI/byte counts and validator/library hashes, fail unresolved resources, and inspect errors plus expected warnings. Use maxIssues 0 or explicitly reject truncation. Allow only enumerated advisory messages justified by fixtures (for example unused tangents or degenerate triangles); do not globally ignore warnings or replace the independent semantic oracle with validator success.

Read-only independent validator probes executed during this audit:

| Manual source mutation | Official result |
|---|---|
| Baseline P/N/T/COLOR VEC3 | Zero errors/warnings; informational UNUSED_MESH_TANGENT. |
| COLOR_0 f32 first component 1.5 | ACCESSOR_NON_CLAMPED error. |
| TANGENT first W = 0 | ACCESSOR_INVALID_SIGN error. |
| TANGENT present, NORMAL absent | Zero errors, MESH_PRIMITIVE_TANGENT_WITHOUT_NORMAL warning; tangent ignored semantics remain profile decision. |

Correction made explicitly during audit: an initial reading treated out-of-range color as a client clamp obligation. The independently run validator contradicts that proposed admission; the acceptance contract should require authored color data already in range. Probe results are independently observed standards evidence, not production behavior.

## Acceptance exit criteria

Root settles the contract and tangent scope first. An independently authored oracle then passes its literal self-tests and all sensitivity controls, candidate passes the finite positive/refusal matrix, existing F1b1/F1b2 and lifecycle/resource/frontend checks remain green, Khronos validates source/output with resolved dependencies, and the new viewer controls pass/fail as intended on the exact final binary. Pin commit/binary/oracle/fixture/renderer/validator hashes and retain any earlier defects separately. Passing these criteria certifies only the admitted full-detail material/attribute profile; it does not establish LOD, extensions, general renderer equivalence or release readiness.
