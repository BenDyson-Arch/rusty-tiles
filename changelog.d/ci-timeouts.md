### Changed

- CI: the Rust, Portable CLI and Official Blender jobs allow 40 minutes instead of 25. macOS x64 runners repeatedly hit the old limit while tests were passing. The Rust job has since grown to about 37 minutes; the parallel unit tests entry covers that.
