# Independent D2 design review (#118)

Reviewed 2026-10-09 against issue #118, the F0 lifecycle and D1 directory publisher.
This note separates the chosen contract from execution evidence. Source observations
and injected controls are not real OS failure observations.

## Chosen policy and its limits

Replace retires any final-path leaf present at hold, including a regular file or
symlink itself; it does not follow a symlink referent. This explicit policy avoids
the unsound alternative of claiming directory-only source admission from an earlier
path metadata check. Exclusive rename constrains the destination slot, not the
identity or type of its source. Replace is current-at-hold, not a preflight snapshot.

Hold, install and restore each use the admitted exclusive primitive. Their safety
properties are separate: holding cannot overwrite an occupied recovery slot,
installation cannot overwrite a competing final entry, and restoration cannot
overwrite a competitor. These operations do not serialize a whole replacement,
reserve an absent destination, prevent an unrelated writer modifying held contents,
or provide crash durability. Parent/mount topology and private-container ownership
remain stable-environment assumptions.

Permission precedes hold. After permission wins, cancellation cannot abandon the
recovery sequence. Successful candidate installation is the sole commit point;
cleanup failure after it is success with diagnostics. A previous output is a recovery
asset, never generic failed scratch. Disable its container's destructor cleanup before
holding old data, including unwind paths. Failed restoration exposes typed output and
previous-output paths; the original install error remains primary and restore errors
remain secondary. Successfully restoring the held entry proves exact bytes/inventory
under source stability, not a fresh snapshot of a mutating tree.

## Concurrent schedule obligations

A holds O; B sees an absent output and installs B; A's installation and restoration
both conflict. B must survive and O must remain at A's actionable recovery path.
A later replacement may instead hold an earlier committed candidate and replace it.
Thus two concurrent Replace attempts can both succeed at distinct commit points;
a receipt cannot promise that its installed tree remains at the output until return.
Neither a process mutex nor unscheduled successful stress runs prove this policy.
The deterministic A/B schedule requires a separate-process control using the actual
exclusive primitives. Stable identity/snapshot replacement or global serialization
would require a stronger mechanism and a different contract.

## Independent evidence ownership

`tests/d2_directory_oracle.py` uses the D1 handwritten TIFF generator and independent
PNG decoder, while independently asserting D2 publication behavior. It checks complete
analytic pixel inventory, six existing-leaf cases, symlink referent preservation,
CreateNew conflict and producer-refusal preservation, and eight actual concurrent
CLI subprocess races. Those races are explicitly unscheduled; they supplement the
runtime's deterministic hold/install/restore controls and do not replace them.
Execution results must identify the candidate hash and source manifest. No silent
native-feature or platform skip counts as passing evidence.

## Required source and fault audit

Review separate hold failure, hold ENOENT, occupied backup slot, install failure plus
successful restore, restore I/O failure, and empty/nonempty/file/symlink restore
competitors. Verify exact original inventory/mode on restoration and machine-readable
recovery paths when restoration fails. Verify successful commit despite old-output
cleanup failure, secondary retained scratch on candidate cleanup failure, and no
implicit destructor retry. Test cancellation on both sides of permission, no late
observer calls, producer finalization before seal, and unwind after hold retaining old
data. Reused former candidate/previous names must not be recursively deleted after
those objects have moved. Frontends must preserve the same primary/secondary/recovery
meaning as the runtime.

## Implementation review and preliminary execution

Source review found the proposed custody rule implemented: candidate and recovery
container relinquish TempDir destruction before namespace moves; failed restoration
returns DirectoryRecovery, and cleanup after successful restoration only removes
an empty holder. Recursive retirement occurs only after candidate installation
commits. The main path preserves the original install error and keeps restore
failure secondary. Permission precedes hold, and no user observation runs afterward.
Under the documented stable parent/private-container assumptions, this review found
no destructive namespace race defect.

The preliminary real native CLI run passed six replacement-leaf cases, six
preservation checks and eight concurrent subprocess races. Its receipt is in
`d2-oracle.json`; a final copied candidate and source manifest must supersede this
preliminary receipt before acceptance. Concurrent failed restoration, when observed,
requires an intact exact old inventory or a complete known analytic candidate at
the typed previous path; unexplained private directories fail the oracle.

Source-proof gaps reported for follow-up: the initial restoration competitor test
covered a nonempty directory, not empty directories/files/symlinks; cancellation
following permission was covered for CreateNew but initially not explicitly for
Replace hold/restoration; occupied hold-slot errors were initially injected rather
than obtained from the real exclusive primitive. These are evidence gaps rather
than demonstrated implementation failures.

The initial CLI recovery mapping used lossy display strings even for accepted
non-UTF8 native paths. Rust PathBuf and Python's native-path conversion preserve
those paths. An actionable CLI recovery contract needs exact native-byte/unit
transport or narrower accepted paths; documentation of a lossy convention alone
cannot make a corrupted path usable. This concern was sent to the implementation
owner; the final disposition and execution evidence must be recorded separately.

