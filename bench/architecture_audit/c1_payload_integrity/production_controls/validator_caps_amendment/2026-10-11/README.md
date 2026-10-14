# Validator-caps amendment of the C1 payload-control expectations (2026-10-11)

The validator-caps slice raises seven `ValidationLimits` defaults; the evidence
is in `bench/architecture_audit/validator_caps`. Every admitted C1 control
compares its whole report, including `limits`, so each admitted expectation
must carry the new values. No refusal depends on a raised ceiling.

This directory amends those expectations **additively**. Nothing outside it is
modified. The C2-amended inputs stay where they are as history:

- `c2_amendment/2026-10-11/*.tar.gz`
- `c2_amendment/2026-10-11/stride_controls/`

Their own inputs (`polygon_correction/2026-10-10`, `stride_controls/2026-10-10`)
are untouched too.

## The one rule

In every admitted case, `expectedReport.limits` takes the new values. `amend.py`
asserts each old value before it replaces it. Nothing else in any report
changes, and no refused case changes.

| Serialized field | Old | New |
| --- | --- | --- |
| `archiveEntries` | 65536 | 524288 |
| `hierarchyVisits` | 65536 | 262144 |
| `references` | 262144 | 1048576 |
| `centralDirectoryBytes` | 16777216 | 33554432 |
| `documentItems` | 65536 | 131072 |
| `memberBytes` | 67108864 | 134217728 |
| `documentDecodedBytes` | 67108864 | 134217728 |

**Behaviour changes are refused.** A `resource_limit` control whose fixture
could depend on a raised ceiling would turn into an admission. That is not a
report delta. `amend.py` lists such a case and writes nothing. There are only
four `resource_limit` controls, all in corrected-205, and none qualifies. They
are `exact-count-18446744073709551615`, `exact-count-18446744073709551616`,
`exact-count-1e400` and `declared-length-overflow`. All four refuse
u64-overflowing values that no finite ceiling admits.

## Inputs, verified by `amend.py` before reading

| Input | sha256 |
| --- | --- |
| `c2_amendment/2026-10-11/corrected-205-c2-fixtures.tar.gz` | `ebf8437f1fc0711688fdef74f7b076939dd55f177bc663159a3527049e5b8481` (manifest `fd6004a4cd4074553d5ff08542fdd9b355d2db86c6035c695dc4513be5e3408c`) |
| `c2_amendment/2026-10-11/polygon-sensitive-5-c2-fixtures.tar.gz` | `94ae2cefc6214ac4ecf1ef972187ecbcd2275f2d44526152be13c46719e50189` (manifest `61237a250af51352098dbabeb282859aa38a0528aa9048c19f8aee82aaccca00`) |
| `c2_amendment/2026-10-11/stride_controls/fixtures/manifest.json` | `8516d51342c559d983e9f7fa83adeb3dc320078a21bf7ff9438c2fb9783e3557` |
| `c2_amendment/2026-10-11/stride_controls/integrity.json` | `dea02a0bd364b2e0f19629fc0e9ce42812ba3a78d027e409b901d3f3687a36f6` (every listed file is also verified) |

## Outputs

| Output | sha256 |
| --- | --- |
| `corrected-205-caps-fixtures.tar.gz` | `b8b0499e456dbc48af4d071d95b12a9765c88add3231c6a73b3fbd07a03035c3` (manifest `55057d9df744d977044d909875f1cbed6c981867ff2d30e7e22085796a4c039e`); 77 admitted cases amended |
| `polygon-sensitive-5-caps-fixtures.tar.gz` | `7f5488810f3bc69dbdcc49df8e2c420315af7b24b6b0c8f44d4ee5d6c5e95ef4` (manifest `3f956fb18f8a7e3e9a42070706a8a105ad197242c5d82f6d13fef5826bcbe9bc`); 1 amended |
| `stride_controls/fixtures/manifest.json` | `a934bc207fe17670cb095f0b4023842dd4241ef1b65c9556bbd65aff4bde7040`; 4 amended |
| `stride_controls/integrity.json` | `925429a64023bf5aa0ed8e1e5e037b694609ef97d593cba30ae8b6ae02010bbc` |
| `amendment-manifest.json` | `da73d74b7b3bc877b558ddb441cb846b1d97b134678a4544204d3d1e2e9984ef` |

- **The two bundles** keep every member, its order and its tar attributes.
  Only `manifest.json` differs. Each manifest also records `capsAmendment`
  provenance, which `driver.py` ignores.
- **The stride lane** is a full byte-identical copy of the C2-amended lane,
  except for `fixtures/manifest.json`. `integrity.json` is regenerated for the
  lane.
- **`driver.py` is unchanged** (sha256 `5bcd8e1e…`).

The live runners were repointed:

- `tests/payload_integrity_oracle.py` now names these bundles and their pins.
- `tests/payload_meshopt_stride_oracle.py` now names this stride lane and its
  manifest pin.
- The `parent1` lane (`stride_parent_control/2026-10-10`) has no admitted case,
  so it needs no amendment.

## Replay

```sh
python3 -I amend.py --check      # rebuild in memory; "identical" when the committed outputs match
python3 -I amend.py              # rewrite the outputs
```

The gzip bytes depend on the zlib build. They were produced with CPython 3.14.7
and zlib-ng 2.3.3 (zlib API 1.3.1). A different zlib may give a different
bundle hash with identical contents.
