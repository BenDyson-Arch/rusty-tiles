/*
 * rusty-tiles: force-included, after rt_basisu_silence.h, ahead of the
 * vendored transcoder/basisu_transcoder.cpp only. See build.rs.
 *
 * That translation unit holds the one definition of basisu::container_abort()
 * (transcoder/basisu_containers_impl.h), which every library container calls
 * when an allocation fails, a size would overflow or a span check fails, and
 * which ends the process with std::terminate(). Every header the file
 * includes, except basisu_astc_helpers.h (its implementation section is
 * enabled by a define in the file itself; it includes only C headers, listed
 * here), is included here first, so the only `terminate` token the macro
 * below rewrites is that call. It becomes rt_basisu_container_failure(),
 * which throws a C++ exception that the shim reports as a typed status;
 * container_abort() has already written its message through the capturing
 * sinks by then.
 */
#ifndef RT_BASISU_CONTAINER_H
#define RT_BASISU_CONTAINER_H

#include <ctype.h>
#include <exception>
#include <fenv.h>
#include <limits.h>
#include <math.h>
#include <stdint.h>
#include <stdlib.h>
#if defined(_MSC_VER)
#include <intrin.h>
#endif

#include "transcoder/basisu_transcoder.h"
#include "transcoder/basisu_astc_hdr_core.h"
#include "transcoder/basisu_xbc7_decoder.h"
#include "zstd/zstd.h"

/* C++ linkage on purpose: MSVC's /EHsc assumes extern "C" functions never
 * throw. Defined in rt_basisu_shim.cpp. */
[[noreturn]] void rt_basisu_container_failure();

namespace std {
using ::rt_basisu_container_failure;
} // namespace std

#undef terminate
#define terminate rt_basisu_container_failure

#endif /* RT_BASISU_CONTAINER_H */
