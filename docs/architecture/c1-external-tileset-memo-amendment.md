# C1 external-tileset traversal: per-document memo amendment (EM)

Status: author amendment, written before the code. The nonauthor final-source
review and the coordinator's gate are still pending. Nothing here records
acceptance.

This note amends the explicit-traversal part of the C1 validator
([c1-payload-integrity-contract.md](c1-payload-integrity-contract.md), which stays
byte-identical because the C1 production-control driver pins its hash) and the
C2 traversal facts of the
[C2 contract](c2-decoded-containment-contract.md). If the texts disagree, this note
governs how an external-tileset placement is evaluated. Everything else is
unchanged.

> **VS-B (2026-10-14).** [validator-traversal-scale-amendment.md](validator-traversal-scale-amendment.md) amends this text and governs where they disagree: summaries carry their facts by value, are capped at 64 MiB and record their live-document peak (VS-B-6, VS-B-7).

## Defect

Each placement of a shared external tileset re-read, re-parsed, re-schema-checked
and re-walked its document. A document entered `K^D` times through a `D`-level DAG
of `K`-way references was parsed `K^D` times. A 517,728-byte `.3tz` (depth 15,
32 KB key-heavy documents) validated for 61-63 s under the default limits and was
then accepted. The binding limit was `total_bytes_read` (2 GiB) at about 29 ns per
byte of JSON re-parse. Findings and archives:
`~/.cache/rusty-tiles-evidence/ext-amplification-20261013/`.

## Terms

- A **placement** is one `Tileset` traversal task: a document name, the referencing
  tile's volume (`parent`) and geometric error, the placement depth, and the C2 link
  (the referencing tile in the C2 tile arena and the referencing content's
  `boundingVolume`, if any).
- The **expansion** of a placement is everything the pre-memo traversal did between
  that `Tileset` task and its `ExitTileset` task: the document's tiles, contents,
  nested placements and their expansions.
- The **reference traversal** is the pre-memo traversal. It is kept in the test build
  behind a switch so that the controls can compare against it.

## Invariants

**EM-1. Result identity.** For every archive and every `ValidationLimits`, validation
returns the reference traversal's result:

- on success, the same report: `tiles`, `contentReferences`, `entries`, `checks`,
  `notInspected` and `payloads`;
- on failure, the same category and message, at the same point of the traversal
  (first-failure precedence);
- the same C2 precheck work units, C2 bytes and decoded charges, and the same C2
  outcome (exclusions, precheck reason, streamed verdict and message).

No report count changes and no limit value changes.

**EM-2. Logical C1 accounting.** `hierarchy_visits`, `references` and
`total_bytes_read` charge every placement exactly what the reference traversal
charged it:

- one visit for the `Tileset` task, one per tile, one for the `ExitTileset` task;
- one reference for the document, one for its `schemaUri` and one per content;
- the document's bytes and its `schemaUri` document's bytes.

These are the placement's **steady charges**. The first inspection of a payload is
still charged once per validation, as before: its member, resource and `schemaUri`
reads and references and its elements are not steady charges. A memoized placement
charges its steady charges without re-reading its documents, so
`total_bytes_read` is a logical charge of the reference traversal's reads, not a
count of physical reads. The physical reads are at most the logical ones. The
2 GiB, 2^18 and 2^20 values stay as they are; they are floors of the logical
traversal.

**EM-3. Summary domain.** A document gets a summary only when one of its placements
completes its whole expansion without failure and without any implicit root
anywhere in that expansion. A document whose expansion holds an implicit root is
always traversed in full. This is not an amplification path: I1 OD-7 makes a
second walk of the same implicit root Unsupported ("repeated subtree member"),
because every walk admits its root subtree first.

**EM-4. Placement-independent facts.** Inside a document, every C1 check is
relative to the document itself:

- `hierarchyBounds` tests a child against its parent tile with the child's own
  `transform`;
- `geometricErrorOrder` tests a tile against its parent tile's error;
- URIs resolve against the document's member name;
- schema, asset, payload, budget, `extras` and exclusion-bit checks read only the
  document and its payloads.

So the first completed placement's verdict on them holds for every placement. The
C2 exclusion bits that the traversal ORs directly (tileset, tile, bounding-volume
and content `extensions`, `metadata`, `gltfUpAxis`) are idempotent, and the first
placement already set them.

**EM-5. Placement-dependent checks.** A memoized placement re-runs exactly these, in
the reference traversal's order:

1. the depth limits: the `Tileset` task's `depth > 128` and each tile's
   `hierarchy_depth`, through the deepest relative tileset and tile depths of the
   expansion;
2. the root's `hierarchyBounds` against the referencing tile's volume, with the
   root's own `transform` ("child bounds escape parent at depth `d`"; a mixed
   region/Cartesian pair stays Unsupported);
