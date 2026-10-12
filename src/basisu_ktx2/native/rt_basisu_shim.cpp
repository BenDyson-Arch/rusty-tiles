// rusty-tiles: the C ABI used by src/basisu_ktx2/ffi.rs.
//
// Compiled with rt_basisu_silence.h force-included, like the vendored sources,
// because the Basis Universal headers it includes contain inline code that
// writes to stdout.
//
// Every entry point catches all C++ exceptions and returns a status code, so no
// exception crosses the C ABI into Rust.
#include <cstddef>
#include <cstdint>
#include <cstring>
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
};

enum Format : std::uint32_t {
    kEtc1s = 0,
    kUastcLdr4x4 = 1,
};

constexpr std::uint32_t kFlagSrgb = 1u << 0;
constexpr std::uint32_t kFlagZstd = 1u << 1;
constexpr std::uint32_t kFlagMipmaps = 1u << 2;
constexpr std::uint32_t kKnownFlags = kFlagSrgb | kFlagZstd | kFlagMipmaps;

std::once_flag g_init_once;

void init_once() {
    std::call_once(g_init_once, [] {
        // No OpenCL; also initializes the transcoder tables.
        basisu::basisu_encoder_init(false, false);
        basisu::enable_debug_printf(false);
    });
}

} // namespace

extern "C" {

// Encodes one RGBA8 image (tightly packed rows) to KTX2. On success the
// returned buffer must be released with rt_basisu_free().
//
// The call is single-threaded: cFlagThreaded is never set, so the encoder's
// job pool has no worker threads. Status, statistics and debug output flags are
// never set, and no statistics are computed.
RT_BASISU_ENTRY std::int32_t rt_basisu_encode(const std::uint8_t *rgba, std::uint32_t width, std::uint32_t height,
                              std::uint32_t format, std::int32_t quality, std::int32_t effort,
                              std::uint32_t flags, std::uint8_t **out, std::size_t *out_len) {
    rt_basisu_clear_diagnostics();
    if (out == nullptr || out_len == nullptr) {
        return kInvalidArgument;
    }
    *out = nullptr;
    *out_len = 0;
    if (rgba == nullptr || (flags & ~kKnownFlags) != 0) {
        return kInvalidArgument;
    }
    basist::basis_tex_format mode;
    switch (format) {
    case kEtc1s:
        mode = basist::basis_tex_format::cETC1S;
        break;
    case kUastcLdr4x4:
        mode = basist::basis_tex_format::cUASTC_LDR_4x4;
        break;
    default:
        return kInvalidArgument;
    }
    try {
        init_once();
        std::uint32_t basis_flags = basisu::cFlagKTX2;
        if (flags & kFlagSrgb) {
            basis_flags |= basisu::cFlagSRGB;
        }
        if (flags & kFlagZstd) {
            basis_flags |= basisu::cFlagKTX2UASTCSuperCompression;
        }
        if (flags & kFlagMipmaps) {
            basis_flags |= basisu::cFlagGenMipsClamp;
        }
        std::size_t size = 0;
        void *data = basisu::basis_compress2(mode, rgba, width, height, width, basis_flags, quality,
                                             effort, &size, nullptr);
        if (data == nullptr) {
            return kEncodeFailed;
        }
        if (size == 0) {
            basisu::basis_free_data(data);
            return kEncodeFailed;
        }
        *out = static_cast<std::uint8_t *>(data);
        *out_len = size;
        return kOk;
    } catch (const std::bad_alloc &) {
        return kOutOfMemory;
    } catch (...) {
        return kException;
    }
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
    } catch (const std::bad_alloc &) {
        return kOutOfMemory;
    } catch (...) {
        return kException;
    }
}

} // extern "C"
