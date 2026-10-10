# Stored-archive reader implementation decisions

Status: implementation in progress, not production acceptance. The
[reviewed contract](archive-read-foundation-contract.md) and its
[preimplementation review](../../bench/architecture_audit/implicit_rewrite/archive_prereq_review/review.md)
remain the baseline. That review pins the original source bytes before coding;
it must not be rebound to later implementation bytes. Fresh final-source review
and executed candidate receipts are still required.

The private format owner exposes caller-supplied ReadLimits and typed ReadError.
One StoredArchive catalog supplies envelope admission, 3TZ index validation and
exact member reads. C1 maps its five existing archive ceilings once and retains
the selected file's identity/stability wrapper. No implicit operation, jobs or
validation policy enters the format owner. Legacy standalone validation uses
this same catalog with natural bounds derived from the actual input length;
those bounds establish no accepted resident-memory policy for that facade.

An independent fixture-design lane identified a descriptor-width detail after
the baseline review. APPNOTE 6.3.10 section 4.3.9.2 associates ZIP64 extra presence
with 64-bit descriptor sizes, while central-directory ZIP64 can also express only
an offset. This finite profile therefore uses a local ZIP64 tag as the explicit
64-bit descriptor declaration, including tiny/no-sentinel local sizes. A
central-only ZIP64 tag with a descriptor and no local ZIP64 tag is Unsupported
because this slice does not certify that ambiguous declaration. A central-only
ZIP64 offset with no descriptor remains admitted, preserving the archive writer's
existing offset representation. A local ZIP64 declaration followed by only a
32-bit descriptor is malformed. Archive-level ZIP64 EOCD/count alone does not
select file descriptor widths. No fallback selects a width merely because it
fits the bytes.
[Primary ZIP specification](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT).

This declaration choice requires actual tiny local/central ZIP64 controls and
fresh final review. It does not assert that all foreign ZIP writers use the
same interpretation, or narrow their valid archives to InvalidInput. Physical
overlap remains an explicit Unsupported profile outcome when records are
otherwise coherent; truncated or inconsistent framing remains InvalidInput.

Full A2 replacement remains held at the literal unavailable-root external
JSON-template conformance gate. The private reader prerequisite is useful with
current C1 and does not select a different implicit product, certify decoded
content or close any release parent.


## Selected-directory owner revision

Fresh independent controls reproduced a further interpretation mismatch before
candidate acceptance: the ZIP library retries an earlier end record after a
selected-directory extra-field parse failure, even when the outer TLV is fully
framed. Its metadata constructor also catches raw I/O failures while retrying.
The raw scanner's selected directory therefore cannot establish which directory
that constructor will allocate/read. Post-construction agreement checks would
occur too late to prove admission before allocation. The exact small controls
and current-source observations are recorded in the
[final-review findings](../../bench/architecture_audit/implicit_rewrite/archive_final_review/findings.md).
These are baseline executions and candidate source observations, not an accepted
candidate result.

The proposed correction is one private StoredArchive over the admitted raw
directory. The format scanner retains checked name, local/data offsets, length,
CRC and central order. The same catalog supplies exact index cardinality and
member lookup; C1 reads/hashes those admitted stored ranges directly with explicit
CRC checks and typed raw I/O. No second ZIP directory constructor, end-record
retry, synthetic filtered source or metadata exemption is required. The selected
end record is authoritative. Declared entry count times the minimum central
header size must fit the admitted directory before catalog allocation.

C1 retains its existing engineering ceilings and held-file identity checks.
Standalone legacy validation can use the same format representation with natural
bounds derived from the actual input length; this selects no new arbitrary
engineering ceiling and establishes no accepted resident-memory policy for that
legacy facade. The legacy error bridge remains distinct from C1's typed errors.
The writer and glTF semantic decoder are unchanged. All name/catalog/table
resident costs must be included in the measured private-reader acceptance.

This representation revision requires independent owner adjudication before
coding and a fresh final-source review after actual file/Cursor, selected-directory,
raw I/O, CRC/index and resource controls execute. The earlier production-ready
handoff covered the discarded pass-through candidate, not this revision.
