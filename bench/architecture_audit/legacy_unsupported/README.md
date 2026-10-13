# H7 controls: legacy 3D Tiles 1.0 content is Unsupported

These are the sensitive controls for the
[C1 H7 amendment](../../../docs/architecture/c1-legacy-content-unsupported-amendment.md). Under
it, b3dm, i3dm, pnts and cmpt payloads are Unsupported. They are never InvalidInput, and they
are never partially inspected. The controls were authored before the production change. They
use only the Python standard library, with no repository import and no producer.

| File | Role |
| --- | --- |
| `generate.py` | Writes `fixtures/` (36 stored 3TZ archives, 44,603 bytes, byte-deterministic) and `fixtures/manifest.json` with the declared expectations |
| `check.py` | Runs the CLI (`--binary`), the CLI against the pre-H7 predictions (`--develop`), the C2/I1 oracles (`--oracle`, which runs `c2_oracle.py --implicit`), or a regeneration identity check (`--regenerate`) |
| `oracle_mutants.py` | 11 oracle mutants on a temporary copy. Each must fail `check.py --oracle` or a pin-free oracle self-test |

## Manifest fields

| Field | Meaning |
| --- | --- |
| `expectedCategory`, `expectedExitCode` | Verdict after H7: `admitted`, `unsupported` (2) or `invalid_input` (3) |
| `rule` | The step that decides the case. `L0` member existence; `L1` final name suffix, before any read; `L2` first four bytes, after the existing read; `T` implicit content template at root admission; `order` an earlier C1 check wins; `generic` a non-legacy unknown suffix; `none` admitted |
| `messageIncludes` | Substrings the CLI error message must contain: `legacy`, the lowercase format name and the member or document name |
| `developCategory` | Predicted verdict of the pre-H7 validator (`origin/develop` 87b7078). It was checked 36/36 on that binary |
| `c2OracleOutcome` | Expected outcome of the amended C2 oracle with `--implicit` |

## Coverage

- **Valid legacy payloads by suffix:** b3dm (lowercase and `.B3DM`), i3dm, pnts, cmpt holding
  a b3dm, and cmpt wrapping a GLB.
- **Corrupt legacy payloads:** magic only, version 2, malformed feature JSON, `RTC_CENTER`
  and a truncated cmpt. Each gets the same verdict, so no field is parsed.
- **Legacy bytes under a glTF name:** `.glb` holding each of the four formats, and `.gltf`
  holding a b3dm.
- **Order:** a missing member stays InvalidInput (L0), checked on both an explicit and a
  retained caller. "Content bounds escape tile" wins over a legacy verdict. Traversal order
  decides between a malformed GLB and a b3dm, tested in both orders. A C2 exclusion elsewhere
  does not mask the verdict. Legacy content is also tested inside an external tileset and
  as a URI referenced twice.
- **Implicit:**
  - `.b3dm`, `.PNTS` and `.cmpt/.` content templates are Unsupported. Their subtree member is
    deliberately absent, so a lazy or missing rule shows up as InvalidInput.
  - b3dm bytes are emitted under a `.glb` template.
  - A subtree template that ends `.b3dm` is not content, so the archive is admitted.
- **The other two callers:** a b3dm in `conversion.json` `retainedContentUris`, and one in
  tileset `extras.rustyTilesSourceContents`.
- **Negative controls:**
  - a GLB triangle and an implicit GLB;
  - legacy words in a directory name, in the stem and in the JSON (substring mutants);
  - unknown `.bin` and `.vctr` suffixes, which keep the generic message.

The C2 and implicit oracles do not model three C1-only paths: the content-volume check,
`conversion.json` and `rustyTilesSourceContents`. For those cases `c2OracleOutcome` records
what the oracle decides from its own scope. The C1 corpus oracle
(`tests/c1_validation_oracle.py`) covers these paths with its own `legacy_*` controls.

## Results recorded on 2026-10-13

These results come from origin/develop 87b7078 and the debug CLI.

- `check.py --develop`: all 36 pre-H7 predictions hold.
- `check.py --binary`: develop fails 26 of the 36 H7 expectations. On 20 of them the category
  differs. On the other 6 only the message differs: i3dm, pnts and cmpt are already
  Unsupported by the generic suffix rule.
- `check.py --oracle`: 36/36. The pre-H7 oracle differs on 26 cases.
- `oracle_mutants.py`: the unmutated copy passes, and all 11 mutants are killed.

With the H7 production change (debug CLI):

- `check.py --binary`: 36/36, including every `messageIncludes` token.
- `check.py --oracle` and `--regenerate`: 36/36, fixtures byte-identical.
- The 22 production mutants of the plan (PM1-PM22) each fail at least one of these controls,
  a C1 corpus control or a Rust control; the clean tree fails none.

Bulk receipts are kept outside git in `~/.cache/rusty-tiles-evidence/h7-20261013/`. That
directory also holds the plan, which includes the production mutant list PM1-PM22.

## Replay

```sh
python3 -I generate.py --out /tmp/h7-fixtures     # or regenerate in place after deleting fixtures/
python3 -I check.py --regenerate --oracle
python3 -I check.py --binary target/debug/rusty-tiles             # after the H7 production change
python3 -I check.py --binary <pre-H7 rusty-tiles> --develop
python3 -I oracle_mutants.py
```
