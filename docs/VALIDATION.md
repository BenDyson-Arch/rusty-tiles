# Validation

`validate` inspects a self-contained `.3tz` archive, read-only, against a fixed, bounded profile. `validate --help` calls it the C1 payload profile. It reports which checks ran, what was not inspected, and the limits it used.

| Interface | Call |
| --- | --- |
| CLI | `rusty-tiles validate PATH [--maxVolume KEY=N]... [--scratchDir DIR] [--expectProfile hlod]` |
| Rust | `validate::inspect(ValidationRequest::new(path))`, or `inspect_with(request, &RunControl)` for cancellation and progress |
| Python | `validate(path, *, lazy_payloads=False, max_volume=None, scratch_dir=None, expect_profile=None, callback=None)` |

Rust returns a typed `ValidationReport`; the CLI and Python serialize it unchanged. There is no external-tool option or subprocess fallback. Options are listed in the [command reference](CLI.md#validate) and limits under [limits](#limits).

## Report

The JSON report contains:

| Field | Meaning |
| --- | --- |
| `ok` | `true` when every listed check passed |
| `archive` | The inspected path |
| `tiles` | Explicit tiles plus available implicit tiles; each implicit root is counted once |
| `contentReferences` | Every content entry read: explicit payloads, external-tileset links and implicit contents |
| `entries` | Archive members |
| `checks` | The checks that ran and passed |
| `notInspected` | What was deliberately not inspected |
| `notInspectedTruncated` | `true` when unsupported extension names were left out of `notInspected` (absent otherwise) |
| `storedBytes` | S, the archive's stored member bytes, used to scale budgets |
| `limits` | The effective limits of this run |
| `limitDerivation` | How each budget was derived |
| `payloads` | One row per payload URI, in byte order: `uri`, `accessorsChecked`, `primitivesChecked`, `vertices` |
| `interfaceCensus` | HLOD archives only: border and omission counts (see [HLOD archives](#hlod-archives)) |
| `scratch` | Present only when scratch files were written: `memoryBacked` and `peakBytes` |
| `warnings` | Present only when some warning applies: a list of `{code, message}`. Warnings never fail validation; `ok` stays `true`. The CLI prints each as `warning: <code>: <message>` on stderr. The one code so far is [`coarseNormalInversion`](#coarse-normals) |
| `resources` | Present when scratch was used (always for HLOD archives) or the request set a memory budget or jobs: the resolved budget, workers, store sizes and scratch-disk bound (see [memory, workers and disk](#memory-workers-and-disk)) |

Payload rows are read on demand. The CLI streams them. Python returns a list by default, or a lazy read-only sequence with `validate(input, lazy_payloads=True)`.

Passing means the listed checks completed within the profile. It does not establish source fidelity, geometric-error accuracy, rendered extent, metadata meaning or material meaning. A stored geometric error is not a proven bound on source or LOD error.

### Checks

`checks` lists these names, in this order, when they run:

| Check | Runs | Meaning |
| --- | --- | --- |
| `archiveIndex` | always | The `@3dtilesIndex1@` index matches the central directory |
| `archiveCrc` | always | Every member's CRC matches |
| `archiveStoredRecordLayout` | always | Member records are disjoint and stored in the admitted layout |
| `contentHashes` | always | Every member is read and hashed in one pass; member paths must be safe |
| `tilesetSchema` | always | Tileset JSON matches the bundled 3D Tiles schema |
| `payloadEnvelope` | always | GLB and glTF envelopes are well formed |
| `bufferRanges` | always | Buffers, views and accessors stay within declared lengths |
| `accessorValues` | always | Decoded accessor values are finite and within declared bounds |
| `primitiveIndices` | always | Indices are in range for their primitives |
| `hierarchyBounds` | always | Each child's declared volume is inside its parent's |
| `implicitAddressingAndAvailability` | implicit tiling | Subtrees, availability bits and tile coordinates are consistent |
| `geometricErrorOrder` | always | Geometric error does not increase from parent to child |
| `replaceContentContinuity` | HLOD | REPLACE refinement never swaps rendered content for an empty cell |
| `resourceReferences` | always | Every referenced resource resolves inside the archive |
| `recordedBudgets` | always | Budgets recorded in `conversion.json` and tile `extras` match the content |
| `decodedContentContainment` | when in domain | Every drawn vertex is inside its tile's volumes (see [decoded containment](#decoded-containment)) |
| `hlodFeatureIds` | HLOD | Every payload carries valid source identity |
| `interfaceSegments` | HLOD | Edges shared between neighbouring tiles match exactly |
| `interfacePoints` | HLOD | Points shared between neighbouring tiles match exactly |
| `interfaceWinding` | HLOD | Faces at shared interfaces keep a consistent orientation |
| `borderDisplacement` | HLOD | A simplified shared border stays within G/8 of the leaf border |
| `sameDepthBorderGap` | HLOD | Sibling tiles deliver identical borders |
| `terminalBorderLock` | HLOD | A leaf's border is not moved by a deeper neighbour |
| `mixedDepthBorderGap` | HLOD | Neighbours at different depths stay within the coarser tile's G/8 |
| `descendantRecovery` | HLOD | Declared recovery roots are consistent (see [descendant recovery](#descendant-recovery)) |

G is a tile's `geometricError`. A failed check is reported as an error whose message starts with the check or rule name, such as `interfaceSegments:`.

## Content

### Supported payloads

Supported payloads are GLB 2.0 and archive-local glTF 2.0, read against the published [glTF 2.0.1 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html) (source revision `8e798b02d254cea97659a333cfcb20875b62bdd4`). Its matrix-alignment rule permits omitted final column padding. Unpublished revisions of the specification are not substituted.

- `asset.version` must be `2.0`. Optional `asset.minVersion` must have numeric major.minor syntax: requirements up to 2.0 are admitted, later ones are Unsupported, and malformed or non-string values are InvalidInput.
- Core numeric buffers, accessor layouts and primitive indices are admitted. Finite values and declared bounds do not prove unit-length normals, material meaning or source fidelity.
- Validation checks the actual GLB envelope and buffer, view, accessor and index bytes, not only declared JSON counts. It checks decoded floating-point values, attribute agreement and declared accessor bounds.
- An accessor without a bufferView is zero-initialized, as glTF specifies. This is distinct from a missing buffer or BIN chunk.
- Sparse accessors, remote resources and unknown required extensions are outside the profile. Unknown required extensions fail as Unsupported. Optional extensions are listed in `notInspected`.
- Known optional metadata and material extensions are read as ordinary buffer views only. This does not validate feature identity, metadata meaning, texture correctness or material rendering.
- Application `extras` and extension-owned JSON are not interpreted by the extension walk.

Valid but unsupported content is reported as Unsupported, distinct from malformed content (InvalidInput) and from an exceeded limit (ResourceLimit).

### Extensions

**KHR_mesh_quantization** permits its extra attribute representations.

**EXT_meshopt_compression** is decoded within the same limits.

- Filters NONE and OCTAHEDRAL are supported; QUATERNION and EXPONENTIAL are Unsupported. A stream with an unsupported filter is Unsupported whatever its header, because filtered streams are refused before their bytes are read.
- OCTAHEDRAL needs ATTRIBUTES mode and a 4- or 8-byte stride. Every element must be a valid K-bit code: the third component encodes 1.0 as `(1 << (K - 1)) - 1`, and |x| and |y| do not exceed it. Anything else is invalid.
- The extension defines filter output only to one unit, so declared bounds on a filtered BYTE or SHORT accessor may differ from decoded values by one unit. All other bounds are exact.
- A compressed view may serve any role, including property-table values.
- Each stream must start with the header byte for its mode: `0xa0` ATTRIBUTES, `0xe1` TRIANGLES, `0xd1` INDICES. Other codec versions, such as vertex v1 (`0xa1`) or index and sequence v0 (`0xe0`, `0xd0`), are invalid. The header is checked after the filter, mode, stride and count.
- Required meshopt may use URI-less fallback placeholders, including an unused placeholder or GLB buffer zero without a BIN chunk. The optional fallback marker has the same buffer-role rules whether or not bytes are present.

**KHR_texture_basisu** ([specification at `1bde276f`](https://github.com/KhronosGroup/glTF/blob/1bde276f7a173876e24c0f403a14dd827dc7912b/extensions/2.0/Khronos/KHR_texture_basisu/README.md)) may be required.

- The image its `source` references must be KTX2. A `mimeType`, required for bufferView images, must be `image/ktx2`. A URI image without one is judged by its bytes; no file-name suffix is needed. A data URI is Unsupported.
- A core fallback `source` may not be a KTX2 image. A texture without a fallback needs the extension in `extensionsRequired`.
- Each image resource is inspected once, however many images share it.
- Each image is checked as a KTX 2.0 (v2.0.4) container only: identifier and header; 2D type (non-2D is invalid for material maps and Unsupported otherwise); `VK_FORMAT_UNDEFINED`; dimensions that are multiples of 4; a `levelCount` the dimensions allow; ETC1S with BasisLZ, or UASTC with no or Zstandard supercompression, and the extension's channel sets; no sRGB for Red and Red-Green textures; no premultiplied alpha for material textures; the DFD basic block; key/value framing with `KTXswizzle` `rgba` and `KTXorientation` `rd`; BasisLZ global data and image descriptors; and every level packed inside the image with lengths that agree with its size.
- Level data is never inflated, transcoded or decoded. The report lists `KTX2 level data and transcoding` and `image decoding` in `notInspected`.

**KHR_mesh_primitive_restart**, an experimental draft ([CesiumGS/glTF `9811e8407d4533500cfc6b10e3bc408345035a6f`](https://github.com/CesiumGS/glTF/blob/9811e8407d4533500cfc6b10e3bc408345035a6f/extensions/2.0/Khronos/KHR_mesh_primitive_restart/README.md)), is inspected for indexed LINE_STRIP only. When the extension is in both `extensionsUsed` and `extensionsRequired`, the maximum index value is a restart. Each restart-delimited strip needs at least two vertices; leading, trailing, consecutive or single-vertex segments are Unsupported. Other restart draw modes are outside the profile. Core POINTS, LINES and TRIANGLES keep their ordinary index rules. This is compatibility with the pinned draft, not ratified glTF or glTF 2.1 conformance; see the [vector draft audit](vector-schema/README.md).

### Data URIs and resource paths

Buffer data URIs are admitted only for `application/octet-stream` or `application/gltf-buffer` with standard base64. The decoded size is checked before allocation, and the bytes get the same buffer, accessor and index checks. Malformed base64 is InvalidInput; other media types or encodings are Unsupported. Image data URIs are Unsupported.

Resources resolve inside the archive; there is no network lookup. URI segments are percent-decoded exactly once as UTF-8 before member lookup. Encoded separators, control characters, malformed escapes and paths that leave the archive are InvalidInput. A decoded literal `%` is part of the file name. Aliases resolve against the referring document's directory.

### Legacy content

3D Tiles 1.0 content (b3dm, i3dm, pnts and cmpt) is Unsupported (exit 2) and never partly inspected. A payload is legacy when:

- its member name ends in `.b3dm`, `.i3dm`, `.pnts` or `.cmpt` (ASCII, case-insensitive), decided before the payload is read; or
- its first four bytes are one of those magics under any other suffix, such as b3dm bytes in a `.glb` or `.bin` member. Only those four bytes are examined.

An implicit content template whose last segment has a legacy suffix is Unsupported before any subtree is read. Nothing after the magic is interpreted, so a cmpt that wraps a GLB is Unsupported too. A missing member is still InvalidInput. Messages name the format and member:

```text
legacy b3dm content is unsupported (glTF 2.0 content only): tile.b3dm
legacy b3dm content is unsupported (glTF 2.0 content only): tile.glb starts with b3dm magic
implicit: unsupported legacy b3dm content template in tileset.json
```

Producers write only GLB. Convert older archives again.

## Archive format

The archive reader admits a bounded stored-3TZ profile.

- One central-directory catalog is used for index validation, member lookup, CRC checks and exact stored-range reads.
- `archiveStoredRecordLayout` requires disjoint local-header, data and descriptor extents. Nested member records are Unsupported. Ordinary ZIP bytes inside one payload are allowed. Duplicate names and malformed framing are InvalidInput.
- Names must be ASCII or flagged valid UTF-8. Legacy non-ASCII encodings, Unicode Path overrides and AES extras are Unsupported.
- Every extra field is framed, and ZIP64 values follow their sentinels exactly. A local ZIP64 declaration selects 64-bit descriptor sizes; central-only ZIP64 with a descriptor is Unsupported. Signed and unsigned descriptors are checked against CRC, sizes and the next record. Other well-framed extra fields are not interpreted.
- Non-regular inputs are Unsupported; a POSIX FIFO is not waited on. Read and seek errors are `io`; running out of bytes early is InvalidInput.
- More than 16 unreferenced members are reported as the first 16 names in byte order and a count.

The 3TZ index is admitted at exactly 24 bytes per other member, as the central-directory count fixes it, and is read in 64 KiB chunks. It is checked by one streamed merge against the catalog. The 3TZ format allows at most 178,956,970 members (producers enforce this too); a larger declared count is Unsupported before any directory allocation. At that ceiling the index is 4,294,967,256 bytes.

### Scratch space

The catalog holds one fixed-width row per member and a log of names. It stays in memory up to the larger of 96 MiB and one eighth of the planned memory (see [memory, workers and disk](#memory-workers-and-disk)); 96 MiB covers every archive of at most 524,288 entries and a 32 MiB directory. A larger catalog moves to scratch files, so resident memory grows by only one bit per member (21.3 MiB at the member ceiling). Per-payload facts move to scratch once they exceed the larger of 100 MiB and one eighth of the planned memory.

Scratch goes to the first disk-backed directory with room among `$TMPDIR`, the user cache directory (`rusty-tiles/scratch`) and the system temporary directory. Memory-backed scratch is used only when nothing else is available; the report's `scratch.memoryBacked` then says so. `--scratchDir DIR` (Python `scratch_dir`) puts scratch inside exactly `DIR`, which must exist; otherwise the request is refused as `usage` before the archive is opened. A memory-backed `DIR` is used and labelled.

A failure of scratch storage is `io`, `resource_limit` or `invalid_state` with the prefix `validation scratch:`. It is never a verdict on the archive.

### Memory, workers and disk

| Surface | Memory budget | Workers |
| --- | --- | --- |
| CLI | `--memory-budget BYTES` (`--memoryBudget`) | `--jobs N` or `auto` |
| Python | `validate(..., memory_budget_bytes=None)` | `jobs=None` |
| Rust | `ValidationRequest::memory_budget(bytes)` | `ValidationRequest::jobs(n)` |

The budget follows `mesh`'s rule and syntax ([memory budget](MESH.md#memory-budget)): the default is the lesser of 4 GiB and half the detected host or cgroup headroom, or 2 GiB when none is detected; an explicit budget is a ceiling that is never raised, and the stores are planned against the lesser of the budget and the detected headroom. Zero or malformed values are `usage` errors. An explicit budget below 512 MiB is `resource_limit`, before the archive is opened. Workers default to every core; an explicit count is a maximum. Each worker beyond the first is admitted against the budget at 64 MiB, and workers take at most half of the planned share: 1 GiB admits 6 workers, 4 GiB admits 30.

The budget sizes the validator's scale stores. Per-item work (one document, one payload decode) stays bounded by the fixed [per-item limits](#limits), so 256 MiB of the budget is left outside the plan for it. Of the rest, P, the stores live at once during the HLOD delivery stage take at most about 0.9·P:

- the delivery store's external sorts: runs of up to about P/6 (at least 24 MiB), merged up to 128 at a time with 256 KiB to 1 MiB buffers, so a sort of up to `singlePassBytes` (128 runs) needs one merge pass. Measured plans on a 16-core host: about 54 GB at 4 GiB with 16 workers (74 GB with one) and 4.4 GB at 1 GiB with its 6 workers (14 GB with one). The fixed 0.4.0 profile merged 32 runs of 24 MiB (0.8 GB) per pass;
- the delivery store's SQLite page cache, P/8 (it was 8 MiB). SQLite's sorter keeps at most the same amount in memory. Its temporary store is in memory only when its bound fits P/16: every row it can hold is a charged delivery row, a tile or a reference, so the bound is `(totalPayloadElements ÷ 128 + hierarchyVisits + references) × 128` bytes. Otherwise, as for every large archive, it stays on disk;
- the catalog page cache (P/32), the resident thresholds above and the stream log (P/32), and the payload-row sort (P/16).

Workers replay HLOD payload geometry (the leaf, edge-face and coarse passes) a bounded window ahead of the pass that consumes them, and sort the delivery store's runs. The pass still consumes placements in order and makes every work charge, read admission and refusal at the point a single worker makes it, so work counts, refusals and reports are those of one worker. SQLite statements and the traversal run on one thread.

Before traversal starts, the estimated scratch peak is checked against free space on the scratch volume: the catalog's terms plus, for an HLOD archive, 7 bytes per stored byte for the delivery store and its sorts (measured: about 2.7 on Walem 2048, at least 5.5 on a 26 GB archive). A short volume is refused as typed insufficient space (`io`, Python `TilesIOError`), with the needed and available bytes. Like every preflight it is not a reservation.

SQLite writes the delivery store's sorter files (about 0.4 GB for Walem 2048) to its own temporary directory (on Unix `SQLITE_TMPDIR`, then `TMPDIR`, then `/var/tmp`), not to the scratch directory; they are outside `scratch.peakBytes` and the bound above. Set `SQLITE_TMPDIR` (or `TMPDIR`) to a disk with room when it matters.

The report's `resources` member states `memoryBudget` (`requestedBytes`, `resolvedBytes`, `source`: `explicit`, `default_ceiling`, `default_headroom` or `default_undetected`, `detectedHeadroomBytes`, `plannedBytes`), `jobs` (`requested`, `cores`, `jobs`, `limitedBy`), `stores` (`sortRunBytes`, `sortFanIn`, `sortBufferBytes`, `singlePassBytes`, `catalogCacheBytes`, `sqliteCacheBytes`, `sqliteTempStore`) and, once checked, `disk` (`estimatedPeakBytes`, `availableBytes`). None of these change a verdict. Every report member except `resources` and `scratch` is identical for every budget and worker count.

The report-less Rust facade `validate_3tz` never keeps its catalog in RAM. With only memory-backed candidates it is refused as `environment` (exit 4; Rust `Error::Scratch(ScratchRefusal::MemoryBackedOnly)`). Set `TMPDIR` to a disk-backed directory.

## Implicit tiling

Implicit roots are read on any explicit tile, including tiles inside external tilesets.

- Every subtree reachable through child-subtree availability is read from an archive member, availability is checked bit by bit, and every available tile is visited once at its exact coordinates.
- Each implicit tile gets the explicit-tile checks: payload integrity, `hierarchyBounds`, `geometricErrorOrder`, `resourceReferences` and the report-level `conversion.json` budgets.
- `checks` lists `implicitAddressingAndAvailability`. `notInspected` lists `recordedBudgets.implicitTileExtras`, because per-tile `extras` budgets are not compared below an implicit root. Metadata meaning (`metadataSemantics`) is not inspected.
- Subtrees on the current path share the 128 MiB `documentDecodedBytes` limit. No expanded tree is built.
- Availability scans count toward `totalBytesRead`. An archive whose reads and scans exceed it is refused with `availability scan exceeds total_bytes_read work limit`.

These are Unsupported, with messages prefixed `implicit: unsupported`:

- JSON content templates (`implicit: unsupported external tileset content template in <member>`);
- JSON-format subtrees;
- region and S2 implicit roots;
- `subtreeLevels` or `availableLevels` above 32.

Archives whose implicit content templates name JSON tilesets therefore need a fresh conversion. Single-node archives with no template still validate; payloads they list in `conversion.json` `retainedContentUris` are referenced and checked.

## External tilesets

Each placement of a shared external tileset is charged its full visits, references and document bytes, as if it were read and walked again. In practice each external tileset document is parsed and checked once while its summaries fit in 64 MiB; beyond that, later placements are walked again with the same result. Later placements re-check only what depends on placement: the root's bounds and geometric error against the referencing tile, the depth limits, and decoded containment through that placement's ancestors. `totalBytesRead` therefore counts logical reads, which may exceed physical reads. Verdicts, messages and report counts are the same as a full re-walk.

## Decoded containment

`hierarchyBounds` checks declared tile volumes. `decodedContentContainment` checks actual geometry: every drawn vertex of every selected static glTF scene instance lies inside its own tile's volume, its content volume, any referencing external-content volume and every ancestor volume. Vertices are placed after node transforms, the glTF Y-up to Z-up rotation and tile transforms. The test is exact, with zero tolerance.

It covers `.3tz` archives whose volumes are axis-aligned boxes (signed permutations) or spheres, explicit and implicit. Below an implicit root, each content is tested against:

- the tile's effective volume: its `TILE_BOUNDING_BOX` row (FLOAT32 rows widened exactly), or the root's box for a level-0 tile without a row;
- every implicit ancestor's volume and the level-0 tile's;
- the implicit root's own JSON volume;
- the referencing external-content volume, when the root is inside an external tileset (after the root's transform);
- every explicit ancestor's volume.

A tile at level 1 or deeper without a row has a derived volume, which is not evaluated; such references are not inspected (`derivedImplicitVolume`). A template header with `metadata` gives `tileMetadata`; subtree metadata tables do not.

The check runs only after the payload and hierarchy checks pass with no exclusion, and only when the whole archive is in its domain. It replays each payload against digests from its first inspection rather than inspecting it again. Otherwise `notInspected` lists `decodedContentBounds.<reason>`, where reason is one of:

`derivedImplicitVolume`, `tilesetExtension`, `tileMetadata`, `upAxisOverride`, `regionVolume`, `rotatedBox`, `gltfExtension`, `sceneSelection`, `skin`, `morphTargets`, `animation`, `quaternionWindow`, `numericSpan`, `workLimit`.

A proven escape fails as InvalidInput. The message names the member, tile depth, volume, node, mesh, primitive and vertex. For implicit content, `depth` is the root's depth plus the level, and `ancestor:k` counts implicit levels, then the root, then explicit ancestors.

Containment has its own limits, which give `notInspected` (`numericSpan` or `workLimit`), never `resource_limit`:

| Limit | Value |
| --- | --- |
| `containmentSpanBits` | 4096 bits, fixed |
| `containmentNodeDepth` | 64, fixed |
| `containmentWorkUnits` | max(2^29, 128·S) units |
| `containmentBytesRead` | max(2 GiB, 4·S) |
| `containmentDecodedBytes` | max(2 GiB, 16·S) |

Work units are charged per operation: per volume test and transform step at each level, per node, index, element, re-read and re-decoded byte, JSON value node, external resource, instance primitive and reference. On an Intel i7-12700KF (Linux, release build), every measured content class cost at most 14.65 ns per unit on an E-core and 8.10 ns on a P-core. At the default budget, containment for one validation therefore takes at most about 7.9 s plus 1,875 s per stored GB on an E-core, or 4.4 s plus 1,037 s per stored GB on a P-core. Other content classes, hosts, debug builds and the rest of validation are not covered by that estimate.

Unsupported glTF extension names declared by payloads are listed in `notInspected` as named: at most the 64 byte-smallest, each at most 256 bytes. When any is left out, the report adds `notInspectedTruncated: true`. A required unsupported feature prevents success. An external validator run separately is responsible for its own coverage.

## HLOD archives

An archive that declares the HLOD profile is held to the HLOD checks: every payload must carry source identity, and the identity, interface and continuity checks run. The profile is declared by `asset.extras.rustyTiles.lod` or the `conversion.json` `lod` equal to `"hlod"`. `mesh` writes such archives.

`--expectProfile hlod` (Rust `ValidationRequest::expect_profile(ExpectedProfile::Hlod)`, Python `expect_profile="hlod"`) requires the profile whatever the archive's labels say, so an archive stripped of its labels cannot pass as a plain one. `hlod` is the only value; any other is refused as `usage` (Python `ValueError`) before the archive is opened.

Every payload holds at most 8,192 triangles, counted over all its scene instances; a larger one fails `triangleCap`. Source identity errors are reported as `hlodFeatureIds` or `sourceIdentity`.

### Interface census

The report's `interfaceCensus` is derived from the archive alone, using decoded leaf keys (`basis`), not producer counters:

| Field | Meaning |
| --- | --- |
| `basis` | `decoded leaf canonical f32 keys` |
| `sharedPoints`, `sharedSegments` | Points and edges shared between leaf tiles |
| `sourceOpenEdges`, `sourceOpenEdgesWithSharedEndpoints` | Open edges, and those whose endpoints are shared |
| `maxPayloadTriangles` | The largest payload, counted over all its scene instances |
| `omittedCells`, `omittedByDepth`, `omissionSources` | Content-less delivered cells (placeholders excluded), by cell depth, and those whose child cells all render |
| `frontierTiles`, `frontierLeaves` | Tiles of the first renderable cover, and the leaves it covers once |
| `contractedBorderArcs`, `contractedBorderVertices` | Shared borders delivered as one chord over two or more leaf edges, and the leaf border vertices those chords remove |
| `droppedComponents`, `droppedByDepth` | Child components that coarse payloads no longer hold, by delivered cell depth |
| `textureRefinableTiles`, `textureBoundTiles` | Tiles refinable to finer texels, and those whose `geometricError` is within 1% of the texture term |
| `coarseNormals` | Coarse tiles that publish `NORMAL` and their faces (`tiles`, `faces`), tiles whose `NORMAL` is not compared (`skippedTiles`), and those with a face shaded against its winding (`invertedTiles`, `invertedFaces`, and the same `ByDepth`, by cell depth), against `boundDegrees` (90); see [coarse normals](#coarse-normals) |

### Coarse normals

For every coarse tile that publishes `NORMAL`, validation compares each face's corner normals with the face's own normal (wound by its corners, after the node transform). A corner normal more than 90° from its face means the face shades against its winding: under double-sided lighting it renders inverted. Such faces are counted in `interfaceCensus.coarseNormals` by cell depth and reported as the `coarseNormalInversion` warning. They never fail validation.

The bound is 90° because that is where shading inverts, whatever smoothing a producer chose. `mesh` keeps every coarse corner normal within 60° of its face ([coarse normals](MESH.md#coarse-appearance)), so normal and position rounding stay far from the bound. Archives written before that change (issue #336) average every face at a vertex and usually carry the warning; rebuilding them removes it. Leaves are not compared: they keep the source's own normals. The comparison is charged to the existing work budget (6 units per face). It does not compare coarse orientation with the source surface, which remains not inspected. The comparison reads a `NORMAL` that is a VEC3 of floats or of normalized 8- or 16-bit integers, one per vertex, in a buffer view without an `EXT_meshopt_compression` filter. Any other coarse `NORMAL` that the earlier checks admit is not compared and never fails validation. Such tiles are counted as `skippedTiles`, and `notInspected` names the reason: `coarseNormalInversion.meshoptFilteredNormal` (for example `OCTAHEDRAL`), `.sparseNormal`, `.viewlessNormal` (zero storage without a buffer view) or `.unsupportedNormalAccessor`. `mesh` writes coarse normals the comparison reads.

### Border rules

A coarse tile may simplify a shared leaf border to chords between retained exact vertices, one chord per arc. Each removed vertex must lie within G/8 of its chord (`borderDisplacement`); junctions and point contacts stay exact. Siblings must deliver identical borders (`sameDepthBorderGap`). A shallower leaf locks its deeper neighbours (`terminalBorderLock`). Other neighbours that are provably no deeper stay within G/8 (`mixedDepthBorderGap`). Faces that use the same edge in the same direction must lie on distinct sides, and coincident faces need coincident source.

On every coarse tile, `seamOwner` rejects new edges or faces on shared points outside their deterministic seam owner, chosen by a hash over whole-source runs of holding leaves.

Not inspected: contraction against source arcs, producer certificates, logical depth, topology receipts, fresh coarse UV charts, source distance, and coarse orientation outside required interfaces.

### Coarse drops

A coarse payload that drops whole components of its children declares how many in glTF `asset.extras.rustyTiles.droppedComponents`. Validation recounts the drops from decoded keys and fails `coarseDrops` (`invalid_input`) when:

- the count differs from the declaration; or
- a dropped point, or a fixed sample point of a dropped face, is farther than the payload's `geometricError` from its surface.

Below last-resort stage 3, drops at or above the children's largest `geometricError` also need cap pressure (children above 8,192 faces) and must go smallest first among components that do not touch a border.

A payload that declares last-resort stage 3 is exempt from those two rules: its border-free components may drop at any size, with the loss measured into G. Only the count and a continuous distance bound apply: every point of each decoded dropped child triangle must lie within `G*(1+1e-6)+1e-6` of the retained surface, with no child-G slack.

- A region is certified when its centroid distance plus its farthest-corner radius fits the bound, or when all three corners fit it against one retained triangle.
- Otherwise it is split into four by midpoints, to depth 32. Unresolved regions fail `coarseDrops`. Running out of effort is refused with the usual budget error.
- Every region and point-to-face test is charged. The depth-first stack holds at most 97 regions.
- Each distance test is charged 9 units, plus 795 more when it needs the slow exact path.

This is a sufficient certificate: it can refuse a valid surface when its budget or depth runs out, but never accepts an escape. Stages 0 to 2 and undeclared drops use fixed samples, with child G added away from vertices; samples give only lower bounds, not continuous certification. None of these checks proves fallback history or distance to unpublished source geometry. HLOD archives whose small drops are undeclared fail this check. The [appendix](#appendix-distance-certificate-arithmetic) gives the numerical argument.

### Last-resort declarations

`asset.extras.rustyTiles.lastResort` is a strict object with exactly two required integer fields: `stage` (0 to 3) and `foldFaces` (0 to 18446744073709551615). Unknown or missing fields and other types are invalid. Absence means no declaration. For example, inside a GLB's `asset`:

```json
{"extras":{"rustyTiles":{"lastResort":{"stage":2,"foldFaces":4}}}}
```

This declares four qualifying faces, which must match the recount.

| Stage | Meaning |
| --- | --- |
| 0 | Inherited folds kept above a stage-2 or stage-3 descendant; not a reduction strategy |
| 1 | New faces and edges on shared points are admitted only in their seam owner |
| 2 | Also admits folds, turns reversed faces to source orientation and removes coincident copies beyond the source count |
| 3 | Clusters each input component while fixing interface points, opening pins and open boundaries that touch a border. Border-free components and holes may drop or close at any size, with the loss measured into G. |

`foldFaces` counts decoded faces in a coincident group (at least two faces on the same corners, either orientation) or incident to an edge used in the same direction by at least two faces. Each qualifying face counts once.

Validation fails `lastResort` when:

- the recount differs from `foldFaces`;
- a stage-0 declaration has no fold-admitting descendant;
- a leaf carries a declaration;
- payloads of one tile disagree on `stage` or `foldFaces`;
- folds appear outside stage 2 or 3 or a declared ancestor of such a tile.

`mesh --json` reports these in `diagnostics.hlod`: `last_resort_tiles`, `last_resort_stages` (counts for stages 1, 2 and 3), `fold_faces` and `declared_tiles` (including stage-0 tiles, with stage and fold count). Tiles are omitted only when the declared constraints alone exceed the triangle cap, or when border-touching open boundaries cannot survive stage 3. An omission propagates upward unless [descendant recovery](#descendant-recovery) succeeds. A last-resort declaration does not itself permit skipping cells or certify distance to the source.

### Texture error

An HLOD archive declares its texture-aware error rule ([mesh guide](MESH.md#texture-aware-published-g)) as `asset.extras.rustyTiles.textureError`: a strict object with exactly `version` (the integer 1), `refinableRatio` (1.25) and `pxPerTexel` (2, 1.25 or `null`). `null` is the `fast` form and binds nothing.

It fails `textureError` when:

- it is `null`, has any other shape or value, or appears on an archive not held to HLOD;
- a tileset declaring `lod: "hlod"` does not declare it;
- it is absent and an HLOD identity payload binds a base-colour texture (removing the declaration never disables the check);
- `conversion.json` records a `mesh_tiler` profile and `pxPerTexel` does not match it: `fast` `null`, `balanced` 2, `quality` and `lossless` 1.25.

The profile match is a consistency check, not forgery protection: `conversion.json` is not hash-bound to the tileset.

With a number `t`, every content tile T whose mean base-colour texel spacing `s(T)` exceeds 1.25 times its children's pooled spacing must have `geometricError >= 16 · s(T) / t`, with relative slack 1e-4. `s = sqrt(A / X)` is recomputed from each payload. `A` is the decoded area of triangles whose material has a base-colour texture. `X` is their UV area on the material's TEXCOORD set times the width × height of the embedded KTX2 or PNG base level. Textured area with no UV area is infinitely coarse. Children are the nearest content descendants, through content-less tiles and single-child copies of the same payloads. Sums are taken once per payload; one preorder pass checks every tile. Both are charged to `totalPayloadElements`.

### Descendant recovery

A declared recovery root lets a coarse tile stand in for omitted descendants. It uses an explicit REPLACE ownership root. Its GLB sets `asset.extras.rustyTiles.descendantRecovery: 1`, and the root tile's `extras.recovery` holds exactly `version`, `root`, `encodedDepth`, `frontier` and `omitted`.

- `version` is 1. Ownership keys hold `first`, `end`, `depth`, `leafBegin` and `leafEnd`. Each frontier entry holds a `key` and its decoded `triangles` count.
- There are one to eight capped inputs, at most 65,536 input triangles, and one to 256 skipped ownership cells.
- Source and leaf intervals must form complete, disjoint covers. Internal ownership depths are at most 30; frontier leaves may reach depth 31.
- The actual refinement children, after resolving document links, must be exactly that frontier, with G no greater than the recovered root's.
- An explicit root or frontier tile must carry its own valid ownership label; an external document link cannot supply it. Only the generated root cell of an implicit header inherits that header's ownership.
- Malformed or partial labels are distinct from absent metadata and are rejected when recovery reads them.

Validation counts terminal payload triangles and leaves, compares the intervals, verifies ownership identities and rebuilds the skipped ownership tree. Border rules use the ownership parents for exact and mixed-depth borders; drop and G checks use the actual refinement.

| Failure | Cause |
| --- | --- |
| `recoveryRelation` | Missing or inconsistent declarations |
| `recoveryCoverage` | Gaps or overlaps in coverage |
| `recoveryBounds` | Cap, input or declaration-size limits |
| `recoveryOwnership` | Ownership defects |
| `recoveryError` | An error below the frontier larger than allowed (or `geometricErrorOrder`) |

Empty cells below content are still invalid: declared omitted nodes must be removed from the hierarchy. Validation checks the declared ownership against delivered terminal coverage; it cannot rebuild an unpublished hierarchy or verify distance to unpublished source geometry.

Recovery work is charged to `totalPayloadElements`, before each operation; a budget refusal never accepts partial coverage. Rank strings use canonical unsigned decimal (at most 20 digits). A declaration is limited to 64 KiB of raw key and string bytes plus value nodes and eight nested levels, before it is serialized; serialization also stops at 64 KiB. Coverage endpoints use indexed terminal prefixes, so a declared frontier does not cause repeated subtree scans.

## Limits

Every key of the report's `limits` is in one of three classes. S, the report's `storedBytes`, is the sum of the stored sizes of every central-directory member, the 3TZ index included. It is fixed when the catalog is admitted, before any budgeted work, and every byte of it is hashed, so a budget cannot grow from what traversal finds.

**Per-item format limits** are fixed:

| Limit | Value |
| --- | --- |
| `jsonBytes`, `jsonDepth` | 8 MiB JSON, depth 64 |
| `documentItems` | 131,072 JSON value nodes per document |
| `memberBytes` | 128 MiB per member |
| `accessorElements` | 4 million accessor elements per payload |
| `documentDecodedBytes` | 128 MiB decoded views per document, also covering an implicit subtree path's members, external buffers and availability tables |
| `hierarchyDepth` | 128 levels |
| `liveDocumentBytes` | 256 MiB of tileset documents held at once on the external-tileset path (raw bytes plus 128 bytes per JSON value node) |
| `containmentSpanBits`, `containmentNodeDepth` | See [decoded containment](#decoded-containment) |

**Volume caps** (`sourceArchiveBytes`, `archiveEntries`, `centralDirectoryBytes`, `archiveStoredBytes`) are opt-in and `null` unless set. Without them the archive is bounded by the file and the format: the 3TZ member ceiling (Unsupported), at least 46 directory bytes per entry, a directory inside the file, and disjoint member extents inside the file.

**Amplification budgets** grow with S:

| Budget | Default |
| --- | --- |
| `hierarchyVisits` | max(262,144, S/8) |
| `references` | max(1,048,576, S/8) |
| `totalBytesRead` | max(2 GiB, 4·S): member-hash and payload/document reads, including repeated reads and index reads |
| `totalPayloadElements` | 16 million + 224·S |
| `containmentWorkUnits`, `containmentBytesRead`, `containmentDecodedBytes` | See [decoded containment](#decoded-containment) |

`totalPayloadElements` adds its floor rather than taking a maximum. Every dense output measured uses up to about 144 units per stored byte, so it validates with a 1.5× margin. Archives up to 2,097,159 stored bytes get exactly the floors. `limitDerivation` gives each budget's floor, its ratio `perStoredByte`, its `source` (`floor`, `scaled` or `explicit`) and, for `totalPayloadElements`, `"additive": true`. `storedBytes`, `limits` and `limitDerivation` depend only on S and the request's overrides.

The default ceilings are sized for default vector output at its 100,000-tile `--maxTiles` default, which uses about 2.4 to 2.5 entries, 1.2 to 1.3 hierarchy visits, one `tileset.json` value node and 920 `vector-build.json` bytes per tile. Content volume is bounded separately: budgets grow with stored bytes, so dense content is not refused for its size alone. Explicit vector archives from 0.3 keep their whole hierarchy in one `tileset.json` of about 50.5 value nodes per tile, and reach the 131,072 value-node limit at about 2,595 tiles.

### Changing limits

`--maxVolume KEY=N` (repeatable; `--max-volume`), Python `max_volume={KEY: N}` and Rust `ValidationRequest::max_volume(LimitKey, N)` set a volume cap or replace a budget. Values may be below the defaults, for a stricter profile for untrusted input. Per-item limits cannot be changed. An unknown or per-item key, a value that is not a decimal integer below 2^64, or a repeated key is refused as `usage` (Python `InvalidRequestError`) before the input is opened.

A refused charge against a cap or budget is `resource_limit`, with a message ending in `(limit KEY=VALUE, SOURCE)`, where `SOURCE` is `floor`, `scaled`, `explicit` or `cap`. The CLI error carries `"limit": {"limit", "value", "observed", "derivation"}`; Python has the same dict as `ResourceLimitError.limit`; Rust has `ValidationFailure::exceeded()`.

Separately bounded ZIP envelope, tail and header reads are outside `totalBytesRead`. These limits are engineering bounds, not a promise about total process memory. Validation memory does not grow with tiles, references or payloads: the traversal keeps only its current path, per-payload facts spill to scratch above their [planned](#memory-workers-and-disk) threshold, and shared external-tileset summaries are capped at 64 MiB.

## Errors, cancellation and progress

| Rust `ValidationFailure` | CLI `error.code` | Exit | Python |
| --- | --- | --- | --- |
| `InvalidInput` | `invalid_input` | 3 | `DataError` |
| `Unsupported` | `unsupported` | 2 | `UnsupportedError` |
| `ResourceLimit` | `resource_limit` | 3 | `ResourceLimitError` |
| `Io` | `io` | 1 | `TilesIOError` |
| `Scratch` | `io`, or `environment` for memory-backed-only scratch | 1, or 4 | `TilesIOError`, or `EnvironmentError` |
| `InvalidRequest` | `usage` | 2 | `InvalidRequestError` |
| `Cancelled` | `cancelled` | 1 | `CancelledError` |
| `Observer` | `observer_failure` | 1 | `ObserverError`; a callback's own exception, including `KeyboardInterrupt`, propagates unchanged |
| `InvalidState` | `invalid_state` | 1 | `TilesError` |

`InvalidState` reports an internal error, such as an exact-arithmetic capacity fault in decoded containment. It is never a verdict about the archive.

Cancellation, through `inspect_with`'s control, a Python callback or Ctrl-C, is checked before each hashed member, traversal task, replayed external-tileset event, implicit tile and containment stream entry, and in the HLOD delivery stage at every charged row and every 10,000 SQLite operations. A Python callback (and so a Python Ctrl-C) runs at every event.

Progress events use these phases; the CLI prints them with `--progress json`.

| Phase | When |
| --- | --- |
| `validate.hash` | Hashing members |
| `validate.traverse` | Walking the hierarchy |
| `validate.delivery` | HLOD archives: the delivery stage's passes; `done` counts the passes started of `total` (20) |
| `validate.delivery.leaves`, `.edges`, `.coarse` | HLOD archives: the leaf, edge-face and coarse geometry replays; `done` counts placements replayed of `total`, with an event every 1,024 |
| `validate.stream` | Streaming decoded content for containment, only when it runs |
| `validate.rows` | Ordering payload rows |

The phase list is open and conditional. `done` never decreases within a phase, but an event may repeat: replaying a shared external tileset re-sends the current `validate.traverse` event every 1,024 replayed events as a heartbeat, and the delivery stage re-sends its current event every 65,536 charged rows and every 10,240,000 SQLite operations. The delivery phases alternate: each replay phase sits between two `validate.delivery` events.

## Design documents

These documents give the full rules and measurements behind this page:

- [payload integrity contract](architecture/c1-payload-integrity-contract.md). Its JSON value-node, member and decoded-view limits (65,536 nodes, 64 MiB) are superseded by the values under [limits](#limits); the meanings are unchanged.
- [decoded containment contract](architecture/c2-decoded-containment-contract.md), including the containment work bound;
- [validator limits](architecture/validator-limits-amendment.md) and [traversal scale](architecture/validator-traversal-scale-amendment.md), including recovery and distance-test work calibration;
- [archive reader contract](architecture/archive-read-foundation-contract.md) and [catalog at scale](architecture/c1-archive-catalog-scale-amendment.md);
- [external tileset reuse](architecture/c1-external-tileset-memo-amendment.md);
- [implicit read contract](architecture/i1-implicit-read-contract.md);
- [legacy content](architecture/c1-legacy-content-unsupported-amendment.md), [KTX2](architecture/c1-ktx2-basisu-amendment.md) and [meshopt OCTAHEDRAL](architecture/c1-meshopt-octahedral-amendment.md) rules;
- [HLOD design](architecture/hlod-040-decision.md), including the seam-owner hash rule and last-resort stages.

## Appendix: distance certificate arithmetic

This section gives the numerical argument behind the stage-3 [coarse drops](#coarse-drops) certificate.

**Pruning.** Each query first rejects faces whose outward-expanded bounding box (expanded by the same reach, rounded outwards) cannot contain the query box. For stage-3 region scans, the same box gives a safe pruning test: if the whole centroid interval box lies strictly outside it on any axis, the exact centroid is farther than the reach R from that face. That face cannot certify all three region corners, because distance to a closed convex triangle is convex: three corner distances within R would put their centroid within R too. The scan charges this box test once and skips that face's corner queries. It never skips on a rounded midpoint alone, an uncertain upper distance or a partly overlapping box. Pruning keeps geometric decisions, subdivision and the original face order, and removes only queries that cannot certify the region or mark its centre as near. Required work only decreases: an accepted archive cannot become a refusal, while one refused only for effort by an earlier release may now pass. For example, one region with a separated centroid and a certifying centroid needs three units; the unpruned scan needed four.

**Static filter.** A binary64 static filter decides most comparisons; an interval routine handles the rest. The filter never relies on a projection being the true closest point. Edge parameters and two nested interpolation parameters are finite binary64 values clamped to [0,1], so their exact interpolation chooses a point q on the closed retained triangle even if the rounded projection is inaccurate.

Let x be the midpoint of the query's interval box, M the maximum absolute coordinate of x and the three retained vertices, and B an upward-rounded L1 radius of the box. With e = 2^-52 the filter uses `E = up(256*e*M + B)`. It is enabled only for `1e-100 <= M <= 1e100` and finite B.

- Upper bound: the computed witness distance d gives `up(d+E)`.
- Lower bound: the computed vector n = x−q is treated as an exact vector, not a certified normal. For every nonzero n, `min_v n·(x-v)/|n| <= distance(x,triangle)`, with the minimum over the three vertices. The filter evaluates that expression as s and returns `max(0, down(s-E))`. If the computed length of n is below 1e-100, the lower bound is zero.

A strict lower-bound rejection or an upper-bound acceptance therefore implies the exact distance predicate. Neither bound increases the published reach. Both the centroid reach and its radius-reduced reach must be decided; otherwise the interval routine runs. Region centroids, radii and midpoint subdivisions use outward intervals, and squared comparisons enclose rounding.

**Error bound.** The bound assumes round-to-nearest binary64 without fast-math reassociation, and uses the product accumulation bound `gamma_k = k*u/(1-k*u)` with `u=2^-53` and exact binary64 chosen parameters.

- One interpolation costs at most 5eM per coordinate; the nested composition costs under 16eM. The final subtraction adds at most 2eM, so vector error is below 32eM in Euclidean norm. Five norm operations plus the square root add under 12eM. Total distance error is below 64eM.
- For the support expression, treating n as exact (with `|n_i|/|n| <= 1` and `|x_i-v_i| <= 2M`): numerator error is below 24eM|n|; the norm's relative error (below 4e) and the division add under 20eM. Total error is below 64eM. Taking the minimum keeps this uniform bound.
- No triangle determinant is divided by, and no well-conditioned projection is assumed. The enabled exponent range prevents overflow in the interpolation, distance norm and support enclosure. It does not protect the fourth-degree determinant products that propose an interior projection; those may overflow or give NaN, but the proposal carries no distance guarantee, and only finite clamped parameters are used, so an inaccurate proposal still selects an exact convex witness.
- Underflow error in the distance norm is below 1e-161; in the support calculation, whose denominator is at least 1e-100, below 1e-220. Both are smaller than eM even at the minimum M.
- The factor 256 leaves more than a factor of three beyond these bounds, including evaluation of E. Next-up and next-down enclose the final additions, subtractions and squaring. Adding B extends each bound from x to every point in its box by 1-Lipschitz continuity.

Outside this domain, or in an undecided band, the interval routine runs, and uncertified cases are refused. Each query increments a fast counter; interval fallbacks also increment a slow counter.

**Cache and determinism.** In the certificate, the last answering face's decoded coordinates are reused only at that face's original position in the current cell's candidate list, with no extra probe or reordering. With or without the cache, each query sees identical coordinates, candidate order and distance results, so early exits, charges, subdivisions and total effort are identical; the cache cannot change a verdict or a refusal at the effort limit. The sampled stage 0 to 2 checks try the cached face first instead. Their charged work depends on the cache, but the cache is a pure function of the archive, since queries run in a fixed order, so verdicts are deterministic and every distance test is charged. Every executed unit of work is charged, and no weights, reach or numerical tolerances depend on the cache.
