### Changed

- Measure HLOD coarse geometric error in both directions against the captured source
  surface after position decode, with deterministic sampling, a documented 5%
  sampling margin, monotone child floors and versioned distance summaries. Select coarse
  primitive representatives from the nearest source surface and provide bounded
  source queries for subsequent appearance baking. Exact leaves are unchanged.

- Reuse identical child measurements, deduplicate indexed controls, cache source
  areas/materials, and restrict source controls to patch boundaries, components
  and slivers. Verify measured positions against decoded GLB bits and refuse
  unsupported sampling depths.

Integrates S1 chain collapse: measure, seed and aggregate by encoded depth, omit collapsed cells from density scheduling, and remove the redundant reuse cache. Preserve open-border decode and exact held-memory accounting.
