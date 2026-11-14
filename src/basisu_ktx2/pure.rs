//! Feature `pure-rust-basis`: KTX2 encoding and decoding in pure Rust, byte-identical to the
//! vendored Basis Universal v2.50 path it replaces (contract and audit in
//! docs/architecture/basis-pure-rust.md).
//!
//! - Encoding: ktx-rs 4.4.3's port of `basis_compressor` with the parameters
//!   `basis_compress2` derives, rewritten as the KTX2 file `create_ktx2_file` writes, with
//!   UASTC levels compressed by zstd-pure-rs (a port of zstd 1.5.7) at level 6.
//! - Decoding: a validating KTX2 front end over ktx-rs's ETC1S and UASTC block decoders and
//!   ruzstd for Zstandard levels.
//!
//! While the C++ remains in the tree, `shadow_tests` run both paths and require identical
//! KTX2 bytes and identical decoded texels.

mod decode;
mod encode;
#[cfg(test)]
mod shadow_tests;

use super::container::{
    MODEL_ETC1S, MODEL_UASTC, SUPERCOMPRESSION_BASIS_LZ, SUPERCOMPRESSION_NONE,
    SUPERCOMPRESSION_ZSTD,
};
pub(crate) use decode::decode_level;
pub(crate) use encode::{encode, zstd_compress, Settings};
