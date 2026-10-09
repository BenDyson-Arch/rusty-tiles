# F1b3: core PBR bindings and authored vertex companions

Implementation slice [#140](https://github.com/BenDyson-Arch/rusty-tiles/issues/140) of #121, based on develop `f43eb7889b04699e755425ae49707765f6204a2a` (merged F1b2 #139). The same `MeshRequest::local_gltf`, CLI `mesh-local-to-3tz`, and Python `mesh_local_to_3tz` advance to this profile. There is no compatibility mode or legacy fallback. This slice does not close #121 or authorize a release.

## Ownership and admission

Retain the [F1b2 document/resource capture contract](f1b2-contract.md), finite limits, source/output checks, whole-document admission, and F0 publication. The pure source decoder owns semantic validation and prepared corner attributes; it sees no paths. A private validated material plan owns the five core texture bindings. The encoder derives global and per-leaf resource closure from that plan and encodes every companion through one channel writer. Runtime has no material or attribute policy.

Admit base-color and metallic-roughness textures under `pbrMetallicRoughness`, and normal, occlusion, and emissive textures on materials. Each binding retains its texture index, optional UV selector, and supported slot-specific scalar fields. UV selector omission means zero; selectors zero and one are supported. Other nonnegative sets are Unsupported; malformed selectors, references, or scalar fields are InvalidInput. Every declared primitive using a material must supply all UV sets its bindings reference, including primitives outside the selected scene. Missing referenced UV sets are InvalidInput.

Normal texture scale accepts any finite number, including zero and negative values. Occlusion strength must be finite in [0,1]. Existing factor, alpha cutoff, double-sided, OPAQUE/MASK, and sampler admission rules remain. Do not invent explicitly serialized defaults, combine textures by filename or role, repack channels, resample, or reinterpret encoded image transfer functions. One source image can serve multiple roles; color-space interpretation belongs to its material binding. Preserve PNG/JPEG bytes.

## Vertex companions

Admit POSITION, optional NORMAL, TANGENT, TEXCOORD_0, TEXCOORD_1, and COLOR_0. UV1 requires UV0, consistent with consecutive semantic sets. No extensions, extra UV/color sets, morph targets, skins, animation, texture transforms, or BLEND are added. Unsupported fields are refused consistently before dependency I/O where metadata permits.

UVs use FLOAT VEC2 or normalized unsigned-byte/unsigned-short VEC2. COLOR_0 uses FLOAT or normalized unsigned-byte/unsigned-short VEC3/VEC4. Store colors as linear RGBA FLOAT with implicit alpha one for RGB inputs; retain absence of COLOR_0 when absent. Admit color components in [0,1]; finite values outside that range are Unsupported by this bounded profile, with no clamping or repair. This admission disposition does not assert that every excluded source is otherwise valid: the pinned Khronos validator reports `ACCESSOR_NON_CLAMPED` as an error. Nonfinite payloads are InvalidInput. Accessor min/max still compare exact raw component extrema, including normalized integer data, before semantic normalization. Width-four accessors require width-four extrema storage.

TANGENT is non-normalized FLOAT VEC4, with unit XYZ within the existing 1e-4 vector tolerance and W exactly -1 or +1. This profile requires authored NORMAL with TANGENT. A normal texture also requires authored NORMAL and TANGENT; missing authored companions are Unsupported, rather than a promise of generated MikkTSpace equivalence. Tangent W must agree across each triangle; mixed handedness is Unsupported because the core specification leaves that triangle's tangent space undefined. Do not require an extra authored normal/tangent orthogonality invariant absent from the core specification.

All vertex counts agree with POSITION. Alignment, buffer-relative overlap, targets, index range, and finite payload checks apply to every declared primitive. Validate each accessor once per semantic role instead of rescanning shared data per primitive. Expand all companions with exactly the same indexed corner selection, reflection permutation, partition membership, and regrouping as positions. Output companions are FLOAT VEC2/VEC3/VEC4; their component conversion tolerance follows the existing f32 contract.

## Tangent-frame bake boundary

For each selected primitive carrying TANGENT, require its accumulated world linear map to be conformal: a uniform absolute scale times an orthogonal matrix, allowing reflections. Check the accumulated Gram matrix against its scalar diagonal with relative tolerance 1e-10; inspecting individual node scales is insufficient. Nonconformal accumulated maps are Unsupported before staging. Existing position/NORMAL-only nonuniform transforms remain governed by the earlier contract.

Within this profile, apply the accumulated linear map to tangent XYZ and normalize, apply inverse transpose to normals and normalize, multiply tangent W by the determinant sign, and apply the established Y-up/Z-up conversion and reflection corner permutation exactly once to every channel. Preserve the authored relationship between normal and tangent. Independent arithmetic and source-versus-output lit rendering must establish the admitted cases.

This is an explicit restrictive producer policy. Current readers do not establish equivalent tangent-space shading after arbitrary nonuniform baking: pinned Cesium and the Khronos sample renderer use different tangent-frame transform conventions there. Supporting those cases needs a proved representation or bake contract; no corrective texture rewrite or silent fallback is introduced.

## Closure, report, and proof

Derive selected image closure from every binding of every used material, and each leaf's exact material -> binding -> texture -> image/sampler closure. Keep logical resource identities; different textures can share an image and retain different samplers. Remap every admitted slot index, preserve binding UV/scalar field presence, publish each selected source image once, and publish no orphan resources.

Report schema remains 3 because the fields and counting meanings are unchanged. Profile advances to `f1b-core-pbr-gltf-v1`. Rust, CLI, Python, and `conversion.json` must agree. Existing root/external byte counts, selected image counts, and routing-error semantics remain unchanged. The changed profile replaces the previous one.

Independent acceptance covers each slot alone and all slots together, UV0/UV1 association, normalized UV/color and RGB alpha, tangents under rotation/uniform scale/reflection/nested transforms, absent/explicit defaults, selected versus unused resources, aliased images, exact closure, typed refusals, and multiple leaf limits. Sensitive controls must detect plausible wrong slot/UV/scalar mappings, lost companions, and reflected tangent handedness. A lit source-versus-output browser check is required for new PBR behavior; the prior unlit base-color probe is regression evidence only. Supplement it with official Khronos validation resolving external images. Replay the earlier geometry/image/URI/lifecycle, installed-wheel, and finite-resource evidence against the final candidate.

Primary semantics: [pinned Khronos glTF specification](https://github.com/KhronosGroup/glTF/blob/8e798b02d254cea97659a333cfcb20875b62bdd4/specification/2.0/Specification.adoc), its core material/texture schemas, and [the prior F1b1 fidelity contract](f1b-contract.md). The conformal limit and finite color admission boundary are declared profile decisions, not additional general glTF requirements.

Placement, metadata/picking, independently justified approximation/error, implicit delivery, and explicit supported-use decisions before legacy mesh deletion remain under #121/#120/#125/#126.
