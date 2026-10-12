# C1 payload integrity: large-mesh tiler amendment (ML0, A-C1-1 to A-C1-5)

Status: **DRAFT for the full (C2-style) treatment.** Written by the ML0 author. It is not frozen.
Before any code it needs: the C1 owner's agreement, an independent oracle author (O) who reads
only the pinned primary texts and this note, hashed mutant controls, and a fresh nonauthor
precode review. Nothing here records acceptance.

This note amends [c1-payload-integrity-contract.md](c1-payload-integrity-contract.md) and
[VALIDATION.md](../VALIDATION.md) for what the large-mesh tiler (`f1d3-large-mesh-contract.md`)
writes. The C1 contract file stays byte-identical (sha256 `3cb5ad34…3e1e3`), because the C1
production-control driver pins its hash, exactly as the
[WB-A amendment](c1-json-admission-wb-a-amendment.md) does. Where the two texts disagree on an
item below, this note governs that item; the C1 text governs everything else.

## Identities

| Item | Identity |
| --- | --- |
| Base | `origin/develop` `1b1ff9d99fe5e66b844d2c0453abdbe6ed20dd26` (C2-WB merged in PR #164; bulk evidence moved out in PR #165) |
| Design | large-mesh design **v6.1** `design.md` `0bf9fd19…441b`, `slices.md` `705bb8f7…f225`, `open-decisions.md` `bbe4f928…ccd1` |
| Reviews | v5 delta `698b3b61…c4ef` (ML0 may start); v6 delta `3f2c0583…ae6d` |
| C1 contract | `c1-payload-integrity-contract.md` `3cb5ad34…3e1e3`; `VALIDATION.md` `9d7b1b36…577e` |
| EXT_meshopt_compression | pinned `5ccac1cb…` text, retained at `bench/architecture_audit/c1_payload_integrity/primary_controls/2026-10-10-pinned-reference-verification/EXT_meshopt_compression.md` (`602608cb…55b0`) |
| KHR_texture_basisu | KhronosGroup/glTF commit `1bde276f7a173876e24c0f403a14dd827dc7912b` (2023-08-28): `README.md` sha256 `6f6d0767b95a8ac4f197c9394a28e8d4c71a2ff3ced2c4fd9357ed1c869b83a6` (12,510 B; last changed in `70b5e410…`, the same revision as C1's `KHR_mesh_quantization` pin), `schema/texture.KHR_texture_basisu.schema.json` sha256 `cc1d31267bb69991807ff5e73a9a3e6029ad3b3b4a76dd2d48f44061abe1c013` (604 B) |
| KTX 2.0 | KhronosGroup/KTX-Specification tag **v2.0.4**, commit `30cb4530a1120ddaf883fd98787edbee0e53270b` (2025-02-20): `ktxspec.adoc` sha256 `900cfd73fa8db623807f1c43dfe7631a89e02d48b153eef26eb9008fcc361ebb` (106,109 B) |
| PNG | W3C **PNG Third Edition**, Recommendation dated 2025-06-24, `https://www.w3.org/TR/2025/REC-png-3-20250624/`, sha256 `a3ac82bb9eb8664b93961a2a9ff6def3d70e3a9f63d92721143c1f391d04edec` (656,574 B) |
| Pin record | Raw bytes and `MANIFEST.md` (sha256 `ed9f5268…d23b`) in `~/.cache/rusty-tiles-evidence/ml0-spec-pins-20261012/`, fetched by the ML0 author on 2026-10-12. The independent primary-controls owner re-fetches and verifies them as part of the full treatment |

## Items and implementing slices

| Item | What | Slice | Changes verdicts? |
| --- | --- | --- | --- |
| A-C1-1 | Admit `KHR_texture_basisu`; inspect KTX2 containers to a stated depth | ML1 item 4 | Yes (Unsupported → success or InvalidInput) |
| A-C1-2 | PNG framing; declared image type agrees with the bytes | ML1 item 4 (optional, O-C1-3) | Yes (success → InvalidInput) |
| A-C1-3 | `EXT_meshopt_compression` stream header bytes: **a recorded C1 defect** | Separate small fix PR, scheduled by the coordinator (not this PR, not ML1) | Yes (success → InvalidInput) |
| A-C1-4 | Volume caps opt-in; non-circular amplification budgets; item caps kept | ML7b (after ML7a) | Only for archives that are ResourceLimit today |
| A-C1-5 | Report message caps | ML7a | No; report and message text only |
| A-C1-6 | **glTF-only tile content** (USER decision 2026-10-12): b3dm joins i3dm, pnts and cmpt as Unsupported legacy content | Separate glTF-only slice, after the vector-fill spike | Yes (b3dm success or InvalidInput → Unsupported); breaking, CHANGELOG-noted |

**Scope.** A-C1-1 to A-C1-5 rely only on the GLB and archive-local glTF paths. None of them uses
the b3dm framing, feature or batch table, `BATCH_LENGTH` or `RTC_CENTER` paths, and no ML slice
depends on b3dm.

## Unchanged

The JSON admission owner and WB-A; the outcome categories and their exit codes; the
ResourceLimit names of item caps; the compound-fault order of existing checks; the thirteen
check names; `image decoding` in `notInspected` for every image; image data URIs stay
Unsupported; meshopt filters other than NONE stay Unsupported; sparse accessors stay
Unsupported; the C2 glTF allowlist (it already lists `KHR_texture_basisu`).

---

## A-C1-1. `KHR_texture_basisu` and KTX2 containers

**Why.** Every textured output of the `fast`, `balanced` and `quality` profiles declares
`KHR_texture_basisu` in `extensionsRequired` (no fallback image). Today `payload.rs:464-476`
refuses any required extension outside four names, so every such archive is Unsupported.

### Definitions

- A **Basis object** is `textures[i].extensions.KHR_texture_basisu`.
- A **KTX2 image** is an image index named as `source` by at least one Basis object.
- The **usage** of a KTX2 image comes only from the five core material roles that C1 already
  walks. It is *colour* if some texture whose Basis object names the image is referenced by a
  `baseColorTexture` or `emissiveTexture`. It is *non-colour* if such a texture is referenced by
  `normalTexture`, `occlusionTexture` or `metallicRoughnessTexture`. It is *conflicting* if both,
  and *unassigned* if neither. References from other extensions assign nothing.

### Rules

All failures are **InvalidInput** unless marked. Numeric fields are little-endian. Sources are
the pinned texts above: "KHR" is the KHR_texture_basisu README, "schema" its texture schema,
"KTX §x" a section of `ktxspec.adoc` v2.0.4.

| Rule | Requirement | Source |
| --- | --- | --- |
| B-1 | `KHR_texture_basisu` is admitted in `extensionsRequired`. It is no longer listed in `notInspected` when used | this amendment |
| B-2 | If a Basis object has `source`, it is a JSON integer (C1's exact integer rule) indexing `images`. `source` is optional in the schema; a Basis object without it names no KTX2 image | schema (`source` is not in `required`) |
| I-1 | A KTX2 image with a `data:` URI is **Unsupported** (existing rule, unchanged) | C1 |
| I-2 | If a KTX2 image has `mimeType`, it is exactly `image/ktx2` | KHR README |
| I-3 | A KTX2 image stored in a bufferView has `mimeType` | glTF 2.0.1 `image.mimeType` |
| C-1 | Bytes 0..12 equal `AB 4B 54 58 20 32 30 BB 0D 0A 1A 0A` | KTX 2.0 |
| C-2 | Length ≥ 80 + 24·n, with n = max(1, `levelCount`) | KTX 2.0 |
| C-3 | `vkFormat` = 0 and `typeSize` = 1 | KHR (ETC1S/BasisLZ or UASTC only); KTX §typeSize, §Use of `VK_FORMAT_UNDEFINED`, §BasisLZ |
| C-4 | 2D: `pixelWidth` > 0, `pixelHeight` > 0, `pixelDepth` = 0, `layerCount` = 0, `faceCount` = 1 | KHR "2D type"; KTX §Texture Type table (row 2D) |
| C-5 | `pixelWidth` and `pixelHeight` are multiples of 4 | KHR |
| C-6 | `levelCount` ≤ 1 + ⌊log2(max(pixelWidth, pixelHeight))⌋. `levelCount` = 0 is **Unsupported** (KTX allows it except for block-compressed formats; whether that covers UASTC under `VK_FORMAT_UNDEFINED` is not settled by the text, so C1 does not decide it) | KTX §levelCount |
| C-7 | `supercompressionScheme` is 1 for ETC1S and 0 or 2 for UASTC (checked with D-3) | KHR |
| C-8 | Every range lies inside the bytes, with checked u64 arithmetic: the header plus level index `[0, 80+24n)`; the DFD (nonempty); the KVD and SGD when their length is nonzero; every level (`byteLength` > 0). The ranges are pairwise disjoint | KTX 2.0 |
| C-9 | For schemes 0 and 2 (no global data), `sgdByteLength` = 0; whenever `sgdByteLength` = 0, `sgdByteOffset` = 0 | KTX §sgdByteOffset, §sgdByteLength |
| C-10 | `uncompressedByteLength` equals `byteLength` for scheme 0 and is 0 for scheme 1 | KTX §levels[p].uncompressedByteLength, §BasisLZ |
| D-1 | The DFD's first u32 (`dfdTotalSize`) equals `dfdByteLength` | KTX 2.0 |
| D-2 | The first descriptor block is the Khronos basic block: vendorId 0, descriptorType 0, versionNumber 2, `descriptorBlockSize` = 24 + 16k with k ≥ 1. Any later blocks frame exactly to `dfdTotalSize` (each block size ≥ 8); their contents are opaque | Khronos Data Format 1.3 |
| D-3 | `colorModel` is 163 (ETC1S, with scheme 1) or 166 (UASTC, with scheme 0 or 2) | KHR |
| D-4 | The multiset of sample channel ids (low 4 bits of `channelType`) is one of: ETC1S {0}, {3}, {0, 15}, {3, 4}; UASTC {0}, {3}, {4}, {6} | KHR, `khr_df.h` |
| D-5 | A Red or Red-Green layout (ETC1S {3} or {3, 4}; UASTC {4} or {6}) has `transferFunction` ≠ 2 (sRGB) | KHR |
| K-1 | The KVD is a sequence of n entries, each a u32 `keyAndValueByteLength` ≥ 2, that many bytes holding a NUL-terminated key, then 0–3 zero padding bytes to a 4-byte boundary (also after the last entry), with `Σ ⌈len_i/4⌉·4 + 4n = kvdByteLength` | KTX §Key/Value Data, §keyAndValueByteLength, §valuePadding |
| K-3 | Keys are sorted by Unicode code point and no key appears twice | KTX §Key/Value Data |
| K-2 | Key `KTXswizzle`, if present, has value bytes `72 67 62 61 00` ("rgba"); key `KTXorientation`, if present, has value bytes `72 64 00` ("rd") | KHR |
| U-1 | Usage is not conflicting | KHR (colour and non-colour rules cannot both hold) |
| U-2 | Colour usage: `colorPrimaries` = 1 (BT709) and `transferFunction` = 2 (sRGB) | KHR |
| U-3 | Non-colour usage: `colorPrimaries` = 0 (UNSPECIFIED) and `transferFunction` = 1 (LINEAR) | KHR |
| U-4 | Colour or non-colour usage: flags bit 0 (`ALPHA_PREMULTIPLIED`) is clear. No core material requires premultiplied alpha | KHR |

**Not checked (stated in VALIDATION.md):** BasisLZ and Zstandard payload decoding or transcoding
(`image decoding` stays in `notInspected`); mip-pyramid completeness (a SHOULD); whether a
texture's layout matches its *type by usage* (for example a standalone occlusion map encoded RGB,
O-C1-2); BasisLZ global data and Zstandard frame contents; DFD sample bit offsets, bounds, `texelBlockDimension` and
`bytesPlane`; the fallback core image of an optional Basis object; KTX2 images that no Basis
object names.

### Order and resources

- B-2 runs in the existing textures step, for each texture after its core `source` check.
- All other rules run in a **new KTX2 step after the existing material texture-info step** and
  before the structural-metadata `schemaUri` step. Images are taken in ascending index; per image
  the rules run in table order (I, C, D, K, then U). The first failure wins.
- Existing steps keep their order. A resolver failure on any image, which happens in the
  existing image step, therefore precedes every KTX2 rule.
- **No new limit and no second read.** KTX2 bytes are the image bytes C1 already resolves and
  charges (URI images) or already charges as views (bufferView images). Facts needed by the KTX2
  step are extracted when the bytes are first held; the outcome order above is still the
  reported one. Parsing is O(image bytes); the level index holds at most 32 entries (C-6 with
  32-bit sizes); DFD samples and KVD entries are bounded by their byte ranges; nothing is
  allocated from a declared count.

### Exit-status and report changes (complete)

| # | Change | Archives |
| --- | --- | --- |
| X1-1 | Unsupported ("required extension KHR_texture_basisu", exit 2) → success, or InvalidInput (exit 3) on a rule above | Archives requiring the extension: large-mesh `fast`/`balanced`/`quality`; legacy `mesh-to-3tz --textureFormat uastc` (`lossless.rs:52-66` makes it required). The legacy verdict is not predicted; O records it (AC-1.9) |
| X1-2 | success with `KHR_texture_basisu` in `notInspected` → success without it, or InvalidInput | Archives using the extension optionally |
| X1-3 | Report | `KHR_texture_basisu` leaves `notInspected` wherever it is used; `image decoding` stays |

### Acceptance criteria (O builds every fixture from the pinned texts, not from rusty-tiles or ML1 output)

C1 decodes no Basis data, so container fixtures may carry synthetic level bytes; AC-1.1 adds real
encodes for realism only.

- **AC-1.1 Positive matrix.** For each admitted (model, scheme, layout): ETC1S/1 with {0}, {3},
  {0,15}, {3,4}; UASTC/0 and UASTC/2 with {0}, {3}, {4}, {6}. A 4×4 one-level KTX2 bound
  three ways: colour usage (for {0}, {3} of UASTC and {0}, {0,15} of ETC1S; BT709 and sRGB),
  non-colour usage (every layout; UNSPECIFIED and LINEAR), and unassigned. Each is carried
  twice: in a GLB with a bufferView image (`image/ktx2`), and in archive-local glTF with a URI
  image (with and without `mimeType`). Also a full 64×64 pyramid (levelCount 7) and levelCount 0.
  Expected: success; `KHR_texture_basisu` absent from `notInspected`; `image decoding` present.
  Plus two real encodes from an encoder that is **not** ML1's (KTX-Software `toktx` or another
  pinned tool), one ETC1S and one UASTC+Zstd, hashes recorded: success.
- **AC-1.2 One-defect negatives.** One fixture per rule B-2 to U-4, each differing from an
  AC-1.1 positive in exactly the named field, each expected InvalidInput and each paired with
  its positive. Required instances include: identifier last byte flipped; length 79; `vkFormat`
  = 1; `typeSize` = 4; `pixelDepth` = 1; `layerCount` = 1; `faceCount` = 6; `pixelHeight` = 0;
  width 6; levelCount one over C-6; scheme 3; ETC1S with scheme 0; UASTC with scheme 1;
  `dfdTotalSize` off by 4; vendorId 1; versionNumber 1; block size not 24+16k; `colorModel` 160;
  ETC1S layout {15}; UASTC layout {5}; ETC1S {3} with sRGB; a level ending one byte past the end;
  a level overlapping the DFD; nonzero SGD on UASTC; a KVD entry overrunning
  `kvdByteLength` by one; `KTXswizzle` "bgra"; `KTXorientation` "ru"; colour usage with LINEAR;
  colour usage with UNSPECIFIED primaries; non-colour usage with sRGB; non-colour usage with
  BT709; premultiplied flag on a used texture; conflicting usage; `mimeType` `image/png` on a
  KTX2 image; a bufferView KTX2 image without `mimeType`; `source` out of range;
  `source` = 1.5; KVD keys out of code-point order; a duplicated key; `levelCount` = 0
  (Unsupported, not InvalidInput). A Basis object **without** `source` is a positive (it names
  no KTX2 image).
- **AC-1.3 Boundaries.** At-limit positive and one-over negative for: levelCount at the C-6
  maximum; width 4 against 2; a KVD that fills `kvdByteLength` exactly; a level ending exactly at
  the end of the bytes.
- **AC-1.4 Unsupported kept.** A KTX2 image by data URI is Unsupported; a payload requiring
  `KHR_texture_basisu` and an unknown extension is Unsupported for the unknown name.
- **AC-1.5 Precedence (three double-defect controls).** (a) image 0 fails C-1 and image 1 fails
  D-1: image 0's message. (b) one image fails C-5 and U-2: C-5. (c) image 0 fails C-1 and image 1's
  URI resolution fails with a typed Io error: Io.
- **AC-1.6 No second charge.** For each AC-1.1 archive, `totalBytesRead` and every serialized
  counter equal those of the same archive with every Basis object removed (and the extension
  removed from the extension lists).
- **AC-1.7 Report.** The full report of each positive equals O's expected report, including the
  lexically ordered `notInspected`.
- **AC-1.8 C2 composition.** An otherwise in-domain explicit archive with KTX2 textures reports
  `decodedContentContainment`.
- **AC-1.9 Legacy witness.** The verdict for a hashed legacy `mesh-to-3tz --textureFormat uastc`
  archive is recorded (no prediction).
- **Mutants (production patches, each must flip its paired control):** usage conflict not
  checked; UASTC channel ids 5 and 6 swapped; ETC1S admitted with scheme 0; `KTXswizzle` not
  checked; C-5 skipped; U-3 checks primaries only; C-6 off by one; C-8 disjointness skipped;
  KTX2 bytes re-read through the resolver (caught by AC-1.6).

---

## A-C1-2. PNG framing and declared image type (optional; O-C1-3)

**Why.** The `lossless` profile writes PNG through core glTF, which C1 already admits without
looking inside images. This item lets `validate` catch truncated or corrupt PNG pages. It is not
needed for admission. Texel exactness of `lossless` is ML6's producer acceptance (independent
decode), not a C1 claim.

