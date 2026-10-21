### Added

- `rusty-tiles mesh` and Python `mesh_to_tileset` run one streamed local-metre mesh pipeline through leaf encoding, certified coarse levels with baked proxy textures and implicit `.3tz` delivery, with rigid WGS84 placement, provisional profiles, memory-capped maximum workers, typed errors and separate runtime diagnostics. `doctor --command mesh` describes the built-in capability.

### Changed

- Breaking: the preparatory leaf-directory/delivery Rust exports and hidden `mesh-leaf-payloads`, `mesh-leaves-to-3tz` and `mesh-large-to-3tz` commands are replaced by the single source-mesh facade. Legacy mesh converters remain temporarily available with deprecated help pending complete tiler acceptance.
