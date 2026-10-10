# Independent stored-archive acceptance fixtures

2026-10-10. This lane owns only this new subtree. It supplies independently
authored tiny archives and a raw framing/CRC/3TZ-index reference for the
[stored-archive foundation](../../../../docs/architecture/archive-read-foundation-contract.md).
Production implementation and final candidate executions are pending the
coordinator's exact source/binary pins. The full A2 rewrite remains held at its
separate representation/conformance gate.

## Independent truth and final fixture matrix

[probe.py](probe.py) directly authors and reads stored ZIP records with Python
`struct`, binascii CRC32 and hashlib MD5. It imports no production decoder,
coordinator probe, `zipfile` reader or implicit expansion. Its GLB is a separately
authored one-point static scene at the origin; its manifest references that GLB.
No native/vector producer, Cargo, install, browser or Git ran in this lane.

The active matrix has **45 archives**, each 915–1,406 bytes: 14 admitted,
24 malformed (`InvalidInput`) and seven outside the finite profile
(`Unsupported`). Every fixture independently agrees when read from an in-memory
stream and a held file. [results.json](results.json) records literal external
archive paths, byte lengths, SHA256s, expected categories and available frozen
CLI comparisons matched by exact archive hash, not merely case label.
The final preparation execution is
`/tmp/rusty-tiles-archive-read-independent-final3/receipt.json`, with driver SHA256
`f55837402c63c61241934fd5ce510fece38ba71f208b80f8e6f9c824b905cd89`.
The raw archives remain in that directory; no generated ZIP/3TZ binaries are
tracked. Exact source bytes and commands for every retained execution are bound
in the lossless JSON receipts listed below.

| Group | Independent coverage |
| --- | --- |
| Physical records | Adjacent records, padding gaps, reordered central entries, nested GLB local record in carrier data or extra-header data, partially overlapping data ranges, duplicate local offset, and separate data/header crossings into the central directory. |
| Stored/ZIP64 sizes | Classic local sizes; ZIP64 local sizes; ZIP64 central and EOCD sizes/counts; central offset-only sentinel without a descriptor. |
| Descriptors | Signed/unsigned 32-bit and 64-bit forms; local ZIP64 extra with no sentinel and 64-bit descriptor; malformed local ZIP64 plus 32-bit descriptor; central-only ZIP64 with bit3 is explicitly unsupported. Missing/truncated descriptors are malformed. |
| CRC/signature | A separately solved four-byte resource has actual CRC32 `0x08074b50`; its unsigned descriptor is valid, despite beginning with the signature word. Four isolated descriptor-word controls exercise one fitting end, two fitting ends, all ends crossing another record, and all ends crossing the central boundary. |
| Extras/identity | Local/central truncated TLV header/body without sentinels, duplicate ZIP64 tags without sentinels, missing required local/central ZIP64 values, opaque well-framed unknown tags, raw name/size disagreement, duplicate logical records with short or full indexes, and local/central Unicode Path overrides. |
| 3TZ index | Exact cardinality, too-short/long index, reversed ordering, incorrect hash/name association, bad offset and wrong raw stored CRC. |
| Resource/I/O models | At and one beyond tiny archive/central/count/member/aggregate-stored ceilings; injected EIO objects in the independent source model; malformed truncation remains distinct. These are not Rust allocation/I/O acceptance. |

The ordinary nested-ZIP case stores a complete one-member ZIP as a known
external GLB buffer. The buffer is declared and captured but not referenced by a
vertex view, so the GLB remains a valid minimal point scene. The outer archive's
only member extents are its own records. Both the reference and frozen C1 pass
it. This is a sensitive control against scanning opaque payload magic and
mistaking an embedded archive for overlapping outer members.

[check_crc.py](check_crc.py) is a supplementary raw stored-range checksum
reader, separately written from the admission reference. It verifies central
declared ranges without applying descriptor, identity or physical overlap
policy. All three nested/partially overlapping controls have independently
verified correct CRCs for every member, including both overlapping ranges.
Thus their profile refusal is not attributed to a hidden checksum defect.
The intentional index CRC control fails that supplementary checksum check.
This reader is not a general ZIP or glTF validator.

## Executed frozen behavior and defects

The frozen portable `4553533` binary has SHA256
`aa71fd57614844488d336dbc991df551fd9294c6d31d82027e1b4a43aedbf64b`.
There were exactly **45 serial frozen validations**, all at `nice -n 10`, with
`RAYON_NUM_THREADS=2` and a ten-second timeout. Subprocesses are reaped before
the next launch. There were no builds or sustained resource workloads.

Executed admission gaps:

* All three fully coherent header/data/partial-data overlaps return success.
* All six malformed or duplicate no-sentinel extra-field controls return
  success. Their independent TLV framing reference rejects them as malformed.
* Disjoint duplicate GLB and duplicate manifest records, each with a shortened
  index that covers only the last logical record, return success. The new
  profile requires duplicate raw names to fail before directory materialization.

Other observed disagreements are finite policy distinctions, not broader ZIP
conformance findings. The frozen path rejects local ZIP64-extra/no-sentinel
64-bit descriptors as malformed, while the settled new profile admits them.
It accepts a central-offset-only bit3 descriptor, while the new profile refuses
central-only ZIP64 descriptor width inference as `Unsupported`. A central-size
only descriptor is rejected as `InvalidInput` by the old path but is unsupported
under the new policy. A local Unicode Path override returns success; the new
exact-raw-name profile excludes local and central overrides.

