### Changed

- Mesh tiler scratch admission: the upfront scratch need is now the largest declared stage
  working set (each external sort charged its streamed runs plus at most one intermediate merge
  group, `S + min(S, F × run)`, and H17's declared pre-pass) with a 10% model margin and
  per-file allocation rounding, instead of the sum of all stages with the whole logical need
  added again as extent overhead. On a 24.4M-triangle mesh the need falls from about
  4,300 to about 1,040 bytes per triangle. Each capture stage rechecks free space for its own
  working set on entry and fails closed if its measured peak exceeds its declared bound.
  Admission also reserves output-side space (leaf staging, archive staging and a replaced
  destination, estimated at 3 × 119 bytes per triangle plus image bytes; 119 was measured before the archive-size reduction, which brings the archive to about 47 bytes per triangle, and is kept as a conservative estimate), summed with scratch
  when they share a filesystem. Diagnostics `capture.scratch_need_bytes` and
  `scratch_logical_need_bytes` change meaning accordingly; new `scratch_stage_bytes`,
  `scratch_stage_peak_bytes` and `output_reserve_bytes` report the model and measurements.