**Trigger.** Images C1 already resolves (URI or bufferView; data URIs stay Unsupported):
- **T-1, declared type.** `mimeType` `image/png` requires the PNG signature; `image/jpeg`
  requires leading bytes `FF D8 FF`; `image/ktx2` requires the KTX2 identifier. Other values
  (for example `image/webp`) are not checked. Mismatch is InvalidInput (glTF 2.0.1: image data
  must match `mimeType` when it is defined).
- **T-2, PNG framing** applies to every image whose bytes begin with
  `89 50 4E 47 0D 0A 1A 0A`. It is keyed on the signature, not on the core texture role, so
  legacy `--textureFormat webp` archives, which put WebP in core `texture.source`
  (`glb_write.rs:388-404`), are unaffected.

**PNG rules** (InvalidInput unless marked; sources are PNG Third Edition §5.2 signature
`#5PNG-file-signature`, §5.3 chunk layout `#5Chunk-layout`, §5.4 naming `#5Chunk-naming-conventions`,
§5.5 CRC `#5CRC-algorithm`, §5.6 ordering `#5ChunkOrdering`, §11.2 critical chunks `#11IHDR`,
`#11PLTE`, `#11IDAT`, `#11IEND` and Table 11.1 `#table111`; P-5's "no bytes after IEND" is this
profile's reading that an image resource is exactly one datastream): P-1 chunks frame exactly: u32 big-endian length
≤ 2^31−1, a 4-byte type of ASCII letters, data, CRC-32 over type and data, and the CRC matches.
P-2 the first chunk is `IHDR` with length 13: width and height in [1, 2^31−1]; (colour type, bit
depth) in {(0: 1,2,4,8,16), (2: 8,16), (3: 1,2,4,8), (4: 8,16), (6: 8,16)}; compression 0; filter
0; interlace 0 or 1. P-3 colour type 3 has exactly one `PLTE` before the first `IDAT`; colour types
0 and 4 have none. P-4 at least one `IDAT`, and all `IDAT` chunks are consecutive. P-5 the last
chunk is `IEND` with length 0, and no bytes follow it. P-6 an unknown **critical** chunk (first
letter upper case, not `IHDR`, `PLTE`, `IDAT` or `IEND`) is **Unsupported**. Ancillary chunks are
opaque apart from P-1. `IDAT` is not inflated; `image decoding` stays in `notInspected`.

**Order.** In the existing image step, immediately after each image's bytes are held: T-1, then
P-1 to P-6 in chunk order. **Resources.** O(image bytes); no allocation.

**Exit-status changes.** success → InvalidInput for archives whose image bytes contradict a
declared `image/png`, `image/jpeg` or `image/ktx2`, or whose PNG framing is corrupt; success →
Unsupported for unknown critical PNG chunks. Producers: the legacy writer derives `mimeType` from
the bytes (`glb_write.rs:375-385`) but falls back to `image/jpeg` for unrecognised bytes, so a
legacy archive with such an image changes. O records the corpus effect.

**Acceptance.** AC-2.1: the 15 valid (colour type, bit depth) pairs at 1×1 with an independent
zlib stream, interlace 1, ancillary chunks and a type-3 `PLTE`: success. AC-2.2: one-defect
negatives for each of P-1 to P-6 and T-1, including a CRC flip in `IDAT`, `IHDR` not first, `IHDR`
length 12, width 0, (3, 16), compression 1, interlace 2, no `IDAT`, split `IDAT`, missing `PLTE`
for type 3, `PLTE` for type 0, missing `IEND`, `IEND` with data, one trailing byte, a length past
the end, an unknown critical chunk (Unsupported), a type byte `0x31`. AC-2.3: WebP bytes in core
`texture.source` without `mimeType`: unchanged verdict. Mutants: CRC not checked; trailing bytes
admitted; PLTE rule skipped.

---

## A-C1-3. `EXT_meshopt_compression` stream headers (recorded defect)

**Status (coordinator ruling).** This is a defect in today's C1, recorded here and fixed by a
separate small PR that the coordinator schedules. This PR does not change code. The rule and
acceptance below are the specification that fix implements.

**Why.** The pinned EXT text requires the first byte of an ATTRIBUTES stream to be `0xa0`
(`EXT_meshopt_compression.md:198`), of a TRIANGLES stream `0xe1` (`:281`) and of an INDICES
stream `0xd1` (`:402`). The linked meshoptimizer 0.25 decoder also accepts vertex codec v1
(`0xa1`), index codec v0 (`0xe0`) and sequence v0 (`0xd0`) (`vertexcodec.cpp:1820-1824`,
`indexcodec.cpp:372-376` in meshopt 0.6.2), so C1 admits streams the extension forbids. ML1's
encoder must pin vertex codec v0 explicitly; meshoptimizer's encoder version is process-global
state.

**Rule H-1.** For every view with an `EXT_meshopt_compression` object, after the existing mode,
stride and count checks and before the decoder runs, the first compressed byte is `0xa0`
(ATTRIBUTES), `0xe1` (TRIANGLES) or `0xd1` (INDICES). Otherwise InvalidInput with its own message
("meshopt stream header"), distinct from "invalid meshopt compressed stream".

**Exit-status changes.** success → InvalidInput for archives holding a *decodable* v1 vertex
stream or v0 index or sequence stream under EXT. Producers: none predicted; meshopt 0.6.2
defaults are vertex v0 and index/sequence v1, and no `src/` call changes them.

**Acceptance.** AC-3.1: the pinned upstream golden stream still passes. AC-3.2: valid streams
from a pinned independent meshoptimizer encoder at vertex v1, index v0 and sequence v0, each
decodable by the linked decoder: InvalidInput with the H-1 message (a stream that fails decoding
anyway does not test H-1). AC-3.3: their v0/v1 twins: success. Mutant: H-1 removed (flips AC-3.2).

**Producer constraint (not a C1 change; settled).** Filters other than NONE stay Unsupported.
Design v6.1 uses no meshopt filter, so every large-mesh profile writes the NONE filter only.

---

## A-C1-4. Volume caps opt-in, amplification budgets, item caps kept

**Why.** Rulings (a)–(d) of the v3 design: validator memory independent of archive size, linear
work, no refusal for size. Today's absolute defaults refuse large archives (for example `autzen`,
1.26 GB, refused in 17 ms for the 1 GiB source cap). **Precondition: this item is implemented
only with or after ML7a**, which spills every structure that scales with the archive. Without
ML7a, lifting the caps would make memory unbounded.

