### Changed

- CI: the Rust toolchain is pinned to 1.100.0 in `rust-toolchain.toml` and `ci/versions.env` instead of following stable, so a new Rust release can no longer fail every pull request with new lints. Every workflow installs the pinned toolchain, the wheel builds pass it to the maturin action, and the native container builds with `rust:1.100.0-trixie` (it used 1.98). `scripts/check_ci_config.py` fails when any of them differs or a workflow installs stable Rust. The toolchain is bumped in its own pull request.
