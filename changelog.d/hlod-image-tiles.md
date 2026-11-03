### Fixed

- Retain reconstructed HLOD and leaf source images in a bounded tile cache backed
  by declared scratch storage, avoiding repeated decoding when textures exceed
  the resident cache budget. Include leaf decode and encode timings in diagnostics.
