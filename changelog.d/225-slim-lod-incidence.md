### Changed

- `mesh`: the H17 LOD pre-pass sorts 30-byte corner records instead of 63-byte
  ones. Capture now names welded vertices by ordinal (12 bytes per triangle,
  kept until the pre-pass reads them), and the pre-pass keys on those ordinals.
  On a 788k-triangle crop the measured pre-pass scratch peak falls from 485 to
  386 bytes per triangle, and the whole-run spill peak from 485 to 401.
  Archives are byte-identical. Inputs with more than 2^40 triangles or more
  than 2^32 welded vertices are refused with a typed error: the triangle count
  before any geometry is read, the welded vertex count straight after the weld.
