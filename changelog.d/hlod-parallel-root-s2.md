### Changed

- HLOD: the root tile's error measurement runs its nearest searches in parallel. The root
  alone builds while every other worker is idle, and its S2 searches (both directed
  distances, the representatives, and the topology controls for moved boundaries,
  closures and openings) now run in fixed batches of 4,096 samples, each from a fresh
  search state. They are folded back in sampling order, so the result depends only on
  the batch size, never on the worker count. Other tiles measure as before. On the
  inputs measured the archive is unchanged; in general a search that exhausts its
  budget may end differently from a fresh start, so the root's G can differ from
  before in its last digits. The root's job holds one batch's source query, search
  frontier and sample window (about 28 MB, part of the stage's minimum memory), and each
  further concurrent batch (about 18 MB) is leased only when the memory budget admits it.
