# F1c2 evidence storage

> **Bulk evidence moved out of the tree (2026-10-12).** 3 of 89 files remain:
> this README plus what CI or tests read. The full directory last existed at
> `e29cd19d2f236dc688e91a80a45170d4a573e20b`
> (`git archive e29cd19d2f23 bench/architecture_audit/f1c2 | tar -x`). Local archive:
> `~/.cache/rusty-tiles-evidence/architecture_audit/f1c2.tar.zst`, SHA-256
> `32a160a60731a81f5a5940f45dd29f4cd4aa153bbc14fad883dbc0d995f62e1a`.
> Paths below that no longer exist refer to that snapshot.

The 30 verbose JSON receipts are stored verbatim as adjacent `.json.gz` files.
[storage-index.json](storage-index.json) maps their original paths to storage
paths and records both uncompressed and compressed sizes and SHA256 values.
The smaller source/artifact manifests, browser ledgers, correction records,
specification snapshots, independent audits and replay drivers remain readable.
Existing compressed command logs are unchanged.

Compression uses `gzip.compress(original_bytes, compresslevel=9, mtime=0)`.
Every archive was compressed twice with identical results and decompressed with
byte-for-byte equality against its original receipt before the original file
was removed. No JSON was reformatted and no result, control, source/artifact pin
or driver pin was edited. The index records the compression runtime; future
compression runtimes need not produce the same compressed bytes to preserve the
original receipt identity.

Original `.json` names and SHA256 values embedded in historical ledgers and
correction records identify the **decompressed bytes**. Use the index to resolve
those logical names; do not replace historical receipt hashes with gzip hashes
or rewrite historical execution paths. The S3 browser ledger's cache names
`final-f1c1-browser.json` and `final-f1b3-browser.json` correspond to indexed
`receipts/f1c1-browser.json` and `receipts/f1b3-browser.json` respectively.
Candidate, feasibility, failed-control, S2 and final S3 receipts retain their
separate names, complete contents and proof limits. Historical external cache
paths and source-manifest receipt identities do not imply that those external
artifacts are committed here.

Verify the indexed archives without writing files:

```sh
python3 -B bench/architecture_audit/f1c2/evidence.py
```

Read a complete receipt directly:

```sh
gzip -dc bench/architecture_audit/f1c2/probes/picking-final.json.gz
```

Extract and verify its original bytes to a new file (existing files are refused):

```sh
python3 -B bench/architecture_audit/f1c2/evidence.py extract probes/picking-final.json --output /tmp/picking-final.json
```

The verifier checks gzip integrity, both sizes and hashes, JSON decoding, unique
index mappings and confined storage paths. It does not rerun the archived domain
checks or promote historical receipts to new source acceptance. For fresh
executions use the original driver commands in
[tests/f1c2_oracle.md](../../../tests/f1c2_oracle.md), the independent
[wrapping audit](audits/wrapping.md), and the retained scripts under
`review_probes/`. Their output arguments still write ordinary JSON files.
