### Changed

- Measure HLOD coarse geometric error in both directions against the captured source
  surface after position decode, with deterministic sampling, a documented density
  margin, monotone child floors and versioned distance summaries. Select coarse
  primitive representatives from the nearest source surface and provide bounded
  source queries for subsequent appearance baking. Exact leaves are unchanged.
