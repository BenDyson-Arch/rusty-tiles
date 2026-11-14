// rusty-tiles: the C ABI used by src/basisu_ktx2/ffi.rs.
//
// Compiled with rt_basisu_silence.h force-included, like the vendored sources,
// because the Basis Universal headers it includes contain inline code that
// writes to stdout.
//
// Every entry point validates its arguments before calling the library and
// catches all C++ exceptions, returning a status code, so no exception crosses
// the C ABI into Rust. That includes the library's internal consistency checks
// in the guarded translation units (rt_basisu_invariant.h), which throw
// InvariantFailure from rt_basisu_invariant_failure() below instead of calling
// abort(). The abort paths that remain are listed in vendor/README.md.
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <memory>
#include <mutex>
#include <new>

#include "encoder/basisu_comp.h"
#include "transcoder/basisu_transcoder.h"

extern "C" void rt_basisu_clear_diagnostics(void);

// Default visibility for the two entry points only, so a release link can be
// told to keep them (CI's encoder link-size measurement) on every platform.
#if defined(_WIN32)
#define RT_BASISU_ENTRY
#else
#define RT_BASISU_ENTRY __attribute__((visibility("default")))
#endif

namespace {

enum Status : std::int32_t {
    kOk = 0,
    kInvalidArgument = 1,
    kEncodeFailed = 2,
    kOutOfMemory = 3,
    kException = 4,
    kContainerRejected = 5,
    kStartFailed = 6,
    kLevelMismatch = 7,
    kTranscodeFailed = 8,
    kInvariantFailed = 9,
    kContainerFailed = 10,
};

// Limits shared with src/basisu_ktx2.rs (MAX_EDGE, MAX_TEXELS, MAX_EFFORT); the
// Rust side checks them first, the shim again so the C ABI never passes an
// out-of-domain request to the library. 16384 is also the library's own edge
// limit. The texel limit bounds the encoder's memory (about 55 bytes per texel
// for ETC1S) far below any size_t overflow in its containers.
constexpr std::uint32_t kMaxEdge = 16384;
constexpr std::uint64_t kMaxTexels = 8192ull * 8192ull;
constexpr std::int32_t kMaxEffort = 10;

// Thrown by rt_basisu_invariant_failure() and rt_basisu_container_failure();
// caught only by the entry points.
struct InvariantFailure {};
struct ContainerFailure {};

enum Format : std::uint32_t {
    kEtc1s = 0,
    kUastcLdr4x4 = 1,
};

constexpr std::uint32_t kFlagSrgb = 1u << 0;
constexpr std::uint32_t kFlagZstd = 1u << 1;
constexpr std::uint32_t kFlagMipmaps = 1u << 2;
// UASTC only: the library's own pack flags, passed through unchanged
// (UASTC_FASTEST_ETC1_HINTS in src/basisu_ktx2.rs). They change only the ETC1
// hint bits stored in each block, used when a block is transcoded to ETC1;
// the UASTC texels are unchanged.
constexpr std::uint32_t kFlagUastcFastestEtc1Hints = 0x180;
static_assert(kFlagUastcFastestEtc1Hints ==
                  (basisu::cPackUASTCETC1FastestHints | basisu::cPackUASTCETC1DisableFlipAndIndividual),
              "UASTC ETC1 hint flags must be the library's pack flag values");
static_assert((kFlagUastcFastestEtc1Hints & (kFlagSrgb | kFlagZstd | kFlagMipmaps)) == 0,
              "UASTC ETC1 hint flags must not overlap the shim's own flags");
constexpr std::uint32_t kKnownFlags = kFlagSrgb | kFlagZstd | kFlagMipmaps | kFlagUastcFastestEtc1Hints;

std::once_flag g_init_once;

void init_once() {
    std::call_once(g_init_once, [] {
        // No OpenCL; also initializes the transcoder tables.
        basisu::basisu_encoder_init(false, false);
        basisu::enable_debug_printf(false);
    });
}

// Records an exception's message with the call's diagnostics. This file is
// compiled with rt_basisu_silence.h, so stderr goes to the capturing sink.
void note_exception(const char *what) {
    std::fprintf(stderr, "rusty-tiles shim: C++ exception: %s\n", what != nullptr ? what : "(no message)");
}

// basisu::basis_compress2() for the 2D LDR requests above, written out because
// that API masks the UASTC pack flags to the effort level and then overrides
// them from the effort, so it cannot express the ETC1 hint flags. Each
// parameter is set as basis_compress_internal() (basisu_comp.cpp) sets it for
// cFlagKTX2 [| cFlagSRGB] [| cFlagKTX2UASTCSuperCompression] with no RDO, no
// threads, no statistics and no generated mips; with no extra pack flags the
// output is byte-identical to basis_compress2() (tests::direct_compress_matches_compress2).
// `extra_pack_flags` (UASTC only) are OR'd into the pack flags after the effort
// sets the pack level. Returns malloc'ed bytes (released by basis_free_data) or
// nullptr.
void *compress(basist::basis_tex_format mode, const basisu::vector<basisu::image> &source_images,
               std::uint32_t flags, std::int32_t quality, std::int32_t effort,
               std::uint32_t extra_pack_flags, std::size_t *size) {
    *size = 0;
    const bool srgb = (flags & kFlagSrgb) != 0;
    basisu::job_pool pool(1);
    basisu::basis_compressor_params params;
    params.set_format_mode(mode);
    params.m_pJob_pool = &pool;
    params.m_y_flip = false;
    params.m_debug = false;
    params.m_debug_images = false;
    params.m_tex_type = basist::basis_texture_type::cBASISTexType2D;
    // Level 0 is the image; any further levels are its caller-authored mips.
    params.m_source_images.resize(1);
    params.m_source_images[0] = source_images[0];
    if (source_images.size() > 1) {
        params.m_source_mipmap_images.resize(1);
        params.m_source_mipmap_images[0].resize(source_images.size() - 1);
        for (std::uint32_t i = 1; i < source_images.size(); ++i) {
            params.m_source_mipmap_images[0][i - 1] = source_images[i];
        }
    }
    params.m_multithreading = false;
    params.m_use_opencl = false;
    params.m_write_output_basis_or_ktx2_files = false;
    params.m_perceptual = srgb;
    params.m_ktx2_and_basis_srgb_transfer_function = srgb;
    params.m_mip_srgb = srgb;
    params.set_ldr_srgb_channel_weights(srgb);
    params.m_mip_gen = false;
    params.m_mip_wrapping = false;
    if (mode == basist::basis_tex_format::cUASTC_LDR_4x4) {
        // The pack level comes from the effort below.
        params.m_pack_uastc_ldr_4x4_flags = 0;
    } else {
        // Replaced from the quality below.
        params.m_quality_level = 1;
    }
    params.m_create_ktx2_file = true;
    if ((flags & kFlagZstd) != 0 && params.m_uastc) {
        params.m_ktx2_uastc_supercompression = basist::KTX2_SS_ZSTANDARD;
    }
    params.m_compute_stats = false;
    params.m_print_stats = false;
    params.m_status_output = false;
    params.m_astc_hdr_6x6_options.m_rec2020_bt2100_color_gamut = false;
    params.m_validate_output_data = false;
    // Quality 100 disables UASTC RDO; the effort sets the UASTC pack level or
    // the ETC1S compression level.
    if (!params.set_format_mode_and_quality_effort(mode, quality, effort, false)) {
        return nullptr;
    }
    if (mode == basist::basis_tex_format::cUASTC_LDR_4x4) {
        params.m_pack_uastc_ldr_4x4_flags |= extra_pack_flags;
    }
    basisu::basis_compressor compressor;
    if (!compressor.init(params) || compressor.process() != basisu::basis_compressor::cECSuccess) {
        return nullptr;
    }
    const basisu::uint8_vec &file = compressor.get_output_ktx2_file();
    void *data = std::malloc(file.size());
    if (data == nullptr) {
        throw std::bad_alloc();
    }
    std::memcpy(data, file.get_ptr(), file.size());
    *size = file.size();
    return data;
}

} // namespace

