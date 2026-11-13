### Added

- Python wheels are also tested inside official Blender 5.2.4 LTS (Python 3.13) on Linux x64, Windows x64 and Apple Silicon macOS, alongside Blender 4.5.14 LTS.
- A rolling development build of `develop`, the GitHub pre-release `dev`, is rebuilt after a nightly full CI pass when `develop` has changed. It has the CLI archives, Python wheels and `SHA256SUMS`, plus the container tag `ghcr.io/bendyson-arch/rusty-tiles:dev`, and uses a development version such as `0.4.1.dev202611121734`. It is not published to PyPI or crates.io. `scripts/install.sh --version dev` installs its CLI; see "Development builds" in `docs/INSTALL.md`.

### Fixed

- `rusty_tiles.__version__` uses the wheel's PEP 440 spelling for pre-release versions (`0.4.0rc1`, not `0.4.0-rc.1`), so it matches `importlib.metadata.version("rusty-tiles")`.
