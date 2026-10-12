# H1 independent capture/tree acceptance

Acceptance is pending. This directory retains the independent reference for
the [H1 contract](../../../docs/architecture/h1-capture-tree-contract.md), plus
the durable location of bulk execution and review evidence.

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

Bulk evidence belongs under
`/home/bend/.cache/rusty-tiles-evidence/h1-acceptance-20261012/`.
The precontract audit there separates static risks and unresolved decisions
from executed defects. Final evidence must bind exact source, test binary,
runner configuration, dump identities and reviewer findings. A small-input
oracle does not establish the resident/scratch bounds or replace the required
spilled ten-million-triangle resource run and final-source nonauthor review.

H1 does not accept tile output, adjacency, geographic placement, textures,
LOD, certification, picking or release readiness. The geometry planning charge
does not bound future encoded textured member size.
