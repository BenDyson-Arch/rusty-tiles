# Bounded archive and payload inspection

C1 implements read-only inspection of self-contained 3TZ archives. Rust uses
`validate::inspect(ValidationRequest::new(path))`; CLI uses `validate PATH`;
Python uses `validate(path)`. Rust returns a typed `ValidationReport`, which
adapters serialize without adding domain decisions. There is no external-tool
option or subprocess fallback.

Supported payloads are GLB 2.0, archive-local glTF 2.0, and modern b3dm version 1
with the 28-byte header and the same embedded GLB proof. b3dm table framing,
JSON and required BATCH_LENGTH are inspected; feature/batch table meaning is
explicitly not inspected. Truncated framing is invalid; unavailable versions
or legacy interpretations do not receive an opaque successful pass.

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
bounded decoding with the NONE filter; unavailable filters are unsupported.
All buffer consumers, including compressed streams, are restricted to declared
buffer lengths. Required meshopt may use URI-less fallback placeholders, including
an unused placeholder or GLB buffer zero without a BIN chunk. The optional fallback
marker has the same buffer-role restrictions whether actual bytes are provided or not.
Application-owned `extras` and extension-owned JSON remain opaque to the generic
extension-declaration walk.
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
The predicate is exact, with zero tolerance. It covers explicit `.3tz` archives with
axis-aligned (signed-permutation) boxes and spheres. It is reported only when the whole archive
is in domain. Otherwise `notInspected` lists `decodedContentBounds.<reason>` entries, for
example `implicitTiling`, `regionVolume`, `rtcCenter`, `numericSpan` or `workLimit`. A proven
escape fails as InvalidInput, and the message names the member, tile depth, volume, node,
mesh, primitive and vertex. C2 runs only after C1 succeeds. Its own caps
(`containmentSpanBits`, `containmentWorkUnits`, `containmentBytesRead`,
`containmentDecodedBytes`, `containmentNodeDepth`; defaults 4096 bits, 2^28 units, 2 GiB,
2 GiB and 64) yield `notInspected`, never ResourceLimit. At the measured release-build unit costs the default work cap bounds
unit-charged C2 work to about 3 s; debug builds are slower, and the separately capped re-reads
and re-decoding (2 GiB each) and per-primitive vertex marking are not included in that
figure. Archives over the cap are reported as `workLimit`. The
frozen reasons are `implicitTiling`, `tilesetExtension`, `tileMetadata`, `upAxisOverride`,
`regionVolume`, `rotatedBox`, `rtcCenter`, `gltfExtension`, `sceneSelection`, `skin`,
`morphTargets`, `animation`, `quaternionWindow`, `numericSpan` and `workLimit`. Neither report success nor a matching
stored geometric error establishes a source-surface or LOD error bound. The
`notInspected` field records these boundaries explicitly. A required unsupported
feature prevents success; an external validator run separately owns its own
coverage and provenance.

ZIP admission uses one selected central-directory catalog for index validation,
member lookup, CRC verification and exact stored-range reads. The report's
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

Fixed ceilings are 8 MiB JSON with depth 64, 64 MiB per member, 1 GiB source archive and total
stored member bytes, 16 MiB central directory, 65,536 archive entries/document
items/hierarchy visits, 262,144 references, 4 million accessor elements per
payload, 16 million total payload elements, 64 MiB decoded views per document,
128 levels of hierarchy depth, and 2 GiB explicit member-hash and
payload/document read work including repeated reads and constructor index reads.
Separately bounded ZIP envelope/tail/header reads are outside that member-work
counter. These admission limits
are engineering bounds, not a total-process RSS promise. No request option
bypasses them. The serialized `limits` describe the actual profile constants.

Rust failures are `ValidationFailure::{InvalidInput, Unsupported, ResourceLimit,
Io, InvalidState}`. CLI JSON codes are respectively `invalid_input`, `unsupported`,
`resource_limit`, `io` and `invalid_state`; exit codes are 3, 2, 3, 1 and 1. Python maps
them to `DataError`, `UnsupportedError`, `ResourceLimitError`, `TilesIOError` and
`TilesError`. `InvalidState` reports an internal invariant breach (a C2 exact-arithmetic
capacity fault); it is never a verdict about the archive.
No job/publication failure wrapper is used for this read-only operation.

Acceptance requires independent valid payload fixtures and sensitive corruption
controls, resource admission and Rust/CLI/installed-wheel parity. This finite
slice does not certify every glTF extension, decoded world-space bounds outside the
C2 domain (when `decodedContentContainment` is absent), metadata semantics, source fidelity, or the #125/#113 release gates.

Primary byte-layout authority is the published [glTF 2.0.1 registry specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html), identified there as source commit `8e798b02d254cea97659a333cfcb20875b62bdd4`. Its matrix-alignment rule permits omitted final column padding. Unpublished GitHub main revisions can differ; they are not silently substituted for this contract.

Implicit archives use a bounded structural presentation of their declared hierarchy. The report explicitly leaves implicit addressing/availability certification unclaimed. A shared archive-wide expansion budget admits at most 65,536 presentation nodes and 64 MiB of serialized node data before retaining descendants; per-expansion caches are separately limited to 64 MiB of source subtree bytes. These limits do not constitute whole-process memory bounds. Archives written by `convert-to-implicit`, which was removed before 0.4.0, list their retained original payloads in `conversion.json` `retainedContentUris`; validation still references and checks those payloads.
