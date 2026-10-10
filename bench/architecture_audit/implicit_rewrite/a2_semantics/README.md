# A2 finite semantic evidence

Read [audit.md](audit.md) for primary semantics, consumer/source pins, concrete
representation decisions, current point/vector use-case dispositions and limits.

[probe.py](probe.py) independently verifies four five-node QUADTREE/OCTREE
design fixtures and rejects twenty-two sensitive corruptions. It includes
source-order addresses with coincident child boxes, exact world transforms,
ordered payload slots, availability ranks, authored errors and overlapping-box
metadata. [results.json](results.json) stores exact results and source identities.

[real_probe.py](real_probe.py) independently reads eleven small current producer
archives and six supplied legacy outputs, checking paired frames, semantic rows,
errors, addresses and payload aliases. [real-results.json](real-results.json)
pins archive bytes and exact local-cell excess without claiming geometry decode.

```sh
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/a2_semantics/probe.py
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/a2_semantics/real_probe.py /tmp/rusty-tiles-a2-probe-artifacts-real-portable-final2
```

The second command requires those exact externally retained probe artifacts.
Both commands are bounded to ten seconds and small fixture limits. No Cargo,
production expander, patched traverser or production implementation participates.
Terminal implicit external-template interpretation and actual viewer acceptance
remain gates; no legacy removal or release is accepted.
