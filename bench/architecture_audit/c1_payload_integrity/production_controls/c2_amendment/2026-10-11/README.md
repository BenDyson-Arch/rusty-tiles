# C2 amendment of the C1 payload-control expectations (2026-10-11)

C2 (decoded-content containment, PR #157) changes the report of every archive
that C1 admits. The C1 controls pinned on 2026-10-10 compare the whole report,
so CI rejected every admitted case. Every refusal still matched, and every
admitted case was still admitted.

This directory amends those expectations **additively**. Nothing outside it is
modified. The original bundles and lanes stay where they are as historical
evidence:

- `polygon_correction/2026-10-10/*.tar.gz`
- `stride_controls/2026-10-10/`

The C2 part of every amended expectation comes from the independent C2
O-explicit oracle (`bench/architecture_audit/c2_decoded_containment/oracle`),
which does not read `src/`. It is **not** copied from production output.
Production's output was compared only afterwards, as the end-to-end check below.

## Inputs and their pins

`amend.py` verifies these before reading:

| Input | sha256 |
| --- | --- |
| `polygon_correction/2026-10-10/corrected-205-fixtures.tar.gz` | `c05406cdb264e386d1ad209abfded1ecc0b2b97da8c8b32ad8ff911263aa46a4` (manifest `40fae6bf…`) |
| `polygon_correction/2026-10-10/polygon-sensitive-5-fixtures.tar.gz` | `add990418822a1118c9c3be2faff04fa0722ae926c2d97769353f90c928a6f2c` (manifest `a26782e5…`) |
| `stride_controls/2026-10-10/fixtures/manifest.json` | `a3c07122e5fc902272bcf668a85e694601551a66556a7be3806132febd592aee` |
| `stride_controls/2026-10-10/integrity.json` | `3662f04fc170af1d3869b6167ab22ba0ab5451c66edc0ff9ebd2ee2616c03b53` (every listed file is also verified) |

## Rules applied to each admitted case

The rules come from the contract's "Report and failure schema" (R7.1, R7.4),
applied to the original `expectedReport`.

1. **The C1 payload label is split.** "scene transforms, skins, animations,
   morph targets and rendered bounds" becomes "skins, animations, morph targets
   and rendered extent", plus "scene transforms" unless the C2 outcome is
   Contained.
2. **Contained:** `checks` gains `decodedContentContainment`, appended after
   the C1 checks, and `notInspected` loses `decodedContentBounds`.
3. **NotInspected(reasons):** `notInspected` keeps `decodedContentBounds` and
   gains one `decodedContentBounds.<reason>` per reason.
4. `notInspected` is sorted and deduplicated.
5. **`limits`** gains the five C2 `ValidationLimits` fields at their defaults:
   - `containmentSpanBits` 4096
   - `containmentWorkUnits` 268435456 (the coordinator's 2^28 decision)
   - `containmentBytesRead` 2147483648
   - `containmentDecodedBytes` 2147483648
   - `containmentNodeDepth` 64

Each amended case also records `c2OracleOutcome`, and each manifest records
`c2Amendment` provenance. `driver.py` and the stride `runner.py` ignore these
extra keys.

**Behaviour changes are refused.** An admitted fixture that the oracle finds
Outside, InvalidNode or InvalidInput, or cannot decide, would mean an admitted
case turns into a failure. That is not a report delta. `amend.py` prints it
loudly and writes nothing. **There were none.**

## Outputs

| Output | sha256 |
| --- | --- |
| `corrected-205-c2-fixtures.tar.gz` | `ebf8437f1fc0711688fdef74f7b076939dd55f177bc663159a3527049e5b8481` (manifest `fd6004a4cd4074553d5ff08542fdd9b355d2db86c6035c695dc4513be5e3408c`) |
| `polygon-sensitive-5-c2-fixtures.tar.gz` | `94ae2cefc6214ac4ecf1ef972187ecbcd2275f2d44526152be13c46719e50189` (manifest `61237a250af51352098dbabeb282859aa38a0528aa9048c19f8aee82aaccca00`) |
| `stride_controls/fixtures/manifest.json` | `8516d51342c559d983e9f7fa83adeb3dc320078a21bf7ff9438c2fb9783e3557` |
| `amendment-manifest.json` | the per-case C2 outcomes and all hashes |

- **The two bundles** keep every member, its order and its tar attributes.
  Only `manifest.json` differs. The tar is PAX and the gzip has mtime 0.
- **The stride lane** is a full copy of `stride_controls/2026-10-10`. Its
  runner reads `fixtures/` beside itself, so the runner, generator, primary
  references and fixture bytes are copied byte-identically. Only
  `fixtures/manifest.json` differs, and `integrity.json` is regenerated for the
  lane.
- **`driver.py` is unchanged** (sha256 `5bcd8e1e…`). Its `run()` already takes
  the fixture directory, so no change was necessary.

## C2 outcomes

There are 82 admitted cases. 78 are Contained and 4 are
NotInspected(sceneSelection). None is Outside.

| Bundle | Admitted | Contained | NotInspected(sceneSelection) |
| --- | --- | --- | --- |
| corrected-205 | 77 | 74 | 3 (scene-less GLBs) |
| polygon-sensitive-5 | 1 | 0 | 1 |
| stride_controls | 4 | 4 | 0 |

## Replay

```sh
python3 -I amend.py --check      # rebuild in memory; "identical" when the committed outputs match
python3 -I amend.py              # rewrite the outputs
```

The gzip bytes depend on the zlib build. They were produced with zlib-ng
1.3.1 and CPython 3.14.7. A different zlib may give a different bundle hash
with identical contents.

## End-to-end check (2026-10-11)

These are the exact CI commands, run with the C2-branch binary
`/home/bend/.cache/c2-int/target/release/rusty-tiles` (sha256 `41a07d60…`):

| Command | Result |
| --- | --- |
| `tests/c1_validation_oracle.py` | exit 0 |
| `bench/architecture_audit/c1/resource_probe.py` | exit 0, 20 runs |
| `tests/c1_producer_alignment.py` | exit 0, 8 b3dm controls |
| `tests/archive_read_oracle.py` | exit 0, 45 cases |
| `tests/payload_integrity_oracle.py` | exit 0: corrected205 205/205, polygon5 5/5 |
| `tests/payload_meshopt_stride_oracle.py` | exit 0: stride6 6/6, parent1 1/1 |

Production's reports match the independently derived expectations exactly.
That includes the position of `decodedContentContainment` at the end of
`checks`. The contract does not fix that position; it was the oracle author's
reading.

- `c1_validation_oracle.py`, `c1_producer_alignment.py` and
  `archive_read_oracle.py` compare outcome categories only, not
  `checks`/`notInspected`, so they needed no amendment.
- The C2 oracle finds no Outside among their archives either.
- `stride_parent_control` has no admitted case, so it needed no amendment.
