# Install rusty-tiles

This guide describes the 0.4.0 development branch. Published 0.3.0 binaries have fewer capabilities; use the documentation at the matching release tag. In particular, portable vector conversion and general grid-free CRS support require 0.4.

## Choose a build

**Start with the standard package.** It handles meshes, local and grid-free georeferenced LAS/LAZ, and GeoJSON/GeoPackage vectors without installing GDAL, PROJ, GEOS or system SQLite. “Default” and “portable” refer to this build.

Choose **native-geospatial** when you need imagery, terrain, other OGR vector formats or an eligible CRS operation outside the standard package's verified grid-free tier. This is a build-time Cargo feature, not a runtime switch or a Python extra. Installing GDAL alongside the standard package does not enable it.

| Distribution | Build | What is installed |
| --- | --- | --- |
| CLI installer, release ZIP/tarball or `cargo-binstall` | Standard | The `rusty-tiles` executable; no system geospatial libraries needed |
| PyPI wheel | Standard | The `rusty_tiles` Python module and its conversion APIs; no CLI executable, preview server or native fallback |
| Cargo with no extra features | Standard | CLI or Rust library, using bundled SQLite and portable JPEG |
| Published native container | Native-geospatial | CLI plus GDAL/PROJ/GEOS/SQLite runtime libraries |
| Cargo with `--features native-geospatial` | Native-geospatial | CLI or Rust library linked to system geospatial libraries you install |

There is no native-geospatial release wheel or separate native CLI ZIP/tarball. Use the container or build from source for those capabilities. The [converter guide](CONVERTERS.md) lists input and placement limits; enabling native-geospatial does not make every CRS or geometry supported.

Conversion runs offline in both builds. The CLI preview needs a separately installed Cesium runtime. Native transformations need the relevant PROJ database and grids already on disk; a constant height offset does not replace a geoid transformation.

## Standard CLI

### Released binaries

