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

## I1 amendment

The I1 implicit reader (`feat/implicit-forest`) changes two things these controls observe. Both
were made in separate labelled commits by the integration helper, directed by the coordinator. No
member pin, archive byte, selected limit or other control changed. The original 2026-10-10 pins
and the C2 pins above remain true statements about the source they reviewed.

### Report labels (I1 contract X-14)

Contract row X-14: "`checks` and `notInspected` labels as above; counts unchanged".

| File | Previous sha256 (C2 amendment) | Amended sha256 | Reason |
| --- | --- | --- | --- |
| `fixtures/manifest.json` | `245baae82b239fc184f5dfe78868b485e06e55a38a026333e63016514b0ed909` | `a68e4afc7c8205c741ccbc32c86019fd744588c7c2230afa2651eaa28538975f` | In the `expectedReport` of the two implicit cases, `nested-metadata-extras` and `subtree-json`, only: `implicitAddressingAndAvailability` moves from `notInspected` into `checks`, right after `hierarchyBounds`; `recordedBudgets.implicitTileExtras` is added to `notInspected` (I1 RC-13, OD-4) |

### Opaque implicit tile extras (I1 OD-4, X-7)

Below an implicit root, I1 reads only the interpreted `TILE_BOUNDING_BOX` and
`TILE_GEOMETRIC_ERROR` columns. The `rustyTile` UTF8 `extras` column is opaque by design (OD-4,
reported as `recordedBudgets.implicitTileExtras`; X-7 removes the old expander's extras parsing),
so it is not a parsed JSON document and no JSON limit applies to it. The
`nested-metadata-extras` control's target document is exactly that string, and every other
document in its archive is strictly smaller on all three facts. Its three one-less runs
(`json_bytes` 3,130, `json_depth` 18, `document_items` 1,556) are therefore admitted, as
executed on the integration branch. The coordinator ruled the `json_bytes` case; the `json_depth`
and `document_items` one-less runs fail for the same reason and were re-scoped with it. The
three equality runs and their exact expected reports are unchanged.

| File | Previous sha256 | Amended sha256 | Reason |
| --- | --- | --- | --- |
| `compiled_context_controls.rs` | `ee9d6444e0ee89a6eeadb046b2bb9841bce58994e69e067f81371c19b17201e1` (C2 amendment) | `a142a385b07676102f82ce93e37d55d01e0dca99033466ab3ce42c4c8b0a8fb9` | The `nested-metadata-extras` one-less runs expect admission, with a comment citing I1 OD-4 |

The 82-execution receipt (`tests/payload_context_receipt.py`) now expects 44 admitted and 38
ResourceLimit rows, and identifies each execution by its control, fact and selected limits.
`context_controls.rs`, the uncompiled 2026-10-10 original, is unchanged
(`3a2959b544285ed568236d62fa1533592e1eeb49f800e215c7560a393e2287fd`).

## C2 slice-3 amendment (2026-10-12)

C2 slice 3 (contract v3.1, sha256 `8bc53429…`, §2 and §6) removes the frozen reason
`implicitTiling`: every implicit root I1 reads is now in C2's domain, and the enum keeps
15 names with `derivedImplicitVolume` in its place. The label change was made by the
coordinator (R) in a separate labelled commit. No member pin, archive byte, selected limit,
C1 outcome, check or count changed; the 82-execution receipt is unchanged.

| File | Previous sha256 (I1 amendment) | Amended sha256 | Reason |
| --- | --- | --- | --- |
| `fixtures/manifest.json` | `a68e4afc7c8205c741ccbc32c86019fd744588c7c2230afa2651eaa28538975f` | `47694c3d609ca4acc0bee97a83290fd3a7aa538886e9777ae5714576b829b372` | In the `expectedReport` of the two implicit cases, `nested-metadata-extras` and `subtree-json`, only: `decodedContentBounds.implicitTiling` leaves `notInspected`. Both stay outside C2 for their payload's own reason (`decodedContentBounds.sceneSelection`, a scene-less GLB), so `decodedContentContainment` does not enter `checks` |

`compiled_context_controls.rs` (`a142a385…`) and the uncompiled `context_controls.rs` are
unchanged. The earlier pins remain true statements about the source they reviewed.

## H7 amendment (legacy content Unsupported, 2026-10-13)

The [C1 H7 amendment](../../../../../../docs/architecture/c1-legacy-content-unsupported-amendment.md)
makes every `.b3dm` content member Unsupported before any table or embedded-GLB byte is read.
Three of these controls target b3dm documents (`b3dm-feature-table`, `b3dm-batch-table`,
`b3dm-embedded-glb`), so neither their equality runs nor their one-less runs reach the target
document any more: all 18 of their executions now expect a typed Unsupported failure whose
message starts `legacy b3dm`. The change was made by the H7 author in a labelled commit. No
member pin, archive byte, selected limit, dominance assertion or other control changed;
`fixtures/manifest.json` is unchanged (`47694c3d…`), and the three cases' `expectedReport`
entries are kept as history but no longer compared.

| File | Previous sha256 | Amended sha256 | Reason |
| --- | --- | --- | --- |
| `compiled_context_controls.rs` | `a142a385b07676102f82ce93e37d55d01e0dca99033466ab3ce42c4c8b0a8fb9` (I1 amendment) | `d89539a74d9cd804389a16845609dd1770596cbf0a1d2c810ecb8e114d160dfe` | An `Outcome` (admitted, resource limit, unsupported) replaces the admitted flag; the three `b3dm-*` controls expect Unsupported |
| `tests/payload_context_receipt.py` | `9a1eb507001fe8553d4ced0abb00303136c30fbc6967a8675335fa2fdc7ea91f` | `ffa0243acadfcc4241185b9ffabfde1b0423cbbe43bb4fc40d359534ab90b582` | The 82-execution receipt expects 35 admitted, 29 ResourceLimit and 18 Unsupported rows (was 44 and 38) and checks that exactly the b3dm controls' 18 runs are Unsupported |

## VS-B amendment (live documents, 2026-10-14)

The [VS-B traversal amendment](../../../../../../docs/architecture/validator-traversal-scale-amendment.md)
adds the per-item limit `liveDocumentBytes` (VS-B-7) to `ValidationLimits`. These controls
select every limit explicitly, so the struct literal gains `live_document_bytes` at its default
(268435456). The field is not serialized in the report's `limits` until VS-C, so the declared
limits JSON is unchanged. No archive, member pin, expectation or other selected limit changed;
`fixtures/manifest.json` is unchanged.

| File | Previous sha256 | Amended sha256 | Reason |
| --- | --- | --- | --- |
| `compiled_context_controls.rs` | `d89539a74d9cd804389a16845609dd1770596cbf0a1d2c810ecb8e114d160dfe` (H7 amendment) | `91cbe51512738bdcaea4120a698bde523687b134cbfd945b8ad01bb5959085b0` | `live_document_bytes` added at its default in the selected limits |
