### Fixed

- HLOD: a coarse cell whose first build publishes nothing now gets one more build before
  the retry ladder, a guard-by-construction contraction that never makes what the fold,
  topology and interface checks refuse. A border vertex whose removal would need a new face
  on three shared border points is replaced by an interior vertex on the leaf lattice, only
  where its fan is one simple disk on its own border chord, the point is a private canonical
  lattice point, orientations stay strictly positive and no new face touches another
  outside their common vertices (exact predicates, checked again on the delivered positions). This
  builds cells that were omitted before (#344's rock fixture: every omission and the root).
  The stage report names the new build `p1` and records every cell P1 ran on
  (`p1_records`: outcome, work against its limit, Steiner rescues and refusals).
- HLOD: the contraction guard's licence for a required border chord no longer counts child
  interface segments that both of their holders keep inside the parent. Such a segment is
  an internal seam after the merge; mixed into the chord's exterior source path it broke
  the path and left the chord with no licence in either direction, so every build of the
  cell refused it.
