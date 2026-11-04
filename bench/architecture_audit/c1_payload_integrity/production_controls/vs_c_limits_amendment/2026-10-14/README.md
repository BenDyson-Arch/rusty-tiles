# VS-C limits amendment of the C1 payload-control expectations (2026-10-14)

[VS-C-1 and VS-C-4](../../../../../docs/architecture/validator-limits-amendment.md): every
successful validation report gains `storedBytes` (S, the sum of the stored sizes of every
central-directory entry, the `@3dtilesIndex1@` index included) and `limitDerivation`
(model 1: per budget its floor, ratio and source `floor` or `scaled`); in `limits` the four
volume caps `sourceArchiveBytes`, `archiveEntries`, `centralDirectoryBytes` and
`archiveStoredBytes` are `null` and `liveDocumentBytes` (268435456) is added. The C1 CLI lanes
compare the whole report, so each admitted expectation must carry this.

This directory amends the earlier outputs **additively**. The H7, C2-WB and
meshopt-header inputs stay as history; `driver.py` and the C1 contract are unchanged.

## The one rule

For every admitted case `amend.py` parses the case archive's ZIP central directory (its own
parser, cross-checked against `zipfile`; never the validator) and computes S. It asserts the
old state first (the four caps equal their old numbers, none of `storedBytes`,
`limitDerivation`, `liveDocumentBytes` present, `entries` equals the directory count, and
each budget equals `max(floor, S*p//q)`, which checks consistency with VS-B), then writes
`storedBytes`, `limitDerivation` (from the rule table), nulls the caps and adds
`liveDocumentBytes`. Nothing else in any report changes.

**Behaviour changes are refused.** The script never writes `expectedCategory` or
`expectedExitCode`, refuses a refused case that carries a report and any admitted case not in
the old state, lists it and writes nothing. No case was refused. No frozen lane compares a
refusal message except parent1, whose message ("meshopt and bufferView stride disagree") is not
a cap or budget refusal and gains no suffix, so no refused case changed. Whether the new binary
changes any verdict is decided by running the oracles: all pass.

## Counts

| Lane | Cases | Admitted amended | Scaled budgets |
| --- | --- | --- | --- |
| corrected-205 | 205 | 73 | none |
| polygon-sensitive-5 | 5 | 1 | none |
| stride6 | 6 | 4 | none |
| parent1 | 1 | 0 (not copied, unchanged) | - |
| header9 | 9 | 3 | none |
| ktx2 | 67 | 13 | `shared-view-heavy-kvd` (S = 8,768,585): visits, references, elements, containment units |

## Inputs, verified by `amend.py` before reading

| Input | sha256 |
| --- | --- |
| `legacy_unsupported_amendment/2026-10-13/corrected-205-legacy-fixtures.tar.gz` | `7fe07fd1341d8fdbc743be9cb7cd0b1fa063731b5678621d652e799ad0101b8d` (manifest `f4ce9c7b…`) |
| `c2wb_cap_amendment/2026-10-12/polygon-sensitive-5-c2wb-fixtures.tar.gz` | `d64eaf9ade5bb7b3dd5d73a5b2398a12083c2bc3ae948a3a250a07139971b25f` (manifest `2d5d56c0…`) |
| `c2wb_cap_amendment/2026-10-12/stride_controls/fixtures/manifest.json` | `83ca40eb831ad802e1ce1fbbfb1b4ede45905761c305dd91821c8b58bd08d345` (every file of its `integrity.json` is also verified) |
| `meshopt_header_amendment/2026-10-12` | every file of its `integrity.json` |

## Outputs

| Output | sha256 |
| --- | --- |
| `corrected-205-vsc-fixtures.tar.gz` | `229e97f7ce0e81b59cef5079106e47fec09320b4cdd63314098f4d845f4ca5e0` (manifest `3f248b5ed05d1b814dcbea71be6445066cabdaca0870345b21d446ed0c74e6ca`) |
| `polygon-sensitive-5-vsc-fixtures.tar.gz` | `f363386d920356499c823a9ded616e945061feb66f049cd5c600797b675478dc` (manifest `d0197d8501786535556ce806ec58523ba65d6a7a5a4ffdde176959f762827e67`) |
| `stride_controls/fixtures/manifest.json` | `38d693ea276bbf9941738ab5ea21589442373daf8161fec4798475dcbd9abe07` |
| `meshopt_header/fixtures/manifest.json` | `22ff1456dba290926fc770ce624f9dbc8c6371c831b7622f2a32e4c4b70b24e9` |
| `ktx2_basisu_amendment/2026-10-13/fixtures/manifest.json` (in place) | `4d6b39fd9714e3a90e5492d609acdad856c8cb949f0efec6696a4ad3c152d848` |

The bundles keep every member, order and tar attribute; only `manifest.json` differs (with
`vsCLimitsAmendment` provenance, which `driver.py` ignores). In the two new lanes only
`fixtures/manifest.json` and `integrity.json` are new files and `runner.py` is copied (it
locates its lane from its own resolved path, so its hash pin is unchanged); every other file
is a relative symlink to its unchanged target. The KTX2 lane regenerates its archives, so, as
VS-B did, it is amended in place: its `runner.py` applies this rule (`amend_cases` from
`amend.py`) after the VS-B rule to the regenerated cases before comparing them with the
manifest, and `amend.py --ktx2 ARCHIVE_ROOT` records the manifest, the runner hash
(`f24e9389…`) and the lane `integrity.json`. `tests/payload_integrity_oracle.py` and
`tests/payload_meshopt_stride_oracle.py` name these outputs and their pins.

amend.py sha256 `820fb6533693e26a18e80c8936375d67cf5454af97a0de302d5f963d52340b04`.

## Replay

```sh
python3 -I amend.py --check                     # "identical" when the committed outputs match
python3 -I amend.py                             # rewrite the bundles and the two lanes
python3 -I amend.py --ktx2 ARCHIVE_ROOT --check # ARCHIVE_ROOT: archives regenerated by the KTX2 generator
```

Produced with CPython 3.14 and the system zlib; a different zlib may give a different gzip hash
with identical contents.
