# Install rusty-tiles

This guide describes the 0.4.0 development branch. The latest published binary can have fewer capabilities than `develop`; check its matching release notes before choosing an input. See the [converter guide](CONVERTERS.md) for format and placement limits.

## Choose a build

| Build | Capabilities | Installation |
| --- | --- | --- |
| Default | Mesh tiling, GLB/glTF packaging, local and verified grid-free LAS/LAZ, GeoJSON and GeoPackage, archive validation and preview | Prebuilt download when released, or Cargo from `develop` |
| Native geospatial | Default capabilities plus other OGR inputs, imagery, terrain and strict native CRS operations | Native container or Cargo with GDAL/PROJ/GEOS and SQLite |
| Python wheel | Path-based default converters and archive validation | Released PyPI wheel or a wheel built from `develop` |

Conversion runs offline. Install a Cesium runtime separately to use the local preview. Native CRS operations need the relevant PROJ database and grids already on disk; a constant height offset does not substitute for a geoid grid.

## Released command-line binaries

Use the [macOS/Linux installer or Windows download instructions](../README.md#install). The installer verifies SHA-256 checksums. Run `rusty-tiles --version` after installation and use that version's documentation.

The default release binaries cover Linux and macOS on x86_64 and ARM64, and Windows x64. Linux binaries require glibc 2.35+ and libstdc++; macOS binaries require macOS 15+. Python wheel requirements differ and are listed below.

The [native container](../README.md#run-the-full-toolset-in-a-container) bundles the geospatial libraries. It is Linux amd64; other architectures require Docker's amd64 emulation. Pin an image version for repeatable deployments and mount your data and any extra datum grids explicitly.

## Build the development CLI

Install stable Rust, Cargo and a C++ compiler, then:

```sh
git clone --branch develop https://github.com/BenDyson-Arch/rusty-tiles.git
cd rusty-tiles
cargo install --path . --locked
rusty-tiles --version
rusty-tiles doctor --command vector --command point-cloud
```

The default build bundles SQLite and uses portable JPEG. It needs no GDAL installation.

For native geospatial conversion, install GDAL 3.12+, PROJ 9.2+, GEOS 3.10+, SQLite, their development headers, `pkg-config` and libclang. Build against shared system SQLite:

```sh
LIBSQLITE3_SYS_USE_PKG_CONFIG=1 cargo install --path . --locked --features native-geospatial
rusty-tiles doctor --json
```

Apply that environment override to native Cargo commands only. Leave it unset for default builds and wheels, which bundle SQLite. The runtime must provide the matching native libraries. See [contributor build instructions](../CONTRIBUTING.md#build) for other optional features.

## Python and Blender

After the first PyPI publication, `python -m pip install rusty-tiles` installs a released wheel. To use the current development code, run these commands from the source checkout with CPython 3.10+:

```sh
python -m pip install 'maturin==1.15.0'
RUSTY_TILES_DISABLE_NATIVE_JPEG=1 python -m maturin build --release --locked --out target/wheels
```

Install the resulting `.whl` using that Python's `python -m pip install /path/to/the.whl`. Choose the wheel matching the platform and install it into the Python environment that will call it. The build commands above use POSIX shell syntax; in PowerShell, set `$env:RUSTY_TILES_DISABLE_NATIVE_JPEG = "1"` before running `python -m maturin build --release --locked --out target/wheels`. See the [Python API guide](../bindings/python/README.md) for supported calls and the isolated Blender acceptance runner.

Release wheels target Linux glibc 2.28+ on x86_64 and ARM64, macOS 15+ on Intel and Apple Silicon, and Windows x64. They contain the default Rust build and use the CPython stable ABI for 3.10 and newer. Wheel availability and testing inside an actual Blender distribution are separate release checks in [#77](https://github.com/BenDyson-Arch/rusty-tiles/issues/77).

## Check the installed build

```sh
rusty-tiles doctor --command mesh-to-3tz
rusty-tiles doctor --command vector --command point-cloud --json
```

`doctor` reports the converters and CRS tier in that binary. Readiness is not a guarantee that every file's CRS or geometry is supported; conversion validates those inputs. An unrestricted `doctor` also checks raster and terrain, so it can report missing native capabilities in an otherwise working default build.

Continue with the [invented mesh quick start](../README.md#quick-start) and the [vector example](../README.md#4-preview). Both run without native geospatial libraries in the development build. Preview requires the separately installed Cesium runtime, and `.3tz` archives must be extracted before serving.
