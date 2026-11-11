### Fixed

- ARM builds (Linux and macOS) produce the same mesh simplification as x86-64: the bundled
  C/C++ libraries are now compiled without fused multiply-add contraction, which GCC and Clang
  enable by default on aarch64 and which changed meshoptimizer's collapse choices. x86-64
  output is unchanged.
- CI: the Windows library tests compile again (a Unix-only CPU-time probe is now gated to Unix),
  and the Python HLOD-expectation test declares the texture rule an HLOD tileset now requires.
- CI: the installed-wheel tests read repository text as UTF-8, so they pass on Windows, whose
  default encoding is not UTF-8.
- Windows: scratch-volume checks on a file (such as the HLOD audit sidecar) query its
  directory, because `GetDiskFreeSpaceExW` refuses a file path.
- CI: the independent HLOD oracle's Node decoder runs on Windows (its entry-point check compared
  a Windows path with a file URL path), and the native-geospatial and portable CLI jobs get
  40 and 60 minutes, which the slower runners need for the full suite.
