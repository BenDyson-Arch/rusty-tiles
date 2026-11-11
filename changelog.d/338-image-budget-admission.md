### Changed

- `mesh` no longer refuses source images over 32 MiB encoded. Each image is
  charged against the memory budget at its actual encoded length plus its
  decoder working memory, in capture, the leaf stage and the coarse stage.
  An image the budget cannot hold is now `resource_limit`, refused before
  capture writes any scratch. A 1 GiB encoded sanity ceiling (`unsupported`)
  and the 16,384 px edge remain. Archives for sources admitted before are
  unchanged; the coarse stage's planned decode charge now uses the largest
  actual encoded image instead of a fixed 32 MiB, so reported memory minima
  of textured runs can be lower.
