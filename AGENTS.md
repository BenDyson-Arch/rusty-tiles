# Foundation migrations

The architecture gate in [docs/architecture/README.md](docs/architecture/README.md)
governs foundation work. Current implementations, tests and historical output
digests are candidates for retention, not specifications. Breaking changes are
permitted; do not preserve an old path solely for compatibility.

Before implementing a foundation slice, define its invariants, supported domain,
failure semantics, ownership and independent acceptance plan. Record concrete
retain/rework/replace/remove decisions and unresolved proof obligations. Resolve
those decisions through the audit and focused probes before coding; this does
not require a separate user approval step within authorized work.

Implement the smallest representation that makes those invariants explicit.
Resolve policy once at its owning boundary; lower stages consume validated
values. Avoid option bags, repeated mode checks, fallback ladders, speculative
frameworks and layout moves without demonstrated dependency benefits. Keep
mathematical and format meaning separate from paths, run control and publication.
Retain a component only for the domain its independent evidence establishes.

When fan-out is requested, assign clear file ownership and keep audit findings,
implementation and acceptance truth distinct. A separate nonauthor review must
inspect the final source and evidence. One coordinator owns shared Cargo builds;
agents must not race builds or copy a binary while its build is running.

Acceptance records must distinguish static observations, executed defects,
profile choices and proof limits. Bind executions to exact source/artifact
identities; use sensitive controls and independent references. A green regression
suite alone does not establish a new contract. Document remaining migration and
removal gates; do not claim broader acceptance or release authorization.

## Autonomous roadmap work

For the authorized 0.4.0 foundation roadmap, own PRs through final-source
independent acceptance, applicable CI, installed-wheel and official-Blender
checks. Investigate failures at their owning boundary, revalidate exact source
and artifact identities, and merge an accepted exact head into `develop` when
the evidence supports it. Evidence-backed merges are authorized without another
confirmation. Continue to the next dependency-ready bounded slice and maintain
a durable roadmap/checkpoint record. Missing proof is unfinished work.

The target is to get as close to a 0.4.0 release-ready candidate as possible.
Do not merge into `main`, tag, publish or release under this authority. Keep
#113/#121 open unless their closure is separately authorized. Preserve the
foundation-first process and GPT-6.1 Sol fan-out with clear file ownership and
a fresh separate nonauthor review; one coordinator owns shared Cargo builds.

## Evidence and tree size

Commit only what CI or tests execute or read, plus small fixtures (aim for about
100 KB each; 1 MB hard limit) and hash manifests. Also keep the independent
oracles, witness generators, self-tests and mutant sets of active validator
families, even when CI does not run them; later slices extend them. Bulk
evidence (raw runs, review transcripts, candidate and precode corpora,
preparation output) goes to `~/.cache/rusty-tiles-evidence/` and is referenced
from a tracked README by SHA-256 and the last commit that held it.
`scripts/check_tree_budget.py` runs in CI and fails when any tracked file
exceeds 1 MB or the tree, `bench/` or `vendor/` exceeds its total budget; raise a
budget only with a stated reason. On 2026-10-14, after
trimming generated result files (listed in `bench/MOVED_EVIDENCE.md`), the tree
budget went from 21.6 to 24.0 MB and `bench/` from 11.8 to 12.5 MB, because real
code grew with the large-mesh tiler and validator-scale work. Trim before raising
again. `vendor/` holds only third-party source vendored
for a shipped feature, one pinned version per component, replaced rather than
accumulated, with its own budget and an exact-path list of upstream files allowed
over 1 MB. The guard sees only the final tree, so a PR that adds
and later removes bulk files must be squash-merged, or the guard must be run on
every commit in the PR range.

## CPU load

Keep local work considerate of other applications. Default to
`CARGO_BUILD_JOBS=2`, `RAYON_NUM_THREADS=2` and `RUST_TEST_THREADS=2`, and run
heavy build/test/resource commands at `nice -n 10`. Serialize heavy workloads
under the coordinator. Agents may perform lightweight reading and bounded
small probes concurrently, but must coordinate sustained expensive work.
Inspect host load before increasing concurrency and reduce it if needed.
Avoid repeating broad verification once applicable meaningful checks pass.

Pass explicit two-worker options for operation-owned pools (for example vector
`--jobs 2`); `RAYON_NUM_THREADS` does not override a custom pool. For broad tests
that infer hardware concurrency, use a two-CPU process affinity where supported
or another actual worker cap. Keep these local execution limits distinct from
the product's supported configuration and resource acceptance claims.

## Changelog

PRs add a `changelog.d/` fragment (format in `changelog.d/README.md`) and must not edit `CHANGELOG.md` directly; fragments are assembled at release.