## Final source re-review disposition

The follow-up source includes actual exclusive-rename hold-slot collision control,
empty-directory/file/Unix-symlink restore competitors, and explicit Replace
cancellation controls through hold, install and failed-install restoration. These
close the identified source-test coverage gaps; parent-run execution results remain
separate evidence until reported. The CLI now supplements display strings with
`nativePaths`: exact `unix-bytes` arrays or `windows-utf16` unit arrays. The Unix
non-UTF8 mapping control calls the actual error-summary helper. Windows UTF-16
execution must be reported from Windows rather than inferred from Unix.

No remaining source blocker was found within the chosen current-at-hold, any-leaf,
nonserialized contract. Platform-specific rename/cleanup behavior and injected
fault execution still need their recorded test results. Source inspection alone
does not prove them. File/symlink retirement is independently exercised through
the real CLI; exact held-file/symlink restoration identity has not been separately
executed by this external harness. The initial eight unscheduled CLI races all
returned two successful commits and produced no actual recovery receipts. Thus
they prove allowed sequential/interleaved replacement outcomes, not the failed
restore frontend transport; deterministic runtime schedules and frontend mapping
controls carry that separate obligation. Final candidate replay remains with the
parent, and any updated receipt must include its binary hash.

## Final Linux replay

The parent replayed both independent scripts against the rebuilt candidate from
`0e57f4b` (implementation `4ee13d5`); source and binary hashes are recorded in
`d2-source-manifest.json`. The final D2 receipt has six replacement leaves, six
preservation checks and eight cross-process races. The unchanged raster profile
also passes all 40 positives, 38 refusals and three sensitivity controls in
`d2-raster-regression.json`. Runtime final replay passes 32 focused tests,
including separately held file/symlink restoration and original directory modes.
Full-suite and frontend-mapping results are recorded separately in
`d2-implementation-results.json`. Windows-specific symlink and UTF-16 controls
remain dependent on CI execution; Linux success does not qualify them.

## Source/output overlap review correction

The earlier "no remaining source blocker" assessment missed a consumer-level
precondition: Replace must not retire the source itself or a directory containing
it. The review on #130 identified this destructive admission gap. Independent
reproduction in `d2-overlap-before.json` records candidate
`4856a19d5670acc83235b91bc5041faca9afb98913acccacc61696ecd0c6e075` returning
success for both identical source/output and source-inside-output. Both runs
replaced/deleted the source. These were executed defects on disposable fixtures,
not hypothetical namespace races or injected I/O failures.

The external oracle now requires InvalidRequest / exit 2, exact input bytes and
complete original inventory, and no hidden staging/recovery work for identical
paths, containing output directories, source/output parent symlink aliases and
hardlink aliases. It also positively requires replacement of a final-leaf symlink
to the source file or containing source directory to preserve the referent. The
source canonical path must be compared with the bound target leaf without
following that leaf's symlink referent; otherwise the safe symlink cases would be
incorrectly rejected. The containing-directory positive supplies its source
through that very output symlink (`output/nested/source.tif`), proving canonical
source binding survives retiring the alias while referent bytes remain intact.

On Windows, merely appending the caller's leaf spelling to a canonical parent
and comparing path strings is insufficient evidence against case/alternate-name
aliases. Existing non-symlink directory identity must be compared with source
ancestry, or equivalent native normalization must be established. Existing regular
file hardlink identity is a separate admission check. Stable parent namespace and
stable input assumptions remain explicit; these checks are not a source filesystem
snapshot or identity-conditional rename against unrelated concurrent mutations.
Final corrected candidate replay is pending and must supersede the earlier
acceptance conclusion.

Corrective guard source re-review: the consumer inspects the bound output leaf
without following symlinks; existing regular files use native same-file identity,
and existing directories compare that identity against each canonical source
ancestor. This avoids path-spelling-only checks and covers hardlinks, parent
aliases and filesystem case aliases without retiring a source container. The
leaf-symlink early return preserves the intentionally safe symlink replacement
semantics. Admission occurs before staging, and the existing source/parent
stability assumptions remain necessary. No additional source defect was found
in that guard. Windows case-alias execution is not qualified by the Linux run;
its conditional oracle control is available but current native CI is Linux-only.

## Corrected overlap replay

The fixed candidate from `5c4bd6d` passes the complete independent D2 oracle:
six overlap refusals with exact inventory preservation, two safe final-link
positives (including input through the link), six general replacement cases,
six preservation checks and eight subprocess races. It also passes all 40
raster positives, 38 refusals and three sensitivity controls. The full native
suite and Clippy pass; `d2-overlap-results.json` records totals and log hashes.
`d2-overlap-source-manifest.json` verifies the tested source and candidate hash.
The original source-deletion reproductions remain recorded separately. This
supersedes the earlier no-blocker assessment; #130 remains held for review.
