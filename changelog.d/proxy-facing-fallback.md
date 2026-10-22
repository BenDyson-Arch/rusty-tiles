### Fixed

- `mesh` no longer refuses real photogrammetry with "proxy has no facing
  same-primitive source appearance donor". When simplification turns a proxy
  face away from every source triangle of its primitive, its texels sample the
  nearest same-primitive source surface regardless of orientation.
