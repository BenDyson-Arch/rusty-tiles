### Added

- Opt-in cargo feature `pure-rust-basis` encodes and decodes KTX2/Basis Universal
  textures in pure Rust (ktx-rs 4.4.3, zstd-pure-rs 0.2.0, ruzstd 0.9.1) instead
  of the vendored C++ encoder. Output is byte-identical to the default build; the
  default build still uses the C++. Licences of the added crates are in
  `licenses/THIRD_PARTY_LICENSES-pure-rust-basis.md`.
