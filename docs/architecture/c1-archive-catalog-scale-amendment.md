# C1 archive catalog at scale: amendment VS-A

Status: author amendment for validator-scale slice VS-A. The nonauthor final-source
review and the coordinator's gate are still pending. Nothing here records acceptance.

This note amends the catalog, closure and report-row parts of the
[archive reader contract](archive-read-foundation-contract.md) and of the C1
validator ([c1-payload-integrity-contract.md](c1-payload-integrity-contract.md),
which stays byte-identical because the C1 production-control driver pins its hash).
If the texts disagree, this note governs how the admitted catalog, the closure check
and the payload rows are stored and ordered. Every verdict rule is unchanged except
the two message changes in CAT-1.

Design: `~/.cache/rusty-tiles-evidence/validator-scale-design-20261013/design.md`
(§1.3, §1.4, §2 rows W1–W5, W12, W13, W15, W17; §4.1 A-C1-2, A-C1-3, A-C1-5), with the
coordinator's decisions beside it. Later slices: VS-B (bounded traversal and C2) and
VS-C (limits model).

## Defect

Before this slice the validator held, for every archive entry, a `String` name, its
stored range, a sorted name ordinal, a 32-byte extent, a whole-index offset table and a
seen byte, plus a `HashSet` of names, a `HashSet` of used names and a
`HashMap` of every member's SHA-256. That is about 0.5 KB per entry, before
traversal. The report's payload rows were a `HashMap` keyed by URI, sorted at the end
and materialized again by the CLI (`serde_json::Value`) and by Python (a list of
dicts). Three paths were `N log N` over string names: the duplicate-name sort, the
index check's offset sort and binary search per row, and a binary search over names per
reference lookup (three or four lookups per reference). The unreferenced-entries
message joined every unreferenced name.

## Terms

- **N**: admitted members, index included. **S**: the sum of their stored sizes.
- **Key** of a name: `(md5 as two little-endian u64, first 8 bytes of SHA-256)`. The
  first part is the `@3dtilesIndex1@` order; the second keeps MD5-colliding groups
  small. Keys never decide identity; names are compared byte for byte.
- **Rank**: a member's position in key order.
- **Resident** and **spilled** catalog: rows and log in memory, or in a scratch
  directory read through fences and a page cache.

## Invariants

**CAT-1. Result identity.** For every archive and every `ValidationLimits`, validation
returns the result it returned before this slice:

- on success, the same report: `tiles`, `contentReferences`, `entries`, `checks`,
  `notInspected`, `limits` and `payloads` (same rows, same URI byte order);
- on failure, the same category and message, at the same point (first-failure
  precedence), with two exceptions:
  1. **Unreferenced entries.** With more than 16 unreferenced members the message lists
     the first 16 names in byte order and then ` and K more`. With 16 or fewer it is
     unchanged: `unreferenced archive entries: a, b, c`.
  2. **The validator's own scratch.** A spilled catalog can fail on its scratch
     storage. This is never a verdict on the archive (CAT-6).

The resident and spilled profiles give byte-identical results (CAT-5).

**CAT-2. Stage order.** Unchanged: central-directory admission per record in
central-directory order; then the directory length check; duplicate names; physical
disjointness; `tileset.json` presence, the last-entry and index-length checks; the
index CRC, then the first failing index row; the hash pass in central-directory order;
traversal with inline C1; the recorded budgets; closure; C2 publication; the
source-change check.

**CAT-3. Index check.** The index is streamed once against the catalog in rank order.
If every row equals the matching non-index catalog row (MD5 and local offset), the
index is admitted; this is the case for every producer archive. Otherwise the first
failing row is decided exactly: the index rows and the catalog pairs are each sorted
by `(md5, offset)` on scratch and merged. Before the first failure every row passes, so
the first failure is the least of each absent pair's earliest row and each present
pair's second row. The message is "not ordered" if the first ordering failure is at or
before that row, otherwise "differs". A CRC mismatch still outranks both. MD5
collisions need no special case: pairs are compared exactly, and a colliding group in
any index order is admitted, as before.

