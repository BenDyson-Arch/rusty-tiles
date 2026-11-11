### Changed

- Release and development builds publish the CLI archives and Python wheels that the full CI run of the same commit built and tested, after smoke-testing each archive on its platform, instead of rebuilding them. They are rebuilt only when that run's artifacts have expired.
- The native container's `Dockerfile` takes PROJ and GDAL from a `GEOBASE` stage or image: `geospatial-source` builds them from their checked source tarballs as before, or a pinned `ghcr.io/bendyson-arch/rusty-tiles-geobase:<gdal>-<proj>@sha256:...` image provides them. The image contents are unchanged.
- While a release candidate is out, development builds are versioned between it and the next candidate (for example `0.4.1rc2.dev202611121734` after `0.4.1rc1`) instead of before every candidate, and replacing the `dev` pre-release no longer interrupts its downloads.
