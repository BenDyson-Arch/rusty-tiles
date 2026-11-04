# Implicit forest consolidation: evidence

Design: [implicit-forest-consolidation.md](../../../docs/architecture/implicit-forest-consolidation.md).
Full-audit tier (output layout). Bulk outputs stay out of the tree; the 2048 output used
for analysis is `/home/bend/scratch/rusty-tiles-scratch/bake-bounded/m2048-final/view/`
(unpacked; not re-baked here), and the CDP cold-load profile is
`~/.cache/rusty-tiles-evidence/load-profile-20261023/m2048-cold.json`.

## Tools

- `simulate_first_view.py OUTPUT [--layout SPEC ...]` reads an unpacked output or `.3tz`,
  rebuilds every tile, and replays CesiumJS's base traversal from the preview's opening
  camera. It reports first-view requests by kind, the dependency depth (round trip on
  which the last request becomes possible; `tileset.json` is 1), and a rough 6-connection,
  200 ms schedule. `--layout` re-lays the same logical tree: `S:INLINE:OBJECTS:RECORDS:LEVELS`
  with develop's partition, a `B` prefix for this branch's budget rule, or the aliases
  `develop` (`4:0:32:1024:8`) and `branch` (`B4:3:480:4096:12`).
- `content_identity.py BEFORE.3tz AFTER.3tz` compares the multisets of content GLB SHA-256
  digests and reports members and bytes by kind.

## Calibration

On the 2048 output the simulator requests exactly the 102 URLs of the CDP profile
(7 JSON, 25 subtrees, 70 GLBs; no URL extra or missing). `--layout develop` on its logical
tree reproduces the parsed output: 200 roots (48/81/71 at 2/3/4 levels), 7 documents and
the same requests.

## Results

### Fresh bakes (tex3, crop3)

Inputs: `~/.cache/rusty-tiles-evidence/h17-20261013/crop/{tex3,crop3}.glb`. Command:
`rusty-tiles --json mesh -i <glb> -o out.3tz --profile balanced --jobs 4 --anchor
133.0584598837835 -12.33625061681309 100`. Release binaries: develop `41af7f6e`, branch
`2e149d92` (the planner commit). These were correctness runs (measure slot 2, CPUs 0-3,16-19,
loaded host), not timing evidence. Outputs were deleted after the checks; their SHA-256:

| archive | develop | branch |
| --- | --- | --- |
| tex3 | `bb6c55590d04d678640313859e20ad2588e83d58151d9dd7f081caaa31454a27` | `7b46b97a23cda82e11393fefa41354d6c9138e0271d3e696bf649b3bb9500635` |
| crop3 | `88d3330bf909564005d662b38aa3a7c2a18d857e985710831da7720cc1566ee7` | `d9a978bff2a240aa9c3e9854cec5bdb0614dad0414b57ac143868ec8a20f1b20` |

| | tex3 develop | tex3 branch | crop3 develop | crop3 branch |
| --- | ---: | ---: | ---: | ---: |
| tiles | 219 | 219 | 219 | 219 |
| implicit roots (levels 2/3/4) | 15 (2/6/7) | 0 | 15 (2/6/7) | 0 |
| documents | 1 | 1 | 1 | 1 |
| members (GLB / subtree / JSON / other) | 219 / 15 / 1 / 2 | 219 / 0 / 1 / 2 | 219 / 15 / 1 / 2 | 219 / 0 / 1 / 2 |
| tileset JSON bytes | 8,796 | 66,053 | 8,796 | 66,053 |
| subtree bytes | 44,216 | 0 | 44,216 | 0 |
| archive bytes | 84,346,669 | 84,350,025 | 78,866,365 | 78,869,721 |
| **first-view requests (JSON / subtree / GLB)** | **22 (1 / 7 / 14)** | **15 (1 / 0 / 14)** | **22 (1 / 7 / 14)** | **15 (1 / 0 / 14)** |
| **dependency depth** | **3** | **2** | **3** | **2** |
| `validate` | pass | pass | pass | pass |

- **Content identity.** `content_identity.py` reports equal multisets of content GLB
  SHA-256 digests (219 each, no digest only before or only after). The digest-set hash is
  `983668cf112060646742e08f0c1bb57fbf2e6a50fdea42b905873a5dcf739391` for tex3 and
  `e27698ef27d66a125e7a3536b11c9f30358696a286bbeb0805492887b48da454` for crop3. Only the names of
  inlined payloads change (`implicit-content/…` to `content/f<first>-d<depth>.glb`).
- **Validator.** Every archive passes with `decodedContentContainment`. The branch reports
  drop only `implicitAddressingAndAvailability` and the `recordedBudgets.implicitTileExtras`
  note, because these outputs no longer contain implicit roots. Implicit roots under the
  new rule are validated by the unit tests (`mixed_tree_…`, `deepest_shallow_tree_…`,
  `mixed_explicit_and_implicit_siblings`, `root_forms_…`).
- **Prediction.** `simulate_first_view.py dev/tex3.3tz --layout branch`, which re-lays
  develop's output, predicts the branch output exactly: 15 requests at depth 2.

### 2048 output (simulated; no 2048 bake)

| layout | requests (JSON / subtree / GLB) | depth | GLBs by round trip | documents | roots | JSON |
| --- | --- | ---: | --- | ---: | ---: | ---: |
| develop (parsed output = CDP profile) | 102 (7 / 25 / 70) | 5 | 13 / 29 / 21 / 7 at 2–5 | 7 | 200 | 120 KB |
| branch rule | 87 (1 / 16 / 70) | 3 | 54 / 16 at 2–3 | 1 | 146 | 164 KB, 11.4k nodes |

The full option grid is in the design document. Reproduce it with:
`simulate_first_view.py <2048 view dir> --layout develop 4:1:32:1024:8 4:2:32:1024:8
4:3:32:1024:8 5:0:32:1024:8 6:0:32:1024:8 7:0:32:1024:8 B4:0:480:4096:12 B4:1:480:4096:12
branch`.

## Tests and checks

`cargo clippy --all-targets -D warnings`; `cargo test --lib` (1,081 passed); and the
integration tests `golden`, `output_digests`, `mesh_tile`, `h26_mesh_core`,
`h26_mesh_frontends`, `implicit_forest`, `implicit_metadata`, `implicit_subtree`,
`i1_implicit_read`, `c1_validate`, `c2_containment`, `native_implicit`, `native_validate`,
`mesh_archive`, `mesh_crs` and `mesh_local_frontends`. No golden or digest changed: none
pins `mesh` delivery output. The planner memory test measures `peakHeldRecords` 4,680
(2^12 leaves) and 4,687 (2^18), the same as before.
