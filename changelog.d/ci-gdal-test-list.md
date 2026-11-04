### Fixed

- CI: the pull-request GDAL job named the `mesh_crs` integration test, which was deleted with the legacy mesh code, so every run failed before testing. It now runs `h26_mesh_frontends`, which covers `mesh --source-crs`.
