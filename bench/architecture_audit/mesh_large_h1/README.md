# H1 independent capture/tree acceptance

Independent local H1 capture/sort/tree acceptance passes: source/tree comparisons,
ten-million-triangle scale, RSS, dense-document planning, corrected scratch
preflight and retained-route processing/browser gates. Exact-head remote CI is
pending; resource evidence has the recorded Linux domain. This directory retains
the independent reference for the
[H1 contract](../../../docs/architecture/h1-capture-tree-contract.md), plus the
durable location of bulk execution and review evidence.

[`tests/h1_tree_oracle.py`](../../../tests/h1_tree_oracle.py) generates small
external-buffer glTF and GLB sources from analytic source truth. Its reference
holds each small mesh in memory, directly interleaves centroid bits and
recursively partitions prefixes, then packs leaf octants and constructs
left-filled synthetic run hierarchies. This deliberately differs from the
production streamed stack and external sorts. The reference compares complete
sorted source records, node identities/ranges/bounds, per-depth bounds and the
minimum leaf depth over every welded-position corner.

Run its controls with:

```sh
python3 tests/h1_tree_oracle.py self-test
```

Controls alter provenance, source coverage, coordinate frame, ordering,
hierarchy identity/packing/ranges, bounds and welded minimum depths. The
greater-than-sixteen-incidents case puts the shallowest leaf after the first
sixteen incidents. These are oracle sensitivity checks; they do not claim
that the production implementation executed a fault.

Frozen oracle SHA-256:
`11526aab2e48f804a719182ae561b804015f75ff246ce53cfad99206d1e17068`.
Six self-tests pass. Final production `f624b704f85111061b4ca2570838950c90027e78`
passes 72 full source/reference-tree executions and eight typed refusal/cleanup
controls in portable, native and optimized artifacts. The corpus covers GLB/external
buffers, indexed/nonindexed and strided positions, reflected/selected nested
instances, degenerate preservation and f32 collapse, packed octants, synthetic
runs, all-corner depth minima and the independent 236-byte full-attribute
geometry planning charge. Separate configurations isolate triangle and byte
budgets. Equal-budget dumps are identical across workers, run budgets, fan-ins
and scratch locations. All 72 matching dump sets are also byte-identical across
portable/native/optimized builds and before/after the physical-allocation correction.

The final portable artifact is
`b431353a4ba3d2c507af013e2f1024c7877c47260e8d7feb5d8dd4796240e2b7`;
the native artifact is
`ee6569469e988e7936a9c8dc88f201697f35387567e1295f61aa1e4aff2be3ae`;
the optimized artifact is
`7887d8bf5ab8359f78bb5a4e9b5d8286c4e1bb6d48408b97e5213499186ba88e`.
The eight refusal cases cover a single over-budget triangle, height, early and
exact member counts, invalid budgets/fan-in, missing resource and accessor
extrema mismatch. One original harness expected the wrong message substring
for the extrema case; it was corrected and all cases rerun under the frozen
oracle. The original mismatch is preserved as an oracle defect.

Independent review also found and executed a production resource counterexample:
the initial record-only scratch allowance of 2,994,176 bytes was below a
SIGSTOP-confirmed 6,643,712 allocated bytes on a 4,096-triangle tiny-run case.
The producer completed and cleaned up; no ENOSPC was injected. This finding
withdrew the earlier source-review statement that no blocker remained.
The owning volume/config correction now reserves actual allocation-unit
rounding, a bounded live-file metadata allowance, directory space and another
logical generation. The report distinguishes logical peak from full preflight
allowance. Conservative stage rechecks can refuse borderline space; there is
no guarantee of success at exactly the reported allowance.

[`physical_scratch_probe.py`](physical_scratch_probe.py) retains the sensitive
Linux witness. With the fixed native artifact it observed 5,083,136 allocated
bytes and 1,241 live files, within the full allowance of 1,150,115,840 bytes and
file cap 16,416. It independently replayed every source triangle after resume,
checked cleanup and rejected a reduced allowance. The snapshot is a lower-bound
witness, not the complete physical peak; the metadata reserve is conservative
policy, not a theorem for arbitrary filesystems, quotas or concurrent writers.

