# Additive F0 module-path preparation correction — unexecuted

The original37-file package remains immutable. Its root actual compile failed with E0583 before tests: explicit `#[path="src/runtime.rs"]` selected child lookup under src rather than the prepared runtime subdirectory. That failed source/compiler/output epoch remains `/tmp/rusty-tiles-vector-f0-execution`; it proves no F0 test behavior.

This new layout copies the unchanged accepted runtime bytes into `isolated/src/runtime/mod.rs`. The new entrypoint points there. Prepared tests.rs, accepted directory.rs/directory_platform.rs and prepared probe assertions are copied byte-identically. Child modules are now siblings of mod.rs. The test's `../../probes` include retains the same path meaning. No production runtime body, helper or test expectation changes.

Root must bind this source/integrity in a fresh output/dependency epoch, reuse the matched dependency recommendations and compile recipe, and execute exactly the same two selected Linux tests at nice10/two CPUs/thread1. The Windows test remains unexecuted native-CI input. Initial compile failure and old freeze are not rewritten or labeled pass. This package was prepared with source reads/copies/hashes only; no Cargo/compiler/native/target execution occurred. All original scope/ResourceLimit/new-capture/production/P2–P5 restrictions continue.
