### Fixed

- HLOD: a cell whose only child is a coarse tile it cannot collapse onto (their shared-border contracts differ between the two depths) no longer fails the bake with `invalid_state` "coarse tiles or interface rows differ from the tree". The child's published depth is final once the cell closes and builds its own tile; before, the child's row was never written. First reached by the 10M-triangle mesh-1e7 scale input.
