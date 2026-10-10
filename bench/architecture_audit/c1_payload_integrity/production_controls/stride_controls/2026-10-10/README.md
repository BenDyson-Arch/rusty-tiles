Additive independent C1 stride control lane

This lane is external to the repository and original 205 + 5 controls. Predictions
were authored before execution against the corrected source. No target binary,
production encoder, production decoder, or project numeric helper generated the
fixtures or expected values. Retained literal zeux meshoptimizer test arrays supply
an 85-byte compressed stream and 48 expected bytes; Python struct independently
interprets the expected bytes as finite little-endian float32 values and windows.
Pinned Khronos core Data Alignment lines1124–1141 defines absent-parent tight
accessor packing and shared vertex attribute requirements. Pinned meshopt extension
lines73–87 defines parent stride equality WHEN defined and decoded physical size.
The extra scalar accessors are unused, not second vertex attributes.

Six public cases: absent-parent baseline; packed unused scalar12; offset4 packed
scalar11; explicit parent12 scalar4; real count13 range failure; parent8 mismatch.
The four positives assert the entire report, all fifteen default limits, thirteen
checks, complete exclusions, and accessor/primitive/vertex counts. Negative cases
assert typed category, exit status, and empty stderr. All outputs are hash-bound.
The public reports do not expose component/decode counts; the compiled Rust module
adds exact24-component acceptance, cap23 and remaining23 resource refusals, and
48-byte acceptance versus47-byte resource refusal for the shared physical view.
Resolver callbacks must remain zero in every private case.

Run generator.py only into a fresh fixtures directory. It refuses to overwrite.
compiled_stride_controls.rs is self-contained and may be integrated as a cfg(test)
crate module by the coordinator; it contains frozen literal JSON and compressed
bytes, avoiding an absolute /tmp include_bytes dependency. Root owns compilation.
The runner requires --binary, --binary-sha256, --source-pin,
--source-pin-sha256 and a fresh --run-dir. It never builds, mutates source or existing
fixtures, or infers an accepted binary from a path. Root owns all target executions.

The supplied frozen 2ebfb74 CLI is recorded in supplied-frozen-target.json as a
PRE-correction sensitivity target, not an accepted corrected source. New corrected
executions require their own exact source/artifact pins. No target has been run by
this lane author. Static fixture checks are recorded separately from acceptance.
