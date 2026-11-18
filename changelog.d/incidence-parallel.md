### Changed

- The HLOD incidence pass generates its interface items in parallel, by chunks
  of leaves, and sorts only the items another leaf's grown box and occupancy
  cells can hold (with every edge that has one face in its leaf). The leaves'
  occupancy masks are kept from capture. Rows, endpoints, counts and the archive
  are unchanged; a stop request is honoured before each leaf.
  `diagnostics.memory.hlod.incidence_jobs` reports the item workers, and the
  `hlod_incidence_reduce` step counts the sorted items.
