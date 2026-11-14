# Pure-Rust Basis Universal (feature `pure-rust-basis`)

Status: 0.4.2, PR (a). The feature is opt-in and the vendored C++ (Basis Universal v2.50,
zstd 1.5.7; `vendor/README.md`) stays the default. PR (b) makes the Rust path the default
and removes `vendor/basis_universal`, the shim and their build; its gate is digest-identical
tex3 and Walem archives from both builds.

## Contract

With the feature, every KTX2 encode and decode in the crate runs in Rust. The module API in
`src/basisu_ktx2.rs` is unchanged: only `encode_mips_unverified` and `decode_level_rgba`
switch backend, so the request checks, the mandatory post-encode verification, the floors,
`base_dimensions` and every caller stay as they are.

- **Encoding** (`src/basisu_ktx2/pure/encode.rs`): ktx-rs's port of `basis_compressor`, with
  the parameters `basis_compress2` derives from the shim's arguments (`Settings::resolve`):
  ETC1S `m_quality_level = round(255·q/100)` and compression level `round(6·effort/10)`;
  UASTC pack level `round(4·effort/10)`, no RDO; sRGB sets perceptual metrics, the sRGB
  transfer and sRGB mips; one thread, no statistics, no mip generation. Caller levels are
  passed as further source images (ktx-rs has no `m_source_mipmap_images`), which gives
  the same slices in the same order. The `.basis` output is rewritten as the KTX2 file
  `basis_compressor::create_ktx2_file` writes: header, level index, the four fixed DFDs
  with the transfer and UASTC channel fixed up, the `KTXwriter` key ("Basis Universal
  2.50") and, without supercompression, the dummy key that aligns the levels to 16 bytes,
  BasisLZ global data (image descriptors, codebooks, tables) at 8-byte alignment, levels
  smallest first. UASTC levels are compressed one frame per level with zstd-pure-rs's
  `ZSTD_compress` at level 6.
- **Extra UASTC pack flags**: `rust_encode_mips` takes flags OR'd into the pack level
  after the effort sets it, such as the ETC1 hint flags 0x180
  (`cPackUASTCETC1FastestHints | cPackUASTCETC1DisableFlipAndIndividual`). Production
  passes the flags of `Params::etc1_hints` (`UASTC_FASTEST_ETC1_HINTS` for leaf UASTC
  pages, 0 otherwise), the same flags the C++ path passes.
- **Decoding** (`src/basisu_ktx2/pure/decode.rs`): a front end that follows the acceptance
  of `ktx2_transcoder` (`init`, `start_transcoding`, `get_image_level_info`,
  `transcode_image_level` to RGBA32 with no flags), including its key/value parse, then
  ktx-rs's `LowlevelEtc1sTranscoder` and `LowlevelUastcLdr4x4Transcoder`. Zstandard levels
  are decompressed by ruzstd, wrapped with the checks zstd makes and ruzstd leaves to its
  caller: the reserved frame-header bit, Frame_Content_Size and the content checksum.
  Every read is bounds-checked, and the level's dimensions are checked against the
  caller's before anything is allocated. Allocations are bounded by the level: its texels
  and, for Zstandard, its block bytes plus ruzstd's window. A frame's window is limited to
  the level's block bytes rounded up to a power of two (at least 1 KiB), because ruzstd
  buffers up to a whole window before it releases decoded bytes; without that limit a
  few-KiB frame declaring a 2^27-byte window could build 128 MiB for a 16-byte level.
- **Panics**: encode and decode run under `catch_unwind`; a panic becomes `InvalidState`
  (encode) or a refused file (decode), never a process exit. None has been observed.

### Where the Rust front end is stricter

It never accepts a file the vendored transcoder refuses (checked on mutants, below). It
refuses these files, which the vendored transcoder decodes:

| Refusal | Why |
|---|---|
| `vkFormat` other than 0, more than one face or layer | outside the supported domain (one 2D ETC1S or UASTC LDR 4x4 chain); `base_dimensions` already refuses them for decoded views |
| a scheme other than BasisLZ for ETC1S, or BasisLZ for UASTC | the vendored transcoder fails these later; same outcome |
| ETC1S with a `KTXanimData` key | ETC1S video (P-frames) is unsupported |
| ETC1S global data shorter than its 20-byte header | the vendored reader copies the header before checking the length |
| a Zstandard level whose recorded uncompressed size is not its block bytes | bounds the allocation by the level (the vendored transcoder allocates any size below 2 GiB) |
| a Zstandard frame whose declared window exceeds the level's block bytes rounded up to a power of two (at least 1 KiB) | bounds ruzstd's buffer by the level (zstd's one-shot decoder writes straight into the level and accepts windows up to 2^31). zstd sizes the window to the content when it knows the content size, as for every level the encoder writes |
| a corrupt Zstandard frame without a checksum whose literal stream ruzstd finds inconsistent (`DecodedLiteralCountMismatch`, `BitstreamReadMismatch`) but zstd 1.5.7 decodes | ruzstd's literal-stream checks are stricter |

## Dependencies (exact pins)

| Crate | Licence | Use | Audit notes |
|---|---|---|---|
| ktx-rs =4.4.3, `default-features = false` | Apache-2.0 AND BSD-3-Clause AND MIT AND Zlib | encoder, block decoders | Port of KTX-Software and Basis Universal. `unsafe_code = "deny"`; its `basisu` module has no `unsafe`. Published 2026-10-30, one maintainer. Pulls ktx-rs-macros, miniz_oxide 0.9, ruzstd, zstd-pure-rs. |
| zstd-pure-rs =0.2.0 | BSD-3-Clause OR GPL-2.0-only (BSD chosen) | level compression | Port of zstd 1.5.7; byte-identical to it at levels 1 to 19 here. About 500 `unsafe` lines; only its compressor is called, on the encoder's own block bytes, never on untrusted input. Owner's choice over mzstd (5.7 MB smaller binary). |
| ruzstd =0.9.1 | MIT | level decompression | Already in the graph through ktx-rs. Safe decoding API, but not `unsafe`-free: `decoding/ringbuffer.rs` (37 uses) and `decoding/decode_buffer.rs` (2) hold `unsafe` on the decoding hot path (the window ring buffer's copies). Called with a window limit per level (above). Used instead of zstd-pure-rs's decoder, whose hot paths use raw pointers and unchecked indexing, because decoded views and validation read untrusted archives. |

Licence texts: `licenses/THIRD_PARTY_LICENSES-pure-rust-basis.md` (the crates the feature
adds to the CLI's graph; `scripts/third_party_licenses.py`).

## Evidence

The shadow tests (`src/basisu_ktx2/pure/shadow_tests.rs`, CI job "Pure-Rust Basis") run
both paths on a generated corpus of 16 images (photo-like, alpha, noise, scalar
occlusion, ORM, normal map, checker, flat, cutout, 1x1 to 384x256, odd sizes), base-only
and full chains:

- ETC1S: 88 encodes (q50 and q90 sRGB, q90 linear, plus q1, q100 and effort 3), identical;
- UASTC: 248 encodes (efforts 0, 1, 2, 5, 8; sRGB and linear; with and without
  Zstandard), identical;
- UASTC with pack flags 0x180 (150) and 0x40 (30) against a C++ oracle entry point
  (`rt_basisu_encode_mips_oracle`, the production entry's parameter setup with any extra
  pack flags), and the production encode with `Etc1Hints::Fastest` equal on both paths;
- zstd-pure-rs against the vendored zstd at levels 1 to 19 on 36 payloads, identical;
- every level of every file decodes to identical texels with both transcoders;
- 300 mutants per base file (20,000 in a longer run): never accepted by Rust and refused
  by the C++; identical texels whenever both decode.

Paired bakes and the basis_encode stage CPU comparison are in the PR description.
