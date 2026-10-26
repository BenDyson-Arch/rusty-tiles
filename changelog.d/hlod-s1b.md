### Fixed

- Refuse HLOD stages whose missing coarse tiles disagree with the producer's total
  or per-depth omission counts, including loss of an entire coarse level.
- Verify that an entirely omitted textured coarse frontier performs no image-cache
  loads or appearance work.
