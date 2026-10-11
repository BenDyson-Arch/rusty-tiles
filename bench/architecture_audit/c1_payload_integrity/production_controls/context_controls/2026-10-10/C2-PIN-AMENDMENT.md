# C2 amendment to the 2026-10-10 compiled context controls

These dated C1 controls are compiled as a live test module (`src/lib.rs`), so the C2
public changes reach them. Two files were amended on `feat/c2-decoded-containment`. No C1
outcome or check changed.

| File | Pinned sha256 (2026-10-10) | Amended sha256 | Reason |
| --- | --- | --- | --- |
| `compiled_context_controls.rs` | `327dee099efab3f4ff66906a08b738cac408bad309a9e28bc8b5b2ff990a65e9` | `ee9d6444e0ee89a6eeadb046b2bb9841bce58994e69e067f81371c19b17201e1` (first amended as `4dbd6033…`) | The five C2 `ValidationLimits` fields are added explicitly at their defaults, in both the struct literal and the declared limits JSON |
| `fixtures/manifest.json` | `22d3f83568de464d7227da273d1419f9dca5390dbf38f665b40b16e819b6f51d` | `245baae82b239fc184f5dfe78868b485e06e55a38a026333e63016514b0ed909` | Only the `expectedReport.notInspected` arrays changed: the C1 payload label split (R7.1), `decodedContentBounds.sceneSelection` for scene-less GLBs (contract reason table), and `decodedContentBounds.implicitTiling` for the two implicit cases |

Historical review records under `final_source_review/` keep the original hashes. They
remain true statements about the reviewed source at that time. Both amendments were made in
separate labelled commits by the C2 format author, directed by the coordinator.

The second amendment of `compiled_context_controls.rs` changes `containment_work_units` from 4294967296 to 268435456. It follows the coordinator's measured cap decision: about 10 ns per vertex unit and 2.8 ns per index unit in release, so 2^28 bounds C2 work at defaults to about 2.7 s.
