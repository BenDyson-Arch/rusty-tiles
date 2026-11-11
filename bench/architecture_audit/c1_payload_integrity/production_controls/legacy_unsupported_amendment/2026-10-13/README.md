# H7 legacy-content amendment of the C1 payload-control expectations (2026-10-13)

The [C1 H7 amendment](../../../../../../docs/architecture/c1-legacy-content-unsupported-amendment.md)
makes legacy 3D Tiles 1.0 content (b3dm, i3dm, pnts, cmpt) Unsupported before any format byte
is interpreted. The frozen corrected-205 bundle holds 12 controls whose content member is a
`.b3dm` and which expected the b3dm v1 framing and table rules: 4 admitted and 8 InvalidInput.

This directory amends the C2-WB-cap outputs **additively**. Nothing outside it is modified;
`c2wb_cap_amendment/2026-10-12/` and its inputs stay as history.

## The one rule

Each of the 12 named `b3dm-*` controls expects `unsupported` (exit 2) with no report, and its
`c2OracleOutcome` is dropped. `amend.py` asserts each one's category before the change and
that its member set holds a `.b3dm`. A `.b3dm` member in any other case is refused, and
nothing is written. Every other case, member, order and tar attribute is copied unchanged.

| Control | Before | After |
| --- | --- | --- |
| `b3dm-zero`, `b3dm-decimal`, `b3dm-exponent`, `b3dm-nonzero` | admitted | unsupported |
| `b3dm-fraction`, `b3dm-u32-overflow`, `b3dm-huge`, `b3dm-null`, `b3dm-missing`, `b3dm-duplicate`, `b3dm-feature-unicode`, `b3dm-batch-unicode` | invalid_input | unsupported |

`polygon-sensitive-5` and the stride lane hold no legacy content and are not amended;
`tests/payload_integrity_oracle.py` keeps the C2-WB-cap polygon bundle.

## Input and outputs

| File | sha256 |
| --- | --- |
| input `c2wb_cap_amendment/2026-10-12/corrected-205-c2wb-fixtures.tar.gz` | `7d844ecf1fc99719b0807f2859bbc6e988ee110283d5f001c25a0eee563fdb7e` (manifest `3ef63c08433e1b69573e83c9aef0fe00972f94b9160e7934b07974bad956ae35`) |
| `corrected-205-legacy-fixtures.tar.gz` | `7fe07fd1341d8fdbc743be9cb7cd0b1fa063731b5678621d652e799ad0101b8d` (manifest `f4ce9c7bd60e7788ca9ec7d6af5c7edf20f08e2c2cc8f25ced32a2d2c9a01815`) |

`driver.py` is unchanged; it ignores the `legacyUnsupportedAmendment` provenance key. With the
H7 debug CLI, `tests/payload_integrity_oracle.py` passes all 210 cases (205 + 5).

## Replay

```sh
python3 -I amend.py --check      # "identical" when the committed outputs match
python3 -I amend.py              # rewrite the outputs
```

Produced with CPython 3.14 and the system zlib; a different zlib may give a different gzip hash
with identical contents.
