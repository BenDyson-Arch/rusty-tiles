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
| Base | `origin/develop` `e29cd19d2f236dc688e91a80a45170d4a573e20b` (C2-WB merged, PR #164) |
| Design | large-mesh design v5 `design.md` `70a1eeec…afd1`, `slices.md` `b0ddfd0c…aa7`, `open-decisions.md` `523ed2cd…eea` |
| Review | v5 precode delta review `review.md` `698b3b61…c4ef` (ML0 may start) |
| C1 contract | `c1-payload-integrity-contract.md` `3cb5ad34…3e1e3`; `VALIDATION.md` `9d7b1b36…577e` |
| EXT_meshopt_compression | pinned `5ccac1cb…` text, retained at `bench/architecture_audit/c1_payload_integrity/primary_controls/2026-10-10-pinned-reference-verification/EXT_meshopt_compression.md` (`602608cb…55b0`) |
| KHR_texture_basisu, KTX 2.0, PNG | **Not yet pinned.** The design's copy of the KHR README (`6f6d0767…83a6`) was fetched from `main`, which C1's provenance rule does not accept. O-C1-1 below |

## Items and implementing slices

| Item | What | Slice | Changes verdicts? |
| --- | --- | --- | --- |
| A-C1-1 | Admit `KHR_texture_basisu`; inspect KTX2 containers to a stated depth | ML1 item 4 | Yes (Unsupported → success or InvalidInput) |
| A-C1-2 | PNG framing; declared image type agrees with the bytes | ML1 item 4 (optional, O-C1-3) | Yes (success → InvalidInput) |
| A-C1-3 | `EXT_meshopt_compression` stream header bytes | ML1 item 5 | Yes (success → InvalidInput) |
| A-C1-4 | Volume caps opt-in; non-circular amplification budgets; item caps kept | ML7b (after ML7a) | Only for archives that are ResourceLimit today |
| A-C1-5 | Report message caps | ML7a | No; report and message text only |

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

All failures are **InvalidInput** unless marked. Numeric fields are little-endian. "[pin]" marks
a rule whose source line O must find in the pinned KTX 2.0 text before freeze; if O cannot,
the rule is dropped, not weakened.

| Rule | Requirement | Source |
| --- | --- | --- |
| B-1 | `KHR_texture_basisu` is admitted in `extensionsRequired`. It is no longer listed in `notInspected` when used | this amendment |
| B-2 | A Basis object has `source`, a JSON integer (C1's exact integer rule) indexing `images` | KHR schema [pin] |
| I-1 | A KTX2 image with a `data:` URI is **Unsupported** (existing rule, unchanged) | C1 |
| I-2 | If a KTX2 image has `mimeType`, it is exactly `image/ktx2` | KHR README |
| I-3 | A KTX2 image stored in a bufferView has `mimeType` | glTF 2.0.1 `image.mimeType` |
| C-1 | Bytes 0..12 equal `AB 4B 54 58 20 32 30 BB 0D 0A 1A 0A` | KTX 2.0 |
| C-2 | Length ≥ 80 + 24·n, with n = max(1, `levelCount`) | KTX 2.0 |
| C-3 | `vkFormat` = 0 and `typeSize` = 1 | KHR (Basis formats only); KTX 2.0 [pin] |
| C-4 | 2D: `pixelWidth` > 0, `pixelHeight` > 0, `pixelDepth` = 0, `layerCount` = 0, `faceCount` = 1 | KHR "2D type", KTX 2.0 §4.1 [pin] |
| C-5 | `pixelWidth` and `pixelHeight` are multiples of 4 | KHR |
| C-6 | `levelCount` ≤ 1 + ⌊log2(max(pixelWidth, pixelHeight))⌋ | KTX 2.0 [pin] |
| C-7 | `supercompressionScheme` is 1 for ETC1S and 0 or 2 for UASTC (checked with D-3) | KHR |
| C-8 | Every range lies inside the bytes, with checked u64 arithmetic: the header plus level index `[0, 80+24n)`; the DFD (nonempty); the KVD and SGD when their length is nonzero; every level (`byteLength` > 0). The ranges are pairwise disjoint | KTX 2.0 |
| C-9 | `sgdByteLength` > 0 for scheme 1, and `sgdByteOffset` = `sgdByteLength` = 0 otherwise | KTX 2.0 [pin] |
| C-10 | `uncompressedByteLength`: equals `byteLength` for scheme 0, is 0 for scheme 1, and is > 0 for scheme 2 | KTX 2.0 [pin] |
| D-1 | The DFD's first u32 (`dfdTotalSize`) equals `dfdByteLength` | KTX 2.0 |
| D-2 | The first descriptor block is the Khronos basic block: vendorId 0, descriptorType 0, versionNumber 2, `descriptorBlockSize` = 24 + 16k with k ≥ 1. Any later blocks frame exactly to `dfdTotalSize` (each block size ≥ 8); their contents are opaque | Khronos Data Format 1.3 |
| D-3 | `colorModel` is 163 (ETC1S, with scheme 1) or 166 (UASTC, with scheme 0 or 2) | KHR |
| D-4 | The multiset of sample channel ids (low 4 bits of `channelType`) is one of: ETC1S {0}, {3}, {0, 15}, {3, 4}; UASTC {0}, {3}, {4}, {6} | KHR, `khr_df.h` |
| D-5 | A Red or Red-Green layout (ETC1S {3} or {3, 4}; UASTC {4} or {6}) has `transferFunction` ≠ 2 (sRGB) | KHR |
| K-1 | The KVD is a sequence of entries, each a u32 `keyAndValueByteLength` ≥ 2, that many bytes holding a NUL-terminated key, then zero padding to a 4-byte boundary. The sequence consumes exactly `kvdByteLength` | KTX 2.0 [pin padding of the last entry] |
| K-2 | Key `KTXswizzle`, if present, has value bytes `72 67 62 61 00` ("rgba"); key `KTXorientation`, if present, has value bytes `72 64 00` ("rd") | KHR |
| U-1 | Usage is not conflicting | KHR (colour and non-colour rules cannot both hold) |
| U-2 | Colour usage: `colorPrimaries` = 1 (BT709) and `transferFunction` = 2 (sRGB) | KHR |
| U-3 | Non-colour usage: `colorPrimaries` = 0 (UNSPECIFIED) and `transferFunction` = 1 (LINEAR) | KHR |
| U-4 | Colour or non-colour usage: flags bit 0 (`ALPHA_PREMULTIPLIED`) is clear. No core material requires premultiplied alpha | KHR |

**Not checked (stated in VALIDATION.md):** BasisLZ and Zstandard payload decoding or transcoding
(`image decoding` stays in `notInspected`); mip-pyramid completeness (a SHOULD); whether a
texture's layout matches its *type by usage* (for example a standalone occlusion map encoded RGB,
O-C1-2); KVD key order and uniqueness; DFD sample bit offsets, bounds, `texelBlockDimension` and
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
  a level overlapping the DFD; nonzero SGD on UASTC; zero SGD on ETC1S; a KVD entry overrunning
  `kvdByteLength` by one; `KTXswizzle` "bgra"; `KTXorientation` "ru"; colour usage with LINEAR;
  colour usage with UNSPECIFIED primaries; non-colour usage with sRGB; non-colour usage with
  BT709; premultiplied flag on a used texture; conflicting usage; `mimeType` `image/png` on a
  KTX2 image; a bufferView KTX2 image without `mimeType`; `source` out of range; `source` absent;
  `source` = 1.5.
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

**PNG rules** (InvalidInput unless marked): P-1 chunks frame exactly: u32 big-endian length
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

## A-C1-3. `EXT_meshopt_compression` stream headers

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

**Producer constraint (not a C1 change).** Filters other than NONE stay Unsupported. If any
large-mesh profile uses OCTAHEDRAL, QUATERNION or EXPONENTIAL filters, a further C1 amendment is
needed first (O-C1-5).

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
- Python and Rust expose the same keys (surface in `f1d3-surface-dispositions.md`).

### Serialization (O-C1-4)

Proposed: `limits` keeps all fifteen names. A volume field without a cap serializes as JSON
`null`; budget fields serialize the absolute cap if given, otherwise `null`. This changes the JSON
type of eight fields (number → null) and the Rust field types (`u64` → `Option<u64>`), a public
API break like C2's O-b. The alternative (u64::MAX) loses precision in JavaScript readers.

### Exit-status changes

| # | Change | Archives |
| --- | --- | --- |
| X4-1 | ResourceLimit (volume cap) → any other verdict | Archives over 1 GiB, 524,288 entries, 32 MiB central directory or 16M payload elements, with no `--max-volume` |
| X4-2 | ResourceLimit (visits, references, reads at today's value) → any other verdict | Archives within the budget but over today's cap |
| X4-3 | Report | `limits` serialization (O-C1-4) |

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

## Open questions (C1 owner unless marked)

- **O-C1-1 (primary texts).** The independent primary-controls owner must pin and fetch
  `KHR_texture_basisu` (README and texture schema) at a Khronos glTF revision, the KTX 2.0
  specification at a revision, and the PNG specification edition, with line references for every
  [pin] rule. The design's `main` copy cannot serve.
- **O-C1-2 (type by usage).** Should C1 also require the layout to match the KHR texture *type*
  implied by usage (for example a standalone `occlusionTexture` must be Red)? Draft: no; the
  allowed-layout and transfer rules are enforced, type matching is listed as not checked.
- **O-C1-3 (scope).** Adopt A-C1-2 (PNG framing and declared-type agreement) now, or defer it?
  It is not needed for admission; its benefit is catching corrupt `lossless` pages.
- **O-C1-4 (coordinator + C1 owner).** `limits` serialization for uncapped fields: `null`
  (proposed) or another form. Either is a public report and Rust API change.
- **O-C1-5 (design author).** Does any profile need meshopt filters (normals, tangents, UVs)? If
  so, a filter-admission amendment is needed before ML4.
- **Producer obligation recorded for ML4 and ML6 (no C1 change).** Per payload, the sum of
  declared view bytes plus meshopt-decoded bytes must stay ≤ `documentDecodedBytes` (128 MiB).
  `B_tile` = 96 MiB bounds the stored member, not the decoded sum.
