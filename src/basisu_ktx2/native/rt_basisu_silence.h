/*
 * rusty-tiles: force-included ahead of every vendored Basis Universal and
 * Zstandard translation unit and the rusty-tiles shim (see build.rs).
 *
 * The vendored library writes status, debug and error text with the C stdio
 * output family, including unconditional error_printf() calls. The library
 * contract of rusty-tiles is that library code never writes to the process
 * stdout or stderr. Rather than patching every call site in the vendored
 * sources, every call to these functions in those translation units is
 * redirected at preprocessing time to rt_basisu_* sinks
 * (rt_basisu_sink.cpp). Writes aimed at stdout or stderr are captured into a
 * bounded per-thread diagnostic buffer; writes to any other FILE pass through
 * unchanged.
 *
 * The real stdio headers are included first so that their declarations are
 * not rewritten. The macros are function-like, so `using ::printf;` and
 * attribute uses such as format(printf, 1, 2) are unaffected.
 */
#ifndef RT_BASISU_SILENCE_H
#define RT_BASISU_SILENCE_H

/* zstd.c defines _GNU_SOURCE ahead of its own libc includes to get glibc's
 * qsort_r. This header now includes libc first, so make the same definition
 * first (g++ already defines it for C++). */
#if defined(__linux__) && !defined(__ANDROID__) && !defined(_GNU_SOURCE)
#define _GNU_SOURCE
#endif

#include <stdarg.h>
#include <stddef.h>
#include <stdio.h>

#ifdef __cplusplus
#include <cstdarg>
#include <cstdio>
extern "C" {
#endif

int rt_basisu_printf(const char *format, ...);
int rt_basisu_fprintf(FILE *stream, const char *format, ...);
int rt_basisu_vprintf(const char *format, va_list args);
int rt_basisu_vfprintf(FILE *stream, const char *format, va_list args);
int rt_basisu_puts(const char *text);
int rt_basisu_fputs(const char *text, FILE *stream);
int rt_basisu_putchar(int ch);
int rt_basisu_fputc(int ch, FILE *stream);
size_t rt_basisu_fwrite(const void *data, size_t size, size_t count, FILE *stream);
void rt_basisu_perror(const char *text);

#ifdef __cplusplus
}
/* Vendored code also spells these std::printf(...). */
namespace std {
using ::rt_basisu_fprintf;
using ::rt_basisu_fputc;
using ::rt_basisu_fputs;
using ::rt_basisu_fwrite;
using ::rt_basisu_perror;
using ::rt_basisu_printf;
using ::rt_basisu_putchar;
using ::rt_basisu_puts;
using ::rt_basisu_vfprintf;
using ::rt_basisu_vprintf;
} // namespace std
#endif

#undef printf
#undef fprintf
#undef vprintf
#undef vfprintf
#undef puts
#undef fputs
#undef putchar
#undef putc
#undef fputc
#undef fwrite
#undef perror

#define printf(...) rt_basisu_printf(__VA_ARGS__)
#define fprintf(...) rt_basisu_fprintf(__VA_ARGS__)
#define vprintf(...) rt_basisu_vprintf(__VA_ARGS__)
#define vfprintf(...) rt_basisu_vfprintf(__VA_ARGS__)
#define puts(...) rt_basisu_puts(__VA_ARGS__)
#define fputs(...) rt_basisu_fputs(__VA_ARGS__)
#define putchar(...) rt_basisu_putchar(__VA_ARGS__)
#define putc(...) rt_basisu_fputc(__VA_ARGS__)
#define fputc(...) rt_basisu_fputc(__VA_ARGS__)
#define fwrite(...) rt_basisu_fwrite(__VA_ARGS__)
#define perror(...) rt_basisu_perror(__VA_ARGS__)

#endif /* RT_BASISU_SILENCE_H */
