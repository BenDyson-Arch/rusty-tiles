### Changed

- CI: the dev and test profile builds the crate at opt-level 1 and its dependencies at opt-level 2, with debug assertions and overflow checks still on. The geometry-heavy large-mesh tests ran unoptimized and took most of the Rust job, which hit its 50-minute limit on #296. The library unit tests drop from 10–12 minutes to 1–1.5 on a 4-job host, and `target/debug/rusty-tiles`, which the oracle steps run, is faster too. Release builds are unchanged.
