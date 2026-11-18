### Fixed

- HLOD: a coarse cell whose first build publishes nothing now gets one more build before
  the retry ladder, a guard-by-construction contraction that never makes what the fold,
  topology and interface checks refuse. A border vertex whose removal would need a new face
  on three shared border points is replaced by an interior vertex on the leaf lattice. This
  builds cells that were omitted before (#344's rock fixture: every omission and the root).
  Tiles the first build publishes are unchanged; the stage report names the new build `p1`.
