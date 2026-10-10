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
- `freeze.json`: hashes of the reviewable contract/preparation inputs. Hashes establish identity, not correctness.

Performed: bounded source/document reads and SHA256 inventory; no Cargo, build,
install, converter/producer, browser, heavy resource execution, Git commit or
external message. No broad gate/release acceptance is claimed. The coordinator
alone schedules later heavy verification at two workers/nice 10.
