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

The include closure of the build below, byte for byte, plus the licence files: the upstream
`encoder/`, `transcoder/` and `zstd/` files that the compiled sources include under this
configuration (`g++ -MM`), `LICENSE`, `NOTICE`, `LICENSES/` and `zstd/LICENSE`. Not copied,
because the build never compiles or includes them:

- upstream tests, tools, WebGL/WASM, Python, OpenCL kernels and documentation;
- `encoder/basisu_wasm_api.{cpp,h}`, `encoder/basisu_wasm_api_common.h`,
  `encoder/basisu_wasm_transcoder_api.{cpp,h}` (upstream's WASM/C API; `src/basisu_ktx2/native/rt_basisu_shim.cpp`
  replaces it);
- `zstd/zstddeclib.c` (decoder-only amalgamation; `zstd/zstd.c` is the full library);
- the SSE kernels and their headers (`basisu_kernels_{declares,imp}.h`,
  `basisu_bc15_spmd_kernels.inl`, `cppspmd_*.h`; compiled only with `BASISU_SUPPORT_SSE=1`),
  `basisu_ocl_kernels.h` (only with OpenCL) and `basisu_transcoder_tables_pvrtc2_alpha_33.inc`
  (under `#if 0`).

The copy is 98 files, 10,852,575 bytes. It is the one Basis Universal version in the tree; an
update replaces it. `scripts/check_tree_budget.py` gives `vendor/` its own 12,000,000-byte
budget and allows exactly `transcoder/basisu_transcoder.cpp` and `zstd/zstd.c` over the 1 MB
per-file limit. Zstandard stays vendored rather than coming from the `zstd-sys` crate: upstream
includes `../zstd/zstd.h` by relative path, so the header would stay here while the library
came from another zstd version.

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
- C++17, `-O2` / `/O2`, `NDEBUG` (so no `assert()` is compiled), no debug info;
- `BASISU_SUPPORT_SSE=0` (no CPU-dependent kernels), `BASISU_SUPPORT_OPENCL=0`,
  `BASISD_SUPPORT_KTX2=1`, `BASISD_SUPPORT_KTX2_ZSTD=1`, `BASISU_DISABLE_ANDROID_ASTC_DECOMP=0`;
- `-fno-strict-aliasing` (required by upstream), `-ffp-contract=off` (no FMA contraction),
  `-fvisibility=hidden`, `-fexceptions` on GCC/Clang; `/bigobj /fp:precise /utf-8 /EHsc` on MSVC
  (the `cc` crate does not enable C++ exception handling on MSVC by itself);
- every vendored translation unit and the shim force-include
  `src/basisu_ktx2/native/rt_basisu_silence.h`, which redirects the C stdio output family to the
  capturing sinks in `src/basisu_ktx2/native/rt_basisu_sink.cpp`. Library text aimed at stdout
  or stderr is kept in a bounded per-thread buffer and attached to the returned error;
- three translation units also force-include a guard header that turns a process-ending call
  into a C++ exception (next section).

### Abort paths

The shim (`rt_basisu_shim.cpp`) validates every argument before calling the library (edges
1..=16384, at most 8192 x 8192 texels, ETC1S quality 1..=100, UASTC without RDO, effort 0..=10,
known format and flags, non-null pointers) and catches every C++ exception, so nothing unwinds
into Rust. Upstream's process-ending calls, and what happens to each:

| Upstream call | When | Now |
|---|---|---|
| `abort()` in `basisu_frontend.cpp` `handle_verify_failure()` and `basisu_backend.cpp` `verify()` | an ETC1S internal consistency check fails | `rt_basisu_invariant.h` redirects it to a throwing function: status 9, `InvalidState` with the check's message |
| `std::terminate()` in `basisu::container_abort()` (`basisu_containers_impl.h`) | a library container's allocation fails, a size would overflow, or a span check fails | `rt_basisu_container.h` redirects it to a throwing function: status 10, `InvalidState` |
| `assert()` | debug builds only | compiled out (`NDEBUG`) |
| `std::bad_alloc`, other C++ exceptions | `new` fails, library throws | caught: status 3 or 4 |
| `exit(0)` in `basisu_transcoder.cpp` | table generators under `BASISD_WRITE_NEW_*` and `#if 0` | not compiled |
| `exit()` in `jpgd.cpp` | JPEG decode failure on WASI only | not compiled; elsewhere `longjmp`, and only for JPEG files, which the shim never passes |

