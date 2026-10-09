# F1b3 independent core-PBR and corner acceptance

`f1b3_oracle.py` independently authors and interprets the [bounded F1b3 profile](../docs/architecture/f1b3-contract.md). It imports no production source decoder, encoder, validator or coordinate helper. It reuses the established independent F1a arithmetic/GLB/index and F1b PNG/JPEG/archive mechanics plus F1b2 physical dependency binding. Its extended scene reader, oriented corner matcher, fixture literals, image-ID binding and conformality check are independently written here.

The current producer profile is `f1b-core-pbr-gltf-v1`, schema 3. This replaces the previous profile without a compatibility branch. Earlier F1b/F1b2 replay helpers now assert this profile; their old four-field scene/multiset matcher retains its established predicates. The new helper injection points in archive inspection and dependency binding reuse container mechanics without replacing old checks or projecting away the new attributes.

## Exact finite scope

There are 26 authored variants at leaf limits 1, 3 and 1000, plus eight external-resource replays at limits 1 and 1000: 94 positive conversions. There are 70 source refusal bundles, each at limits 1 and 1000: 140 refusal checks. Use the source manifest and reported cases for executed counts rather than assuming a receipt remains current after code changes.

The fixtures cover all five core material slots separately and together; scrambled texture indices; ORM sharing and samplers; one encoded image used in both color and linear roles; absent versus explicit factors/selectors/scale/strength; UV1 f32/outside-unit/normalized u8/u16; COLOR RGB/RGBA f32/normalized u8/u16 and implicit alpha one; both tangent W signs; proper uniform scale/rotation, reflection, nested nonuniform node scales whose accumulated transform is conformal, nonorthogonal authored N/T, all-channel interleaving, indexed/unindexed primitives, and retained nonuniform geometry without tangents. Selected nonzero image IDs include two logical images with identical encoded bytes and an external URI alias. Every material-slot binding carries independently checked original image identity, so equality of pixels/encoded bytes does not conceal an identity swap.

Each oriented triangle corner compares POSITION, presence/value of NORMAL/TANGENT/UV0/UV1/COLOR, and material meaning. All fields rotate together for cyclic winding equivalence. Source RGB becomes canonical RGBA with alpha one, and output color must be f32 VEC4. UV/color storage uses the existing `max(1,abs(value))*2^-24` component tolerance; XYZ normal/tangent comparison uses 2e-7, W compares exactly. Source vector unit admission is 1e-4. Conformality is derived from the accumulated column Gram matrix at relative tolerance 1e-10 rather than inspecting local node scales. Reflection independently flips W and permutes every associated corner. No orthogonality repair is applied.

The exact archive inspector resolves all core slots, checks per-leaf material/texture/image/sampler closure, exact global selected images and original-ID filenames, unchanged PNG bytes/pixels/MIME, ZIP CRC, member inventory, independent 3TZ index, conservative stored-vertex/ancestor bounds, routing metric and schema/report/frontend parity. It does not infer source correctness from a conversion report or archived historical output digest.

Refusals include every slot's malformed index, missing chosen UV and excluded UV2/extension; malformed new attribute counts/shapes/ranges/nonfinite payloads; invalid sign/unit tangent, missing authored companions, nonconformal selected tangent transform even without a normal texture, mixed per-triangle W; unsupported finite color range; wrong normalization/component combinations; nonconsecutive UV sets and unsupported UV/color sets; scalar ranges; and semantic metadata before a missing dependency. Source encoding contradictions are checked against the core Khronos attribute table, independently of candidate behavior. Out-of-range finite color is the deliberately documented Unsupported profile boundary; the official validator reports ACCESSOR_NON_CLAMPED as an error, and no broader-validity claim or clamping is made.

## Independent sensitivity and replay

Literal self-tests independently establish UV1, RGBA, normalized RGB fractions, reflected tangent/sign/up-axis and corner order before any candidate is called. Controls detect dropped slots or attributes, altered UV1/color-alpha/tangent-XYZ/W, wrong winding/slot/scale/strength, omitted explicit selectors, wrong original-image IDs, and equal-byte logical-image reassignment. CRC-valid independently authored archives pass complete inspection first, then fail the same predicate for missing/orphan/changed non-base images and leaf slot/UV/normal-channel corruption. Existing F1b/F1b2 image/geometry/resource sensitive checks remain active.

```sh
python3 -S -B tests/f1b3_oracle.py --self-test
python3 -S -B tests/f1b3_oracle.py --binary /absolute/path/to/rusty-tiles \
  --json-output /tmp/f1b3-oracle.json
python3 -S -B tests/f1b3_oracle.py --static-fixtures tests/fixtures/f1b3
```

`--binary` always runs self-tests before conversion. Static authored source files are under `tests/fixtures/f1b3/`; their manifest records authoring provenance, every asset SHA256/length, source document hash, selected image IDs and unique physical dependency accounting.

