### Changed

- `mesh` output layout: shallow subtrees (height 1 to 3) are now written as explicit
  tiles while their document has room, and only become implicit roots when a
  document exceeds its caps. Documents are packed to 480 tile objects, 4,096
  records and 12 levels instead of being split at 32 objects. This cuts the
  dependent requests a viewer needs before the first content. In a simulated
  CesiumJS first view of the 2048-texture output, requests fall from 102 to 87
  and the deepest dependency chain from 5 to 3 round trips. Content GLB bytes are
  unchanged. Inlined tiles' payloads move from `implicit-content/…` to
  `content/f<first>-d<depth>.glb`, and the JSON, subtree and member set of every
  mesh archive changes.
