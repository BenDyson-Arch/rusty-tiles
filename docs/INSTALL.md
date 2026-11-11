# Install rusty-tiles

Most users need only the **standard package**. It converts meshes, LAS/LAZ point clouds and GeoJSON/GeoPackage vectors, and packages, validates and previews tilesets, without GDAL, PROJ, GEOS or system SQLite. “Default” and “portable” in build instructions refer to this build.

## Choose a build

| Distribution | Build | What is installed |
| --- | --- | --- |
| Installer script, release ZIP or `.tgz`, `cargo-binstall` | Standard | The `rusty-tiles` executable |
| `pip install rusty-tiles` | Standard | The `rusty_tiles` Python module; no CLI executable |
| Cargo with no extra features | Standard | CLI or Rust library, with bundled SQLite |
| Container `ghcr.io/bendyson-arch/rusty-tiles` | Native-geospatial | CLI plus GDAL, PROJ, GEOS and SQLite runtime libraries |
| Cargo with `--features native-geospatial` | Native-geospatial | CLI or Rust library linked to system geospatial libraries |

The [native-geospatial build](#native-geospatial-cli) is optional. Use it only for imagery, terrain, Shapefile and other OGR vector formats, or a CRS operation outside the standard package's verified grid-free tier. It is a build-time Cargo feature, not a runtime switch or a Python extra. Installing GDAL alongside a standard binary or wheel does not enable it. There is no native wheel or native ZIP/tarball.

The [converter guide](CONVERTERS.md) lists input and placement limits for each build. Conversion runs offline in both builds. The CLI preview needs a separately installed Cesium runtime.

## Standard CLI

### macOS and Linux

```sh
curl -fsSL https://raw.githubusercontent.com/BenDyson-Arch/rusty-tiles/main/scripts/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
rusty-tiles --version
```

The installer downloads the latest release into `~/.local/bin` and verifies its SHA-256 checksum. It needs no Rust or administrator privileges. Pass options after `sh -s --`: `--version 0.4.0` pins a release and `--prefix DIRECTORY` changes the install directory. Add the `export PATH` line to your shell's startup file to keep it in new terminals.

Binaries cover x86_64 and ARM64. Linux needs glibc 2.35+ and libstdc++; on musl-based systems such as Alpine, build with Cargo or use the container. macOS needs version 15 or later.

### Windows

1. Open [the latest release](https://github.com/BenDyson-Arch/rusty-tiles/releases/latest) and download `rusty-tiles-0.4.0-x86_64-pc-windows-msvc.zip` (the version in the name matches the release) under **Assets**.
2. Extract the ZIP and open PowerShell in the extracted folder.
3. Run `.\rusty-tiles.exe --version`.

To run `rusty-tiles` from any folder, add the extracted folder to your user `PATH` and reopen PowerShell. The Windows download is x64 only.

Each release also lists `SHA256SUMS` for its ZIP and `.tgz` archives. With `cargo-binstall` installed, `cargo binstall --manifest-path Cargo.toml rusty-tiles` in a source checkout downloads the same prebuilt binary.

### Build from source

Install stable Rust, Cargo and a C++ compiler, then build a release tag (or `main`):

```sh
git clone --branch v0.4.0 https://github.com/BenDyson-Arch/rusty-tiles.git
cd rusty-tiles
cargo install --path . --locked
rusty-tiles doctor --command mesh --command point-cloud --command vector
```

Leave `LIBSQLITE3_SYS_USE_PKG_CONFIG` unset: standard builds bundle SQLite. To build without installing, use `cargo build --release --locked` and run `target/release/rusty-tiles`.

## Python and Blender

```sh
python -m pip install rusty-tiles
```

Install into the Python environment that will call it, including Blender's bundled Python. Wheels need CPython 3.10 or newer and use the stable ABI, so one wheel covers later Python versions. They are built for:

- Linux x86_64 and ARM64 (manylinux_2_28, glibc 2.28+)
- macOS 15+ on Intel and Apple Silicon
- Windows x64

Wheels are tested inside official Blender 4.5.14 LTS on Linux x64, Windows x64 and both macOS architectures. Linux ARM64 has no official Blender 4.5 bundle.

The wheel contains **only the standard conversion APIs**: meshes, local and grid-free LAS/LAZ, GeoJSON/GeoPackage vectors, tileset packaging and archive validation. It does not include the CLI, `doctor` or `preview`, and it cannot convert imagery, terrain or OGR-only inputs or use native CRS operations. Installing GDAL or its Python bindings does not change this. See the [Python API guide](../bindings/python/README.md).

To build a wheel from a source checkout, with CPython 3.10+, Rust and a C++ compiler:

```sh
python -m pip install 'maturin==1.15.0'
python -m maturin build --release --locked --out target/wheels
python -m pip install target/wheels/rusty_tiles-*.whl
```

Leave `LIBSQLITE3_SYS_USE_PKG_CONFIG` unset for wheel builds.

## Check the installed build

```sh
rusty-tiles doctor --command mesh --command point-cloud --command vector --json
```

The JSON reports `nativeGeospatial` and, per command, the backends and CRS tiers. A standard binary reports native-geospatial as unavailable; this is expected. Select the commands you intend to use: an unrestricted `doctor` checks every converter, so a standard build reports raster and terrain as unsupported and exits with code 2. `doctor` checks the build and its resources; conversion still validates each input and CRS operation.

Preview and archive validation are in both CLI builds, so the standard CLI can preview imagery and terrain produced elsewhere. Extract a `.3tz` archive before previewing it. `validate` checks archives, not imagery or terrain directories.

Continue with the [mesh quick start](../README.md#quick-start) or the [demos](DEMOS.md#standard-package).

## Native-geospatial CLI

This optional build adds imagery and terrain conversion, Shapefile and other OGR vector formats, and native GDAL/PROJ CRS operations. Enabling it does not make every CRS or geometry supported; see the [converter guide](CONVERTERS.md).

### Container

The published image contains the native CLI with GDAL 3.12.4, PROJ 9.8.1, GEOS and SQLite. Your host needs Docker, not GDAL. The image is Linux amd64; other architectures need Docker's amd64 emulation.

```sh
docker run --rm --platform linux/amd64 --network none \
  ghcr.io/bendyson-arch/rusty-tiles:v0.4.0 doctor --command raster --command terrain

docker run --rm --platform linux/amd64 --network none \
  -v "$PWD:/data" -w /data ghcr.io/bendyson-arch/rusty-tiles:v0.4.0 \
  raster -i orthophoto.tif -o imagery --max-zoom 14
```

Tags are `v0.4.0` and `latest`; pin a version for repeatable deployments. The image contains no Python, datasets, Cesium runtime or extra datum grids, and PROJ networking is off. Mount required grids and set `PROJ_DATA` to a path list that includes them and `proj.db`. With a rootful Docker daemon, add `--user "$(id -u):$(id -g)"` so new outputs belong to you.

To build the image from a source checkout:

```sh
docker build --target runtime -t rusty-tiles:native .
docker run --rm --network none rusty-tiles:native doctor --json
```

### Build against system libraries

In addition to Rust and a C++ compiler, install GDAL 3.12+, PROJ 9.2+, GEOS 3.10+, SQLite, their development headers, `pkg-config` and libclang. CI tests GDAL 3.12 and 3.13. From the source checkout:

```sh
LIBSQLITE3_SYS_USE_PKG_CONFIG=1 cargo install --path . --locked --features native-geospatial
rusty-tiles doctor --command raster --command terrain --command vector
```

The SQLite override makes Rust, GDAL and PROJ share one system SQLite; use it for native builds only. In PowerShell, set `$env:LIBSQLITE3_SYS_USE_PKG_CONFIG = "1"` first; MSVC native builds also need `$env:VCPKGRS_DYNAMIC = "1"` and a matching shared-library SDK.

Matching shared libraries must be present at runtime, so copying the binary alone to another machine is not enough. PROJ needs its local `proj.db` and any grids the chosen operation requires; conversion never downloads them. Missing resources or an ineligible strict operation fail the conversion instead of falling back to a ballpark transform. A constant height offset does not replace a geoid transformation.

Both builds install an executable named `rusty-tiles`, so one replaces the other at the same Cargo install root; use separate `--root` directories to keep both. Use `doctor`, not the file name, to tell them apart. The native build uses GDAL/GEOS for all GeoJSON and GeoPackage geometry, so triangulation can differ from the standard build, and switching builds needs a fresh vector reuse baseline.

Check native readiness with:

```sh
rusty-tiles doctor --command raster --command terrain --command vector --json
```

Native imagery and terrain demos are in [DEMOS.md](DEMOS.md#native-imagery-and-terrain).
