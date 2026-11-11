/*
 * rusty-tiles: force-included, after rt_basisu_silence.h, ahead of the two
 * vendored translation units whose internal consistency checks call abort():
 * encoder/basisu_frontend.cpp (handle_verify_failure) and
 * encoder/basisu_backend.cpp (verify). See build.rs.
 *
 * Every header those two files include is included here first, so the rest of
 * each file is the only code that sees the macro below. It turns their
 * abort() calls into rt_basisu_invariant_failure(), which throws a C++
 * exception that the shim (rt_basisu_shim.cpp) catches and reports as a
 * typed status. The check's own message has already gone through the
 * capturing sinks by then, so it is attached to the returned error.
 *
 * Both checks are plain functions called from ordinary encoder code (no
 * destructor or noexcept frame), and the encoder runs single-threaded, so the
 * exception unwinds to the shim on the calling thread.
 */
#ifndef RT_BASISU_INVARIANT_H
#define RT_BASISU_INVARIANT_H

#include <cstdlib>
#include <stdlib.h>
#include <unordered_map>
#include <unordered_set>

#include "transcoder/basisu.h"
#include "encoder/basisu_backend.h"
#include "encoder/basisu_frontend.h"
#include "encoder/basisu_opencl.h"

/* C++ linkage on purpose: MSVC's /EHsc assumes extern "C" functions never
 * throw. Defined in rt_basisu_shim.cpp. */
[[noreturn]] void rt_basisu_invariant_failure();

namespace std {
using ::rt_basisu_invariant_failure;
} // namespace std

#undef abort
#define abort rt_basisu_invariant_failure

#endif /* RT_BASISU_INVARIANT_H */
