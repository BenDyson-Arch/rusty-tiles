### Changed

- Mesh HLOD: a staged reduction (`Staged`, `N1`, `N2`) that is still above twice the
  triangle cap after twelve steps is abandoned. Its proposal then fails without the
  separate-sheets retry. In the measured replays, which also skipped that retry, no such
  reduction had led to an accepted proposal and no tile changed. The coarse ladder now
  spends less time on proposals that would fail anyway.
