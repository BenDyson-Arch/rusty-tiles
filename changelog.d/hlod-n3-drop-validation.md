### Fixed

- Validation: a coarse tile declaring last-resort stage 3 may now drop border-free components at any size, as the last resort allows. Previously `coarseDrops` refused such a drop when the children's faces fit the 8,192-triangle cap or a smaller component was kept. The declared count and the geometricError bound still apply, and every other tile keeps both rules.
