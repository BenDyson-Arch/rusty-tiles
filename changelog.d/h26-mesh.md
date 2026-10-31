### Added

- `rusty-tiles mesh` and Python `mesh_to_tileset` run one streamed local-metre mesh pipeline through leaf encoding, HLOD coarse levels (sampled error, textured) and implicit `.3tz` delivery, with rigid WGS84 placement, provisional profiles, memory-capped maximum workers, typed errors and separate runtime diagnostics. `doctor --command mesh` describes the built-in capability.

### Changed

- Breaking: the preparatory leaf-directory/delivery Rust exports and hidden `mesh-leaf-payloads`, `mesh-leaves-to-3tz` and `mesh-large-to-3tz` commands are replaced by the single source-mesh facade. Legacy mesh converters (`mesh-to-3tz`, `mesh-local-to-3tz` and their Python functions) remain temporarily available with deprecated help; their removal is a 0.4.0 gate, not 0.4.x work.

### Deprecated

- `mesh-to-3tz`, `mesh-local-to-3tz`, Python `mesh_to_3tz` and `mesh_local_to_3tz`: use `mesh` / `mesh_to_tileset`. Options do not carry over one to one; see the mesh migration notes.
