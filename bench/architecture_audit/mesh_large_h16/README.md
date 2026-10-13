# H16 delivery preparation

The [contract](../../../docs/architecture/h16-delivery-contract.md) defines the
streamed Tiles 1.1 implicit forest and 3TZ delivery boundary. A test-only module
checks H1 packed/synthetic addresses, coordinate-bound conversion, exact member
count arithmetic and the explicit pending-publication error. Four preparation
tests and all-target Clippy pass; a fresh nonauthor review accepts this scope.

[preparation-acceptance.json](preparation-acceptance.json) binds the exact source
and cached test/review receipts. Bulk evidence is retained under
`~/.cache/rusty-tiles-evidence/h16-delivery-20261013/` by SHA-256.

Publication wiring waits for M1 and H8 to merge. It must import the actual shared
serializer types. No hierarchy output, archive streaming, C2 validation, viewer
traversal or resource acceptance is established by this preparation.
