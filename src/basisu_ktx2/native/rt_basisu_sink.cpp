// rusty-tiles: stdio sinks for the vendored Basis Universal library.
//
// This translation unit is compiled WITHOUT rt_basisu_silence.h so that it can
// reach the real stdio functions for pass-through writes to ordinary files.
// Text aimed at stdout or stderr never reaches those streams: it is appended
// to a bounded per-thread buffer that the Rust wrapper reads after a failed
// call and attaches to the returned error.
#include <atomic>
#include <cstdarg>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>

namespace {

constexpr std::size_t kDiagnosticCap = 4096;
constexpr std::size_t kFormatCap = 1024;

thread_local std::string t_diagnostics;
std::atomic<std::uint64_t> g_sink_calls{0};

bool is_console(std::FILE *stream) { return stream == stdout || stream == stderr; }

void capture(const char *text, std::size_t length) {
    g_sink_calls.fetch_add(1, std::memory_order_relaxed);
    try {
        t_diagnostics.append(text, length);
        if (t_diagnostics.size() > kDiagnosticCap) {
            t_diagnostics.erase(0, t_diagnostics.size() - kDiagnosticCap);
        }
    } catch (...) {
        // Diagnostics are best effort; never let allocation failure escape.
    }
}

int capture_format(const char *format, va_list args) {
    char buffer[kFormatCap];
    int written = std::vsnprintf(buffer, sizeof(buffer), format, args);
    if (written < 0) {
        return written;
    }
    std::size_t kept = static_cast<std::size_t>(written) < sizeof(buffer) - 1
                           ? static_cast<std::size_t>(written)
                           : sizeof(buffer) - 1;
    capture(buffer, kept);
    return written;
}

} // namespace

extern "C" {

int rt_basisu_vfprintf(std::FILE *stream, const char *format, va_list args) {
    if (!is_console(stream)) {
        return std::vfprintf(stream, format, args);
    }
    return capture_format(format, args);
}

int rt_basisu_vprintf(const char *format, va_list args) { return capture_format(format, args); }

int rt_basisu_printf(const char *format, ...) {
    va_list args;
    va_start(args, format);
    int result = capture_format(format, args);
    va_end(args);
    return result;
}

int rt_basisu_fprintf(std::FILE *stream, const char *format, ...) {
    va_list args;
    va_start(args, format);
    int result = rt_basisu_vfprintf(stream, format, args);
    va_end(args);
    return result;
}

int rt_basisu_puts(const char *text) {
    capture(text, std::strlen(text));
    capture("\n", 1);
    return 1;
}

int rt_basisu_fputs(const char *text, std::FILE *stream) {
    if (!is_console(stream)) {
        return std::fputs(text, stream);
    }
    capture(text, std::strlen(text));
    return 1;
}

int rt_basisu_putchar(int ch) {
    char c = static_cast<char>(ch);
    capture(&c, 1);
    return static_cast<unsigned char>(c);
}

int rt_basisu_fputc(int ch, std::FILE *stream) {
    if (!is_console(stream)) {
        return std::fputc(ch, stream);
    }
    return rt_basisu_putchar(ch);
}

std::size_t rt_basisu_fwrite(const void *data, std::size_t size, std::size_t count, std::FILE *stream) {
    if (!is_console(stream)) {
        return std::fwrite(data, size, count, stream);
    }
    capture(static_cast<const char *>(data), size * count);
    return count;
}

void rt_basisu_perror(const char *text) {
    if (text != nullptr) {
        capture(text, std::strlen(text));
    }
}

// Clears this thread's captured diagnostics.
void rt_basisu_clear_diagnostics(void) { t_diagnostics.clear(); }

// Copies up to `capacity` bytes of this thread's captured diagnostics into
// `buffer` and returns the number of bytes available.
std::size_t rt_basisu_diagnostics(char *buffer, std::size_t capacity) {
    std::size_t length = t_diagnostics.size();
    if (buffer != nullptr && capacity != 0) {
        std::memcpy(buffer, t_diagnostics.data(), length < capacity ? length : capacity);
    }
    return length;
}

// Process-wide number of redirected console writes (test sensitivity control).
std::uint64_t rt_basisu_sink_calls(void) { return g_sink_calls.load(std::memory_order_relaxed); }

} // extern "C"
