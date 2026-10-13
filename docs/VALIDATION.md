# Bounded archive and payload inspection

C1 implements read-only inspection of self-contained 3TZ archives. Rust uses
`validate::inspect(ValidationRequest::new(path))` (or `inspect_with(request, &RunControl)`
for cancellation and progress); CLI uses `validate PATH [--maxVolume KEY=N]...
[--scratchDir DIR]`; Python uses `validate(path, *, lazy_payloads=False, max_volume=None,
scratch_dir=None, callback=None)`. Rust returns a typed `ValidationReport`, which
adapters serialize without adding domain decisions. There is no external-tool
option or subprocess fallback. The limits are described under "Limits" below.

Supported payloads are GLB 2.0 and archive-local glTF 2.0. Unavailable versions
or legacy interpretations do not receive an opaque successful pass.

Legacy 3D Tiles 1.0 content (b3dm, i3dm, pnts and cmpt) is Unsupported, never
InvalidInput, and is never partially inspected. A payload counts as legacy in
two cases:

- its archive member name ends in `.b3dm`, `.i3dm`, `.pnts` or `.cmpt` (ASCII,
  case-insensitive). This is decided before the payload is read;
- its first four bytes are one of those magics, for example b3dm bytes in a `.glb`
  or `.bin` member. The magic decides under any non-legacy suffix (a legacy suffix
  decides first, before any read); only those four bytes of the existing read
  are examined.

An implicit content template whose normalized last segment has one of those
suffixes is Unsupported at root admission, and no subtree is read. Nothing after
the four-byte magic is interpreted, so the version, the tables, `RTC_CENTER` and
cmpt inner tiles make no difference: a cmpt that wraps a GLB is Unsupported too.
A missing member is still InvalidInput. The message names the format and the
member, for example
`legacy b3dm content is unsupported (glTF 2.0 content only): tile.b3dm`.
Producers write only GLB; convert older archives again. See the
[C1 H7 amendment](architecture/c1-legacy-content-unsupported-amendment.md).

