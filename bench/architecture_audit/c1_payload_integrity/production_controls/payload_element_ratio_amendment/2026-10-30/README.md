# Payload-element budget amendment of the C1 payload-control expectations (2026-10-30, re-derived 2026-10-31)

The default `totalPayloadElements` budget is 16,000,000 + 130·S, an additive budget in the form of
its time statement, instead of max(16,000,000, 16·S); it is still model 1
([measured basis](../../../../../docs/architecture/validator-limits-amendment.md)). It was
max(16,000,000, 77·S) when this directory was first written (PR #304); PR #309 re-derived the
outputs in place from the same VS-C inputs, before any release, so that legitimate dense producer
output validates with a 1.5× margin. Every admitted report carries `limitDerivation`, so each
admitted expectation changes in exactly two fields:
- `limitDerivation.budgets.totalPayloadElements` becomes
  `{floor: 16000000, perStoredByte: [130, 1], source, additive: true}`, with `source` "scaled" iff
  130·S > 0;
- `limits.totalPayloadElements` becomes 16,000,000 + 130·S.

This directory amends the VS-C 2026-10-14 outputs **additively**; they stay as history.

## The rule

For every admitted case, `amend.py` recomputes S from the case archive with the VS-C script's
own ZIP central-directory parser (never the validator). S must equal the report's
`storedBytes`, and the old entry and limit must be exactly the 16-per-byte rule's. Otherwise it
refuses, lists the case and writes nothing. It never writes `expectedCategory` or
`expectedExitCode`, and a refused case must carry no report. `--check` rebuilds everything in
memory and compares it byte for byte with the committed files.

## Counts

| Lane | Cases | Admitted amended | Above the former floor (16·S > 16,000,000) |
| --- | --- | --- | --- |
| corrected-205 | 205 | 73 | none |
| polygon-sensitive-5 | 5 | 1 | none |
| stride6 | 6 | 4 | none |
| header9 | 9 | 3 | none |
| ktx2 (in place; the runner applies the rule after VS-B and VS-C) | 67 | 13 | `shared-view-heavy-kvd` (S = 8,768,585: 675,181,045) |

The lanes copy only `runner.py`, which resolves its own directory. Their manifest and
`integrity.json` are new, and every other file is a relative symlink to its unchanged target.
