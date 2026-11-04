# C1 amendment: KHR_texture_basisu and KTX2 container integrity

**Status: proposed with its implementation (2026-10-13); in effect when the pull request
merges after the coordinator's nonauthor audit.** It amends
[c1-payload-integrity-contract.md](c1-payload-integrity-contract.md), which stays
byte-identical because the C1 production-control driver pins it. Where the texts disagree
about textures or images, this note governs. H7's
[legacy-content amendment](c1-legacy-content-unsupported-amendment.md) is unchanged.

## Why

H8 writes KTX2/Basis Universal textures through `KHR_texture_basisu`, the default texture
format, with the extension in `extensionsRequired`. C1 refused every required extension
outside its supported list, so `validate` refused the project's own default output as
Unsupported ("required extension KHR_texture_basisu").

## Authorities

| Text | Pin | SHA-256 |
| --- | --- | --- |
| KHR_texture_basisu README | KhronosGroup/glTF `1bde276f7a173876e24c0f403a14dd827dc7912b` | `6f6d0767…83a6` |
| Extension schema | same commit | `cc1d3126…e013` |
| KTX File Format Specification 2.0 | KhronosGroup/KTX-Specification v2.0.4 `30cb4530a1120ddaf883fd98787edbee0e53270b` | `900cfd73…1ebb` |
| BasisLZ global data appendix (included by the above) | same commit, `appendices/basislz-gdata.adoc` | `162ceee5…d45c` |

The first three were pinned by ML0 (`~/.cache/rusty-tiles-evidence/ml0-spec-pins-20261012/`).
The appendix and the small texts are retained in the control lane's `primary/`, with
`pins.json`; the 106 KB KTX specification is pinned by hash only.

## Domain

`KHR_texture_basisu` joins C1's supported extensions, so it may appear in
`extensionsRequired` and is no longer listed in `notInspected` as an unknown extension.
C2 already counted it as texture-only (`GLTF_ALLOWLIST`), so containment is unaffected.

### Texture and image rules (InvalidInput)

T1. The extension object's `source`, when present, references an image in range.

T2. That image is KTX2 by declaration or content, as glTF decides formats: a `mimeType`,
when present, must be `image/ktx2`; a bufferView image must have one (glTF requires
`mimeType` with `bufferView`); a URI image without `mimeType` needs no file-name suffix, and
its bytes must then be a KTX2 container (K1 refuses anything else).

T3. When the document uses the extension, a core `texture.source` (the fallback) may not
reference a KTX2 image: one declared `image/ktx2`, or a URI image without `mimeType` whose
bytes start with the KTX2 identifier. The extension permits `image/ktx2` only for its own
`source`.

T6. A `data:` image URI stays Unsupported ("image data URI") and is decided first, before
any declaration rule, so a data URI is Unsupported whether or not it has a `mimeType`.

