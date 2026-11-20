### Fixed

- HLOD: P1 no longer proposes a cell the contraction fold guard refuses for a moved book
  wing. Removing a border vertex that is the third corner of a face on a source book edge
  (two faces using an interior edge in one direction) kept the edge's counts but moved the
  face, which the guard's book rule refuses; P1's collapse checks, flips and Steiner rescues
  now apply that rule itself (`folds::book_refused`, shared with the guard). A Steiner
  rescue refused by it counts under a new reason (`steinerRefused` index 6).

### Changed

- HLOD: each P1 record in the stage report (`p1_records`) counts the faces of a proposal the
  contraction fold guard refused, by rule (`guardRefused`, per pass: reversed, shared face,
  zero area, coincident, shared edge, book) and how many of them have a Steiner corner
  (`guardSteiner`).
