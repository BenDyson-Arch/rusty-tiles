### Changed

- Construct shared HLOD borders from common source-arc certificates, preserving
  canonical same-depth edges and bounding mixed-depth gaps by geometric error.
- Keep coarse payloads within 8,192 triangles using bounded topology cleanup and
  remeshing, with explicit omission frontiers and optional durable audit receipts.

### Fixed

- Refuse HLOD stages whose missing coarse tiles disagree with the producer's total
  or per-depth omission counts, including loss of an entire coarse level.
- Verify that an entirely omitted textured coarse frontier performs no image-cache
  loads or appearance work.