Use the [macOS/Linux installer or Windows download instructions](../README.md#install). The installer verifies SHA-256 checksums. Run `rusty-tiles --version` and use that version's documentation.

Standard release binaries cover Linux and macOS on x86_64 and ARM64, and Windows x64. Linux binaries require glibc 2.35+ and libstdc++; macOS binaries require macOS 15+. These operating-system requirements remain even though no geospatial libraries are needed. Python wheel requirements differ and are listed below.

### Build from source

Install stable Rust, Cargo and a C++ compiler, then:

```sh
git clone --branch develop https://github.com/BenDyson-Arch/rusty-tiles.git
cd rusty-tiles
cargo install --path . --locked
rusty-tiles --version
rusty-tiles doctor --command mesh-to-3tz --command point-cloud --command vector
```

Do not set `LIBSQLITE3_SYS_USE_PKG_CONFIG` for this build: standard builds bundle SQLite. No GDAL/PROJ/GEOS SDK is needed.

## Native-geospatial CLI

### Use the container

The [native container](../README.md#run-the-native-geospatial-container) bundles the geospatial libraries. It is Linux amd64; other architectures require Docker's amd64 emulation. Your host needs Docker, not a GDAL installation.

```sh
docker run --rm --platform linux/amd64 --network none \
  ghcr.io/bendyson-arch/rusty-tiles:latest doctor --command raster --command terrain
```

Pin an image version for repeatable deployments. Mount your data and any extra datum grids explicitly; the image does not bundle every grid. To use the current development source, [build the container](../README.md#build-the-container) from the checkout.

### Build against system libraries

In addition to Rust and a C++ compiler, install GDAL 3.12+, PROJ 9.2+, GEOS 3.10+, SQLite, their development headers, `pkg-config` and libclang. From the source checkout:

```sh
LIBSQLITE3_SYS_USE_PKG_CONFIG=1 cargo install --path . --locked --features native-geospatial
rusty-tiles doctor --command raster --command terrain --command vector
```

This command uses POSIX shell syntax. In PowerShell, set `$env:LIBSQLITE3_SYS_USE_PKG_CONFIG = "1"` before the Cargo command; MSVC native builds also need `$env:VCPKGRS_DYNAMIC = "1"` and a matching shared-library SDK. The container is the packaged native option when you do not want to configure an SDK.

The SQLite override is required so Rust, GDAL and PROJ share one system SQLite. Apply it to native Cargo commands only. Matching shared libraries must be available at runtime too; copying this binary alone to another machine does not supply them. PROJ needs its local `proj.db` and any grids required by the chosen operation. Missing resources or an ineligible strict operation cause conversion to fail rather than use a ballpark transform.

Both builds use the executable name `rusty-tiles`. Installing either build at the same Cargo install root replaces the other; use separate `--root` directories if you need both. Use `doctor` to identify capabilities, not the filename. GeoJSON and GeoPackage vector conversion uses the GDAL/GEOS backend throughout the native build, so changing builds can change triangulation and requires a fresh vector reuse baseline.

`native-jpeg` is a separate optional codec feature. Both geospatial builds use portable JPEG unless that feature is selected; it does not enable raster, terrain or CRS operations. See [contributor build instructions](../CONTRIBUTING.md#build).

## Python and Blender

Python wheels contain **only the standard conversion APIs**. They support meshes, local/grid-free LAS/LAZ, GeoJSON/GeoPackage vectors, tileset packaging and archive validation. They do not expose raster, terrain, OGR-only inputs, native CRS fallback, `doctor` or `preview`. Installing GDAL or its Python bindings does not change the wheel. For native conversion, use the native CLI/container or the Rust library with `native-geospatial` enabled.

After the first PyPI publication, install a released wheel into the Python environment that will call it:

```sh
python -m pip install rusty-tiles
```

To use the current development code, run these commands from the source checkout with CPython 3.10+, Rust and a C++ compiler:

```sh
python -m pip install 'maturin==1.15.0'
RUSTY_TILES_DISABLE_NATIVE_JPEG=1 python -m maturin build --release --locked --out target/wheels
```

Install the resulting platform-matching wheel with that Python's `python -m pip install /path/to/the.whl`. In PowerShell, set `$env:RUSTY_TILES_DISABLE_NATIVE_JPEG = "1"` before the maturin command. Leave `LIBSQLITE3_SYS_USE_PKG_CONFIG` unset. See the [Python API guide](../bindings/python/README.md) for available functions and installation testing inside Blender's actual Python runtime.

Release wheels target Linux glibc 2.28+ on x86_64 and ARM64, macOS 15+ on Intel and Apple Silicon, and Windows x64. They use the CPython stable ABI for 3.10 and newer. Candidate wheels have passed official Blender 4.5.14 LTS acceptance on Linux x64, Windows x64 and both macOS architectures; Linux ARM64 has no official Blender 4.5 bundle. Publication and installation from PyPI remain separate checks in [#77](https://github.com/BenDyson-Arch/rusty-tiles/issues/77).

## Check the installed build

For standard conversion:

```sh
rusty-tiles doctor --command mesh-to-3tz --command point-cloud --command vector --json
```

For native conversion:

```sh
rusty-tiles doctor --command raster --command terrain --command vector --json
```

The JSON includes `nativeGeospatial` and per-command backends and CRS tiers. A standard binary reports native-geospatial as unavailable; that is expected and does not prevent supported standard conversions. An unrestricted `doctor` checks every converter, so it reports missing raster/terrain capability and exits with code 4 in a standard build. Select the commands you intend to use. Readiness checks the build and resources; conversion still validates each input and exact CRS operation.

Continue with the [mesh quick start](../README.md#quick-start) or the [standard-package demos](DEMOS.md#standard-package). Native imagery/terrain demos are in a separate section. Preview and archive validation are included in both CLI builds: the standard CLI can preview imagery and terrain generated elsewhere, although it cannot convert those sources. `.3tz` archives must be extracted before previewing; `validate` checks archives, not imagery or terrain directories.