### Disposition of the fifteen C1 `ValidationLimits` fields

| Field (serialized) | Today | Disposition |
| --- | --- | --- |
| `jsonBytes`, `jsonDepth`, `documentItems` | 8 MiB, 64, 131,072 | Item cap, kept |
| `memberBytes`, `documentDecodedBytes`, `accessorElements` | 128 MiB, 128 MiB, 4,000,000 | Item cap, kept |
| `hierarchyDepth` | 128 | Item cap, kept |
| `archiveStoredBytes`, `sourceArchiveBytes` | 1 GiB each | **Opt-in volume cap**: no default |
| `archiveEntries`, `centralDirectoryBytes` | 524,288; 32 MiB | **Opt-in volume cap** (also unmapped in the private `ReadLimits`) |
| `totalPayloadElements` | 16,000,000 | **Opt-in volume cap** |
| `hierarchyVisits` | 262,144 | **Budget** V by default; an absolute cap is opt-in |
| `references` | 1,048,576 | **Budget** R by default; an absolute cap is opt-in |
| `totalBytesRead` | 2 GiB | **Budget** B by default; an absolute cap is opt-in |

The C2 fields are A-C2-1's. The new I1 item cap is A-I1-2's.

### Budgets (defaults)

Let `entries` be the number of entries in the selected central-directory catalog and `S` the
sum of their stored data lengths (the stored-3TZ profile has no compression). Both are known
before traversal.

