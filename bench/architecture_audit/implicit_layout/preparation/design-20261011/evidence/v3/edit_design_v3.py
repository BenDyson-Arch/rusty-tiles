"""One-shot v3 text edits to design.md (exact-match replacements; fails loudly if any anchor is missing)."""
import sys
p = sys.argv[1]
s = open(p).read()


def rep(old, new):
    global s
    if old not in s:
        raise SystemExit('anchor missing: ' + old[:100])
    s = s.replace(old, new)


rep("# P-IMPL: implicit layout without external-JSON content templates (v2)",
    "# P-IMPL: implicit layout without external-JSON content templates (v3)")
rep("""Status: **v2 for PN's delta review**. It responds to the NO-GO in
`/tmp/pimpl-precode-review-20261011/review.md` (`16205c1c…`).
- [change-log.md](change-log.md) maps R-1 to R-14 to the changes. v1 is in [v1/](v1/).""",
    """Status: **v3 for PN's delta review**. v2 responded to the NO-GO in
`/tmp/pimpl-precode-review-20261011/review.md` (`16205c1c…`). v3 applies the user's decisions on
PD-11 to PD-13 (2026-10-11).
- [change-log.md](change-log.md) has a v3 section and maps R-1 to R-14 (v2).
- v1 is in [v1/](v1/) and v2 in [v2/](v2/).
- v3 changes:
  - **PD-11.** Vector archives no longer carry duplicate raw sources. `--reuse-tileset` reads the
    archive's published payloads through a reversible rebase (§4.7, §5.4).
  - **PD-12.** Per-tile entry and visit counts are recorded for the coordinator's validator-cap
    slice (§5.1).
  - **PD-13.** Accepted: I1 excludes link tiles from `budgets.tiles`.""")
rep("""- **Payloads.** Implicit-tile payloads have `δ(t)` baked into the GLB scene roots as
  `(δ0, δ2, −δ1)` (`translate_content`, kept). `δ(r) = 0` gives a byte copy. A `.gltf` can occur
  only at r.""",
    """- **Payloads (v3: reversible rebase).** Implicit-tile payloads have `δ(t)` baked into the GLB
  scene root as `(δ0, δ2, −δ1)` by replacing **only the root node object's text** in the JSON
  chunk with its serialization after adding δ to its `translation` (absent means 0).
  - Every other JSON byte, the BIN chunk and the source JSON chunk alignment are untouched.
  - The writer returns the raw root node text and the raw JSON chunk length for each rewritten
    payload (§4.7).
  - `δ(r) = 0` gives a byte copy. A `.gltf` can occur only at r.
  - Today's `translate_content` re-serializes the whole JSON chunk. That is not reversible for
    meshopt payloads, whose raw key order is not sorted: 0 of 544 recoverable by inverse splice
    (§5.4).""")
rep("""- The writer packs breadth-first. Each skeleton tile is taken with its *unit*: its explicit leaves,
  its implicit-root tiles, and the retained source names they add (vector).""",
    """- The writer packs breadth-first. Each skeleton tile is taken with its *unit*: its explicit leaves
  and its implicit-root tiles.""")
rep("""  t without transform, extras (retained list, below)}`.""", """  t without transform}`.""")
rep("""**Retained sources (vector, `SourcePolicy::Retain`; R-3, PD-11 (b)).**
- A document that holds implicit roots lists, in its top-level `extras.rustyTilesSourceContents`,
  the raw `t/*` sources of every payload rewritten under those roots.
- The document budget counts these names.
- Point cloud, mesh and M2 (`Remove`) delete those sources instead.""",
    """**No retained sources (v3; PD-11 user decision).**
- No producer keeps a rewritten raw payload. Every member is referenced by a tile, a template or a
  link.
- No document carries `rustyTilesSourceContents`.
- Vector reuse reads the published payloads instead (§4.7).""")
rep("""  Every name a template produces for available content exists. Every written member is referenced
  by a tile, a template, a link, or a document's retained-source list (R-3).""",
    """  Every name a template produces for available content exists. Every written member is referenced
  by a tile, a template or a link. Vector's `vector-build.json` stays referenced through
  `asset.extras.vectorBuildStateSha256`, as today.""")
