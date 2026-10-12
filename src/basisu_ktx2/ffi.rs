//! Hand-written declarations for `native/rt_basisu_shim.cpp` and
//! `native/rt_basisu_sink.cpp`, linked as static libraries by `build.rs`.

use std::os::raw::c_char;

pub(super) const STATUS_OK: i32 = 0;
pub(super) const STATUS_INVALID_ARGUMENT: i32 = 1;
pub(super) const STATUS_OUT_OF_MEMORY: i32 = 3;
pub(super) const STATUS_INVARIANT_FAILED: i32 = 9;
pub(super) const STATUS_CONTAINER_FAILED: i32 = 10;

pub(super) const FORMAT_ETC1S: u32 = 0;
pub(super) const FORMAT_UASTC_LDR_4X4: u32 = 1;

pub(super) const FLAG_SRGB: u32 = 1 << 0;
pub(super) const FLAG_ZSTD: u32 = 1 << 1;
pub(super) const FLAG_MIPMAPS: u32 = 1 << 2;

extern "C" {
    /// Encodes tightly packed RGBA8 rows to KTX2. On success `*out` owns
    /// `*out_len` bytes that must be released with [`rt_basisu_free`].
    pub(super) fn rt_basisu_encode(
        rgba: *const u8,
        width: u32,
        height: u32,
        format: u32,
        quality: i32,
        effort: i32,
        flags: u32,
        out: *mut *mut u8,
        out_len: *mut usize,
    ) -> i32;

    pub(super) fn rt_basisu_free(data: *mut u8);

    /// `which == 0` throws through the replacement the encoder's checks call
    /// instead of `abort()`; otherwise calls the library's real
    /// `container_abort()`. Returns the status an entry point reports.
    #[cfg(test)]
    pub(super) fn rt_basisu_selftest_failure(which: i32) -> i32;

    /// Decodes one mip level to RGBA8. `out_len` must equal the level's
    /// `width * height * 4`; the decoder's level dimensions are returned.
    pub(super) fn rt_basisu_decode_level_rgba(
        ktx2: *const u8,
        ktx2_len: usize,
        level: u32,
        out: *mut u8,
        out_len: usize,
        out_width: *mut u32,
        out_height: *mut u32,
    ) -> i32;

    /// Copies this thread's captured library diagnostics and returns their
    /// full length. Cleared at the start of every shim call.
    pub(super) fn rt_basisu_diagnostics(buffer: *mut c_char, capacity: usize) -> usize;

    /// Process-wide count of library writes aimed at stdout/stderr that the
    /// sinks captured instead.
    #[cfg(test)]
    pub(super) fn rt_basisu_sink_calls() -> u64;
}

/// This thread's diagnostics from the most recent shim call.
pub(super) fn diagnostics() -> String {
    let mut buffer = vec![0u8; 4096];
    // SAFETY: the buffer is valid for `buffer.len()` bytes.
    let length =
        unsafe { rt_basisu_diagnostics(buffer.as_mut_ptr().cast::<c_char>(), buffer.len()) };
    buffer.truncate(length.min(buffer.len()));
    String::from_utf8_lossy(&buffer).trim().to_owned()
}
