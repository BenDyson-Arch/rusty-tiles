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
constexpr std::uint32_t kKnownFlags = kFlagSrgb | kFlagZstd | kFlagMipmaps;

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
    if (width == 0 || height == 0 || width > kMaxEdge || height > kMaxEdge ||
        static_cast<std::uint64_t>(width) * height > kMaxTexels) {
        return kInvalidArgument;
    }
    if (effort < 0 || effort > kMaxEffort) {
        return kInvalidArgument;
    }
    basist::basis_tex_format mode;
    switch (format) {
    case kEtc1s:
        if (quality < 1 || quality > 100) {
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
