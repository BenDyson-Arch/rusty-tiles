# Vendored native sources

## Basis Universal v2.50 (`basis_universal/`)

The in-process KTX2 encoder used by `src/basisu_ktx2.rs`. Built by `build.rs` with the `cc`
crate and the target's C/C++ compiler; it has no other system dependency.

| | |
|---|---|
| Upstream | <https://github.com/BinomialLLC/basis_universal> |
| Tag | `v2_50` |
| Commit | `9bebe16726b3a61c8c213eeee3b7cffb462ef34e` (2026-07-22, "Merge pull request #458 from vkedwardli/fix/netbsd") |
| Licence | Apache-2.0 (`basis_universal/LICENSE`, attribution in `basis_universal/NOTICE`) |
| Bundled third-party code | Zstandard, BSD-3-Clause (`basis_universal/zstd/LICENSE`); `qoi.h` and `tinydds.h`, MIT; `tinyexr.h`, BSD-3-Clause; `android_astc_decomp.cpp`, Apache-2.0. Licence texts are in `basis_universal/LICENSES/` and the file headers |

### What was copied

The upstream `encoder/`, `transcoder/` and `zstd/` directories plus `LICENSE`, `NOTICE` and
`LICENSES/`, byte for byte, except files the build never compiles or includes:

- `encoder/basisu_wasm_api.{cpp,h}`, `encoder/basisu_wasm_api_common.h`,
  `encoder/basisu_wasm_transcoder_api.{cpp,h}` (upstream's WASM/C API; `src/basisu_ktx2/native/rt_basisu_shim.cpp`
  replaces it);
- `zstd/zstddeclib.c` (decoder-only amalgamation; `zstd/zstd.c` is the full library).

No vendored file is modified. `basis_universal.sha256` lists every vendored file. To check the
copy against upstream:

```sh
(cd vendor/basis_universal && sha256sum -c ../basis_universal.sha256)
git clone https://github.com/BinomialLLC/basis_universal /tmp/basisu-upstream
git -C /tmp/basisu-upstream checkout 9bebe16726b3a61c8c213eeee3b7cffb462ef34e
(cd /tmp/basisu-upstream && sha256sum -c "$OLDPWD/vendor/basis_universal.sha256")
```

`vendor/.gitattributes` marks the tree `-text` so checkouts keep upstream bytes on every platform.

### Build configuration (`build.rs`)

The same configuration on every target and in every Cargo profile:

- the encoder source list from upstream CMake's `ENCODER_LIB_SRC_LIST`, plus
  `encoder/3rdparty/android_astc_decomp.cpp` (upstream's default) and `zstd/zstd.c`;
- C++17, `-O2` / `/O2`, `NDEBUG`, no debug info;
- `BASISU_SUPPORT_SSE=0` (no CPU-dependent kernels), `BASISU_SUPPORT_OPENCL=0`,
  `BASISD_SUPPORT_KTX2=1`, `BASISD_SUPPORT_KTX2_ZSTD=1`, `BASISU_DISABLE_ANDROID_ASTC_DECOMP=0`;
- `-fno-strict-aliasing` (required by upstream), `-ffp-contract=off` (no FMA contraction),
  `-fvisibility=hidden` on GCC/Clang; `/bigobj /fp:precise /utf-8` on MSVC;
- every vendored translation unit and the shim force-include
  `src/basisu_ktx2/native/rt_basisu_silence.h`, which redirects the C stdio output family to the
  capturing sinks in `src/basisu_ktx2/native/rt_basisu_sink.cpp`. Library text aimed at stdout
  or stderr is kept in a bounded per-thread buffer and attached to the returned error.

### Known limits

- Upstream terminates the process on internal invariant failures (`abort()` in
  `basisu_backend.cpp` and `basisu_frontend.cpp` `verify()`, and `std::terminate()` in
  `container_abort()`, used for impossible container sizes). Their messages are silenced, but these
  paths are not recoverable errors.
- Distributing binaries that link this code requires shipping `basis_universal/NOTICE` and the
  licence texts alongside them. The release packaging does not do this yet.
