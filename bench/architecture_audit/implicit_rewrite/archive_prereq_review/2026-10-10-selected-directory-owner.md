# Selected-directory ownership revision — 2026-10-10

Separate nonauthor design adjudication after the first candidate's independent
failure controls. The original [preimplementation review](review.md) and its
source inventory remain frozen. This addendum reviews a new representation;
it does not retroactively accept the discarded pass-through candidate.
[Dated input pins](2026-10-10-selected-directory-sources.json) distinguish live
candidate observations from the proposed successor. No production edits, Git,
Cargo, builds, external network or new CLI executions occurred in this review.

**Decision: one private `StoredArchive<R: Read + Seek>` built exclusively by the
existing format scanner is the smallest justified replacement. Implementation
may proceed after recording these decisions; production acceptance remains open.**
The scanner's admitted catalog must be authoritative for every subsequent C1
index, name, hash and member read. No downstream `ZipArchive` constructor or
alternative end-record/directory parser remains on that path.

The independent final-review receipt executes a coherent one-entry outer archive
whose payload contains a nested ZIP. Ordinary input is refused as one-entry 3TZ;
adding a fully framed empty NTFS tag makes frozen production accept the nested
two-entry directory. The reviewer also executed duplicate-name and raw-name/
ZIP64-override controls. These establish interpretation drift; they do not prove
an actual unbounded allocation. Source observations show the current candidate
still admits one interpretation then constructs another reader. Agreement checks
after the second constructor cannot prove admission before its allocation.

## Required finite representation and interfaces

- Construct the private archive from selected source, known length and the five
  caller-owned limits. Retain exact UTF-8 name, local offset, data offset, size,
  CRC and central order once. Private construction prevents callers forging
  admitted facts. Unknown well-framed extra tags stay opaque and cannot acquire
  new identity, length, offset or directory-selection meaning downstream.
- Expose only borrowed member facts/enumeration, typed exact-name lookup, complete
  bounded member reads and streaming hash/CRC needed by C1. Missing lookup is
  distinct from source I/O; only missing `tileset.json` maps to MissingManifest.
  Preserve existing finite Unicode/AES/surplus-ZIP64 dispositions without adding
  a timestamp/NTFS parser to satisfy the discarded ZIP implementation.
- Use this exact central order for the last-index rule, independent of physical
  sorting. Exact checked `24 * (entry_count - 1)` is the index length. Every row
  must match one non-index catalog offset and raw-name MD5, hashes stay ordered,
  duplicate offsets fail, and all non-index entries are covered exactly once.
  Streaming fixed 24-byte rows is sufficient; if bytes are materialized, admit
  that exact size before allocation. No second local-header/name interpretation
  is needed after the scanner verified them against central facts.
- Each data read stays inside its admitted stored range. Stop at exact size,
  without reading the next header; check zero-length CRC too. CRC mismatch and
  truncation are typed InvalidInput. Underlying seek/read InvalidData and other
  infrastructure errors stay Io, without message matching or parser retry.
  Success of a partial stream cannot certify the member CRC.

C1 can borrow its selected File into the archive, then drop that borrow for the
same before/after identity checks. It still owns safe path policy, JSON/payload
roles, resource closure, content hashes/reports and aggregate read-work policy.
The archive owns format ranges, exact lookup and CRC. Repeated/index/member reads
must consume the declared C1 work meter or have narrowly stated excluded
envelope/index work; a new archive-owned total-work policy is unnecessary.
Validated catalog facts do not make mutable source bytes an atomic snapshot.

## Admission, resource and legacy decisions

Check source length before tail allocation; selected directory range/count and
checked `entries * 46 <= directory_bytes` before catalog reservation. Check host
range/allocation failures, each variable record extent, cumulative name bytes,
member sizes and total stored bytes. Retained names are charged to directory
bytes; fixed facts, name lookup/duplicate tables, physical extent/candidate tables
and index coverage structures are all O(entries) resident costs. Measure actual
layout/peak coexistence and avoid duplicating every String just for lookup.
A vector in central order plus sorted lookup indices is sufficient; no generic
archive registry or self-referential representation is required.

Legacy path/open-file validation may use the same representation with natural
input-derived bounds: source/directory/member/stored bytes at most known length,
entry count at most length/46. Disjoint admitted ranges justify the stored-byte
bound. These mathematical bounds select no arbitrary engineering defaults and
do not establish an accepted legacy resident-memory policy. Its private error
bridge must preserve typed causes; C1 maps ReadError directly. Unrelated listing,
writer helpers and geometry/payload algorithms gain no acceptance by association.

## Independent stop gate

Fresh final-source review must prove the actual C1 and legacy reader call graph
contains no second ZIP directory constructor, fallback or synthetic filtered
source. Replay ordinary/empty-NTFS/empty-timestamp outer-directory controls with
the same selected outer catalog, duplicate/Unicode/ZIP64 controls, and the earlier
physical overlap/descriptor matrix. Inject faults on the actual selected central
header/name/extra reads and on data/CRC/index reads; raw failures cannot become
successful earlier-directory selection or MissingManifest. Prove Cursor/held
File agreement, exact last-central-index ownership, count/byte admission before
allocation and measured catalog/name/table scaling. Bind final source/artifact
identities and applicable checks before merge. Full A2 remains HELD; no partial
implicit product, release-scope reduction or legacy-removal authority follows.
