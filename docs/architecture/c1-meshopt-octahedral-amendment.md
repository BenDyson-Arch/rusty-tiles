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

R5. Declared accessor bounds on an OCTAHEDRAL-filtered view, for BYTE or SHORT
components, are admitted within one unit of this decoder's output. Appendix B specifies
filter results "to one unit in the last place", and conforming decoders differ there:
meshoptimizer's SIMD paths (including an rsqrt-based 8-bit path) and the scalar pseudocode
can disagree by one unit depending on the host. An exact comparison would make the verdict
depend on the host and would refuse spec-valid files. Every other view, including
meshopt NONE views and unfiltered integer data, is still compared exactly. The tiler
declares no `NORMAL` bounds on filtered views, so its output does not rely on the
tolerance.

R6. Mode is checked before the decoder runs: OCTAHEDRAL on a TRIANGLES or INDICES view is
InvalidInput ("meshopt index modes admit only the NONE filter"), as is any stride other
than 4 or 8 ("needs byteStride 4 or 8"), whatever the stream's content. On a big-endian
host the filter is Unsupported, because the element checks and the linked filter assume
little-endian storage.

C1 still does not inspect metadata or feature semantics (`notInspected`). A compressed
property-table view is decoded, sized and charged like any other view.

## Acceptance

- C1 corpus (`tests/c1_validation_oracle.py`, standard library only; run by
  `tests/c1_validate.rs` and CI).
  - Positive cases:
    - `meshopt_metadata_octahedral` uses the upstream golden NONE POSITION and
      `decodeFilterOct12` NORMAL streams. Its declared NORMAL bounds are the published
      int16 values, and its UINT64/UINT32 property-table columns are raw-group
      ATTRIBUTES streams written from the spec text, plus a raw UINT8 column.
    - `meshopt_octahedral_boundary` has |x| and |y| equal to 1.0.
    - `meshopt_octahedral_bounds_one_unit_{8,16}` declares bounds computed by the
      pseudocode in binary32. These are one unit from meshoptimizer's SIMD output on
      x86-64: 110 against 111, and 32759 against 32758.
  - InvalidInput cases:
    - `meshopt_table_truncated` and `meshopt_table_wrong_count`.
    - `meshopt_octahedral_filter_mismatch`: no valid K.
    - `meshopt_octahedral_x_over_one` and `_y_over_one`: a valid 1.0, but |x| or |y| is
      greater.
    - `meshopt_octahedral_stride` and `_stride16`: valid codes where a stride/4 reading
      would look, so only the stride rule refuses them.
    - `meshopt_octahedral_triangles` and `_indices`: spec-written index streams whose
      values are valid 8-bit codes, so only the mode rule refuses them.
    - `meshopt_octahedral_bounds_beyond_one_unit_{8,16}`.
    - `meshopt_none_bounds_one_unit`: the tolerance does not reach NONE views.
  - Unsupported: `meshopt_quaternion_unsupported`.
  - None of these cases panics.
- The C2 oracle's meshopt decoder (`c2o/meshopt.py`, re-pinned) implements filter 1
  independently from the pseudocode. Its self-test calls `decode_view` for:
  - the upstream `decodeFilterOct8`/`Oct12` outputs, within one unit;
  - the |x| = |y| = 1.0 boundary;
  - the 1.0, |x| and |y| refusals;
  - stride 12;
  - TRIANGLES and INDICES views that carry the filter.

  It reports Contained for the positive cases and Undecided for each stream-level
  hostile case. It does not check declared NORMAL bounds; C1 refuses those first.
- Production and oracle mutants
  ([mesh_archive_size/mutants.py](../../bench/architecture_audit/mesh_archive_size/mutants.py))
  include the independent audit's mutants A–H and the oracle survivors. Each one is
  killed by its named tests; the record is in the evidence store.
