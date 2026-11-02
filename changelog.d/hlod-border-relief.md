### Fixed

- Relieve the shared borders of mesh cells whose exact border alone fills much of the 8,192
  triangle cap: above 85%, their shared arcs are
  certified at a coarser ladder step, identical for every owner, and every tile on those
  arcs publishes at least eight times its certified displacement as geometric error.
  Relieved tiles contract by collapse along each chord; a contraction that stalls is handed
  to the last resort. The archive validator's border displacement rule
  is now G/8 per tile. Cells below the trigger with no inherited relief are unchanged.