The glTF contract uses the published [glTF 2.0.1 registry specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html),
identified by revision `8e798b02d254cea97659a333cfcb20875b62bdd4`.
`asset.version` must be `2.0`. Optional `asset.minVersion` must have numeric
major.minor syntax: requirements through 2.0 are admitted, later requirements
are Unsupported, and malformed or non-string values are InvalidInput. This gate
does not infer support for future specifications. It admits core
numeric buffers, accessor layouts and primitive indices. Finite stored values and declared accessor bounds do not
prove unit-length normals, rendered material meaning or source/LOD fidelity.
KHR_mesh_quantization permits
its specified extra attribute representations. EXT_meshopt_compression supports
bounded decoding with the NONE filter; unavailable filters are unsupported. Each
compressed stream must start with the header byte the extension requires for its
mode (`0xa0` ATTRIBUTES, `0xe1` TRIANGLES, `0xd1` INDICES); other codec versions,
such as vertex v1 (`0xa1`) or index and sequence v0 (`0xe0`, `0xd0`), are invalid
even though the linked decoder can read them. The header is checked after the
filter, mode, stride and count checks: a stream with an unsupported filter is
reported unsupported whatever its header, because C1 refuses filtered streams
before reading their bytes.
All buffer consumers, including compressed streams, are restricted to declared
buffer lengths. Required meshopt may use URI-less fallback placeholders, including
an unused placeholder or GLB buffer zero without a BIN chunk. The optional fallback
marker has the same buffer-role restrictions whether actual bytes are provided or not.
Application-owned `extras` and extension-owned JSON remain opaque to the generic
extension-declaration walk.
`KHR_texture_basisu` ([KhronosGroup/glTF `1bde276f`](https://github.com/KhronosGroup/glTF/blob/1bde276f7a173876e24c0f403a14dd827dc7912b/extensions/2.0/Khronos/KHR_texture_basisu/README.md))
may be required. The image its `source` references must be KTX2: a `mimeType`, required
for bufferView images, must be `image/ktx2`, and a URI image without one is judged by its
bytes (no file-name suffix is required). A data URI stays Unsupported. A core fallback
`source` may not be a KTX2 image, and a texture without a fallback requires the extension in
`extensionsRequired`. Each image resource is inspected once (one pass per bufferView, however
many images share it). Each such image is
checked as a KTX 2.0 (v2.0.4) container only: identifier and header, 2D type (non-2D is
invalid for material maps and unsupported otherwise), `VK_FORMAT_UNDEFINED`, dimensions that
are multiples of 4, a `levelCount` the dimensions allow, ETC1S with BasisLZ or UASTC with no
or Zstandard supercompression and the extension's channel sets, no sRGB for Red and
Red-Green textures, no premultiplied alpha for material textures, the DFD basic block,
key/value framing with `KTXswizzle` `rgba` and `KTXorientation` `rd`, BasisLZ global data and
image descriptors, and every level packed inside the image with lengths that agree with its
size. Level data is never inflated, transcoded or decoded; the report lists
`KTX2 level data and transcoding` and `image decoding`. See the
[KTX2 amendment](architecture/c1-ktx2-basisu-amendment.md).
C1 also inspects the experimental `KHR_mesh_primitive_restart` draft at
[CesiumGS/glTF revision `9811e8407d4533500cfc6b10e3bc408345035a6f`](https://github.com/CesiumGS/glTF/blob/9811e8407d4533500cfc6b10e3bc408345035a6f/extensions/2.0/Khronos/KHR_mesh_primitive_restart/README.md).
Its admitted draw mode is indexed LINE_STRIP: the maximum unsigned component
value is a restart command when the extension appears in both `extensionsUsed`
and `extensionsRequired`. Core POINTS, LINES and TRIANGLES keep their ordinary
index rules. Each admitted restart-delimited strip has at least two vertices;
leading, trailing, consecutive or singleton segments are Unsupported in this
finite profile, rather than declared malformed by the draft. Other restart
draw modes are outside C1. This is pinned draft
compatibility, not a claim of ratified glTF extension or glTF 2.1 conformance;
see the [vector draft audit](vector-schema/README.md).

Buffer data URIs are admitted only for `application/octet-stream` or
`application/gltf-buffer` with standard base64 encoding. Decoded size is admitted
before allocation and its bytes receive the same buffer/accessor/index proof.
Malformed base64 is InvalidInput; other inline media/encodings are Unsupported.
Image data URIs remain Unsupported, with no image decoder claim.

Sparse accessors, remote resources and unknown required extensions are
outside this first profile. Valid unsupported content is distinct from malformed
content and from an exceeded resource ceiling.

Inspection checks the actual GLB envelope and buffer/view/accessor/index bytes,
not merely declared JSON counts. A permitted accessor without a bufferView is
zero-initialized under glTF semantics; its values have no stored source bytes.
This case is distinct from a referenced buffer or BIN payload that is missing. It checks decoded floating-point values,
attribute agreement and applicable declared accessor bounds. Local resource
references resolve within the admitted archive; no network lookup occurs.
Archive-local URI segments decode percent escapes exactly once as UTF-8 before
member lookup. Encoded separators, controls, malformed escapes and paths escaping
the archive fail as InvalidInput. A decoded literal percent remains filename
data; aliases resolve against the referring document directory. Independent
controls include escaped spaces, a literal percent, and a wrong encoded member
name that must not satisfy the decoded reference.
Known optional metadata and material extensions do not grant a semantic
validation claim: the report records what was not inspected. Reading their
ordinary buffer views is not proof of feature identity, metadata meaning,
texture correctness or material rendering.

The report separates payload checks from archive CRC/index/hash/reference and
hierarchy checks. `hierarchyBounds` means containment of declared tile volumes.
`decodedContentContainment` is the separate [C2 check](architecture/c2-decoded-containment-contract.md):
every drawn vertex of every selected static glTF scene instance lies inside its own tile's
volume, its content volume, any referencing external-content volume and every ancestor volume.
The vertex is placed after node transforms, the glTF y-up to z-up step and tile transforms.
The predicate is exact, with zero tolerance. It covers `.3tz` archives with
axis-aligned (signed-permutation) boxes and spheres, explicit and implicit alike (C2 slice 3).
Below an implicit root, every content the I1 reader emits is a reference, and its vertices are
tested against the tile's effective volume (its `TILE_BOUNDING_BOX` row, FLOAT32 rows widened
exactly, or the root's box for a rowless level-0 tile), every implicit ancestor's, the
level-0 tile's, the implicit root's own JSON volume, the referencing external-content volume
when the root roots an external tileset (after the root's transform), and every explicit
ancestor's. A tile at level 1 or deeper without a row has a derived volume that C2 does not
evaluate; a reference through one is `notInspected` as `derivedImplicitVolume`. A template
header with `metadata` is `tileMetadata`; subtree metadata tables are not. It is reported only
when the whole archive is in domain. Otherwise `notInspected` lists
`decodedContentBounds.<reason>` entries, for example `derivedImplicitVolume`, `regionVolume`,
`numericSpan` or `workLimit`. A proven escape fails as InvalidInput, and the
message names the member, tile depth, volume, node, mesh, primitive and vertex; for implicit
content, `depth` is the root's depth plus the level, and `ancestor:k` counts implicit levels,
then the root, then explicit ancestors. C2 runs only after C1 succeeds. Its own caps
(`containmentSpanBits` 4096 bits and `containmentNodeDepth` 64, fixed; and the budgets
`containmentWorkUnits` max(2^29, 128·S) units, `containmentBytesRead` max(2 GiB, 4·S) and
`containmentDecodedBytes` max(2 GiB, 16·S), with S the archive's stored member bytes) yield
`notInspected`, never ResourceLimit. Units are charged per operation
(per volume test and transform step at each level, per node, index, element, re-read and
re-decoded byte, JSON value node, external resource, instance primitive and reference); the
formula is in the C2 contract's work bound (C2-WB amendment). The unit budget is set from a
measured class matrix, not a proof: on the measurement host (Intel i7-12700KF, Linux, release
build) every class in the C2-WB matrix cost at most 14.65 ns per unit on an E-core and 8.10 ns
on a P-core (median of 7 clean A/B runs per class), so the precheck and streaming of one
validation stay within about 7.9 s plus 1,875 s per stored GB on an E-core (about 4.4 s plus
1,037 s per stored GB on a P-core) at the default budget.
Classes outside that matrix, other hosts, debug builds and C1's own inspection time are not
covered. Archives over the budget are reported as `workLimit`. The
frozen reasons are `derivedImplicitVolume` (slice 3; it replaces `implicitTiling`), `tilesetExtension`, `tileMetadata`, `upAxisOverride`,
`regionVolume`, `rotatedBox`, `gltfExtension`, `sceneSelection`, `skin`,
`morphTargets`, `animation`, `quaternionWindow`, `numericSpan` and `workLimit`. H7 removed
`rtcCenter`: its only carrier, a b3dm feature table, is Unsupported. Neither report success nor a matching
stored geometric error establishes a source-surface or LOD error bound. The
`notInspected` field records these boundaries explicitly. A required unsupported
feature prevents success; an external validator run separately owns its own
coverage and provenance.

ZIP admission uses one selected central-directory catalog for index validation,
member lookup, CRC verification and exact stored-range reads. The catalog holds one
fixed-width row per member in `@3dtilesIndex1@` MD5 order and the names in a log. It
stays in memory up to 96 MiB (every archive of at most 524,288 entries and a 32 MiB
directory); a larger catalog moves to a scratch directory, created at its first spilled
file, so resident memory does not grow with the entry count except for one bit per
member (21.3 MiB at the member ceiling). Scratch goes to the first disk-backed
directory with room among `$TMPDIR`, the user cache directory (`rusty-tiles/scratch`)
and the system temporary directory; memory-backed scratch is used only when nothing
else is available, and the report's `scratch.memoryBacked` then says so. The index is checked by one streamed merge against the catalog. More than 16
unreferenced members are reported as the first 16 names in byte order and a count. A
failure of the validator's own scratch storage is `io`, `resource_limit` or
`invalid_state` with the prefix "validation scratch:", never a verdict on the archive.
`--scratchDir DIR` (Python `scratch_dir`) puts the scratch inside exactly `DIR`, which must
exist (otherwise the request is refused as `usage` before the archive is opened); a
memory-backed `DIR` is used and labelled. The report-less `validate_3tz` facade never puts
its catalog in RAM: with only memory-backed candidates it is refused as `environment`
(exit 4, Rust `Error::Scratch(ScratchRefusal::MemoryBackedOnly)`); set `TMPDIR` to a
disk-backed directory.
The report's `payloads` rows are read on demand: the CLI streams them, and Python's
`validate(input, lazy_payloads=True)` returns them as a lazy read-only sequence (by
default they are a list). See the
[catalog-at-scale amendment](architecture/c1-archive-catalog-scale-amendment.md). The report's
`archiveStoredRecordLayout` check requires disjoint local-header/data/descriptor
extents; coherent nested member records are Unsupported. Ordinary nested ZIP
bytes within one opaque payload are permitted. Duplicate logical names and
malformed framing are InvalidInput.

The finite identity profile accepts ASCII names and flagged valid UTF-8 names.
Legacy non-ASCII encodings, Unicode Path overrides and AES extras are Unsupported.
All extra-field TLVs are framed, and ZIP64 values follow their sentinels exactly.
A local ZIP64 declaration selects 64-bit descriptor sizes; central-only ZIP64
with a descriptor is Unsupported. Signed and unsigned descriptors are reconciled
against CRC, sizes and the next physical boundary. Other well-framed extra fields
remain opaque. This is a bounded stored-3TZ profile; see the
[archive reader contract](architecture/archive-read-foundation-contract.md).

Nonregular inputs are Unsupported; POSIX FIFO admission does not wait for a
writer. Underlying archive read/seek errors retain their I/O causes, while actual
premature byte exhaustion is InvalidInput.

## Limits

Every key of the report's `limits` belongs to one of three classes
([limits amendment](architecture/validator-limits-amendment.md)). S, the report's
`storedBytes`, is the sum of the stored sizes of every central-directory member (the 3TZ
index included), fixed when the catalog is admitted.

- **Per-item format limits** are fixed and cannot be changed: 8 MiB JSON (`jsonBytes`) with
  depth 64, 131,072 JSON value nodes per document (`documentItems`), 128 MiB per member
  (`memberBytes`), 4 million accessor elements per payload, 128 MiB decoded views per
  document, 128 levels of hierarchy depth, 256 MiB of live tileset documents
  (`liveDocumentBytes`) and the C2 span and node-depth limits.
- **Volume caps** (`sourceArchiveBytes`, `archiveEntries`, `centralDirectoryBytes`,
  `archiveStoredBytes`) are opt-in and `null` unless set. Without them the archive is bounded
  by the file and the format: the 3TZ member ceiling (Unsupported), at least 46 directory
  bytes per entry, a directory inside the file, and disjoint member extents inside the file.
- **Amplification budgets** are `max(floor, ⌊S·p/q⌋)`: `hierarchyVisits` max(262,144, S/8),
  `references` max(1,048,576, S/8), `totalBytesRead` max(2 GiB, 4·S) (explicit member-hash
  and payload/document reads, including repeated reads and constructor index reads),
  `totalPayloadElements` max(16 million, 16·S) and the three C2 budgets above. Archives up to
  1 MB of stored bytes have exactly the floors. The report's `limitDerivation` gives each
  budget's floor, its ratio `perStoredByte` and its `source` (`floor`, `scaled` or
  `explicit`); `storedBytes`, `limits` and `limitDerivation` depend only on S and the
  request's overrides.

`--maxVolume KEY=N` (repeatable; `--max-volume`), Python `max_volume={KEY: N}` and Rust
`ValidationRequest::max_volume(LimitKey, N)` set a volume cap or replace a budget. Values may
be below the defaults (a stricter profile for untrusted input); per-item limits cannot be
given. An unknown or per-item key, a value that is not a decimal integer below 2^64, or a
repeated key is refused as `usage` (Python `InvalidRequestError`) before the input is opened.
A refused charge of a cap or budget is `resource_limit` whose message ends with
`(limit KEY=VALUE, SOURCE)` (`SOURCE` is `floor`, `scaled`, `explicit` or `cap`); the CLI
error carries `"limit": {"limit", "value", "observed", "derivation"}`, Python
`ResourceLimitError.limit` the same dict, and Rust `ValidationFailure::exceeded()`.

Each placement of a shared external tileset is charged its full visits,
references and document bytes, as if it were read and walked again. In fact each
external-tileset document is parsed and checked once. Later placements re-check
only what depends on the placement: the root's bounds and geometric error against
the referencing tile, the depth limits, and decoded containment through that
placement's ancestors. So `total_bytes_read` counts logical reads, which may exceed
physical reads. Verdicts, messages and report counts are the same as a full
re-walk ([memo amendment](architecture/c1-external-tileset-memo-amendment.md)).

The 3TZ index is not a payload member: it is admitted at exactly 24 bytes per
other member, as the central-directory count fixes it, and read in 64 KiB chunks.
The 178,956,970-member 3TZ ceiling, which producers also enforce, is a format
rule rather than a limit: a larger declared count is Unsupported before the entry
limit and before any directory allocation. At the ceiling the index is
4,294,967,256 bytes.
Implicit availability scans count toward the same `totalBytesRead` work
counter; an archive whose reads and scans together exceed it is refused with
"availability scan exceeds total_bytes_read work limit".
Separately bounded ZIP envelope/tail/header reads are outside that member-work
counter. These admission limits
are engineering bounds, not a total-process RSS promise. The serialized `limits` are the
effective values of the validation.

The entry, visit, reference, central-directory, value-node, member and decoded
ceilings are sized for default (implicit) vector output at its 100,000-tile
`--maxTiles` default: measured pre-forest archives use about 2.4-2.5 entries, 1.2-1.3
presentation nodes (hierarchy visits under I1), one `tileset.json` value node and 920 `vector-build.json`
bytes per tile. Content volume is bounded separately; the budgets grow with the archive's
stored bytes, so dense content is not refused for its size alone.
Explicit vector archives, written by the `--explicit` mode removed in 0.4.0,
keep their whole hierarchy in one `tileset.json` of about 50.5 value nodes per
tile. They reach the 131,072 value-node ceiling at about 2,595 tiles. Measured costs at these ceilings are recorded in
[bench/architecture_audit/validator_caps](../bench/architecture_audit/validator_caps/README.md).
The pinned C1 contract record
([c1-payload-integrity-contract.md](architecture/c1-payload-integrity-contract.md),
JSON value nodes, payload bytes and view bytes) still states the earlier
65,536 value nodes and 64 MiB member and decoded-view bytes. The values above
supersede it; the meanings are unchanged.

Rust failures are `ValidationFailure::{InvalidInput, Unsupported, ResourceLimit,
Io, Scratch, InvalidRequest, Cancelled, Observer, InvalidState}`. CLI JSON codes are
respectively `invalid_input`, `unsupported`, `resource_limit`, `io`, `io` (or `environment`
for memory-backed-only scratch), `usage`, `cancelled`, `observer_failure` and
`invalid_state`; exit codes are 3, 2, 3, 1, 1 (or 4), 2, 1, 1 and 1. Python maps them to
`DataError`, `UnsupportedError`, `ResourceLimitError`, `TilesIOError` (or
`EnvironmentError`), `InvalidRequestError`, `CancelledError`, `ObserverError` (a callback's
own exception, including `KeyboardInterrupt`, propagates unchanged) and `TilesError`.
Cancellation (`inspect_with`'s control, a Python callback or Ctrl-C) is checked before each
hashed member, traversal task and implicit tile; progress events have the phases
`validate.hash`, `validate.traverse` and `validate.rows` (CLI `--progress json`). `InvalidState` reports an internal invariant breach (a C2 exact-arithmetic
capacity fault); it is never a verdict about the archive.
No job/publication failure wrapper is used for this read-only operation.

Acceptance requires independent valid payload fixtures and sensitive corruption
controls, resource admission and Rust/CLI/installed-wheel parity. This finite
slice does not certify every glTF extension, decoded world-space bounds outside the
C2 domain (when `decodedContentContainment` is absent), metadata semantics, source fidelity, or the #125/#113 release gates.

Primary byte-layout authority is the published [glTF 2.0.1 registry specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html), identified there as source commit `8e798b02d254cea97659a333cfcb20875b62bdd4`. Its matrix-alignment rule permits omitted final column padding. Unpublished GitHub main revisions can differ; they are not silently substituted for this contract.

Implicit roots are read by the I1 implicit reader, on any explicit tile, including tiles inside explicit external tilesets. Every subtree reachable through child-subtree availability is read from a confined archive member, availability is checked bit by bit, and every available tile is visited once at its exact coordinates. Each implicit tile then receives the explicit-tile checks: payload integrity, `hierarchyBounds`, `geometricErrorOrder`, `resourceReferences` and the report-level `conversion.json` budgets. The report lists `implicitAddressingAndAvailability` in `checks`; it lists `recordedBudgets.implicitTileExtras` in `notInspected`, because per-tile `extras` budgets are not compared below an implicit root. Decoded containment of implicit content is the C2 check above (C2 slice 3); uninterpreted metadata (`metadataSemantics`) remains uninspected. The subtrees on the current path share the 128 MiB `document_decoded_bytes` limit; no expanded presentation tree is built. JSON content templates are Unsupported ("implicit: unsupported external tileset content template in <member>"), as are JSON-format subtrees, region and S2 implicit roots, and `subtreeLevels` or `availableLevels` above 32. Consequently, default point-cloud, vector and deep `mesh-to-3tz` archives written before the bounded implicit forest layout, which split their hierarchy into `implicit-tileset-*.json` documents, are Unsupported; convert them again. Archives written by `convert-to-implicit`, which was removed before 0.4.0, are Unsupported when they have more than one node. Single-node convert archives have no JSON template and still validate; they list their retained original payloads in `conversion.json` `retainedContentUris`, and validation still references and checks those payloads. See the [I1 implicit read contract](architecture/i1-implicit-read-contract.md).
