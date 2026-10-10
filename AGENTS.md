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
