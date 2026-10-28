# HLOD support-check CPU pass

This bounded follow-up to S1b preserves the producer at `7f2a1c7e`.
The measured producer is `3382d55bc060a3a40e3803d6c375efcb60169c80`;
`ad57df7a` only corrects its resource comment. PR #288 remains stacked on S1b.

Three interleaved develop/candidate pairs per dataset compare whole user+system
CPU on slot 2, actual CPUs 0 and 2, two workers. Develop is `8a5eda68`.
Tex3 medians are 62.218971 -> 60.671785 seconds (passes the +5% target).
Crop3 medians are 15.273388 -> 17.851549 seconds (+16.88%, fails).
The failure is not waived. The change preserves all guards and all five
tex3/crop3/collinear/book/sheet archives and normalized producer facts.

Bulk evidence was generated outside git at
`~/.cache/rusty-tiles-evidence/hlod-s1b-20261026/cpu-20261027/` and has never
been committed. Receipt source revision is `3382d55b`; final comment checks
are bound to `ad57df7a`. Selected immutable receipts and SHA-256:

- `paired-summary-v1.json`: `c7ec3de42b7608f940a03a38ffbb3f14ed34e8ab3907fe896e41450a6b251f1e`
- `second-v1-identity.json`: `05f612f4b6a64728a8db1de0accf3f19b5567aacb63f28c353173656f788e017`
- `independent-review-v1.md`: `1aec0be5b41ebb8e693e541ffe47139d98bd3fac2c287b4fe260b9613741915d`

The review independently checks source ownership, the identity-decode proof,
all five archive hashes, audit facts and the CPU medians. Including opening
pins, the derived peak is below 1,680 bytes per merged triangle under the
unchanged 1,700-byte charge. Complete-job allocator controls measured 1,111.
Final lint/library receipts and the follow-up review remain in that evidence
directory; they distinguish inherited D1-validator failures from this CPU work.

Final follow-up `independent-review-v2.md` SHA-256:
`1ce574ae2a87bb1e52d1f761c8ed77cc8bca06eb868bf2591dbc69ae9bba77c4`. It confirms the comment fix, executable-source
equivalence, artifacts, memory proof and final checks, and retains the CPU failure.
The CI-like library lane has 1093 passes and the same 12 legacy D1/S5 failures
as frozen S1b (no new failures); test execution is 515.31 s versus 548.44 s.
Clippy, fmt, tree and changelog pass. This is not full integration or release acceptance.

## Second pass, after S1c (2026-10-28, PR #290)

Measured producer `3661e713`, base develop `e3ee6045`; tex3/crop3 archives byte-identical
to develop. Ten interleaved pairs vs `9af55ee1` (slot 1): tex3 whole x1.041, coarse x1.055;
crop3 whole x1.085, coarse x1.108 (ratio of medians). S2, D4 and witness searches are now
cheaper than in 9af55ee1; the remaining crop3 excess is S1b's fold-retry passes, fold source
rebuilds and witnesses. Bulk evidence: `~/.cache/rusty-tiles-evidence/hlod-cpu-20261028/`,
never committed. `FINAL-3661e713.md` SHA-256: `1c713cb9f3ee24f4ade7270dfb30167aa459854ad9e8a6ea89c69e93487ab39d`.
