# Interrupted tracks: recovery and disposition

## Current continuation after the release split

The user accepted a correction after PR #171: **0.4 ships Tiles 1.1/glTF 2.0;
0.5 makes the official Tiles 2.0/glTF 2.1 transition**. The
[roadmap](04-tiles2-roadmap.md) owns that sequence. Draft consumer support is
not a 0.4 gate. The snapshot below remains historical recovery evidence, not
an instruction to restore the superseded new-format shipping target.

The retained meshopt correction (#176) and bounded terrain correction (#184)
are merged. Resume
the shared tile/output work against a real 1.1 producer and current viewer;
reuse existing types and validated writers where their evidence applies.
Terrain LOD continues as implicit-forest output because implicit tiling stays in
0.4. C2 slice 3 (#179) is merged within its bounded implicit-validation scope. Legacy mesh
parent/atlas performance stays deferred, as in the ledger below; the superseded
mesh paths are not migrated to the shared serializer.
New 3TZ work is justified only by a concrete
0.4 streaming/resource requirement and stays at the packaging boundary.
The encoder remains in 0.4; GLB fills (#75, #185) are merged with their
bounded rendering and picking evidence. PR #168 remains closed as superseded;
useful decisions and code remain preserved. No feature was accepted by this
correction, and no interrupted work was deleted.

H6 retires all vector/point explicit production modes before M1; prerequisite
fills #185, outline #186 and point bounds #183 are merged. The first shared
serializer caller is vector's default implicit forest, including its explicit
skeleton/external-document roles. H7 retires all b3dm/i3dm/pnts/cmpt validation
through its remaining contract, oracle and audit gates; third-party explicit
reading stays retained. H14 maintains the [public-surface inventory](public-surface-inventory.md#acceptance-and-removal-sequence)
without claiming implementation acceptance for these future retirements.
Claude owns H6, H7 and H11 (C2 8-corner fast path); H14 is docs-only.

## Historical recovery snapshot

Snapshot: **2026-10-12**. This ledger records read-only worktree inspection and
GitHub PR metadata for the streamlined 0.4 direction. It is a recovery record,
not implementation acceptance. Superseded and deferred work remains unfinished;
neither label means done, accepted or merged.

## Recoverable identities

All paths below are existing checkouts under `/tmp/`. `M` means unstaged tracked
changes; `??` means untracked. No checkout was deleted, stashed or rewritten by
this audit. The exact heads, rather than historical handoff prose, identify the
interrupted source.

| Checkout | Branch | Exact HEAD | Dirty files |
| --- | --- | --- | --- |
| `rusty-tiles-c2s3-int` | `feat/c2-implicit-containment` | `11054670e4f93b3285c9af76f7ac29d03b402660` | `?? bench/architecture_audit/c2_decoded_containment/acceptance/production-mutants/slice3/report.json` |
| `rusty-tiles-terrain-lod` | `feat/terrain-lod` | `be90813254ed8ea294b6f665b502de16c32c7f1e` | `M src/terrain.rs`; `M src/terrain/raster.rs`; `M tests/t1_terrain_oracle.py` |
| `rusty-tiles-ml0` | `ml0/contracts` | `fe1ed0158b5bc5fe15c2f1a3d9f2ba38d3f62acd` | Clean |
| `rusty-tiles-ml1` | `ml1/encoder-spike` | `4e728ab57e17fcfcbdabe83314480a967520be43` | Clean |
| `rusty-tiles-c1-meshopt` | `fix/c1-meshopt-headers` | `538c0bde4623e8ad9561e444a19769274805eaa9` | Clean |
| `rusty-tiles-c2-fast` | `perf/c2-corner-test` | `a768929638ff87cd6386fdfa4be74dbfe4a8e613` | `M src/validate/containment.rs` |
| `rusty-tiles-vec-perf` | `perf/vector-ingest` | `a768929638ff87cd6386fdfa4be74dbfe4a8e613` | `M src/vector/pipeline/store.rs` |
| `rusty-tiles-mesh-perf` | `perf/mesh-parent-atlas` | `a768929638ff87cd6386fdfa4be74dbfe4a8e613` | Clean |
| `rt-gltf-spike` | Detached HEAD | `a768929638ff87cd6386fdfa4be74dbfe4a8e613` | Clean |

The C2s3 local branch reports 23 commits ahead and three behind its tracking ref;
do not resolve that divergence by resetting it. Meshopt has four local commits
above the recorded develop base. A clean worktree proves no uncommitted edits;
it does not prove that a spike ran or that a track is accepted.

The coordinator preserved all nine identities in
`/home/bend/.cache/rusty-tiles-evidence/04-direction-20261012/manifest.json`,
SHA-256 `0eb10130a1bb0854fa6ef90309001c3943d3008f3690aebaa1b0b4a169122b3f`.
Its per-checkout directories contain incremental `commits.bundle` files where
needed, binary staged/unstaged patches where present, and an untracked archive
for the C2s3 report. Each artifact has a byte count and SHA-256 in the manifest.
Bundles **require the recorded base commit**; apply patches to the recorded
head. This is not a standalone full repository backup. In particular, preserve
the untracked report archive: a tracked-file patch cannot recover it.

The coordinator also preserved 33 v1–v6.1 design Markdown files under
`/home/bend/.cache/rusty-tiles-evidence/04-direction-20261012/design-texts/`.
Its `manifest.json` SHA-256 is
`cf2e80ad8a3dd99068f817504568b6ddc52b1665cb5833cccfda801edfe8430f`.
This text snapshot excludes bulk design evidence and does not freeze or accept
the superseded design.

## PR state at inspection

All three PRs target `develop`; the heads match the worktrees above. Executed
checks belong to those committed heads and exclude dirty local corrections.

| PR | State | GitHub merge state | Checks observed | Disposition |
| --- | --- | --- | --- | --- |
| [#163](https://github.com/BenDyson-Arch/rusty-tiles/pull/163), ML1 encoder spike | Open, draft | `DIRTY` (conflicts) | CI, five installed-wheel targets and four official-Blender targets succeeded; optional distribution-Blender and release checks skipped | Preserve; extract/rebase the bounded encoder work after its owner/profile is settled |
| [#168](https://github.com/BenDyson-Arch/rusty-tiles/pull/168), ML0 umbrella/contracts | Open, draft | `BEHIND` | Reported CI checks succeeded; release acceptance skipped | Superseded by streamlined direction; close as superseded only after the replacement plan PR exists, with a link to it |
| [#169](https://github.com/BenDyson-Arch/rusty-tiles/pull/169), terrain LOD | Open, ready | `BEHIND` | CI, five installed-wheel targets and four official-Blender targets succeeded; optional distribution-Blender and release checks skipped | Preserve; finish the bounded local correction and review its final source |

Read-only branch PR queries found no PR for `feat/c2-implicit-containment` or
`fix/c1-meshopt-headers`. This audit did not mutate any PR, ref or issue.

## Decisions and remaining proof

| Track | Retain/rework/defer decision | Known defect or proof limit | Next bounded action |
| --- | --- | --- | --- |
| C1 meshopt correctness | **Finish** the format correctness slice independently of ML0 | Candidate pins stream headers to `0xa0` ATTRIBUTES, `0xe1` TRIANGLES and `0xd1` INDICES; linked-decoder tolerance is broader. Nine independent primary-backed controls and sensitive mutants are retained, but this snapshot records no fresh final-source acceptance | Review producer and validator bytes against the retained primary rule; run exact-head focused controls and applicable platform/package gates. Reuse existing archive fixtures without expanding archive scope |
| Terrain LOD | **Finish a bounded correction**, then reassess publication integration against the chosen 0.4 owner | Dirty code changes the clipped-cell overlay argument, synthetic stride coverage and nonfinite test-raster validity. Dirty oracle changes a skirt-box negative from a parent to a leaf to isolate decoded skirt containment. PR checks predate these edits; neither the proposed mathematical simplification nor changed negative is accepted here | Preserve patch, settle clipped-edge geometry with an independent reference and sensitive controls, finish nonauthor review, then revalidate the exact final source. Avoid unrelated terrain redesign |
| ML1 encoder | **Fold useful encoder work** into the retained glTF/texture path | `src/basisu_ktx2.rs` explicitly has no producer caller. Per-call container verification and error floors do not prove material quality. Vendored upstream invariant failures can abort/terminate; `vendor/README.md` says release packaging does not yet ship required notices/licenses. PR conflicts remain | Decide the actual caller and bounded texture profile; preserve source/licensing provenance and address lifecycle, packaging and independent decoded-image acceptance before integrating |
| glTF-only spike | **Fold the direction/probes** into the retained glTF content path | `/tmp/rt-gltf-spike` is clean and detached at the develop baseline; this worktree contains no unique tracked implementation or uncommitted probe. Its existence supplies no execution evidence | Recover any external spike receipts separately if they exist; implement the selected glTF-only content boundary in the replacement slices with fresh evidence |
| ML0 | **Supersede** the old F1d3/ML1–ML10 umbrella and amendments | Old text says READY TO FREEZE, not accepted. Its `mesh-local-to-3tz`, implicit-forest and archive-ceiling coupling does not govern the streamlined plan. Historical decisions/evidence remain recovery material | Carry only explicitly retained product decisions into the new owner/contracts. Link replacement plan from #168 and close it as superseded, without marking its proof complete |
| C2s3/I1 integration | **Preserve and reassess** useful containment/implicit logic | The current report binds head `11054670…2660`: even `clean` compiled but failed `c2_oracle_parity::production_matches_the_slice3_sweep_of_every_i1_control`, with `BQ.references-pair` flipped and `pass: false`. The clean baseline is not acceptance | Diagnose the references-pair discrepancy at its owning boundary; separate reusable decoded/frame mathematics from retired publication assumptions. Require only dependencies demanded by the selected representation; the old implicit/archive integration is not a mandatory prerequisite |
| C2 performance | **Conditional retention** for the implementation still used by 0.4 | Dirty patch adds 321 lines of corner testing and hoisted exact constants. Static search finds `walk_marked` only defined in that file, with no production caller; the patch does not establish a speedup or correctness parity | Preserve patch; after owner/representation selection, integrate only if needed and prove identical verdict, first outside vertex, faults and resource accounting against the retained reference, then measure a bounded benefit |
| Vector performance | **Conditional retention** for the spool still used by 0.4 | Dirty patch replaces DELETE journal with MEMORY and adds savepoint rollback coverage. It is uncommitted, and the local test source is not an executed result. In-memory rollback storage needs resource consideration | If this SQLite spool survives consolidation, verify rollback, cancellation/cleanup and bounded memory effects; measure the retained workload before landing |
| Legacy mesh parent/atlas performance | **Defer** | Worktree is clean at baseline; no unique committed or dirty implementation was found. Optimizing a retiring mesh path creates no replacement acceptance | Preserve checkout identity; revisit only if a concrete implementation is retained and measurement identifies a relevant bottleneck |
| 3TZ-specific expansion | **Pause** | Existing archive evidence remains historical evidence for its established domain; it does not require new archive writer/index/member-ceiling work in the streamlined direction | Reopen only if a selected delivery requirement establishes a concrete need. Preserve current code and controls until replacement callers and removal gates are settled |

The C2 failure report is
`/tmp/rusty-tiles-c2s3-int/bench/architecture_audit/c2_decoded_containment/acceptance/production-mutants/slice3/report.json`.
Its listed command is historical report metadata, not a command executed by this
audit. No builds, broad tests or performance workloads ran during consolidation.

## Resume authority

`/home/bend/Documents/tinyowl/rusty-tiles/F1D2_HANDOFF.json` is a stale historical
handoff: it still contains R2 preimplementation instructions and old F1d2 resume
work. Preserve its evidence pointers, but do not resume those actions without
checking current source, merged work and this replacement direction. Historical
source and artifact identities remain historical; they cannot certify dirty
corrections or new ownership boundaries.

Finishing any retained track requires its own final-source independent review
and relevant execution evidence. A green historical PR, preserved patch,
superseded contract or deferred optimization cannot close the corresponding
proof obligation or release gates #113/#121.
