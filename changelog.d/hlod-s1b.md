### Changed

- Construct shared HLOD borders from common source-arc certificates, preserving
  canonical same-depth edges and bounding mixed-depth gaps by geometric error.
- Keep coarse payloads within 8,192 triangles using bounded topology cleanup and
  remeshing, with explicit omission frontiers and optional durable audit receipts.
- Report omission totals and per-depth border, topology and frontier counts in
  public HLOD diagnostics, including zeros.
- Measure opened-hole loss within the depth budget, reject border contacts, and
  report attempted/published operations and affected tiles' final geometric error.

### Fixed

- Refuse HLOD stages whose missing coarse tiles disagree with the producer's total
  or per-depth omission counts, including loss of an entire coarse level.
- Verify that an entirely omitted textured coarse frontier performs no image-cache
  loads or appearance work.

- Reconstruct contracted border strips without inverted or zero-area faces, retain
  their interior ring, and count directed seam incidences against source subarcs.
- Bound chord emission work by distinct owners, and add operation locations to
  the optional external audit sidecar.
