### Changed

- `preview` lights the scene with a fixed sun from the north-west, 35° above
  the horizon at the scene's centre, instead of Cesium's clock-driven sun. The
  view no longer depends on the hour the page was loaded; at night the default
  sun lit meshes from below and left them flat.
