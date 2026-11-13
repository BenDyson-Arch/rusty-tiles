### Changed

- `mesh` no longer refuses source images over 32 MiB encoded. Each image is
  charged against the memory budget at its actual encoded length plus its
  decoder working memory, in capture, the leaf stage and the coarse stage.
  Capture reads every image's header (PNG `IHDR` or the first JPEG frame
  header) before it writes any scratch, and an image whose decode the budget
  cannot hold is `resource_limit` there; each decode is checked again against
  the decoder's own header, which must declare the same dimensions
  (`invalid_input` otherwise). A 1 GiB encoded sanity ceiling (`unsupported`)
  and the 16,384 px edge remain. Malformed or unsupported image headers are now
  refused at that preflight, before capture. The error kinds are the
  decoder's, except that an image both over a header limit (edge, 16-bit) and
  corrupt later in its stream now reports the header refusal (`unsupported`)
  instead of the corruption (`invalid_input`).
- Breaking error type: a `mesh` source image over 32 MiB encoded (and at most
  1 GiB) that the memory budget cannot hold is now `resource_limit` (exit 1;
  Python `ResourceLimitError`), refused with "image capture: decoding a source
  image needs N bytes ... but the memory budget leaves B for image decoding".
  It was `unsupported` (exit 2; `UnsupportedError`) "encoded image exceeds
  32 MiB admission ceiling" before, whatever the budget. One that fits is now
  converted. Over 1 GiB stays `unsupported`, now "encoded image exceeds 1 GiB
  admission ceiling".
- Archives for sources admitted before are unchanged at the budgets the digest
  tests use, with two planning caveats. An image of at most 32 MiB with more
  than 4 encoded bytes per texel (an incompressible RGBA PNG, or one inflated
  by ancillary chunks) now gets a higher, correct leaf-stage charge, which can
  move a leaf's density step at a budget right at that threshold. The coarse
  stage's decode charge now uses the largest actual encoded image instead of a
  fixed 32 MiB, so for every source that declares images (textured, or with
  images no material samples) the coarse stage's job count, image cache and
  reported memory minima can change; coarse planning does not change archive
  bytes, and image-free sources never had this term. The output digest tests
  run at default budgets, not at these thresholds.
