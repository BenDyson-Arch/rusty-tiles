# syntax=docker/dockerfile:1
# Matching Debian ABI, linked GDAL/PROJ SDK and a Python-free runtime.
#
# PROJ and GDAL come from the geospatial-build stage, which is GEOBASE: either the pinned
# base image that .github/workflows/geobase.yml builds from geospatial-source and pushes to
# ghcr.io/bendyson-arch/rusty-tiles-geobase:<gdal>-<proj> (pin it by digest:
# GEOBASE=ghcr.io/...:<gdal>-<proj>@sha256:...), or `geospatial-source`, which builds them
# here. Versions and checksums come from ci/versions.env (scripts/check_ci_config.py).
ARG GEOBASE=geospatial-source
# The Rust toolchain, equal to rust-toolchain.toml (ci/versions.env RUST_VERSION).
ARG RUST_VERSION=1.100.0

# The source tarballs, checked by SHA-256. CI replaces this stage with its cached copy of the
# same two files (--build-context geospatial-tarballs=DIR), which geospatial-source checks
# again, so a download.osgeo.org outage does not fail the build.
FROM scratch AS geospatial-tarballs
ARG PROJ_VERSION=9.8.1
ARG GDAL_VERSION=3.12.4
ADD --checksum=sha256:af5b731c145c1d13c4e3b4eeb7d167e94e845e440f71e3496b4ed8dae0291960 \
    https://download.osgeo.org/proj/proj-${PROJ_VERSION}.tar.gz /proj.tar.gz
ADD --checksum=sha256:68844ae29557b7efae4292c3b4cb3a3b8a79d14b765b89c5a7b17cbae7fa715a \
    https://download.osgeo.org/gdal/${GDAL_VERSION}/gdal-${GDAL_VERSION}.tar.gz /gdal.tar.gz

FROM debian:trixie-slim AS geospatial-source
ARG PROJ_VERSION=9.8.1
ARG PROJ_SHA256=af5b731c145c1d13c4e3b4eeb7d167e94e845e440f71e3496b4ed8dae0291960
ARG GDAL_VERSION=3.12.4
ARG GDAL_SHA256=68844ae29557b7efae4292c3b4cb3a3b8a79d14b765b89c5a7b17cbae7fa715a
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake g++ make pkg-config sqlite3 libsqlite3-dev libtiff-dev libgeos-dev \
    && rm -rf /var/lib/apt/lists/*
COPY --from=geospatial-tarballs /proj.tar.gz /tmp/proj.tar.gz
RUN echo "$PROJ_SHA256  /tmp/proj.tar.gz" | sha256sum -c - \
    && tar -xzf /tmp/proj.tar.gz -C /tmp \
    && cmake -S /tmp/proj-$PROJ_VERSION -B /tmp/proj-build -DCMAKE_BUILD_TYPE=Release \
       -DCMAKE_INSTALL_PREFIX=/usr/local -DBUILD_TESTING=OFF -DBUILD_APPS=OFF -DENABLE_CURL=OFF \
    && cmake --build /tmp/proj-build -j4 && cmake --install /tmp/proj-build \
    && install -D -m 0644 /tmp/proj-$PROJ_VERSION/COPYING /usr/local/share/doc/rusty-tiles-system/proj/COPYING \
    && rm -rf /tmp/proj-build /tmp/proj-$PROJ_VERSION /tmp/proj.tar.gz
COPY --from=geospatial-tarballs /gdal.tar.gz /tmp/gdal.tar.gz
COPY scripts/build-native-gdal.sh /tmp/build-native-gdal.sh
RUN echo "$GDAL_SHA256  /tmp/gdal.tar.gz" | sha256sum -c - \
    && tar -xzf /tmp/gdal.tar.gz -C /tmp \
    && /tmp/build-native-gdal.sh /tmp/gdal-$GDAL_VERSION /tmp/gdal-build \
    && install -D -m 0644 /tmp/gdal-$GDAL_VERSION/LICENSE.TXT /usr/local/share/doc/rusty-tiles-system/gdal/LICENSE.TXT \
    && rm -rf /tmp/gdal-build /tmp/gdal-$GDAL_VERSION /tmp/gdal.tar.gz

FROM ${GEOBASE} AS geospatial-build

FROM rust:${RUST_VERSION}-trixie AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
    libclang-dev libsqlite3-dev libtiff-dev libgeos-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
COPY --from=geospatial-build /usr/local/ /usr/local/
RUN ldconfig
ENV LIBSQLITE3_SYS_USE_PKG_CONFIG=1
WORKDIR /src
# Keep fixture, documentation and CI-only edits out of the release build cache.
COPY Cargo.toml Cargo.lock build.rs ./
COPY src/ src/
COPY vendor/ vendor/
COPY bindings/python/Cargo.toml bindings/python/Cargo.toml
COPY bindings/python/src/ bindings/python/src/
COPY preview/ preview/
COPY docs/schema/ docs/schema/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release --features native-geospatial \
    && mkdir /out-bin \
    && cp target/release/rusty-tiles /out-bin/rusty-tiles

FROM debian:trixie-slim AS native-runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libstdc++6 libsqlite3-0 libtiff6 libgeos-c1t64 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=geospatial-build /usr/local/lib/libproj.so* /usr/local/lib/
COPY --from=geospatial-build /usr/local/lib/libgdal.so* /usr/local/lib/
COPY --from=geospatial-build /usr/local/share/proj /usr/local/share/proj
COPY --from=geospatial-build /usr/local/share/gdal /usr/local/share/gdal
RUN ldconfig && ! command -v python && ! command -v python3
ENV PROJ_DATA=/usr/local/share/proj PROJ_NETWORK=OFF

FROM native-runtime AS runtime
COPY --from=build /out-bin/rusty-tiles /usr/local/bin/rusty-tiles
# Licence and upstream notices of vendored code (scripts/third_party_notices.py).
COPY LICENSE /usr/share/doc/rusty-tiles/
# Rust crates (scripts/third_party_licenses.py) and the GDAL/PROJ licences from the source builds;
# the Debian libraries keep their copyright files under /usr/share/doc/<package>/.
COPY licenses/THIRD_PARTY_LICENSES-native-geospatial.md /usr/share/doc/rusty-tiles/THIRD_PARTY_LICENSES.md
COPY --from=geospatial-build /usr/local/share/doc/rusty-tiles-system/ /usr/share/doc/rusty-tiles/third_party/system/
COPY vendor/basis_universal/LICENSE vendor/basis_universal/NOTICE /usr/share/doc/rusty-tiles/third_party/basis_universal/
COPY vendor/basis_universal/LICENSES/ /usr/share/doc/rusty-tiles/third_party/basis_universal/LICENSES/
COPY vendor/basis_universal/zstd/LICENSE /usr/share/doc/rusty-tiles/third_party/basis_universal/zstd/
RUN rusty-tiles doctor --json
ENTRYPOINT ["rusty-tiles"]
