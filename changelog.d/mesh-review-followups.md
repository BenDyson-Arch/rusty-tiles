### Changed

- Mesh tiler: a non-UTF-8 `--source-offset-file` is an invalid request rather than an I/O
  error. The output-side scratch reservation is rechecked once image dimensions are known,
  with 4 bytes per source texel for `lossless` PNG pages and 1 for KTX2 profiles (4/3 mips),
  so textured lossless runs are no longer under-reserved. The H17 pre-pass is measured and
  bounded like the capture stages, and diagnostics add `run_scratch_peak_bytes`.