3. the root's `geometricErrorOrder` against `min(referencing tile error, document
   geometricError)` ("geometricError increases at depth `d`"). The `document
   geometricError` term is redundant at a memoized placement: the first placement
   already enforced `root error <= document geometricError`, and the tolerance is
   monotone in the bound. Only the referencing tile's error is
   placement-dependent. Dropping the term is an equivalent mutant;
4. C2: every tile fact and every payload reference of the expansion is registered
   again, in the reference traversal's order. Each tile gets its new arena parent
   and depth. The root gets the placement's referencing tile and external-content
   volume. So each reference's level chain, its chain exclusion bits, its precheck
   step and its phase-2 stream are composed through this placement's own ancestors
   and transforms.
5. the cycle check: a placement of a document that is on the active path takes the
   full traversal. A summarized document cannot close a cycle. If it could, its
   completed first expansion would have reached a document that reaches it, and
   that document would then have found the cycle.

   This proof assumes that member contents do not change during one validation, so
   that the documents a document names are the same at every placement; the
   archive identity check and the phase-2 re-read enforce this only at the end. It
   also assumes that an expansion is placement-independent (URIs resolve against
   the document's own name, and every child and content is visited
   unconditionally) and that implicit subtrees cannot name tileset documents. A
   later change that makes traversal placement-dependent, such as conditional
   children, invalidates the proof and makes the test load-bearing.

**EM-6. Precedence and fallback.** A placement uses its summary only if, before it,
`task_visits + visits`, `reference_visits + references` and
`bytes_read + bytes` are each within their limits and the depth bounds of EM-5.1
hold. Then the reference traversal could not have failed inside the expansion
except at EM-5.2 or EM-5.3, since every other check is placement-independent
(EM-4), and memoized C2 registration has no verdict. Otherwise the placement is
traversed in full, with its nested placements again memoized under the same rule.
This reaches the reference traversal's failure point with the same counters, so
the first failure and its message are the reference traversal's. A full re-walk
after a cap miss always ends in failure: the caps that refused the summary are the
same caps the full traversal then exceeds. So the fallback cannot reintroduce
amplification.

**EM-7. Bounded, non-circular work.**

- A summary's charges are measured once, over a completed expansion, and applied
  before its C2 registration. Nothing is charged from the work it is meant to bound.
- A memoized placement does O(1) C1 work. Its C2 registration is O(tiles +
  references) of its expansion. Every registered tile is at least one visit the
  placement was charged, and every registered reference at least one charged
  reference.
- So the C2 contract's arena bounds still hold: at most `hierarchy_visits` tile
  facts and at most `references` references. The C2-WB precheck charges each
  reference with the unchanged formula against the unchanged 2^29 cap. Phase 2 is
  unchanged.
- Each distinct document is parsed and schema-checked once per validation, unless
  EM-3 or EM-6 sends it to the full traversal.

**EM-8. Storage.** There is one summary per distinct summarized document. It holds
the steady charges, the report counts, the two relative depths, the root's volume,
transform and errors, and one compact event per tile, payload reference and nested
placement of the document itself. Nested documents are shared by reference, not
copied. Tile and reference events index the first placement's C2 facts, which the
arena keeps. The events are bounded by the first traversals' visits and references.

## Report and API

There are no report, CLI, Python or public Rust API changes. The C1 checks list, the
counts and the `limits` object are unchanged. `docs/VALIDATION.md` states that
`total_bytes_read` is the logical charge of EM-2.

## Controls (author)

These are the in-crate tests in `src/validate/memo_controls.rs` (module
`validate::memo_controls`):

- **Differential.** Generated DAG archives are validated with and without the memo
  and must give the same result. They vary shared documents, transforms, content
  volumes, `schemaUri`, payloads and depths, and sweep `hierarchy_visits`,
  `references`, `total_bytes_read` and `hierarchy_depth` across every trip point.
  The comparison covers the serialized report or the error category and message,
  plus the C2 work units.
- **Per-placement controls.** In each, the same document is valid under its first
  placement and fails under a later, memoized one:
  - C1 root escape;
  - root geometric error;
  - C2 ancestor escape through the referencing tile's `transform`;
  - C2 external-content escape;
  - the reported depth, with the same document placed at depths 1 and 3;
  - depth limit;
  - each counter limit.
- **Engagement.** On a shared DAG, each distinct document is parsed once.
- **Mutants.** Each must fail at least one control: a replay that reuses the first
  placement's C2 link; a skipped root containment check; a skipped root
  geometric-error check; a missing, short or unsubtracted charge; a skipped depth
  bound; replay without references; replayed tiles keeping the first placement's
  absolute depth; a nested placement missing from its enclosing
  summary; the memo disabled.
  - Removing the active-path test of EM-5.5 is equivalent by that proof. The test is
    kept as a defensive guard.

Driver and results: `bench/architecture_audit/external_memo/`.

The parity suites, the hostile archives before and after, and the mutant runs are
recorded in `~/.cache/rusty-tiles-evidence/ext-memo-20261013/`.
