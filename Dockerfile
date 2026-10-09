# syntax=docker/dockerfile:1
# Matching Debian ABI, linked GDAL/PROJ SDK and a Python-free runtime.
FROM debian:trixie-slim AS geospatial-build
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake g++ make pkg-config sqlite3 libsqlite3-dev libtiff-dev libgeos-dev \
    && rm -rf /var/lib/apt/lists/*
ADD --checksum=sha256:af5b731c145c1d13c4e3b4eeb7d167e94e845e440f71e3496b4ed8dae0291960 \
    https://download.osgeo.org/proj/proj-9.8.1.tar.gz /tmp/proj.tar.gz
RUN tar -xzf /tmp/proj.tar.gz -C /tmp \
    && cmake -S /tmp/proj-9.8.1 -B /tmp/proj-build -DCMAKE_BUILD_TYPE=Release \
       -DCMAKE_INSTALL_PREFIX=/usr/local -DBUILD_TESTING=OFF -DBUILD_APPS=OFF -DENABLE_CURL=OFF \
    && cmake --build /tmp/proj-build -j4 && cmake --install /tmp/proj-build \
    && rm -rf /tmp/proj-build /tmp/proj-9.8.1 /tmp/proj.tar.gz
ADD --checksum=sha256:68844ae29557b7efae4292c3b4cb3a3b8a79d14b765b89c5a7b17cbae7fa715a \
    https://download.osgeo.org/gdal/3.12.4/gdal-3.12.4.tar.gz /tmp/gdal.tar.gz
COPY scripts/build-native-gdal.sh /tmp/build-native-gdal.sh
RUN tar -xzf /tmp/gdal.tar.gz -C /tmp \
    && /tmp/build-native-gdal.sh /tmp/gdal-3.12.4 /tmp/gdal-build \
    && rm -rf /tmp/gdal-build /tmp/gdal-3.12.4 /tmp/gdal.tar.gz

FROM rust:1.98-trixie AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
    libclang-dev libsqlite3-dev libtiff-dev libgeos-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
COPY --from=geospatial-build /usr/local/ /usr/local/
RUN ldconfig
ENV RUSTY_TILES_DISABLE_NATIVE_JPEG=1
ENV LIBSQLITE3_SYS_USE_PKG_CONFIG=1
WORKDIR /src
# Keep fixture, documentation and CI-only edits out of the release build cache.
COPY Cargo.toml Cargo.lock build.rs ./
COPY src/ src/
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
RUN rusty-tiles doctor --json
ENTRYPOINT ["rusty-tiles"]
