### Fixed

- Relieve the shared borders of mesh cells whose exact border alone nearly fills the 8,192
  triangle cap: their shared arcs are certified at a coarser ladder step, identical for every
  owner, and every tile on those arcs publishes at least eight times its certified
  displacement as geometric error. Relieved tiles contract by collapse along each chord. The
  archive validator's border displacement rule is now G/8 per tile. Cells below the trigger
  are unchanged.
