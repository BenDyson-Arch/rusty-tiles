### Fixed

- `mesh`: a source of many isolated tiny triangles is no longer refused with "mesh run spill peak … exceeded the declared scratch need". The whole-run spill check now holds the run to the larger of capture's declared need and the HLOD stage's own bound, whose phases are each declared and admitted as they start; isolated triangles reach the HLOD incidence pass's worst case of six interface items per triangle.
- `mesh`: under a cgroup v2 memory limit, reclaimable page cache and slab charged to the cgroup (for example a preceding compile's object files) count as available memory, as the kernel reclaims them before an out-of-memory kill. Anonymous memory and shmem/tmpfs still count as used.
