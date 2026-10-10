# C1 payload-integrity preparation

These are contract preparation records, not implementation acceptance.
Baseline: `c8cbfb187de863ae21fe904275794894aa0a8381` on the isolated
`feat/c1-payload-integrity` worktree. The archive-reader PR remains separately
owned by the coordinator; acceptance/rebase to its exact final head is required.

- [Contract](../../../docs/architecture/c1-payload-integrity-contract.md): current C1 domain, owners, consumed limits, outputs, dispositions and independent acceptance plan.
- [Independent next-scope audit](next-foundation-audit-20261010.json): immutable byte-for-byte copy; SHA256 `07428a3642dc3c68683d2e5dcc054c6d4a4fb4c790e5db7f744c204433b67a50`.
- [Source inventory](source-inventory.json): file pins and statically observed live gates/callers; not executed defects.
- [Reference plan](reference-plan.json): independent authorities/golden provenance and explicitly incomplete source revalidation.
- [Adjudication](adjudication.md): questions requiring a nonauthor owner's preimplementation decision.
- [Independent selected-primary controls](primary_controls/README.md): separate owner verified the exact published source and executed fourteen tiny controls, demonstrating six valid-input rejection defects; this author only inspected those records.
- [Primary evidence binding](primary-evidence-binding-20261010.json): exact source/binary/receipt identities and six finite findings; this author verified all 108 raw-bundle manifest file hashes without executing fixtures.
- `freeze.json`: original preparation freeze, preserved unchanged with its complete input snapshot under `history/20261010-initial/`.
- `freeze-20261010-primary-controls.json`: revised contract/preparation hashes binding the new independent records. Hashes establish identity, not correctness.
- [Fresh pinned references](primary_controls/2026-10-10-pinned-reference-verification/verification.md) and [nonauthor dispositions](prereq_review/review.md): A9 provenance settled and concrete architecture choices recorded; A13/A14 still block a whole-slice coding decision.
- `prereq-disposition-binding-20261010.json` and `freeze-20261010-prereq-dispositions.json`: current revision binds those independent records; prior own-input snapshot retained under `history/20261010-post-primary/`.
- `unsigned-consumer-inventory-20261010.json`: static integer bypass/target comparison inventory; A14 remains unresolved across these actual consumers.

Performed by the contract author: bounded source/document/receipt reads and SHA256 inventory; no Cargo, build,
install, converter/producer, browser, heavy resource execution, Git commit or
external message. No broad gate/release acceptance is claimed. The coordinator
alone schedules later heavy verification at two workers/nice 10.
