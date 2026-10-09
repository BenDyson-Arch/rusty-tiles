# F1b2: bounded local glTF resource binding

This is the next implementation slice of [#121](https://github.com/BenDyson-Arch/rusty-tiles/issues/121), based on develop `641e1b461f3b11621fd528fb05bfba45a6f930d6`. It extends `MeshRequest::local_gltf`, `mesh-local-to-3tz`, and Python `mesh_local_to_3tz`. The [F1b1 geometry, texture, material, partition, bounds and delivery contract](f1b-contract.md) continues to apply, except for the resource admission and report changes specified here. There is no compatibility branch or legacy fallback. This does not close #121 or authorize a release.

## Ownership

The consumer reads the root document once, asks the pure decoder for a validated resource plan, binds every declared dependency, and captures owned bytes before the first observer callback or workspace creation. The decoder receives only a parsed document and borrowed byte slices. It never sees paths or opens files. Leaf encoding and F0 publication retain their existing ownership. Production does not flatten resources into a synthetic GLB or pass paths to a later decoder.

All declared resources, including unused buffers and images, participate in admission and output-overlap validation. Only the selected scene's used image closure is published. Logical buffer and image identities remain separate from physical file identities; aliasing files does not merge glTF indices or material associations.

## Documents and buffers

Admit a regular root JSON glTF document or GLB 2 container. JSON allows ordinary JSON trailing whitespace; GLB retains its strict framing and space padding checks. GLB may omit BIN when its buffers are external. Only buffer zero may use an embedded BIN source. Every other buffer requires a supported local URI. Every external buffer must contain at least its declared byteLength; all views and accessors are bounded by that declared prefix. Embedded BIN retains the at-most-three zero padding rule.

Known unsupported semantics and malformed metadata are rejected from the parsed document before dependency I/O. Every dependency URI is validated before the first dependency is opened. This gives a forbidden URI or excluded extension deterministic admission behavior even when another dependency is missing. Payload-dependent validation follows capture.

Image source is exactly one of URI or bufferView. A bufferView image requires PNG/JPEG mimeType. A URI image may omit mimeType, in which case its signature determines PNG/JPEG; an explicit MIME/signature disagreement is InvalidInput. Full F1b1 image framing, 8-bit channel, decoder, dimension and aggregate pixel checks remain required. Image/accessor overlap is compared within the same logical buffer; equal offsets in different buffers do not overlap.

## Local URI profile

Use the canonical document parent as the base, never the process working directory. Accept relative UTF-8 URI/IRI paths using unreserved ASCII filename characters and raw Unicode; reserved filename data must be percent-encoded. Parse separators and URI structure before decoding. Decode valid percent escapes exactly once within each slash-separated component; decoded bytes must be UTF-8. `%25` represents a literal percent character and `%23` can represent a filename hash. A raw hash denotes a fragment and is excluded.

Apart from recognized URI structural forms below, raw ASCII outside the unreserved filename set is InvalidInput, including spaces, backslashes, controls and unescaped reserved data. Malformed escapes and invalid decoded UTF-8 are also InvalidInput. The portable filename check applies after this syntax check, so raw `<` is InvalidInput and encoded `%3c` is Unsupported. Schemes (including data/file/network), authorities, absolute paths, queries, fragments, encoded slash/backslash, decoded portable filename characters `<>:"|?*`, and trailing filename dot/space are Unsupported. Normalize dot components after decoding; contained parent traversal is allowed, traversal above the base is Unsupported. A trailing slash or terminal decoded dot/parent component cannot name a dependency file and is InvalidInput. URI length is at most 4096 bytes and slash-separated component count at most 128 before normalization.

Inspect every relative component and reject dependency symlinks, including symlinks whose targets are inside the base. Dependencies must be regular files and remain canonically contained in the base. Root symlink-leaf rejection retains the earlier InvalidInput behavior. A dependency that aliases the root document is Unsupported. Missing files and real filesystem permission failures remain Io; observed changes during preparation are InvalidInput. Output paths that overlap or physically alias the document or any dependency are InvalidRequest before staging, callbacks or publication.

## Snapshot and finite limits

Deduplicate captured files by physical identity, retaining one immutable byte allocation per unique dependency. Recheck every declared alias and each captured file's identity, length and modification time after reading and at the final preparation sweep. Keep the identity handles until that sweep, then close them before callbacks. Reading uses bounded chunks with cancellation checkpoints. On Unix, no-follow/non-blocking leaf opening prevents a raced symlink or FIFO leaf from silently changing the read policy. These checks are observations, not a secure-openat confinement claim against hostile directory replacement.

Stable source files during preparation are a precondition. The snapshot does not promise an atomic view of several files at one instant, nor detect an adversary restoring observable metadata. Once preparation finishes, callbacks may change the working directory or delete/rewrite source files without changing the prepared conversion.

Admission ceilings: JSON document 1 MiB; GLB document 32 MiB; each dependency 32 MiB; unique root plus external captured bytes 64 MiB; 32 logical buffers; sum of declared logical buffer bytes 32 MiB; 32 images, textures and samplers. At most 64 URI requests and 65 unique retained source handles, plus a fixed temporary allowance. Copied encoded image bytes remain bounded to 32 MiB even when files are aliased. Existing triangle, accessor, leaf, image edge/pixel and coordinate limits remain in force. These are engineering admission bounds, not hard measured process RSS guarantees. No resource discovery, network fetch, worker pool or external encoder is added.

## Report and proof

MeshReport becomes schema_version 3 with profile `f1b-local-gltf-v1`. `source_bytes` continues to count the root document only. New `external_files` and `external_bytes` count unique captured dependency files and their actual lengths, including unused dependencies; aliases count once. Published `images`, `image_bytes` and `image_pixels` keep their selected-closure meaning. Rust, CLI, Python and conversion.json must agree.

Independent acceptance must prove multi-buffer geometry and buffer-relative overlap, exact PNG/JPEG forwarding, URI interpretation, confinement, alias deduplication, whole-document admission, output safety, callback independence, typed failures and archive closure. Include controls that corrupt resource selection and URI interpretation while retaining plausible output. Measure bounded resource behavior separately from sampled descriptors and process RSS. Preserve existing F1a/F1b1 geometry, image, material and viewer evidence.

Primary semantics: [glTF 2.0.1](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html), registry commit `8e798b02d254cea97659a333cfcb20875b62bdd4`, especially relative resources, buffer byteLength and image sources; [RFC 3986 section 2.4](https://www.rfc-editor.org/rfc/rfc3986.html#section-2.4) for parsing components before a single decoding pass. This profile deliberately excludes URI forms and filesystem cases beyond the evidenced local subset.
