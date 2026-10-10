# Fresh selected-reference verification, 2026-10-10

Five exact selected references were independently fetched and read in this lane.
[fetch-receipt.json](fetch-receipt.json) binds repository, full revision, raw URL,
HTTP result, retrieval time, byte length and SHA256; the original response bytes
are retained beside it. This addendum makes no new CLI executions. The fourteen
original execution records, driver and raw bundle remain unchanged, as checked
by [original-artifact-guard.json](original-artifact-guard.json).

| Selected reference | Revision | Fresh raw SHA256 |
| --- | --- | --- |
| KHR_mesh_quantization | `70b5e4101725950fb8f3f94dd72c37e758ce5daf` | `f59a411d42b52ff7099b98d57fcb2bf4cb2835e7f543d55ab2b08e40c9bec6dc` |
| EXT_meshopt_compression | `5ccac1cb0f85197c4f78c3ddff0b12cdccd79da3` | `602608cb4e3402f189e8c4b235be680093ea07c68aff87ab5d974992283555b0` |
| Modern b3dm | `4d781014b52294759834018a931223b98ac1ce47` | `539762ac799aa8b685035647682b54a40b728904c51ee0685838e29a1ce04f3e` |
| KHR_mesh_primitive_restart draft | `9811e8407d4533500cfc6b10e3bc408345035a6f` | `44276f9cd67e65efe0c8ccc1cae23f2a845c1e34bf190935eb0c0333a2536707` |
| meshoptimizer published golden test | `3d8e9b8a2a2b9a5becfc6fbc512307b04207ab5d` | `70488d804844802804fc8d4eb302779ea392f746f97df654cc2fabe0ca72c6ac` |

The selected [quantization text](KHR_mesh_quantization.md), lines 16–103, marks
the extension ratified and written against glTF 2.0. It requires
extensionsRequired for additional representations and supplies separate mesh
and morph attribute tables, four-byte attribute alignment, and normalization
equations with signed minimum clamping. Quantized declared extrema are stored
component values. This verifies the source of layout/representation rules; it
does not establish dequantization transforms, morph rendering, tangent lighting,
unit-normal accuracy or the current implementation's complete table coverage.

The selected [meshopt text](EXT_meshopt_compression.md), lines 8–141 and the
Mode 0 header description, marks the extension ratified against glTF 2.0. It
defines integer compressed-range/count/stride fields, parent decoded length
equal to count times stride, matching parent stride when supplied, mode-specific
stride/count restrictions and NONE as the omitted filter default. C1's NONE-only
filter boundary remains an explicit finite Unsupported choice. URI-less
placeholders require sufficiently large logical lengths; GLB index 1 or higher
is a recommendation, and fallback marking is optional. Marked fallback buffers
may have only meshopt-view parent references and cannot source compressed bytes.
Absent actual fallback data requires the extension. These logical byte/layout
rules do not prove temporary allocator alignment, FFI safety, capacity admission
or arbitrary codec correctness.

The selected [b3dm text](modern-b3dm.adoc), lines 47–153, defines little-endian
modern version 1, a 28-byte header, total eight-byte alignment, embedded GLB
eight-byte start alignment and terminal zero padding outside its header length.
It delegates table padding to table specifications and requires BATCH_LENGTH
with uint32 meaning. This format is deprecated in 3D Tiles 1.1; reading its
selected modern framing does not grant legacy-header or feature/batch semantic
acceptance. The additionally fetched pinned feature-table text requires trailing
space JSON padding to its eight-byte endpoint and defines aligned binary fields.
Batch-table semantic/padding rules were not separately reread in this addendum.

The selected [restart draft](KHR_mesh_primitive_restart-draft.md), lines 14–73,
explicitly remains Draft. Both root extension declarations are required. Maximum
U8/U16/U32 values mean restart only for LINE_LOOP, LINE_STRIP, TRIANGLE_STRIP and
TRIANGLE_FAN. C1's indexed LINE_STRIP-only profile and minimum two ordinary
vertices per segment are engineering choices: the draft does not declare empty
or singleton segments invalid. Preserve their Unsupported classification and
ordinary core index rules. No ratification, other draw-mode or full draft claim
follows from this read.

The selected [golden test](meshopt_decoder.test.js), lines 10–27, independently
publishes one 85-byte compressed stream and its 48-byte expected output for four
12-byte elements. [golden-byte-comparison.json](golden-byte-comparison.json)
records extraction without importing or running the upstream decoder or any
production generator. The retained C1 archive's compressed bytes match; its old
generator's two literal sequences match the independently published arrays.
The selected compressed header is `0xa0`, consistent with the selected meshopt
Mode 0 appendix. Additional upstream tests for other encodings/filters were not
used to expand C1's profile. Literal identity is fresh provenance, not a new
codec execution or a full compressed-bitstream proof.

## Independent b3dm integer decision

The selected pinned [b3dm feature-table schema](b3dm.featureTable.schema.json)
uses JSON Schema 2020-12 and references the pinned
[globalPropertyInteger definition](featureTable.schema.json). Numeric values
have integer type with minimum zero. The separate
[JSON Schema 2020-12 validation primary, section 6.1.1](https://json-schema.org/draft/2020-12/json-schema-validation#section-6.1.1)
defines integer by zero fractional part; section 4.2 does not add machine numeric
precision bounds. Thus numeric BATCH_LENGTH `0.0` or `0e0` has integer meaning
independently of glTF's JSON rule. The uint32 format range remains applicable.
Fractional values remain forbidden even when binary floating point rounds them
to an integer. This primary was opened through the web tool; a separate direct
raw download returned HTTP 403, so no complete raw HTML snapshot is claimed.

Current `src/validate/payload.rs` uses `Value::as_u64` for BATCH_LENGTH. This is
an additional static correction seam for the same exact integer interpretation;
no new b3dm execution defect is asserted here. The schema also admits a binary
body-offset form, which was noted without claiming support or adjudicating
feature semantics outside the existing finite C1 profile.

These bounded reads settle selected-reference provenance and the rules listed
above for preimplementation A9 adjudication. They do not certify the retained
owner, the full extensions, complete C1 acceptance, A2 or a release. Final source
and behavior still require independent acceptance under the existing profile.