**CAT-4. Lookups.** A name lookup computes its key, finds the first row with that key
(binary search in memory, or a fence binary search and one page), and compares names
exactly across the run of equal keys. The last lookup is memoized. Duplicate names are
detected as equal names within one equal-key run of the sorted catalog.

**CAT-5. Profiles.** The catalog stays resident when
`N·86 + centralDirectoryBytes ≤ 96 MiB`, which holds for every archive within the
default entry (2^19) and directory (32 MiB) limits. Otherwise rows and log go to
scratch: one fence per page of `⌈N / 2^19⌉` rows and a 16 MiB direct-mapped page
cache. External sorts run with `R` = 64 MiB, `F` = 32, `b` = 1 MiB (rows) and `R` = 24 MiB
(extents and index diagnosis). The choice is fixed
from the end record before any record is read. Scratch is created only at the first
spilled file (a deferred `Scratch`), after its free-space preflight against the
estimate `2·(N·84 + CD) + 2·64·N` bytes. So no archive within the default limits
creates scratch. A test-only profile (`RUSTY_TILES_FORCED_SPILL=1` in test builds and
in `validate-forced-spill` builds; never in production) spills every store, sorts runs
of a few records with fan-in 2, keeps two fences and one cached page.

**CAT-6. Scratch failures.** Scratch errors map as follows: I/O and free-space
refusals are `Io` (exit 1); size overflow is `ResourceLimit`; a corrupt or truncated
scratch file, a poisoned sorter or an invalid configuration is `InvalidState`. All
carry the prefix `validation scratch:`. Scratch files are removed on drop and on error.

