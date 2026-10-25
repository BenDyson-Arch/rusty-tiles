### Changed

- Measure HLOD coarse geometric error in both directions against the captured source
  surface after position decode, with deterministic sampling, a documented 5%
  sampling margin, monotone child floors and versioned distance summaries. Select coarse
  primitive representatives from the nearest source surface and provide bounded
  source queries for subsequent appearance baking. Exact leaves are unchanged.
- Bound every source/proxy nearest query with a per-query work budget and a guaranteed
  greedy witness; exhausted searches keep their best (overestimated) distance and are
  counted in `hlod.errorMeasurement.nearest`.
- Admit the source index as walk scratch before writing it, skip omitted cells (they
  hold no content), and keep the sampling density over published source at depths
  with omissions.
