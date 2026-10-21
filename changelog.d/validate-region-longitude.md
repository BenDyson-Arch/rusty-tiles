### Fixed

- `validate`: region containment measured a whole-circle longitude span
  (`west = -π`, `east = π`) as empty. A child region spanning every longitude
  passed inside a narrower parent, and a whole-circle parent rejected children
  crossing the antimeridian. The rounding tolerance now also applies at the
  parent's west edge.
