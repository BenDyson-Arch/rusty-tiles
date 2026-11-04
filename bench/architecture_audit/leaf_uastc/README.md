# Balanced leaf colour codec

Owner decision: 2026-11-03. Evidence: `/home/bend/.cache/rusty-tiles-evidence/walem-leaf-fidelity-20261103/REPORT.md`
(SHA-256 `32078a3ebaec483357a1be1cc48ea6f2a0dce591899fdbba0b78941686af7255`).
The implementation baseline is `f1c69172`; implementation acceptance is recorded at
`~/.cache/rusty-tiles-evidence/leaf-uastc-20261103/FINAL.md`.

Retain all quality/decode floors, escalation, page accounting, coarse and
non-colour policies. Replace balanced opaque leaf colour ETC1S q90 with
UASTC pack 0 + Zstd, no RDO. MASK retains its coverage-tested policy because
the source measurement does not establish alpha coverage fidelity.

Invariants: geometry, partition, density, sampler and HLOD codec policy stay
fixed. Every authored level is decoded and gated; misses escalate and structural
failures remain hard errors. Lossless and ID pages retain PNG. Page workspace
bounds already include UASTC escalation, so changing the initial mode must not
add a second encoder lease or reduce worker admission. Independent acceptance:
archive/member comparison, KTX2 structural validator, source-referenced tex3
PSNR, slot-1 CPU pairs and 4 GiB stage admission, plus full Walem delivery.

Full acceptance report: `~/.cache/rusty-tiles-evidence/leaf-uastc-20261103/FINAL.md`,
SHA-256 `3310f7ccdc9b183782d286ff0b323a7177885568be6b23740bd942d8004b7a1c`. Executed source: `abbc3520b7ea184bee503c65e4250aada1baa508`;
raw evidence was never tracked. Full Walem: 807.573 MB leaves, 973.328 MB archive,
2,658 converter CPU seconds. All 832 coarse GLBs and all leaf page dimensions
are unchanged. Published-G floor 0/12; max-honest-G 5/12. The report distinguishes
three fixed-worker CPU pairs from the actual five-worker admission pair.
