# C1 meshopt stream-header controls (2026-10-12)

Additive lane for C1 rule H-1 (A-C1-3 in the ML0 amendment, draft PR #168). Nothing
outside this directory is modified. The C1 contract is not edited because
`driver.py` pins its bytes.

## Rule

The pinned `EXT_meshopt_compression` text (KhronosGroup/glTF
`5ccac1cb0f85197c4f78c3ddff0b12cdccd79da3`, SHA-256 `602608cb…55b0`, Appendix A) gives
exactly one header byte per mode:

| Mode | Allowed | Forbidden, but read by the linked meshoptimizer 0.25 decoder |
| --- | --- | --- |
| ATTRIBUTES | `0xa0` | `0xa1` (vertex codec v1) |
| TRIANGLES | `0xe1` | `0xe0` (index codec v0) |
| INDICES | `0xd1` | `0xd0` (sequence v0) |

Every other first byte is also forbidden. C1 checks the byte after the mode, stride and
count checks and before decoding: `invalid_input`, "meshopt stream header …".

## Controls

`generator.py` uses only the standard library and the retained primary texts (`primary/`
are relative symlinks to `validator_caps_amendment/2026-10-11/stride_controls/primary`,
hash-checked against `pins.json`). It reads the allowed byte per mode from the spec
text, so each case's verdict depends on the spec text alone: admitted if and only if
the first byte equals that byte. The stream bytes come from the pinned upstream test
literals (zeux/meshoptimizer `3d8e9b8a`, `js/meshopt_decoder.test.js`):

| Case | Stream source | Verdict |
| --- | --- | --- |
| `attributes-0xa0` | `decodeVertexBuffer` | admitted, full report |
| `attributes-0xa1` | `decodeVertexBufferV1` (decodes to the same bytes) | invalid_input |
| `attributes-0xa2` | `decodeVertexBuffer`, header substituted | invalid_input |
| `triangles-0xe1` | `decodeIndexBufferV1_More` | admitted, full report |
| `triangles-0xe0` | `decodeIndexBuffer32` (same body and indices) | invalid_input |
| `triangles-0xe2` | `decodeIndexBufferV1_More`, header substituted | invalid_input |
| `indices-0xd1` | `decodeIndexSequence` | admitted, full report |
| `indices-0xd0` | `decodeIndexSequence`, header substituted | invalid_input |
| `indices-0xd2` | `decodeIndexSequence`, header substituted | invalid_input |

`0xa1`, `0xe0` and `0xd0` decode with the linked decoder; the Rust controls in
`src/content_integrity/payload/meshopt_header_controls.rs` prove it. So these three
cases fail only through H-1. With H-1 removed, all three are admitted. With a mutant
that re-admits only `0xe0` for TRIANGLES, `triangles-0xe0` is admitted. The oracle
fails in both cases.

`runner.py` is a byte copy of the C2-WB stride runner. `integrity.json` pins every
generated file and the generator. `tests/payload_meshopt_stride_oracle.py` runs this
lane as `header9` and pins `fixtures/manifest.json`.

## Replay

```sh
python3 -I generator.py   # into a copy of this directory without fixtures/ and integrity.json
```
