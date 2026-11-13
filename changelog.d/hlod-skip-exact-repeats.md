### Changed

- Mesh HLOD: the coarse ladder no longer recomputes work whose result it already has.
  A later build reuses an earlier build's failed border contraction when its contraction
  arguments are the same (`Staged`, `N1` and `N2` repeat the primary build's; `N3Recovery`
  repeats `N3`'s when both caps are equal), `Fix2` replays the primary build's failed
  proposals when its contraction cannot differ, and remesh round 0 reuses the full-cap
  proposals when cleanup left the mesh unchanged. Archives are byte-identical; the run
  diagnostics report the skipped work as `repeat_reuse`. Reuse is off while the
  `RT_HLOD_AUDIT_DIR` sidecar records every attempt.