Callable interfaces for root-owned Rust/CLI/installed-wheel parity are `VARIANTS`, `EXTERNAL_VARIANTS`, `write_fixture(root, variant, external=False) -> Path`, `inspect(source, archive, leaf_limit) -> dict`, and `refusal_bundles() -> [(name,bundle,kind), ...]`. `f1b2_oracle.write_bundle` writes refusal bundles without accepting or repairing invalid metadata. `viewer_fixture()` independently authors a separate lit analytical panel source.

## Lit renderer and Khronos proof

`f1b3_viewer.py` loads the independently authored original source and every extracted output leaf with pinned Three `0.180.0` GLTFLoader and WebGLRenderer in a common glTF frame. It uses the original MeshStandardMaterial PBR pipeline without shader or material overrides. Six panels exercise MR, normal, occlusion, emissive, base color/COLOR and all channels together, with UV0/UV1 differences, nearest/clamp, nonzero normal XY, authored negative tangent W and positive uniform scale. A fixed directional light plus AmbientLight produces a real indirect term for occlusion. Camera, lighting, framebuffer pixels, loaded leaves/render commands, request/page errors, renderer and browser versions are recorded. Maximum sampled RGB source/output difference must be <=12/255; every deliberate wrong UV/slot/W, dropped MR/normal/occlusion/emissive, wrong color/normal scale/occlusion strength control must exceed the same tolerance. Every source panel must visibly render.

```sh
python3 -S -B tests/f1b3_viewer.py --binary /absolute/path/to/rusty-tiles \
  --three-dir /existing/node_modules/three \
  --node-modules /existing/node_modules --chromium /existing/chrome \
  --json-output /tmp/f1b3-viewer.json
```

The earlier Cesium unlit and routing viewers remain separate regression evidence. This standalone GLTF renderer proves extracted-leaf material appearance, not 3D Tiles traversal. Lit claims are for the pinned Three/browser, analytical samples and stated lighting/tolerance; they do not claim universal glTF BRDF pixels or arbitrary filtering equivalence. Reflection and nested/nonorthogonal frame arithmetic remain explicit independent corner cases; any extension of viewer scope must be recorded. Replay requires an existing pinned Three package; the runner downloads nothing.

An initial independent Cesium `1.146.0` lit probe matched source/output pixels but failed the required occlusion sensitivity: dropping occlusion produced RGB difference zero despite local diffuse spherical harmonics. Inspection of its pinned ordinary-PBR shader showed indirect lighting is not multiplied by occlusion and no strength application in the material stage. This renderer limitation is not an observed converter defect. The failed sensitivity was retained as a limitation and prompted the separate standard Three renderer; no threshold was loosened and no custom shader was used to manufacture occlusion behavior. Primary implementation evidence is [Three r180 GLTFLoader](https://github.com/mrdoob/three.js/blob/r180/examples/jsm/loaders/GLTFLoader.js) and its [ambient-occlusion shader](https://github.com/mrdoob/three.js/blob/r180/src/renderers/shaders/ShaderChunk/aomap_fragment.glsl.js).

`tests/fixtures/f1b3_validate.cjs` uses the official Khronos `gltf-validator` with a separately authored confined external-resource callback. Unlike a validateBytes-only call, it resolves the emitted shared images and records callback URI/path/length/hash. It rejects errors/truncated results and records warnings for review, with no global ignored-issue list. Replay sources and extracted output leaves, pinning the validator version and package/entry hashes.

```sh
node tests/fixtures/f1b3_validate.cjs /existing/validator/node_modules \
  /absolute/confined/root source.gltf t/0.glb
python3 -S -B tests/f1b3_khronos.py --binary /absolute/path/to/rusty-tiles \
  --validator-modules /existing/validator/node_modules \
  --source-commit EXECUTED_PRODUCER_COMMIT --json-output /tmp/f1b3-khronos.json
```

Final producer `92c7e02433efc2031df835290cd05d72a392ebdc`, portable binary SHA256 `e8ac65740cf396bd8d1d79327a54a7ed98817466a202b067d37c23dc024f1960`, passed the lit comparison with maximum RGB difference zero. The ten sensitive maximum differences are UV 84, slot 94, W 43, MR 56, normal 28, occlusion 52, emissive 143, color 73, scale 28 and strength 52. Chrome is `153.0.8010.12`. The final Khronos `2.0.0-dev.3.10` replay validated all 26 static sources and all 104 emitted leaves at ceiling 3, resolving external resources, with zero errors/warnings. [Receipts/provenance](../bench/architecture_audit/f1b3/) pin the executed binary and all independent artifacts. Later candidates need replay. This suite certifies the bounded full-detail PBR/attribute profile only. It does not close #121, establish generated MikkTSpace or nonconformal tangent support, prove LOD/error approximation or authorize release.
