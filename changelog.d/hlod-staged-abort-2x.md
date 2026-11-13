### Changed

- Mesh HLOD: a staged reduction (`Staged`, `N1`, `N2`) that is still above twice the
  triangle cap when its twelfth step is done is abandoned, and that proposal fails without
  the separate-sheets retry. Across the measured inputs, no such reduction was ever repaired
  into an accepted proposal, so the coarse ladder spends less time on proposals that would
  fail anyway.
