### Fixed

- `mesh`: image-free input no longer reserves a leaf texture workspace (about
  3 GiB for `balanced`), and the leaf stage refuses a plan whose single job does
  not fit available memory even when it builds no page.