// The replacement for abort() in the encoder's consistency checks
// (rt_basisu_invariant.h).
[[noreturn]] void rt_basisu_invariant_failure() {
    throw InvariantFailure{};
}

// The replacement for std::terminate() in basisu::container_abort()
// (rt_basisu_container.h).
[[noreturn]] void rt_basisu_container_failure() {
    throw ContainerFailure{};
}

extern "C" {

// Encodes one RGBA8 image (tightly packed rows) to KTX2. On success the
// returned buffer must be released with rt_basisu_free().
//
// The call is single-threaded: the encoder's job pool has one thread and
// multithreading is off. Status, statistics and debug output flags are
// never set, and no statistics are computed.
struct RtBasisuMipLevel {
    const std::uint8_t *rgba;
    std::size_t length;
    std::uint32_t width;
    std::uint32_t height;
};

RT_BASISU_ENTRY std::int32_t rt_basisu_encode_mips(const RtBasisuMipLevel *levels, std::size_t level_count,
                              std::uint32_t format, std::int32_t quality, std::int32_t effort,
                              std::uint32_t flags, std::uint8_t **out, std::size_t *out_len) {
    rt_basisu_clear_diagnostics();
    if (out == nullptr || out_len == nullptr) {
        return kInvalidArgument;
    }
    *out = nullptr;
    *out_len = 0;
    if (levels == nullptr || level_count == 0 || level_count > 15 ||
        (flags & ~kKnownFlags) != 0 || (flags & kFlagMipmaps) != 0) {
        return kInvalidArgument;
    }
    const std::uint32_t width = levels[0].width, height = levels[0].height;
    if (width == 0 || height == 0 || width > kMaxEdge || height > kMaxEdge ||
        static_cast<std::uint64_t>(width) * height > kMaxTexels) {
        return kInvalidArgument;
    }
    std::size_t full_levels = 1;
    for (std::uint32_t edge = width > height ? width : height; edge > 1; edge >>= 1) {
        ++full_levels;
    }
    if (level_count != 1 && level_count != full_levels) {
        return kInvalidArgument;
    }
    std::uint64_t total_texels = 0;
    std::uint32_t mip_width = width, mip_height = height;
    for (std::size_t i = 0; i < level_count; ++i) {
        const RtBasisuMipLevel &level = levels[i];
        if (level.rgba == nullptr || level.width != mip_width || level.height != mip_height ||
            level.length != static_cast<std::uint64_t>(mip_width) * mip_height * 4) {
            return kInvalidArgument;
        }
        total_texels += static_cast<std::uint64_t>(mip_width) * mip_height;
        mip_width = mip_width > 1 ? mip_width >> 1 : 1;
        mip_height = mip_height > 1 ? mip_height >> 1 : 1;
    }
    if (total_texels > 2 * kMaxTexels) return kInvalidArgument;
    if (effort < 0 || effort > kMaxEffort) {
        return kInvalidArgument;
    }
    basist::basis_tex_format mode;
    switch (format) {
    case kEtc1s:
        if (quality < 1 || quality > 100 || (flags & kFlagUastcFastestEtc1Hints) != 0) {
            return kInvalidArgument;
        }
        mode = basist::basis_tex_format::cETC1S;
        break;
    case kUastcLdr4x4:
        // 100 is the only UASTC setting used: no RDO.
        if (quality != 100) {
            return kInvalidArgument;
        }
        mode = basist::basis_tex_format::cUASTC_LDR_4x4;
        break;
    default:
        return kInvalidArgument;
    }
    try {
        init_once();
        // Never set cFlagGenMips*: the encoder never generates a level. One
        // image deliberately remains base-only. Caller pixels are copied
        // before encode.
        basisu::vector<basisu::image> source_images;
        source_images.resize(static_cast<std::uint32_t>(level_count));
        for (std::size_t i = 0; i < level_count; ++i) {
            source_images[static_cast<std::uint32_t>(i)].init(
                levels[i].rgba, levels[i].width, levels[i].height, 4);
        }
        void *data = compress(mode, source_images, flags, quality, effort,
                              flags & kFlagUastcFastestEtc1Hints, out_len);
        if (data == nullptr) {
            return kEncodeFailed;
        }
        if (*out_len == 0) {
            basisu::basis_free_data(data);
            return kEncodeFailed;
        }
        *out = static_cast<std::uint8_t *>(data);
        return kOk;
    } catch (const InvariantFailure &) {
        return kInvariantFailed;
    } catch (const ContainerFailure &) {
        return kContainerFailed;
    } catch (const std::bad_alloc &) {
        return kOutOfMemory;
    } catch (const std::exception &error) {
        note_exception(error.what());
        return kException;
    } catch (...) {
        note_exception(nullptr);
        return kException;
    }
}

// Test oracle for the pure-Rust encoder (src/basisu_ktx2/pure/shadow_tests.rs), not used in
// production: rt_basisu_encode_mips's compress() with any `extra_pack_flags` OR'd into the
// UASTC pack flags, not only the ETC1 hint flags production uses. The caller validates the
// request as for rt_basisu_encode_mips. Compiled only with feature pure-rust-basis (build.rs
// defines RT_BASISU_ENCODE_ORACLE), like rt_basisu_selftest_compress2 below, so the default
// build has neither entry.
#if defined(RT_BASISU_ENCODE_ORACLE)
RT_BASISU_ENTRY std::int32_t rt_basisu_encode_mips_oracle(const RtBasisuMipLevel *levels,
                                                         std::size_t level_count, std::uint32_t format,
                                                         std::int32_t quality, std::int32_t effort,
                                                         std::uint32_t flags, std::uint32_t extra_pack_flags,
                                                         std::uint8_t **out, std::size_t *out_len) {
    rt_basisu_clear_diagnostics();
    if (out == nullptr || out_len == nullptr || levels == nullptr || level_count == 0 ||
        level_count > 15 || (flags & ~(kFlagSrgb | kFlagZstd)) != 0 || effort < 0 || effort > kMaxEffort ||
        (format != kEtc1s && format != kUastcLdr4x4) || (format == kEtc1s && extra_pack_flags != 0)) {
        return kInvalidArgument;
    }
    *out = nullptr;
    *out_len = 0;
    try {
        init_once();
        const basist::basis_tex_format mode =
            format == kEtc1s ? basist::basis_tex_format::cETC1S : basist::basis_tex_format::cUASTC_LDR_4x4;
        basisu::vector<basisu::image> source_images;
        source_images.resize(static_cast<std::uint32_t>(level_count));
        for (std::size_t i = 0; i < level_count; ++i) {
            source_images[static_cast<std::uint32_t>(i)].init(
                levels[i].rgba, levels[i].width, levels[i].height, 4);
        }
        void *data = compress(mode, source_images, flags, quality, effort, extra_pack_flags, out_len);
        if (data == nullptr || *out_len == 0) {
            basisu::basis_free_data(data);
            *out_len = 0;
            return kEncodeFailed;
        }
        *out = static_cast<std::uint8_t *>(data);
        return kOk;
    } catch (const InvariantFailure &) {
        return kInvariantFailed;
    } catch (const ContainerFailure &) {
        return kContainerFailed;
    } catch (const std::bad_alloc &) {
        return kOutOfMemory;
    } catch (const std::exception &error) {
        note_exception(error.what());
        return kException;
    } catch (...) {
        note_exception(nullptr);
        return kException;
    }
}
#endif // RT_BASISU_ENCODE_ORACLE

// For the tests: `which == 0` throws through the encoder checks' abort()
// replacement; anything else calls the library's real container_abort(). Both
// must come back as their status rather than end the process.
RT_BASISU_ENTRY std::int32_t rt_basisu_selftest_failure(std::int32_t which) {
    rt_basisu_clear_diagnostics();
    try {
        if (which == 0) {
            std::fprintf(stderr, "rusty-tiles shim: self-test invariant failure\n");
            rt_basisu_invariant_failure();
        }
        // The real container_abort(), compiled with rt_basisu_container.h.
        basisu::container_abort("rusty-tiles shim: self-test container failure %d\n", which);
    } catch (const InvariantFailure &) {
        return kInvariantFailed;
    } catch (const ContainerFailure &) {
        return kContainerFailed;
    } catch (...) {
        return kException;
    }
}

// Test reference, not used in production: encodes with basisu::basis_compress2(),
// the library's simple API the shim used before it set the parameters itself, so
// the tests can show compress() reproduces it byte for byte. The tests pass only
// requests rt_basisu_encode_mips() accepts, without the hint flags. Compiled only
// with RT_BASISU_ENCODE_ORACLE, like rt_basisu_encode_mips_oracle above.
#if defined(RT_BASISU_ENCODE_ORACLE)
RT_BASISU_ENTRY std::int32_t rt_basisu_selftest_compress2(const RtBasisuMipLevel *levels, std::size_t level_count,
                                                          std::uint32_t format, std::int32_t quality,
                                                          std::int32_t effort, std::uint32_t flags,
                                                          std::uint8_t **out, std::size_t *out_len) {
    rt_basisu_clear_diagnostics();
    if (out == nullptr || out_len == nullptr || levels == nullptr || level_count == 0 || level_count > 15 ||
        (flags & ~(kFlagSrgb | kFlagZstd)) != 0 || format > kUastcLdr4x4) {
        return kInvalidArgument;
    }
    *out = nullptr;
    *out_len = 0;
    try {
        init_once();
        std::uint32_t basis_flags = basisu::cFlagKTX2;
        if (flags & kFlagSrgb) {
            basis_flags |= basisu::cFlagSRGB;
        }
        if (flags & kFlagZstd) {
            basis_flags |= basisu::cFlagKTX2UASTCSuperCompression;
        }
        basisu::vector<basisu::image> source_images;
        source_images.resize(static_cast<std::uint32_t>(level_count));
        for (std::size_t i = 0; i < level_count; ++i) {
            if (levels[i].rgba == nullptr ||
                levels[i].length != static_cast<std::uint64_t>(levels[i].width) * levels[i].height * 4) {
                return kInvalidArgument;
            }
            source_images[static_cast<std::uint32_t>(i)].init(
                levels[i].rgba, levels[i].width, levels[i].height, 4);
        }
        const basist::basis_tex_format mode = format == kEtc1s ? basist::basis_tex_format::cETC1S
                                                               : basist::basis_tex_format::cUASTC_LDR_4x4;
        std::size_t size = 0;
        void *data = basisu::basis_compress2(mode, source_images, basis_flags, quality, effort, &size, nullptr);
        if (data == nullptr || size == 0) {
            basisu::basis_free_data(data);
            return kEncodeFailed;
        }
        *out = static_cast<std::uint8_t *>(data);
        *out_len = size;
        return kOk;
    } catch (...) {
        return kException;
    }
}
#endif // RT_BASISU_ENCODE_ORACLE

// Base-only legacy ABI. Requests to generate mips are rejected by the common entry.
RT_BASISU_ENTRY std::int32_t rt_basisu_encode(const std::uint8_t *rgba, std::uint32_t width, std::uint32_t height,
                              std::uint32_t format, std::int32_t quality, std::int32_t effort,
                              std::uint32_t flags, std::uint8_t **out, std::size_t *out_len) {
    if (width > kMaxEdge || height > kMaxEdge) {
        rt_basisu_clear_diagnostics();
        if (out != nullptr) *out = nullptr;
        if (out_len != nullptr) *out_len = 0;
        return kInvalidArgument;
    }
    const std::size_t bytes = static_cast<std::size_t>(width) * height * 4;
    const RtBasisuMipLevel level{rgba, bytes, width, height};
    return rt_basisu_encode_mips(&level, 1, format, quality, effort, flags, out, out_len);
}

void rt_basisu_free(std::uint8_t *data) {
    if (data != nullptr) {
        basisu::basis_free_data(data);
    }
}

// Decodes one mip level of a KTX2 file to tightly packed RGBA8 with the
// vendored Basis Universal transcoder. `out_len` must equal the level's
// width * height * 4; the decoded level dimensions are returned.
RT_BASISU_ENTRY std::int32_t rt_basisu_decode_level_rgba(const std::uint8_t *ktx2, std::size_t ktx2_len,
                                         std::uint32_t level, std::uint8_t *out,
                                         std::size_t out_len, std::uint32_t *out_width,
                                         std::uint32_t *out_height) {
    rt_basisu_clear_diagnostics();
    if (ktx2 == nullptr || out == nullptr || out_width == nullptr || out_height == nullptr ||
        ktx2_len > UINT32_MAX) {
        return kInvalidArgument;
    }
    *out_width = 0;
    *out_height = 0;
    try {
        init_once();
        std::unique_ptr<basist::ktx2_transcoder> transcoder(new basist::ktx2_transcoder());
        if (!transcoder->init(ktx2, static_cast<std::uint32_t>(ktx2_len))) {
            return kContainerRejected;
        }
        if (!transcoder->start_transcoding()) {
            return kStartFailed;
        }
        basist::ktx2_image_level_info info;
        if (!transcoder->get_image_level_info(info, level, 0, 0)) {
            return kLevelMismatch;
        }
        *out_width = info.m_orig_width;
        *out_height = info.m_orig_height;
        const std::size_t pixels =
            static_cast<std::size_t>(info.m_orig_width) * static_cast<std::size_t>(info.m_orig_height);
        if (pixels == 0 || pixels > UINT32_MAX || out_len / 4 != pixels || out_len % 4 != 0) {
            return kLevelMismatch;
        }
        if (!transcoder->transcode_image_level(level, 0, 0, out, static_cast<std::uint32_t>(pixels),
                                               basist::transcoder_texture_format::cTFRGBA32, 0,
                                               info.m_orig_width, info.m_orig_height)) {
            return kTranscodeFailed;
        }
        return kOk;
    } catch (const InvariantFailure &) {
        return kInvariantFailed;
    } catch (const ContainerFailure &) {
        return kContainerFailed;
    } catch (const std::bad_alloc &) {
        return kOutOfMemory;
    } catch (const std::exception &error) {
        note_exception(error.what());
        return kException;
    } catch (...) {
        note_exception(nullptr);
        return kException;
    }
}

} // extern "C"
