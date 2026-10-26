# S1b: scalable shared borders

Governing contract: [D11 and S1b](hlod-040-decision.md#d11--scalable-borders-owner-decision-2026-10-24).
Implementation and acceptance are in progress; this note is not a passed gate.

Retain exact leaves, frozen ownership addresses, exact decoded-position incidence,
D4 representative identities, S2 source measurements and upward omission. Replace
permanent leaf-resolution coarse locks with shared level contracts. Remove any
superseded implementation rather than retaining compatibility paths.

Every contract must identify its source arc, every incident owner, common decoded
junctions, ordered subarcs and a displacement certificate against the leaf-decoded
arc. With depth increasing toward leaves, the predetermined scale ladder is
nonincreasing: epsilon_d <= E_d/16 and published G >= max(E_d, measured error,
child G). Certificates include codec displacement and all parameter breakpoints.
Same-depth owners use identical segments. Exact terminal-leaf contacts are pinned.
Unwelded source defects and T-junctions are counted, never tolerance-matched.

Parent inputs remain at most eight capped children. Border reconstruction precedes
interior simplification, S2 measurement and S3 charting. Permitted interior topology
loss is measured and reported; anything touching a border stays protected. Stalled
interiors must be remeshed before reporting the capped-frontier last resort. Omitted
cells hold no geometry and have only omitted ancestors. Delivery must reconcile
actual omitted cells with the producer's total and per-depth counts.

Graph extraction, ordered source-arc storage, shortcut construction and remeshing
need explicit bounded-memory and total-work arguments. Scratch admission precedes
writes, including random-access state and spill. Oversized incidence owner groups
must stream; no owner-pair enumeration or source-sized parent merge is acceptable.
These remain implementation proof obligations, not assumptions supplied by a scale
benchmark. Arithmetic fixtures cover 500M/billion inputs without baking them.

Acceptance uses exact-source receipts, adversarial geometry and texture fixtures,
decoded complete seam censuses, leaf-byte/ID equality and the existing resource
ledgers. S5's independent validator and oracle remain separately owned. The final
PR must list the contract and frontier changes its checks need to follow.

Local evidence: `~/.cache/rusty-tiles-evidence/hlod-s1b-20261026/`. Re-measure
`652f1800` and the final implementation on the same lane and cameras. Per-view byte
non-regression uses each dataset's restacked S3 baseline (tex3: 0.378/0.770/3.599 MB;
crop3: 0.418/0.849/3.881 MB), exact bytes and complete dependency accounting.
The mesh-1e7 sanity bake requires coordinator clearance; its command and receipts
will be prepared, while all local bakes remain tex3/crop3 only.

The cap-fit proof must distinguish general bounded work from a constructive surface
domain. A stalled, consistently oriented height-field patch with simple projected
boundary loops can be reconstructed by constrained Delaunay triangulation, using
only its boundary and mandatory interface vertices/edges. Flooding across boundary
constraints preserves all holes. With B boundary vertices, I mandatory interior
vertices and b loops, T = 2I + B + 2b - 4 per connected patch; source interior
sampling does not appear. Projection collisions, folds, nonmanifold edges and
crossing constraints are outside this constructive domain. They still enter bounded
cleanup/clustering and the explicitly reported last-resort frontier. This replaces
an unproved claim that vertex clustering alone guarantees cap fit. No parent input
or output cap is relaxed. S2 measures the reconstructed surface before S3 runs.