```text
V = 2^18 + 4·(entries + Σ_{distinct tileset JSON members} value_nodes + 8·Σ_{distinct subtree members} Scan)
R = 2^20 + 4·(entries + Σ_{distinct tileset JSON members} value_nodes)
B = 2^31 + 4·S
```

- `value_nodes` is the C1 JSON admission count of the member's document. `Scan` is I1 §2a step
  2's sum of distinct `View` availability byte lengths of that subtree member.
- **Credit timing.** A member's term is added once, when that member is first admitted (after
  its JSON admission, or after I1 §2a step 2 for a subtree), before any visit or reference it
  causes is charged. A later re-entry of the same member adds nothing.
- **Charges are today's counters, unchanged:** `task_visits` plus I1's `P_t` reservations against
  V; `reference_visits` (including I1's `E` reservations) against R; `bytes_read` (member reads,
  index work and I1 availability scans) against B. They are compared at the same points as today,
  with the same messages and ResourceLimit category.
- **Why non-circular.** The credit depends only on bytes and JSON nodes of distinct members, and
  every visit is charged, including re-entry of a shared document. A DAG of depth d that reaches
  2^d visits through one shared document earns one credit, so it still exceeds V. B is fixed
  before traversal.
- **Base terms in controls.** The base terms are the defaults 2^18, 2^20 and 2^31. Private
  controls (`inspect_selected`) may set them explicitly, as they set other limits today; no public
  caller can lower them.