```sh
python3 bench/architecture_audit/mesh_large_h1/physical_scratch_probe.py \
  --binary /path/to/immutable-lib-test-binary \
  --work /path/to/fresh-cache-directory --label exact-source
```

The optimized ten-million-triangle indexed planar-grid run uses a 120 MB input
payload and passes full independent streaming source replay, exact-once leaf
coverage, node bounds/ranges and all-corner welded depths. The tree has 3,413
nodes, 2,048 leaves, height seven and 1,089 welded vertices, matching independent
analytic grid/count reasoning. It completes in 21.19 seconds with peak RSS
97,202,176 bytes (92.70 MiB), below the declared 239,806,080 bytes (228.70 MiB).
Logical scratch peak 2,010,160,020 bytes is below 7,450,571,392; periodic physical
allocation estimate 2,010,230,784 is below the full 14,948,427,008-byte allowance.
Periodic non-atomic samples do not establish the exact physical peak. Inputs
and binary hashes remain unchanged and scratch cleanup has zero entries.

The dense 65,536-node document passes the full small recursive reference and
uses peak RSS 46,952,448 bytes, below its declared 264,958,592-byte bound. Its
short-lived workspace escaped periodic allocation sampling; a zero sample is
not a zero-allocation claim. [`resource_probe.py`](resource_probe.py) retains the
Linux measurement harness with two workers, pinned CPUs 4/5, whole-process
`wait4` peak RSS, endpoint input/binary hashes and cleanup checks. Generator and
audit commands are available through `tests/h1_tree_oracle.py --help`.

Integration `3eaae549a394fd3627aa79374bad5a849c39f169` retains the complete H1,
shared source parser, spill and mesh-archive production tuple unchanged from
`f624b704f85111061b4ca2570838950c90027e78`. Execution receipts retain their actual
immutable artifact identities. Integrated native tests pass 693 cases with
five ignored; native/portable clippy, formatting and tree-budget checks pass.
Latest-develop retained mesh processing and validation gates show no regressions
and byte-identical archives. Fresh GPU Cesium normal/refined probes pass for
mesh-local and GLB-wrap, with unchanged screenshots, requests and bytes, and no
load, console or server errors. The initial browser invocation used nonexistent
input paths; its failed receipt is retained as a coordinator invocation defect.
Exact integrated-head remote CI remains pending.

Bulk evidence belongs under
`/home/bend/.cache/rusty-tiles-evidence/h1-acceptance-20261012/`, with resource and
performance receipts under the sibling `h1-coordinator-20261012/` directory.
`final-independent-index.json`, SHA-256
`8a1036cc53aa1fb6854671c80c6f56a1fcaafbd257518c9bf53b829bb48d352a`,
binds final portable/native/optimized small receipts, scale/dense/resource and
processing/browser receipts, stopped witness, original counterexample,
retraction and integrated source manifest. `final-independent-review.txt`,
SHA-256
`bd2334b0c684a3c6fad0138b03af2c0a0bdf3bd6798b77e9e467c68f8459ee70`,
records the bounded local PASS and proof limits. Earlier pending-scale and
initial reviews remain separate historical evidence. Source-label and physical
sample wording corrections retain their original sidecars and verified
field-preserving correction chains; producer outputs did not change.

H1 does not accept tile output, adjacency, geographic placement, textures,
LOD, certification, picking or release readiness. It captures positions and
authored references; full normal/tangent/UV/color capture must be added before
source stability ends when a real tiler consumer arrives. The geometry planning
charge does not bound future encoded textured member size. Available-memory,
RSS and stopped-allocation observations here have their recorded Linux domain;
they do not establish all-platform resource behavior. #113/#121 remain open.
