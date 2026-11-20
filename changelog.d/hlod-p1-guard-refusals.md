### Changed

- HLOD: each P1 record in the stage report (`p1_records`) counts the faces of a proposal the
  contraction fold guard refused, by rule (`guardRefused`, per pass: reversed, shared face,
  zero area, coincident, shared edge, book) and how many of them have a Steiner corner
  (`guardSteiner`). Decisions and archives are unchanged.