The full-cardinality duplicate GLB and the duplicate local-offset case return
old `InvalidInput` from 3TZ index cardinality. The central Unicode Path override
also fails at that later index check. Those failures **mask the format-owner
identity decision**; an identical CLI category does not demonstrate that a
duplicate gate ran before allocation. The final private-reader integration needs
separate source/instrumented acceptance to establish the earlier causal owner.

The historical batch was authored before the raw-name priority was settled.
Its exact [frozen-e2cba051.py](frozen-e2cba051.py) source and 43-case/40-validation
receipt remain retained. Five subsequent identity validations bind the separate
[frozen-identity-ee6f8d7.py](frozen-identity-ee6f8d7.py) source and 45-case receipt.
These sources remain historical; current expected categories are in the active
matrix. In particular the old duplicate-same-name/local-offset expectation was
superseded from physical `Unsupported` to earlier logical `InvalidInput`.

The earlier header-labelled central-boundary fixture shifted the data end;
the final matrix strengthens it so the variable header itself crosses the
central start. That final byte sequence has not been frozen-executed and must
be candidate-tested. Results match historical executions only by archive hash
so that label correction cannot silently reuse the weaker byte sequence's
receipt. Three redundant historical controls were removed when identity controls
were added, preserving the bounded active count of 45.

## Finite decisions and proof limits

The [primary APPNOTE 6.3.10](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT)
describes local/data/descriptor records, optional descriptor signatures and
ZIP64 fields. This lane applies the coordinator-set finite disjoint-extent and
exact raw UTF-8 identity profile; it does not infer universal ZIP rejection from
one library's behavior. Adjacent ends and padding are admitted. Well-framed
ordinary unknown extras stay opaque. Unicode Path `0x7075` is an explicit
unsupported identity override. Any separate AES/other semantic exclusion belongs
in the final contract and private-reader acceptance; the matrix does not claim
an AES test. In particular an opaque NTFS extra is not a reason to inspect an
embedded archive or invent its members.

Local ZIP64-extra presence selects a 64-bit descriptor even without sentinels.
Central-only ZIP64 with bit3 is unsupported rather than silently interpreted as
32-bit or labelled malformed. Central offset-only ZIP64 with no descriptor
remains admitted. These are bounded choices settled with the coordinator, not
source-code-derived specifications.

A physical header/data intersection between coherent distinct records is
`Unsupported`; malformed bounds or truncation into the central/source region is
`InvalidInput`. Duplicate raw logical names are `InvalidInput` first. Candidate
descriptor ends are compared against the next physical local start and central
start: exactly one fitting end wins, multiple fitting ends or all matching ends
crossing a coherent local boundary are unsupported, and no matching end or
central/source truncation is malformed.

Two matching signed/unsigned ends require a size equal to the signature word,
about 128 MiB. The lane avoids manufacturing a large resource test. Its isolated
four-word controls prove the finite candidate-selection arithmetic only; they
are **not full archives**, and must be complemented by the private Rust helper's
independent tests. All actual tiny 32/64 descriptor forms and the signature-CRC
archive remain full byte-level fixtures.

The reference deliberately admits only the literal fixture forms: no EOCD
comments, ZIP64 end extensions, compression/encryption, or arbitrary name
encoding. It is not the product's reader or its resource implementation.
Python starts with already-created tiny byte arrays. Limit traces show modeled
gate order but cannot prove Rust allocation, table scaling, pre-allocation
failure, bounded read behavior or OS I/O origin. EIO is injected into the model,
not into the production kernel path. No whole-operation RSS, scratch or descriptor
count claim follows. The real held-file/Cursor, limit, I/O, CRC and allocation
instrumentation gates remain requirements for final private-reader acceptance.

## Candidate stop and retained receipt bindings

The candidate invocation must be supplied the coordinator's exact binary hash
and source-pin JSON hash. The driver checks those received artifact identities,
records the opaque source pin and tests all 45 categories; it writes a full
receipt before failing on a disagreement. A separate nonauthor final review must
independently bind that source pin to the compiled source and executed binary;
the driver's receipt of an opaque coordinator assertion is not that proof.

The resulting C1 integration is expected to use one native stored catalog and
reader from the raw admitted ranges. A downstream ZIP metadata fallback would
reintroduce competing identities and interpretation. Direct member/index reads
must preserve causal I/O versus malformed CRC/truncation, and complete C1 member
reads still own full member CRC verification. Source/final staging identity,
before/after file stability, JSON/geometry/metadata policies and public reports
remain their existing owners. None of these archive fixtures proves decoded
content bounds, frame accuracy, implicit conformance, converter lifecycle,
publication, source retirement or release readiness.

Lossless receipts are retained as JSON gzip with zero gzip timestamp:

* `frozen-raw.json.gz`: first historical 43 fixtures and 40 CLI executions.
* `frozen-identity-raw.json.gz`: updated 45 fixtures and five new CLI executions.
* `independent-raw.json.gz`: final active 45-fixture Cursor/held-file reference.
* `crc-ranges.json.gz`: final independent stored-range checksum facts.
* `frozen-crc-ranges.json.gz`: checksum facts over both historical fixture sets.

Each compressed/expanded SHA256 and expanded length is recorded in
`results.json`; full receipts retain literal commands, stdout/stderr, external
archive hashes and source/driver pins. Final candidate data will be appended
only after the pinned implementation is supplied and executed.
