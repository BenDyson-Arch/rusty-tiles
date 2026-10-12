# C2-WB cap amendment of the C1 payload-control expectations (2026-10-12)

C2-WB (the C2 work-bound correction) raises the default `containmentWorkUnits` from 2^28
to 2^29 (evidence: `bench/architecture_audit/c2_decoded_containment/acceptance/README.md`,
"C2-WB gate"). Every admitted C1 control compares its whole report, including `limits`, so
each admitted expectation must carry the new value.

This directory amends the validator-caps outputs **additively**. Nothing outside it is
modified; `validator_caps_amendment/2026-10-11/` and its inputs stay as history.

## The one rule

In every admitted case, `expectedReport.limits.containmentWorkUnits` changes from
268435456 to 536870912. `amend.py` asserts the old value before replacing it. Nothing else in
any report changes, and no refused case changes.

**Behaviour changes are refused.** An admitted control whose expected report carries
`decodedContentBounds.workLimit` could change verdict under a larger cap. `amend.py` lists
such a case and writes nothing. There are none: of the 82 admitted cases, 78 are Contained and
4 are `sceneSelection`.

## Inputs, verified by `amend.py` before reading

| Input | sha256 |
| --- | --- |
| `validator_caps_amendment/2026-10-11/corrected-205-caps-fixtures.tar.gz` | `b8b0499e456dbc48af4d071d95b12a9765c88add3231c6a73b3fbd07a03035c3` (manifest `55057d9d…`) |
| `validator_caps_amendment/2026-10-11/polygon-sensitive-5-caps-fixtures.tar.gz` | `7f5488810f3bc69dbdcc49df8e2c420315af7b24b6b0c8f44d4ee5d6c5e95ef4` (manifest `3f956fb1…`) |
| `validator_caps_amendment/2026-10-11/stride_controls/fixtures/manifest.json` | `a934bc207fe17670cb095f0b4023842dd4241ef1b65c9556bbd65aff4bde7040` |
| `validator_caps_amendment/2026-10-11/stride_controls/integrity.json` | `925429a64023bf5aa0ed8e1e5e037b694609ef97d593cba30ae8b6ae02010bbc` (every listed file is also verified) |

## Outputs

| Output | sha256 |
| --- | --- |
| `corrected-205-c2wb-fixtures.tar.gz` | `eee7f3777b0c96926f7fec860ff2d4446c3dd78313298cae4d5ee3e1cda4189f` (manifest `09c195d7e13b6e8635abe89a07283a365913d36ddd43997648b5f198c12dce3a`) |
| `polygon-sensitive-5-c2wb-fixtures.tar.gz` | `1355d6975deb5e9c08ab803fcaf2166149e3b2d38c5204e64fb999e1fee71a10` (manifest `3b59cb2ab47180111817c6a71583b83e5b09412404acbe6914644e0aef8cf3b8`) |
| `stride_controls/fixtures/manifest.json` | `c89a9382a90783f0a95df668abd1babf9f53719457f9cef0b80552abd21397ff` |

The bundles keep every member, order and tar attribute; only `manifest.json` differs (with
`c2wbCapAmendment` provenance, which `driver.py` ignores). The stride lane is a byte-identical
copy except `fixtures/manifest.json`, with `integrity.json` regenerated. `driver.py` is
unchanged. `tests/payload_integrity_oracle.py` and `tests/payload_meshopt_stride_oracle.py`
now name these outputs and their pins.

## Replay

```sh
python3 -I amend.py --check      # "identical" when the committed outputs match
python3 -I amend.py              # rewrite the outputs
```

Produced with CPython 3.14.7 and zlib-ng 2.3.3; a different zlib may give a different gzip hash
with identical contents.
