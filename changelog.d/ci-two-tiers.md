### Changed

- CI: pull requests now run a fast Linux tier: `Rust`, `Python` and native geospatial for GDAL 3.13 only. The portable and encoder-link matrices, Python wheels, wheel acceptance, the Python-free runtime and GDAL 3.12 moved to a full tier that runs on pushes to `main` and `develop`, nightly and manually. A failing push or nightly run on `develop` opens or updates one "Full CI matrix failing on develop" issue, and the next green run closes it. The release workflow now requires the full tier's `Full matrix` and `Rust` jobs for the tagged commit, so a pull-request run cannot satisfy it.
- CI: `playwright install-deps` is bounded to 180 seconds, because a stalled apt mirror held browser jobs for minutes.
