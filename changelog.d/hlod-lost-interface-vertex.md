### Fixed

- HLOD: a bake no longer stops with `invalid_state` ("children of … lost … interface vertices") when a remesh round would remove a border point contact (a shared vertex with no shared edge in that cell). The round now keeps the point, or is refused so the next strategy runs. First reached by the full Walem model.
