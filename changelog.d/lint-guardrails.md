### Changed

- Development: `#[allow]` is denied in favour of `#[expect(lint, reason = ...)]`, `unsafe` blocks need `SAFETY` comments, and `scripts/lint_debt.py` stops library code from gaining lossy casts, `unwrap`/`expect`, hash-order iteration or calls to the methods banned in `clippy.toml` (environment reads, host CPU counts, worker pools, deletions, wall-clock time, `process::exit` outside their owners). No output change.
