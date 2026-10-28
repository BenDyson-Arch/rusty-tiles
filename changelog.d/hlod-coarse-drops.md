### Added

- HLOD D11 coarse drops (owner decision 2026-10-28): a coarse cell that cannot otherwise fit the 8,192-triangle cap (its contracted input above the cap) drops whole components with no border contact, smallest first (bounding-box diagonal, then triangle count), even above its G floor. Published G covers the dropped surface (S2's fixed controls on every dropped triangle) and stays at least the children's G. The 10M-triangle mesh-1e7 city gets a root and coarse depths 0–3, which it omitted before.
- Coarse payloads declare dropped child components in glTF `asset.extras.rustyTiles.droppedComponents`. Validation (`--expect-profile hlod` or a declared HLOD archive) recounts them from decoded keys and refuses `coarseDrops` when the count differs from the declaration, when a dropped point or a fixed sample point of a dropped face lies farther than the payload's geometricError from its surface, or when a drop at or above the children's largest geometricError comes without cap pressure (children within 8,192 faces) or while a smaller border-free child component is kept. The interface census gains `droppedComponents` and `droppedByDepth`; Rust `InterfaceCensus` gains `dropped_components` and `dropped_by_depth`. Topology receipts gain `coarseDroppedComponents` (present when nonzero).

### Changed

- HLOD: a coarse cell drops no component, ordinary (below its G floor) or coarse, unless its input exceeds the cap, and drops at or above the children's geometricError go smallest first. Before, ordinary drops could happen at any cell that failed another strategy, in any order.

- An HLOD archive whose coarse payload omits child components without declaring them, or whose geometricError does not cover them, now fails validation (`coarseDrops`). Archives from this release declare every drop, including small drops below the G floor, so a coarse payload holding such a drop gains the declaration.
