# Hostile `totalPayloadElements` archives

These generators build the adversarial work classes of the `totalPayloadElements` time statement
(`docs/architecture/validator-limits-amendment.md`). They were written by the non-author reviewers
of PR #304. CI does not run them. Use them to re-measure the cost per charged unit after a change
to validator work or weights.

- `bombs_lib.py KIND N_GLBS OUT.3tz [pad] [extra]`: classes c1 `comp64`, c2 `hlodseq`, c3 `hloddup`,
  c4/c5 `hlodleaf`, and `hugecount` (refused before any work).
- `leafrand.py N_LEAVES TRIANGLES SEED OUT.3tz`: class c6. Leaves of nondegenerate triangles at
  distinct pseudo-random lattice points, with `source_triangle` tables, so every leaf point, edge
  and face is a random-key scratch row. The time-statement archives are `leafrand.py N 8192 7`
  for N = 10, 40, 150 and 400. N = 10 gives S = 1,520,767 and SHA-256 `f92af116…514849b`.
- `enc.mjs`: meshopt streams from the upstream meshoptimizer encoder (vertex codec v0). It needs
  Node and the `meshoptimizer` 1.3.0 npm package. `MESHOPT_JS` names the package directory
  (default `./meshoptimizer`). Encoded streams are cached in `RT_HOSTILE_CACHE`
  (default `/tmp/rt-hostile-enc`).

Measure CPU time per unit with the run pinned to one core, as
`taskset -c CORE rusty-tiles --json validate OUT.3tz --scratchDir DISK_DIR`, and divide by the
observed units. Pass `--maxVolume totalPayloadElements=N` to stop a class at a fixed budget.
