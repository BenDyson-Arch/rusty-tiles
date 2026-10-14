# C1 amendment: EXT_meshopt_compression OCTAHEDRAL filter

**Status: proposed with its implementation (2026-10-14); in effect when the pull request
merges after a nonauthor review.** It amends
[c1-payload-integrity-contract.md](c1-payload-integrity-contract.md), which stays
byte-identical because the C1 production-control driver pins it. Where the texts disagree
about meshopt filters, this note governs. Other amendments are unchanged.

## Why

The large-mesh tiler now stores leaf `NORMAL` in its lossy profiles (`fast`, `balanced`,
`quality`) as 12-bit octahedral codes behind the OCTAHEDRAL filter. It also
meshopt-compresses the 4- and 8-byte property-table columns of every leaf and proxy, with
the NONE filter. Before this amendment, C1 refused every filter other than NONE as
Unsupported, so `validate` would have refused the default mesh output.

## Authorities

| Text | Pin | SHA-256 |
| --- | --- | --- |
| EXT_meshopt_compression README | KhronosGroup/glTF `521a91d491f02bb7db7d445b35f591d09d2a64ce` | `602608cb…55b0`, the same text C1 and C2 already pin |
| KHR_mesh_quantization README | same commit | `f59a411d…c6dc` |
| EXT_structural_metadata README | CesiumGS/glTF `3d-tiles-next` `7182f40bd2bb5c090188efc4f66f2404cba266f0` | `2f5579b8…b566` |
| meshopt_decoder.test.js (golden streams) | zeux/meshoptimizer `3d8e9b8a2a2b9a5becfc6fbc512307b04207ab5d` | already retained in the C1 primary controls |

Copies are kept in `~/.cache/rusty-tiles-evidence/mesh-size-20261014/spec/`.

What the texts allow:

- Compressed property tables. "The extension works on a bufferView level and as such is
  agnostic of how the data is used" (EXT_meshopt_compression, Overview). "The parent
  `bufferView` properties define a layout which can hold the data decompressed from the
  extension object." EXT_structural_metadata stores each property array in "a glTF
  buffer view" and constrains only the view's `byteOffset` alignment. The tiler aligns
  the parent views to 8 bytes in the fallback layout, so the decoded bytes are the same
  tightly packed column.
- Octahedral normals. Appendix B, filter 1, decodes to "8-bit or 16-bit normalized
  integers"; KHR_mesh_quantization lists `NORMAL` VEC3 as *byte normalized* or *short
  normalized*. The tiler writes SHORT normalized VEC3 with `byteStride` 8; the fourth
  component is padding.

## Rules

R1. `filter` NONE and OCTAHEDRAL are decoded. QUATERNION and EXPONENTIAL stay
Unsupported ("meshopt filters other than NONE and OCTAHEDRAL"), whatever the stream
holds.

R2. OCTAHEDRAL requires mode ATTRIBUTES and `byteStride` 4 or 8. Otherwise the view is
InvalidInput. This is the extension's validity rule.

R3. Each decoded element is a valid K-bit code before the filter runs. The third
component equals `(1 << (K - 1)) - 1` for some `2 <= K <= N`, where N is 8 or 16 bits,
and `|x|` and `|y|` are at most that value. Anything else is InvalidInput ("meshopt
OCTAHEDRAL element E is not a valid K-bit octahedral code"). The text leaves decoding
of such values unspecified, so C1 refuses them instead of guessing.

R4. The filter runs in the linked meshoptimizer decoder after the existing header,
mode, stride, count and allocation checks. Peak memory and byte charges are unchanged:
the filter works in place.

R5. Declared accessor bounds on a filtered view are compared exactly with this
decoder's output, as for every view. The text specifies filter output only to one unit
in the last place, so a producer that declares `min`/`max` for a filtered attribute can
disagree with C1 by one unit. The tiler declares no `NORMAL` bounds. This limit is
recorded here and is not relaxed.

C1 still does not inspect metadata or feature semantics (`notInspected`). A compressed
property-table view is decoded, sized and charged like any other view.

## Acceptance

- C1 corpus (`tests/c1_validation_oracle.py`, standard library only; run by
  `tests/c1_validate.rs` and CI). `meshopt_metadata_octahedral` is positive. It uses the
  upstream golden NONE POSITION stream and the golden `decodeFilterOct12` NORMAL stream.
  Its declared NORMAL bounds are the published decoded int16 values. It has UINT64 and
  UINT32 property-table columns written as raw-group ATTRIBUTES streams from the spec
  text, plus a raw UINT8 column. The hostile cases are:
  - `meshopt_table_truncated`, `meshopt_table_wrong_count`,
    `meshopt_octahedral_filter_mismatch` and `meshopt_octahedral_stride`: each
    InvalidInput.
  - `meshopt_quaternion_unsupported`: Unsupported.

  None of them panics.
- The C2 oracle's meshopt decoder (`c2o/meshopt.py`, re-pinned) implements filter 1
  independently from the pseudocode. Its self-test reproduces the upstream
  `decodeFilterOct8`/`Oct12` outputs within one unit and refuses an invalid "1.0". It
  reports Contained for the positive case and Undecided for each hostile case.
- Production mutants
  ([mesh_archive_size/mutants.py](../../bench/architecture_audit/mesh_archive_size/mutants.py)):
  skipping the filter, skipping the K-bit check, skipping table decoding in the LOD
  reader, misreading UINT64 columns, writing raw columns and swapping octahedral inputs.
  The named tests catch each one.
