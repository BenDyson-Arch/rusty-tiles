### Changed

- `mesh` coarse tiles: the coarse stage now reserves memory for one source-image decode
  for the whole stage, instead of one per parent job, and decodes source images one at a
  time. Large textures no longer serialise coarse jobs under the memory budget: on a
  10 M-triangle scene with 89 8192² textures at a 4 GiB budget with `--jobs 2`, the
  stage runs 2 jobs instead of 1 and its wall time falls by about a third. Archives are
  byte-identical; `diagnostics.memory.hlod` reports the new `decode_bytes`.
