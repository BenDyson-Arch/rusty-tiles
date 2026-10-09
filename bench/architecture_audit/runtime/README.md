# Publication and vector transaction probes

These standalone audit probes target baseline `8dfd74bd87dd23c86278e98d736c3f5912c246cf` on Linux. They are excluded from the normal test suite and make no production changes. Recorded runs used Rust/Cargo 1.98.0, default features, two build jobs, and development debug information disabled.

The [results](evidence/publication-vector.json) include exact commands, exit codes, stderr, binary and fixture hashes, archive inventories, conversion reports, and JSONL geometry reports. [Fixtures and actual converter archives](evidence/publication-vector-artifacts/) are retained. Each injected-source file has baseline and instrumented hashes in the [instrumentation manifest](evidence/publication-vector-instrumentation.json), with a [reviewable patch](evidence/publication-vector-instrumentation.patch).

| Probe | Method | Observed result |
| --- | --- | --- |
| Feature rollback side effects | Natural baseline converter; oversized square polygon with 5KB metadata plus one valid point; `maxBytes=4096`, `skipInvalid` | Success archive has one point, one skipped feature, and one fragment, but `fragmentedPolygons=1`; JSONL retains both a successful partition report and a skipped report for the wholly omitted polygon. SQL rollback does not roll back counters or report output. |
| Feature rollback strict control | Same natural input without `skipInvalid` | Data exit 3; no archive published. |
| Infrastructure error classification | Isolated full source copy; inject `Error::Io(ENOSPC)` in the real feature accept callback inside its savepoint | With `skipInvalid`, success archive omits the affected point and reports it as skipped. Without `skipInvalid`, data exit 3 rather than the original I/O category. |
| Infrastructure control | Unmodified baseline with the identical two-point fixture | Both points accepted; no skipped features. |
| Directory publication conflict | Execute the real copied private publisher; deterministically create a competing empty output directory immediately after `exists()` returned false | Publisher succeeds with `force=false`; final output inode equals staged directory inode and differs from competing directory inode. The competing empty directory was replaced. |
| Archive publication control | Execute real copied `Job` archive publication; create a competing file after job preflight | Output conflict; competing bytes preserved. |
| Encoded candidate cleanup control | Natural baseline converter; two points with 1.5KB metadata each; `maxBytes=4096` | Root reports `routingReason=bytes`; final archive has exactly two referenced leaf GLBs, zero unreferenced content members, and zero missing references. |
| Accepted inventory control | Natural baseline converter; 20 points; `maxVertices=4` | All eight packaged content members are referenced; no missing references. |

The ENOSPC failure is synthetic; the machine did not run out of disk. Only the portable reader's classification was executed. The directory race uses deterministic injected interleaving, not probabilistic scheduling or a naturally observed concurrent race. It executes the original publisher's check and rename, with a single intervening competing `mkdir`; source hashes and the patch show the change. The archive control exercises the real publisher, not a substitute ZIP implementation.

The cleanup controls contradict a claim that rejected candidate content necessarily leaks into final archives. They establish that these fixtures finalize without orphan content. They do **not** establish that cleanup is correct for all inputs, errors, crashes, or implicit hierarchies, nor measure temporary disk amplification. `encoding.rs` writes content before final budget acceptance; `reuse.rs` subsequently removes unreferenced content. Those broader behaviors remain untested here.

The rollback result demonstrates surviving side effects; a replacement contract must explicitly distinguish attempted operations from committed feature statistics. This audit does not assume that any existing algorithm, helper, transaction, or cleanup implementation deserves retention. The per-subsystem decision still requires independent correctness and scalability evidence.

## Reproduce

Run from the repository root with unchanged production source relative to the pinned baseline. Generated copies, binaries, and new results go into a fresh cache directory. The script refuses to overwrite an existing source destination and verifies baseline source before copying it.

```bash
AUDIT_WORK=$(mktemp -d "${XDG_CACHE_HOME:-$HOME/.cache}/rusty-tiles-113.XXXXXX")
CARGO_TARGET_DIR="$AUDIT_WORK/target" CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked --bin rusty-tiles
cp "$AUDIT_WORK/target/debug/rusty-tiles" "$AUDIT_WORK/baseline-rusty-tiles"
python3 bench/architecture_audit/runtime/prepare_publication_vector.py \
  --repo . --destination "$AUDIT_WORK/source" --evidence "$AUDIT_WORK/evidence"
CARGO_TARGET_DIR="$AUDIT_WORK/target" CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 \
  cargo build --locked --manifest-path "$AUDIT_WORK/source/Cargo.toml" \
  --bin rusty-tiles --example audit-publication
python3 bench/architecture_audit/runtime/run_publication_vector.py \
  --baseline-binary "$AUDIT_WORK/baseline-rusty-tiles" \
  --injected-binary "$AUDIT_WORK/target/debug/rusty-tiles" \
  --publication-binary "$AUDIT_WORK/target/debug/examples/audit-publication" \
  --work-parent "$AUDIT_WORK" --evidence "$AUDIT_WORK/evidence"
```

Python assertions check the observed audit behaviors. They deliberately demonstrate defects and controls; they are not desired-behavior regression tests and must not be added to the normal suite. The recorded command paths identify temporary work directories that have since been removed; archived outputs and fixtures are retained in the evidence directory.