CI's `KTX2 encoder link` job checks on the Unix targets that the three guarded objects no longer
reference `abort` or `std::terminate` and do reference the replacements. The tests run hostile
and degenerate requests in a child process (`hostile_inputs_return_errors_without_ending_the_process`,
with an abort control) and call the real `container_abort()` and the invariant replacement
through the shim.

Remaining paths, accepted:

- **`std::terminate()` when an exception meets a `noexcept` frame.** The two redirected calls
  happen in ordinary encoder functions and container operations; none is reached from a
  destructor or `noexcept` function on the encode or decode path, but this is not proven for
  every container use. A failure there still ends the process, as it did before.
- **`std::terminate()` in `job_pool`'s constructor and destructor (`basisu_enc.cpp`).** Only for
  failed or still-joinable worker threads; the shim never enables threading, so no pool has
  workers.
- **Exhausted memory outside the library's containers.** Rust's own allocation failure aborts
  the process, and an operating system may end a process that overcommits before any
  allocation fails. The texel limit bounds one encode at about 3.4 GiB (about 55 bytes per
  texel for ETC1S, 40 for UASTC, as measured for the design); the caller budgets for it.
- **Stack overflow and undefined behaviour in library code**, e.g. a library bug on unusual
  input. Neither can be caught. The encoder runs on the calling thread and is tested on Rust's
  default 2 MiB thread stack; the hostile tests cover degenerate shapes and extreme values.
- After a redirected failure, memory the library held in raw allocations may leak; the call
  that failed returns an error and the process continues.

### Verification and quality floors

`encode_ktx2` verifies every result: an independent container parse, every level decoded by the
vendored transcoder, an error floor against the 4x4 block-mean reconstruction, and a detail
floor: the decoded luma/alpha detail within 4x4 blocks must correlate with the input's (ETC1S at
least 1/5, UASTC at least 1/2) when the image has at least 1,024 texels and mean within-block
variance of at least 64. On pure noise the error floor alone accepts an unrelated image (1.1-1.3x
the block-mean error against 0.7-1.2x for a good ETC1S decode); the correlation separates them
(good ETC1S at least 0.38, unrelated at most 0.03).

The test `profile_class_floors_hold_with_an_independent_decoder` decodes every level with the
pure-Rust `basisu` 0.1.0 transcoder (a port of the reference transcoder; a test-only dependency),
requires it to match the vendored transcoder byte for byte, and checks the provisional profile
table of the design on 256x256 fixtures. Measured (identical on all five targets) and floors:

| Profile | Class | Mode | PSNR (floor) | Max error (floor) | Normal mean / p99 (floor) |
|---|---|---|---|---|---|
| fast | colour, sRGB | ETC1S q40 | 32.66 dB (22) | 42 (56) | |
| fast, balanced | data-R | ETC1S q90 | 38.29 dB (30) | 61 (80) | |
| fast, balanced, quality | data-RGB | UASTC + Zstd | 41.34 dB (30) | 20 (32) | |
| fast, balanced, quality | normal | UASTC + Zstd | 38.12 dB (30) | 86 (112) | 1.01 / 10.44 deg (3 / 12) |
| balanced | colour, sRGB | ETC1S q90 | 34.71 dB (22) | 24 (32) | |
| quality | colour, sRGB | UASTC + Zstd | 41.55 dB (34) | 15 (24) | |
| quality | data-R | UASTC + Zstd | 50.96 dB (30) | 4 (8) | |

PSNR and angle floors are the design's coarse class floors; maximum-error floors are about 1.3x
the measurement. `lossless` is PNG and does not use this encoder. A normal fixture with a hard
height step instead of a ramp measured p99 14.1 degrees, above the design's 12-degree floor:
hard creases in normal maps exceed it.

### Licences in distributed artefacts

`scripts/third_party_notices.py` lists the notices (`LICENSE`, `NOTICE`, `LICENSES/*.txt`,
`zstd/LICENSE`) and fails if the vendored tree gains another licence file. The release archives
carry them under `third_party/basis_universal/`, the wheels under `.dist-info/licenses/`
(`pyproject.toml` `license-files`), the crate under `vendor/`, and the container image under
`/usr/share/doc/rusty-tiles/third_party/`. Release, wheel acceptance and CI's Rust job (crate,
also held under crates.io's 10 MB limit) check them byte for byte; the packaged-runtime test
checks the image.
