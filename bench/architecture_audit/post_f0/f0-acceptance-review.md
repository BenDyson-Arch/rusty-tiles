# F0 final acceptance review

Baseline: d69ba2a7facb99b5b3f5a6efba4e4039bef26be5. Scope: promised opaque
regular-file packaging, single-run observations/failure arbitration, completed
stored ZIP/3TZ serialization and file installation. Scene closure, arbitrary
filesystem interference, source snapshotting, directory replacement and crash
durability are not acceptance requirements of this slice.

## High severity: relative output can redirect after validation (fixed in `c777180`, verification recorded separately)

`src/package.rs:232` computed a resolved output for source-overlap checks, but
baseline staging (`:438`) and install (`:483`) used the original request path.
A synchronous observer can call set_current_dir after read-only resolution.
With named members, a stable selected source may have a physical filename
`out.3tz` while its archive name is `tileset.json`; such a source is valid.
A relative Replace destination `out.3tz`, initially outside and distinct from
that source, then resolves into it after callback CWD mutation. The publisher
replaces the source and returns success. The source remained stable during
reading; this is a publication redirection defect, not a snapshot requirement.

Independent isolated Rust executable `/tmp/f0-cwd-acceptance-probe.rs` used the
public API against the existing built library. Before fix:

```
result_ok=true initial_destination_exists=false selected_source_preserved=false selected_source_now_zip=true result_output=Ok("out.3tz")
```

After focused fix, repeating the same isolated executable against rebuilt core:

```
result_ok=true initial_destination_exists=true selected_source_preserved=true selected_source_now_zip=false result_output=Ok("/tmp/f0-cwd-probe-3692688/initial/out.3tz")
```

Fix is limited to `src/package.rs`: retain the checked resolved absolute output
in Resolved before callbacks/staging. All later file side effects and result
identity use that path. `tests/package.rs` adds a self-subprocess regression,
so process-global CWD mutation cannot interfere with parallel tests. It verifies
selected source preservation, intended destination, resolved result identity,
and independently readable archive payload. Existing result-path assertion now
uses canonical output for macOS /private and Windows verbatim path normalization.
No commit made. Parent owns documentation, adapter assertion changes and CI.

Validation: `cargo test --locked --test package` with shared target,
CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=2, dev/test debug=0: 12 passed, 0 failed.
`git diff --check` passed. Cross-platform reruns are pending this final patch.

## Remaining acceptance review

No additional must-fix production defect established. Read source and existing
evidence/tests for these properties; no new broad suite rerun in this pass:

- `runtime.rs:271–291,379–395`: first fatal cause remains primary before
  permission; later distinct causes become secondary. Publication failure after
  permission is separately recorded. Attempt lifetime finalizes unused runs.
- `runtime.rs:313–336,339–375,415–449`: observations run outside gate lock;
  admission is counted and completion records error before releasing admission;
  seal/permission require closed admission with no active callbacks. Package is
  synchronous and performs every callback before sealing.
- `runtime.rs:453–520`: writable staging and sealed TempPath borrow the original
  Attempt; sealing consumes writable storage, syncs it and closes the File.
  F0 producer does not clone the File or run background writers. Future consumers
  must prove their own worker and alias lifetimes rather than infer them here.
- `runtime.rs:544–604,620–653`: installation permission is separate from actual
  tempfile persist/persist_noclobber success. Failure cleans owned candidates;
  unsuccessful cleanup retains actionable paths. After success a surviving
  temporary name is diagnosed, never unconditionally unlinked.
- `archive3tz.rs:95–218,224–334`: stored local headers carry known actual sizes
  and backpatched streaming CRCs; index is final, MD5 tuples sorted nondecreasing;
  central ZIP64 handles count/offset sentinels. Per-chunk/index/member/central
  checkpoints propagate observer/cancellation errors. There is no finalizing
  Drop or stderr on this serializer's failure path.
- `package.rs:103–179,228–385`: pure named-request/container validation precedes
  read-only resolution; symlink/nonregular source admission, UTF-8 names,
  manifest/index/name/size constraints and canonical/hardlink output overlap
  checks precede scratch creation. Source size/mtime checks are expressly partial.
- CLI adapter uses fallible package progress transport (`main.rs:640–652`) and
  builds summaries from returned receipt, with no artifact reread. Python
  adapter waits for synchronous callback/signals and preserves original callback
  exception identity (`bindings/python/src/lib.rs:159–179,570–599`), with no
  postcommit callback/check_signals in this operation.

Existing runtime tests cover competing create-new, replacement/failure,
pre/post-permission cancellation, first cause, active observer refusal, reentry,
run reuse, sync/install/cleanup injection, consumed publication and reoccupied
postcommit names. Existing independent codec tests cover actual local/index
CRC/size and ZIP64 count/offset boundaries, including persistent payload,
central-directory and flush faults through package abort cleanup.

Limits remain explicit: OS/filesystem calls can block and cancellation is
cooperative; inventory/index memory scales with member count; callback panic is
outside the Observer contract; sources must stay stable; package success does
not certify tileset/resource semantics. Legacy converters do not use F0 and
remain outside these acceptance claims. A green prior CI matrix does not cover
the newly fixed output binding until its rerun completes.

Coordinator note: the focused fix and cross-platform path assertions are committed
in `c777180f178df5cc2f524f7b20f4251ba9d522e0`. The durable isolated source is
[f0_cwd_probe.rs](f0_cwd_probe.rs); the maintained subprocess regression is in
`tests/package.rs`. Final suite/CI evidence belongs to `docs/architecture/f0-evidence.md`.
