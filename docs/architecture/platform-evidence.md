# Publication primitive evidence and limits

Audit date: 2026-10-09. This supports the proposed foundation scope in #113;
it does not certify the current publisher or select a complete replacement.
The source baseline locks `tempfile` 3.27.0. Baseline executable probes used
Rust 1.98.0; the current stable Rust documentation consulted below is 1.99.0,
so implementation acceptance still requires the selected toolchain/platform.

## Separate the properties

A single `atomic` flag would conceal distinct obligations:

| Property | Required meaning | What does not establish it |
| --- | --- | --- |
| No-clobber | Create-new never overwrites a destination inserted by another writer | An earlier existence check |
| Complete visibility | A newly installed artifact is complete to readers | Merely creating the destination before filling it |
| Replacement outcome | The new artifact is installed, or the old artifact/recovery location is accounted for | A generic error string after moving the old path |
| Cleanup | Private temporary names are removed or retained locations are reported | Assuming `Drop` cannot fail |
| Crash durability | Committed data/names survive the explicitly covered failure model | Flush, rename or successful return considered in isolation |

These are contract obligations, not five user-facing switches. F0 implements
and tests a precise completed-file publication contract; it does not advertise
power-loss durability or a universal filesystem transaction. Directory
replacement remains a separate scoped implementation.

## Primary-source constraints

Rust documents `fs::rename` as replacing an existing target where permitted;
on Unix a directory target may be replaced if it is empty. It is not a
create-new primitive. The deterministic directory probe is consistent with
this documented behavior. [Rust `fs::rename`](https://doc.rust-lang.org/stable/std/fs/fn.rename.html).

The locked tempfile API says `persist_noclobber` never overwrites an existing
target, but warns that moving the temporary name is not universally atomic and
may leave an extra hard link. Its `persist` replacement does not itself sync
the file or containing directory; `close` exposes deletion errors that implicit
cleanup would hide. These distinctions must remain visible in the design.
[tempfile 3.27.0 `NamedTempFile`](https://docs.rs/tempfile/3.27.0/tempfile/struct.NamedTempFile.html).

Microsoft documents that `MoveFileExW` directory moves stay on the same drive,
and `MOVEFILE_REPLACE_EXISTING` fails when the target is an existing directory.
File replacement must not be generalized into an identical directory guarantee.
This is not a claim about every Windows filesystem/API option.
[Microsoft `MoveFileExW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).

## Consequences for a clean foundation

- Stage on the destination filesystem; do not add a silent copy/delete fallback
  under a promise of complete publication. Unsupported guarantees fail before
  attempting weaker behavior.
- Select create-new versus replace once, using one publication-policy enum.
  Encoders do not recheck `force`, decide filesystem semantics or inspect the
  converter's identity.
- Hide platform primitives behind a small internal publisher boundary. Its
  outputs describe committed, not-committed or recovery-needed state; never
  reconstruct that state from message text or a second existence check.
- Treat leftover temporary names after successful publication as cleanup
  diagnostics. They cannot turn a committed core outcome into an ordinary
  failed conversion. Conversely, do not report success just because a target
  happens to exist after a failed primitive.
- Seal the completed artifact only after its required writes/finalizers and
  event producers finish. A sealed Rust type enforces ownership in our API;
  it is not an OS lock against arbitrary external writes.
- Keep no-replace conflict, unsupported capability and resource failure distinct.
  A platform branch at this boundary is justified; copying it into each
  converter or adapter is not.

The state model explores the logical contract under explicit primitive
assumptions. The F0 implementation must refine that model with actual pinned
library/OS behavior, targeted competing-writer tests and filesystem error
injection on supported platforms. Model success does not discharge those tests.
