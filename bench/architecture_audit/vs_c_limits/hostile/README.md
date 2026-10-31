# Hostile `totalPayloadElements` archives

These generators build the adversarial work classes of the `totalPayloadElements` time statement
(`docs/architecture/validator-limits-amendment.md`). The non-author reviewers of PR #304 wrote
`bombs_lib.py`, `leafrand.py` and `enc.mjs`; `rowclasses.py` and `rows_control.py` come with the
constant-cost scratch rows. CI runs only `rows_control.py`. Use the others to re-measure the cost
per charged unit after a change to validator work or weights.

- `bombs_lib.py KIND N_GLBS OUT.3tz [pad] [extra]`: classes c1 `comp64`, c2 `hlodseq`, c3 `hloddup`,
  c4/c5 `hlodleaf`, and `hugecount` (refused before any work).
- `leafrand.py N_LEAVES TRIANGLES SEED OUT.3tz`: class c6. Leaves of nondegenerate triangles at
  distinct pseudo-random lattice points, with `source_triangle` tables, so every leaf point, edge
  and face is a random-key scratch row. The time-statement archives are `leafrand.py N 8192 7`
  for N = 10, 40, 150 and 400. N = 10 gives S = 1,520,767 and SHA-256 `f92af116…514849b`.
- `rowclasses.py KIND N_LEAVES TRIANGLES SEED OUT.3tz [K] [COARSE]`: two more scratch-row
  classes on leafrand's leaves (2026-10-30, constant-cost scratch rows). `twin` puts N/2 leaves
  under each of two coarse cells, the second cell's leaves at the first's positions, so every key
  is shared across the cells: ancestor requirements, multi-edge faces and coincident corner sets.
  `chain` puts N leaves with every triangle doubled under a single-child chain of K coarse tiles
  sharing one payload that doubles COARSE of those triangles: descendant-incidence, edge-face and
  coincident-face lookups at every chain tile. The time-statement archives are `twin N 8192 3`
  and `chain N 8192 5 OUT 24 128` (K stays under `jsonDepth`).
- `rows_control.py check --binary BIN --work DIR`: CI runs it. Four classes at a small size with
  plain buffers (no Node): leafrand, twin (valid, through the border images), chain and c9
  revfan. It checks each verdict through the CLI and a structured budget refusal. On revfan it
  also asserts the units each added chain tile charges, from the 320-unit seeks of the lookups
  that tile repeats. This is a timing-free signal: the former 128-unit seeks fail it (develop
  f02e8774 charges 12,094 units per tile, the floor is 30,720).
  - `rows_control.py generate` writes larger plain-buffer archives.
  - `rows_control.py revfan OUT K 64 1360 32 25` writes review 1's c9 archives byte for byte (K = 1,
    100, 122).
- `enc.mjs`: meshopt streams from the upstream meshoptimizer encoder (vertex codec v0). It needs
  Node and the `meshoptimizer` 1.3.0 npm package. `MESHOPT_JS` names the package directory
  (default `./meshoptimizer`). Encoded streams are cached in `RT_HOSTILE_CACHE`
  (default `/tmp/rt-hostile-enc`).

Measure CPU time per unit with the run pinned to one core, as
`taskset -c CORE rusty-tiles --json validate OUT.3tz --scratchDir DISK_DIR`, and divide by the
observed units. Pass `--maxVolume totalPayloadElements=N` to stop a class at a fixed budget.
