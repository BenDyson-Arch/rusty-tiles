### Fixed

- The CLI now handles SIGINT and SIGTERM (Unix) as a cancellation of the running command. `mesh` and the other spilling commands remove their `rusty-tiles-spill-*`, `.tiles-mesh-*` and `.tiles-stage-*` directories before exiting with status 130 (SIGINT) or 143 (SIGTERM) and a cancelled error, instead of leaving them on disk. A second signal exits immediately. Adds the `signal-hook` dependency on Unix.
- The `spill_memory_is_flat_in_input_size` test no longer fails on process-to-process VmHWM noise: it compares the smallest peak of three fresh subprocesses per input size against a documented allowance.
- A test now pins that `escalation_reasons`, the escalation diagnostics and the published bytes of codec-verification escalations are identical for one worker and many.
