### Changed

- The HLOD stage admits the shared-border scratch from the graph it builds, not from a
  per-depth worst case: once the border graph exists, the borders are admitted once, with
  free space preflighted, for the graph, every depth's maps and the segments and owner
  references each depth can emit. On Walem 2048 the admission falls from 1.11 GB to
  0.57 GB (measured peak 0.36 GB). A bake short of space is refused at that point, before
  any border level runs. Output is unchanged.