rep("""| Point cloud (PP) | `write_forest(…, Remove)`; `tree.members = output.members`; encoder `…-implicit-v3` |
| Vector (PV) | `write_forest(…, Retain)`. Every report""",
    """| Point cloud (PP) | `write_forest(…)`; `tree.members = output.members`; encoder `…-implicit-v3` |
| Vector (PV) | `write_forest(…)`; no retained sources. `vector-build.json` records the payload map for reuse (§4.7). Every report""")
rep("""| Legacy mesh `tile.rs:510-540` (PP) | `write_forest(…, Remove)`; stage `output.members` instead of hard-coded directories |
| M2 `tileset.rs:153-177` (PP) | **Its writer call changes** to `write_forest(…, Remove)` (R-12a); encoder string |""",
    """| Legacy mesh `tile.rs:510-540` (PP) | `write_forest(…)`; stage `output.members` instead of hard-coded directories |
| M2 `tileset.rs:153-177` (PP) | **Its writer call changes** to `write_forest(…)` (R-12a); encoder string |

`SourcePolicy` (v2) is gone: every producer deletes the raw sources of rewritten payloads.""")
rep("""**`ForestOutput.tiles` (R-6).** One record per tile of H, in preorder, with role, written extras, and
`(member, bytes)` per content.""",
    """**`ForestOutput.tiles` (R-6).** One record per tile of H, in preorder, with role, written extras, and
`(member, bytes)` per content. Each rewritten payload also carries its rebase record: δ, the raw
root node text, the raw JSON chunk length, and the raw sha256.""")
rep("""- **Retained:** `translate_content`; `Subtree` encoding; the schema; `content_suffix`.""",
    """- **Retained:** `Subtree` encoding; the schema; `content_suffix`.
- **Replaced:** `translate_content` becomes `rebase_payload` (the root-node splice) plus its exact
  inverse `restore_payload`. Both are PW's, and PV uses the inverse in reuse.""")
rep("""- **Change needed: the retained-source gate (R-3).** I1-B drops the `implicitTiling` condition at
  `validate.rs:1204` and honours `extras.rustyTilesSourceContents` on every tileset document
  (ownership plan, I1 asks 1).
- **Asks:** W11 witnesses (I1 asks 3) and link tiles in `budgets.tiles` (PD-13).""",
    """- **Retained-source gate: no I1 change (v3 drops v2's R-3 ask).** New outputs carry no
  `rustyTilesSourceContents`.
  - I1 moves `validate.rs:1204-1217` into `prepare_tileset` unchanged (I1 `:680-681`), so older
    archives that carry the list under an implicit document root keep validating, as today.
  - Older *chunked* archives are Unsupported under I1 for their JSON templates in any case, not
    because of the list.
- **Link tiles in `budgets.tiles` (PD-13, accepted).** I1 does not count tiles whose only content is
  an external tileset document in the `budgets.tiles` comparison (`validate.rs:1662`). This is an
  I1 interface item. P emits no link count.
- **Asks:** the W11 witnesses (ownership plan, I1 asks).""")
rep("""    - a retained source missing from its list;
""", """    - a retained raw source left in the archive (v3: unreferenced member);
""")
rep("""    - `tiles` = |H| + link tiles (or |H| if I1 adopts PD-13);""",
    """    - `tiles` = |H| + link tiles (I1's counter), while the `budgets.tiles` comparison uses |H|
      (PD-13);""")
rep("""  - default vector archives remain about 2× explicit ones (PD-11).""",
    """  - default vector archives no longer carry duplicate raw sources. For example the perf-lane
    vector drops from about 37 MB to about 19 MB, near `--explicit` (PD-11);
  - pre-v3 archives are not `--reuse-tileset` baselines ("run a fresh conversion").""")
rep("""- PD-12 (archive-level caps) for the coordinator.""",
    """- PD-12: the archive-level caps are raised by the coordinator's validator slice. The §5.1 counts
  are its inputs.""")
rep("""- `entries` and `visits` exceed 65,536 in every 100k case, in any layout (PD-12).""",
    """- `entries` and `visits` exceed 65,536 in every 100k case, in any layout. The coordinator's
  validator slice raises the caps (PD-12). The v3 per-tile counts below are its inputs.""")
rep("""Today's root document grows with N (the retained-source list), and reaches the node cap at about
65k tiles.""",
    """Today's root document grows with N (the retained-source list), and reaches the node cap at about
65k tiles.

V3_SCALE""")
rep("""## 7. Proof limits and remaining gates""", """V3_REUSE

## 7. Proof limits and remaining gates""")
open(p, 'w').write(s)
print('ok')
