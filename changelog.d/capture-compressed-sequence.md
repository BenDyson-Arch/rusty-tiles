### Changed

- Mesh capture keeps its retained triangle and corner-depth streams as
  LZ4-compressed blocks in scratch (pure-Rust `lz4_flex`, LZ4 block format, no
  frame). Capture writes fewer bytes, later stages re-read fewer, and measured
  scratch is lower. Output is unchanged; declared and preflight scratch bounds
  still count these files at their uncompressed size.
