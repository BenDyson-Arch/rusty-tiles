# F1b1 independent texture and resource acceptance

The independent oracle exercises the first bounded [#121 slice](../docs/architecture/f1b-contract.md): full-detail local GLB triangles with supported embedded base-color PNG/JPEG, UVs, materials and samplers, delivered through shared archive images. It imports no production loader, writer, validator, atlas, sampling kernel or coordinate helper. It reuses only the independently authored F1a Python matrix, GLB reader, bounds and 3TZ-index helpers.

Passing this suite does not close #121. External resources, richer PBR/attributes/extensions, approximation/LOD, implicit hierarchy, metadata/picking and legacy migration remain separate obligations.

## Fixture and oracle provenance

`f1b_oracle.py` writes GLBs directly with Python `struct`/JSON, retaining negative document fields. Its PNG author uses literal asymmetric 3×2 RGBA pixels and `struct`/`zlib`; the independent reader checks chunk CRCs, exact zlib closure, scanline lengths, all five PNG filters and decoded RGBA pixels. Alpha values are 0, 64, 128, 192 and 255; row reversal, alpha changes and corner UV swaps are distinguishable.

The frozen JPEG is authored from six literal RGB colors with Pillow 12.3.0, quality 100, subsampling 0. `tests/fixtures/f1b/manifest.json` records its encoded SHA256 and independently decoded RGBA pixels. With Pillow available, the oracle decodes JPEG with Pillow and checks the manifest. Without Pillow, it accepts only the exact frozen fixture SHA and checks the previously independently decoded pixel receipt; this is receipt replay, not a new JPEG decode. Arbitrary JPEG inspection requires Pillow. Losses already present in source JPEG are preserved; no new quality assertion is made.

Static fixtures and source image assets are under `tests/fixtures/f1b/`. The manifest records authoring settings and asset hashes. `bench/architecture_audit/f1b1/independent-provenance.json` records Python/Pillow/zlib/Node/browser versions, oracle/fixture/viewer hashes, pinned Cesium/Playwright versions and executed candidate hashes. The [glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html) supplies normalized UV, sampler and alpha semantics; [3D Tiles 1.1](https://docs.ogc.org/cs/22-025r4/22-025r4.html) supplies coordinate and hierarchy semantics. Geometry accuracy is checked first against literal reflected/nested analytical positions, inverse-transpose normals and associated UV corners.

## Replay

```sh
python3 -B tests/f1b_oracle.py --self-test
python3 -B tests/f1b_oracle.py --binary /absolute/path/to/rusty-tiles \
  --json-output /tmp/f1b-oracle.json
python3 -S -B tests/f1b_oracle.py --self-test
```

`--binary` always runs sensitive oracle self-tests before invoking the candidate. Missing tools or failed cases fail rather than silently skip. The frozen suite has 28 positive variants with leaf limits 1, 3 and 1000 (84 conversions), and 45 ineligible-source controls at limits 1 and 1000 (90 refusals). It verifies Unsupported exit 2 and InvalidInput exit 3, the expected typed error kind and absence of output parent/work. It compares CLI `meshReport` to embedded `conversion.json` and independently calculated facts.

Every accepted source triangle appears exactly once in the complete oriented multiset, including duplicates, degenerates, thin triangles, tall spikes and disconnected components. Cyclic corner reindexing rotates positions, normals and UVs together; reversed winding does not match. UV storage follows the contract's `max(1, abs(value))*2^-24` component tolerance. Materials compare exact supported JSON values and field presence after canonicalizing numeric dependencies through image bytes/pixels/MIME and sampler semantics. Source normals and position storage use the independently established F1a tolerances.

Positive cases cover normalized u8/u16 UVs, byte-strided position/normal/UV interleaving, UVs outside [0,1], untextured UV preservation, absent normals, indexed/unindexed sources, nested/reflected/nonuniform transforms, instancing, default scene selection, explicit empty versus absent samplers, all six core minification filters, all wrap modes, two textures sharing an image, two published images and selection of image 1 while excluding image 0. The reordered spike is the same source geometry with reversed primitive order; self-tests independently compare both multisets.

The archive reader resolves only the declared `../textures/{source_id}.png|.jpg` paths, checks unchanged encoded source bytes, per-leaf exact material/texture/image/sampler closure, global selected image closure, conservative decoded-vertex and ancestor bounds, the routing error formula, exact inventory, ZIP CRCs and independently parsed 3TZ hashes/offsets. It recomputes unique published image byte/pixel counts; source admission ceilings include unused images while these report counts exclude them.

There are 21 sensitive mutation controls for missing triangles, winding, corner UV association, normals, factors, alpha cutoff, double-sidedness, sampler/default field omission, image alpha/rows, encoded byte changes with unchanged pixels, missing/orphan shared images, wrong texture indices/URIs and unused leaf images. Refusals cover all URI classes, other texture channels/attributes/extensions/extras, BLEND, invalid references/counts/layout/MIME, PNG CRC/zlib/IEND/trailing data/APNG, 16-bit RGB/RGBA including unused images, missing JPEG EOI, image count/edge/per-image/aggregate-pixel ceilings.

Reusable APIs are `fixture(triangles=8, variant='standard', transformed=True)`, `inspect(source, archive, leaf_limit)`, `rejection_fixtures()`, `viewer_fixture()`, `shared_uv_fixture(primitives=4096, vertices=300000)` and `resource_fixtures()`. The last two support separate measured resource receipts; the small analytical multiset matcher intentionally has quadratic complexity and is not a boundary benchmark. Shared-accessor fixtures exercise repeated primitive references without asserting a timing threshold. Process RSS, sampled descriptors and sampled scratch remain distinct measurements, and sampled maxima do not prove true peaks. Resource/failure/frontend receipts owned by the producer tests complement this oracle.

## Real viewer appearance

```sh
python3 -B tests/f1b_viewer.py \
  --binary /absolute/path/to/rusty-tiles \
  --cesium-dir /home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/cesium/Build/Cesium \
  --node-modules /home/bend/.cache/rusty-tiles-117-browser-cache/node_modules \
  --chromium /usr/bin/chromium --json-output /tmp/f1b-viewer.json
```

Existing pinned local assets are required; no downloads occur. Independently authored OPAQUE and MASK panels use nearest/clamp sampling. The viewer projects twelve analytical texel centers, reads actual WebGL framebuffer pixels after rendered frames and checks their RGB values against literal source colors (tolerance 12 of 255). MASK hides alpha 0/64 while OPAQUE ignores texture alpha. A horizontal UV flip and a MASK→OPAQUE mutation must each fail the same appearance comparison. Render commands and two selected full-detail leaves are required; page errors, request failures or external requests fail.

The viewer uses a display-only Earth-fixed frame, a fixed face-on camera, black background and Cesium `CustomShader` UNLIT lighting to isolate base-color sampling from environmental lighting. It preserves the source base-color texture/factor/sampler/alpha pipeline and introduces no source extension. This is representative appearance evidence for one pinned renderer and sample layout, not universal lighting, GPU precision, color-management, arbitrary filter appearance or a pixel-perfect whole-image claim. The separate F1a viewer continues to establish near/far traversal for the retained routing metric.

Frozen initial receipts are `bench/architecture_audit/f1b1/independent-oracle.json` and `independent-viewer.json`. Their binary SHA identifies what ran; they are not substitutes for replaying the final build.