- **Monotone.** Each budget is at least today's default (its base term), and every charge is
  unchanged. So **no verdict changes for an archive that today's defaults do not refuse with
  ResourceLimit.**

### Opt-in absolute caps

- CLI `validate --max-volume KEY=N`, repeatable, with keys `archive-bytes`, `source-bytes`,
  `entries`, `central-directory-bytes`, `payload-elements`, `visits`, `references`, `reads` (and
  A-C2-1's `containment-units`, `containment-bytes`, `containment-decoded`). An unknown key or a
  non-integer N is a usage error (exit 2 under CLI argument rules).
- A given cap applies with today's meaning and message. For `visits`, `references` and `reads`
  it applies **in addition to** the budget.
- Python `validate(path, max_volume={...})` takes the same keys (coordinator ruling O-U4, for
  CLI/Python parity); Rust takes them on the validation request. Surface in
  `f1d3-surface-dispositions.md`.

### Serialization (decided by the coordinator, O-C1-4)

`limits` keeps all fifteen names. A volume field without a cap serializes as JSON **`null`**;
budget fields serialize the absolute cap if one was given, otherwise `null`. VALIDATION.md
documents this. It changes the JSON type of eight fields (number → null) and the Rust field types
(`u64` → `Option<u64>`), a public API break like C2's O-b, listed in the migration notes.

### Exit-status changes

| # | Change | Archives |
| --- | --- | --- |
| X4-1 | ResourceLimit (volume cap) → any other verdict | Archives over 1 GiB, 524,288 entries, 32 MiB central directory or 16M payload elements, with no `--max-volume` |
| X4-2 | ResourceLimit (visits, references, reads at today's value) → any other verdict | Archives within the budget but over today's cap |
| X4-3 | Report | `limits` serialization: `null` for uncapped fields |

### Acceptance criteria

- **AC-4.1 Parity.** Every C1, I1 and C2 corpus archive, control and mutant that is not
  ResourceLimit under today's defaults gives an identical verdict, category, message and report,
  except the `limits` object.
- **AC-4.2 At-cap pairs for each budget**, computed by O from the archive bytes alone: an archive
  whose charges equal V (R, B) succeeds; the same archive with one more charge (one more visit,
  reference or read byte, with no added credit) is ResourceLimit with today's message.
- **AC-4.3 DAG control.** An external tileset referenced by 2 tiles at each of d levels
  (2^d re-entries of one document) is ResourceLimit at the first d where O's V is exceeded, and
  succeeds at d − 1.
- **AC-4.4 Re-entry earns nothing.** Two archives that differ only in whether a shared document
  is referenced once or twice: the second's V equals the first's, and its charge is larger by the
  re-entry's visits.
- **AC-4.5 Valid deep tree stays linear.** A chain of 120 external documents (depth ≤ 128)
  succeeds, and its charges are linear in its bytes (O's formula).
- **AC-4.6 Opt-in caps.** For each key: equality succeeds, one over is ResourceLimit with
  today's message, and omission removes the refusal. `autzen` (1.26 GB) is no longer refused for
  size (ML7b acceptance).
- **AC-4.7 Credit timing.** A subtree whose `P_t` is covered only by its own `Scan` credit
  succeeds; the same subtree with that credit withheld (a mutant) is ResourceLimit.
- **Mutants:** credit added per re-entry; credit added after the charge; `Scan` counted per alias
  instead of per distinct source; volume cap still applied by default; base term dropped.

---

## A-C1-5. Report message caps

**Why.** ML7a spills the structures behind two report strings that today grow with
attacker-chosen names.

- **N-1 `notInspected`.** Entries that are glTF `extensionsUsed` names outside the supported set
  (today's `payload.rs:515-519`) are capped at the 64 lexically smallest distinct names. If more
  exist, one entry `…and N more extension names` (U+2026, N the omitted count) is appended
  **after** the lexically sorted list. Every fixed-vocabulary entry (check exclusions,
  `decodedContentBounds.*`, `image decoding`, and so on) is always listed.
- **N-2 Unreferenced entries.** The InvalidInput message lists the first 16 unreferenced names in
  sorted order. With more than 16 it ends `, … (N total)`. Up to 16 names, the message is
  byte-identical to today's.

**Changes.** No verdict or category changes. Reports with more than 64 unknown extension names,
and messages with more than 16 unreferenced entries, change text.

**Acceptance.** AC-5.1: 64 and 65 unknown names (the 65th is the marker), with fixed entries
present in both. AC-5.2: 16 and 17 unreferenced entries. AC-5.3: whole-report parity on every
corpus archive with fewer than 64 names and 17 entries. Mutants: fixed entries dropped by the cap;
the marker sorted into the list.

---

## A-C1-6. glTF-only tile content (USER decision, 2026-10-12)

**Decision.** glTF 2.0 is the only supported tile content: GLB, plus archive-local `.gltf`. The
3D Tiles 1.0 tile formats b3dm, i3dm, pnts and cmpt are dropped as legacy. A separate glTF-only
slice implements it, after a spike settles how vector fills ship as plain glTF in Cesium. This
PR changes no code.

**Rule.** The payload kind is decided from the member name, as today (`validate.rs:504-513`):
`.glb` is GLB and `.gltf` is archive-local glTF. Every other content suffix, now including
`.b3dm`, is **Unsupported**, before any of its bytes are parsed, with one fixed message naming
the suffix as legacy 3D Tiles 1.0 content (i3dm, pnts and cmpt are already Unsupported today, as
"payload format outside GLB/local glTF 2.0"). C1's b3dm paths leave the profile: the 28-byte
framing, feature-table and batch-table JSON admission, `BATCH_LENGTH`, the `RTC_CENTER` key
detection and the `b3dm feature and batch table semantics` exclusion.

**Exit-status and report changes (complete).**

| # | Change | Archives |
| --- | --- | --- |
| X6-1 | success or InvalidInput (exit 0 or 3) → **Unsupported** (exit 2) | Every archive with b3dm content, including today's vector output with fills (`vector/pipeline/encoding.rs:939` writes fills as b3dm) and third-party 1.0 archives |
| X6-2 | Report | `b3dm feature and batch table semantics` never appears in `notInspected`; C2's `rtcCenter` exclusion becomes unreachable (A-C2 scope note) |
| X6-3 | CHANGELOG | "Breaking: `validate` reports b3dm, i3dm, pnts and cmpt content as Unsupported; only glTF 2.0 tile content is supported." |

**Acceptance (O, from this text).** AC-6.1: a minimal valid modern b3dm wrapping a valid GLB is
Unsupported with the legacy message; so is a malformed b3dm (the bytes are not parsed, so
malformation cannot win). AC-6.2: `.i3dm`, `.pnts` and `.cmpt` members are Unsupported. AC-6.3:
an upper-case `.B3DM` suffix is Unsupported (the name is lower-cased, as today). AC-6.4: every
GLB and `.gltf` corpus archive keeps its verdict and report. AC-6.5: the C1 fixtures and context
controls that hold b3dm are re-pinned to Unsupported, listed with their hashes. Mutant: b3dm still
routed to the old framing path (flips AC-6.1).

## Open questions and settled items

- **Settled (coordinator, 2026-10-12):** primary texts pinned (identities table; the
  primary-controls owner re-verifies them); `null` serialization (A-C1-4); Python `max_volume`;
  no meshopt filters in any profile; A-C1-3 is a recorded defect with its own fix PR.
- **O-C1-2 (C1 owner).** Should C1 also require the layout to match the KHR texture *type*
  implied by usage (for example a standalone `occlusionTexture` must be Red)? Draft: no; the
  allowed-layout and transfer rules are enforced, type matching is listed as not checked.
- **O-C1-3 (C1 owner).** Adopt A-C1-2 (PNG framing and declared-type agreement) now, or defer it?
  It is not needed for admission; its benefit is catching corrupt `lossless` pages.
- **Producer obligation recorded for ML4 and ML6 (no C1 change).** Per payload, the sum of
  declared view bytes plus meshopt-decoded bytes must stay ≤ `documentDecodedBytes` (128 MiB).
  `B_tile` = 96 MiB bounds the stored member, not the decoded sum.
