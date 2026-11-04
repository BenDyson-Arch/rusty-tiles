### Changed

- `mesh`: capture stores only the vertex attributes that some selected
  primitive carries. Its sort records and its retained triangle sequence
  used to reserve 60 bytes per corner for every attribute; a source with
  normals and colours now stores 190 bytes per triangle instead of 286, and
  one with a single texture coordinate set stores 130.
- `mesh`: the cell tree is built over capture's retained triangle sequence,
  so capture no longer writes a second, 93-byte-per-triangle copy in rank
  order. On a 788k-triangle crop the measured whole-run scratch peak falls
  from 401 to 293 bytes per triangle (normals and colours) and from 524 to
  357 (texture coordinates and images). Archives are byte-identical.