T4. A texture that carries the extension but no core `source` requires the extension in
`extensionsRequired` ("To use ... without a fallback, define KHR_texture_basisu in both
extensionsUsed and extensionsRequired"). With a fallback, `extensionsUsed` alone suffices.

T5. The existing declaration walk still makes an extension object outside `extensionsUsed`
InvalidInput.

Texture, sampler and material references are now admitted before image resources are read
(previously image reads came first), because the image rules depend on texture roles. This
changes only the order of failures when a document has both a bad texture/material
reference and a failing image read; no single-fault verdict changes. Documents that do not
use the extension keep their verdicts: T2 and T3 run only for images that an extension
`source` references, or that a fallback references when the extension is used.

### KTX2 container rules

Each KTX2 resource referenced by an extension `source` is inspected once: an external
image after its existing (charged) resolver read, and a bufferView once after the image
pass, however many images share it. All failures
are InvalidInput except K2's Unsupported case.

K1. The 12-byte identifier; an 80-byte header.

K2. Texture type: `pixelWidth`, `pixelHeight` > 0 (BasisLZ/ETC1S and UASTC are
block-compressed), `faceCount` 1 or 6, cubemap rules. A valid non-2D type (3D, cubemap,
array) is InvalidInput when a material texture-info role uses the texture (the extension
requires 2D for material maps) and Unsupported otherwise (outside this profile).

K3. `vkFormat` 0 (`VK_FORMAT_UNDEFINED`) and `typeSize` 1.

K4. `pixelWidth` and `pixelHeight` are multiples of 4. Powers of two are not required: the
extension only notes that exporters may choose them.

K5. `1 <= levelCount <= floor(log2(max(width, height))) + 1`. This is checked before the level
index is read, so a hostile `levelCount` costs nothing and at most 32 entries exist.

K6. The DFD starts right after the level index; `dfdTotalSize == dfdByteLength`, a multiple
of 4; its first block is the Khronos basic block, version 2, of `24 + 16·samples` bytes.

K7. Format pairing (extension "KTX header fields"): `colorModel` 163 (ETC1S) with
`supercompressionScheme` 1 (BasisLZ) and channels RGB, RGB+AAA, RRR or RRR+GGG; or
`colorModel` 166 (UASTC) with scheme 0 or 2 and exactly one channel UASTC_RGB, _RGBA,
_RRR or _RG. Any other model, scheme (ZLIB, vendor, reserved) or channel set is refused.
`texelBlockDimension` is 4x4. Red and Red-Green textures (ETC1S RRR or RRR+GGG,
UASTC_RRR, UASTC_RG) must not use the sRGB transfer function (2), whatever their usage. A
texture a material uses must not set `KHR_DF_FLAG_ALPHA_PREMULTIPLIED` (no core material
requires premultiplied alpha).

K8. Key/value data starts right after the DFD (offset 0 when absent); pairs are at least 2
bytes with a NUL-terminated key, 4-byte aligned, and sum exactly to `kvdByteLength`.
`KTXswizzle`, when present, is `rgba` and `KTXorientation` is `rd` (extension "Additional
requirements").

K9. Supercompression global data exists only for BasisLZ, which requires it; it starts at the
next 8-byte boundary; offset 0 when absent.

K10. Level index: levels are packed smallest first, each starting where the previous
section or level ends (rounded up to 16 bytes, `lcm(16, 4)`, for unsupercompressed UASTC),
nonempty, inside the image, and the base level ends at the end of the image. This refuses
offsets past EOF, gaps, overlaps and trailing bytes. `uncompressedByteLength` is the level's
UASTC block bytes (`16·ceil(w/4)·ceil(h/4)`), equal to `byteLength` for scheme 0, and 0 for
BasisLZ.

K11. BasisLZ global data: `sgdByteLength == 20 + 20·imageCount + endpoints + selectors +
tables + extended`, `extendedByteLength == 0` (ETC1S), imageCount = levelCount for 2D; each
ImageDesc has flags 0 (P-frames need animation arrays), a nonempty RGB slice inside its
level, and an alpha slice inside its level exactly when the DFD has two samples (otherwise
zero offset and length).

## Not inspected

The report adds `KTX2 level data and transcoding` and keeps `image decoding`. C1 does not
inflate Zstandard or BasisLZ data, transcode, or decode any texel. It does not check:

- the extension's colour-space-by-usage rule (BT709/sRGB for colour data,
  unspecified/linear for non-colour data). Only the usage-independent Red/Red-Green sRGB
  rule is checked; KTX-Software's own `rg_reference_basis` is BT709 linear, which the
  usage rule forbids for every usage, so enforcing it would refuse common files;
- the premultiplied flag of textures no material uses, and extension-defined material
  requirements;
- the SHOULD for full mip pyramids under mipmapping samplers;
- key/value rules other than framing, `KTXswizzle` and `KTXorientation`: key order and
  uniqueness, UTF-8 without BOM, reserved `KTX`/`ktx` prefixes, `KTXwriterScParams`
  requiring `KTXwriter`, `KTXanimData`/`KTXcubemapIncomplete` use, and every other
  metadata value;
- valuePadding, sgdPadding and mipPadding byte values;
- DFD blocks after the first, `bytesPlane` values and sample fields other than the
  channel id (bit offsets and lengths, positions, lower and upper; KDF 5.6.11/5.6.14);
- endpoint, selector and table data of BasisLZ global data;
- fallback (PNG/JPEG) image contents.

## Bounds and accounting

The container owner allocates nothing and reads only bytes already admitted: external
image bytes are charged against the member/resource allowance and `totalBytesRead` before
the read, as before; bufferView image bytes belong to an admitted view charged to
`documentDecodedBytes`. Its work is linear in the bytes of the resource it inspects: the
key/value scan reads each key/value byte once (looking for each key's NUL), plus at most 32
level entries and 32 ImageDescs.

Each resource is inspected once, so the work is charged through the bytes already counted.
External images are read, and charged, once per image. A bufferView is inspected once after
the image pass, with the material flag ORed over every image using it as a source, so N
images sharing one view cost one scan, not N. An audit found the first version scanned the
view once per image, uncharged: about 9 minutes of CPU for one small member at default
limits. The control `shared-view-heavy-kvd` (4,000 images over one view holding an 8 MiB
key/value pair) and a Rust control counting inspections would fail if per-image scanning
returned. No new limit or counter is introduced.

Hostile headers are refused by arithmetic on the 80-byte header and index: `levelCount`
before the index, every range by checked u64 arithmetic, and overlapping or out-of-order
levels by the packed-offset rule.

## Proof and its limits

Independent lane:
`bench/architecture_audit/c1_payload_integrity/production_controls/ktx2_basisu_amendment/2026-10-13/`.
A standard-library oracle written from the pinned texts recomputes every verdict; 67
archive controls (13 admitted, 54 refused) cover:

- upstream KTX-Software reference files: ETC1S RGB+A, ETC1S with mips, UASTC, UASTC+Zstd
  with mips, and an upstream file carrying `KTXswizzle ra01`;
- one real in-repo encoder output and packed containers;
- byte-patched single-fault negatives for each rule above, except the cases listed below.

Oracle mutants (37) and production mutants (30) each flip their named control.

The oracle is a parallel derivation from the same texts (the audit notes that it mirrors
production's structure), so the independent weight rests on the pinned upstream positives.
No third-party writer output other than KTX-Software is included.

Some rules have no single-fault control, because another rule always fires with them:

- the ETC1S/UASTC scheme pairing: BasisLZ requires global data, and other schemes forbid it;
- the DFD position: moving the DFD moves everything after it;
- a zero-length level: its descriptor or length rule also fails;
- the level-range check: the packed-offset and trailing-byte rules also catch a level
  past EOF.

The real-output gate validates an H8-encoded, H16-packaged crop of the real mesh with
`decodedContentContainment`.

This establishes container integrity for these finite rules, not texture correctness,
transcodability or rendering.