**CAT-6a. Scratch placement.** Spill is never put in RAM silently. At the first spilled
file the scratch parent is the first **disk-backed** candidate that passes the free-space
preflight, in this order: `$TMPDIR` when set, the user cache directory plus
`rusty-tiles/scratch` (`$XDG_CACHE_HOME` or `~/.cache` on Linux, `~/Library/Caches` on
macOS, `%LOCALAPPDATA%` on Windows; created if missing), then the system temporary
directory. Memory-backed (tmpfs/ramfs) candidates are detected by the `spill` volume
inspection (#166) and skipped. If every candidate is memory-backed, `validate` uses the
first that fits and the report says so: the report gains `scratch` (`memoryBacked`,
`peakBytes`) whenever anything spilled, and only then, so reports of archives that do
not spill are unchanged. The report-less `validate_3tz` facade instead refuses with
`io` "no disk-backed scratch directory". VS-C adds `--scratch-dir`.

**CAT-7. Report rows.** First-inspected payloads add one 42-byte row (name location
and counts) during traversal. Repeat references take their counts from the C2 per-URI
facts, as before, without a re-charge. At report time the rows are ordered by a
most-significant-chunk external sort over 64-byte name chunks: each pass orders only
the rows whose names tied so far. The rows are then read back one at a time with names
from the catalog. `ValidationReport::payloads` is a `PayloadRows` (length, random
access, iteration, streamed `Serialize`). The CLI writes the report streamed, with
bytes identical to the former `serde_json::Value` output. Python's
`report["payloads"]` stays a list of the same dicts, so the report remains plain JSON
data; `validate(input, lazy_payloads=True)` returns a lazy read-only sequence,
`ValidationPayloads`, instead, which keeps Python memory constant in the row count.

**CAT-8. Closure.** The `used` set is one bit per rank. Closure scans the bits in rank
order, counts unreferenced members and keeps the 16 smallest names in a bounded heap.
Only the `vector-build.json` digest is kept from the hash pass.

## Memory bound

Resident memory that depends on N is the `used` bitmap, `8·⌈N/64⌉` bytes (21.3 MiB at
the 3TZ member ceiling of 178,956,970; VS-B adds two more bits per rank). Everything else is a
configuration constant:

| Stage | Resident bound |
| --- | --- |
| catalog build | resident profile: rows, extents and log ≤ 96 MiB + sort scratch; spilled profile: the row sorter (`R` = 64 MiB) and the extent sorter (`R` = 24 MiB), `1.5·R + (F+1)·b` each, ≤ 129 + 45 MiB, plus the log writer |
| lookups | resident ≤ 96 MiB; spilled: fences ≤ 12 MiB + page cache 16 MiB |
| index check | one 65,520-byte chunk; on failure, two sorters with `R` = 24 MiB |
| payload rows (walk) | one sorter with `R` = 32 MiB |
| payload rows (report) | the first pass with `R` = 64 MiB, the later passes and the placed rows with `R` = 24 MiB, live together after the walk's storage is released: ≤ 1.5·64 + 2·1.5·24 MiB plus buffers, beside the resident catalog (≤ 96 MiB) and the retained name log |

This slice does not bound the C2 tile arena, reference list and per-URI table, the
external-document memo, or the live documents of the path. Those grow with tiles and
references and are VS-B's (design M8–M15). Until VS-C, the default visit, reference
and byte limits bound them.

## Work bound

Central-directory admission reads the directory in 64 KiB chunks and each local header
with its name in one read. Every stage is linear in N for fixed `R`, `F` and fence
count: two external sorts of fixed-width records (at most two merge passes up to the
member ceiling), one streamed index pass, one sequential row pass for duplicates and
one for extents; the duplicate pass reads names only within an equal-key run, so it does
no random log reads in practice. A lookup is one key hash, one fence binary search and
at most one page read plus one name read; names are compared once. Closure is one
bitmap scan plus one random name read per unreferenced member (only failing archives
have any). Row ordering costs one pass per 64-byte chunk of the rows that still tie, and
each pass reads only that chunk of each tied name, so a name of length ℓ costs O(ℓ) in
all: linear in the payload name bytes.

## API and report changes (breaking)

- `ValidationReport::payloads` is `PayloadRows` instead of `Vec<PayloadReport>`.
- `ValidationReport::summary_json` and `ValidationReport::write_json` are new.
- Python `validate()` gains `lazy_payloads` (default `False`: a list, as before).
- The unreferenced-entries message is capped (CAT-1).
- `ValidationReport::scratch` (`ScratchReport`) and the JSON `scratch` member appear
  only when something spilled (CAT-6a).
- Python with `lazy_payloads=True`: `report["payloads"]` is not a `list`.
  `report["payloads"] == [...]` is False and `json.dumps(report)` raises `TypeError`;
  use `list(report["payloads"])`.
- `validate_3tz` spills a large catalog to disk-backed scratch and refuses when only
  memory-backed scratch is available.

The JSON report keeps its shape, key order and row order.

## Acceptance plan

1. **Reference index check.** The pre-slice algorithm (offset sort, binary search per
   row, seen table) is kept in the tests as the reference. Randomly defected indexes
   are checked under both profiles against it (each verdict class at least 50 times),
   and the merge diagnosis is checked on synthetic MD5 collisions.
2. **Forced-spill parity.** Every in-crate parity suite (C2 slice-1, C2-WB, slice-3,
   S3-18 sweep, forest, I1, W corpus), the EM controls and every validator and archive
   unit test, run under the resident and the forced-spill profile, with zero
   disagreements; the C1 CLI oracles against a `validate-forced-spill` binary.
3. **CLI byte identity.** `write_json` equals the former `Value` output on every
   controlled report.
4. **Sensitive mutants** of the index merge, the diagnosis, lookups, duplicate
   detection, the page cache, the closure cap and the row order.
5. **Scale.** Peak RSS flat in N from small archives up to the scale-gate (d)
   archives, and catalog time linear in N; see the evidence README.

## Unresolved obligations

- The bitmap is `N/8` bytes, bounded by the format ceiling, not O(1) (design OQ-1).
- Whole-validation RSS is not flat in N until VS-B bounds the traversal facts.
- The scratch label and `--scratch-dir` are VS-C's.
