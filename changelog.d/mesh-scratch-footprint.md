### Changed

- Spill sort runs are written as chunk files of at most 8 MiB, and readers remove each chunk
  as soon as it is consumed, so a streamed merge and the output it feeds share scratch space
  and intermediate merges free their inputs while writing. Record order and content are
  unchanged. The mesh tiler also releases its rank-ordered triangle projection (93 bytes per
  triangle) once capture has joined corner depths, instead of retaining it through leaves and
  coarse levels. The scratch stage model charges these lifetimes, including the coarse incidence pass.
