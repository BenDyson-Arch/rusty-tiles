# Independent archive-read acceptance

Read [audit.md](audit.md) for executed defects, finite decisions and proof limits.
[results.json](results.json) binds 45 final tiny fixtures, lossless receipts and
45 historical frozen validations. Final production acceptance is pending.

Run the independent reference without a product binary, using a fresh path:

```sh
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/archive_read_acceptance/probe.py --work /tmp/rusty-tiles-archive-read-independent-replay
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/archive_read_acceptance/check_crc.py /tmp/rusty-tiles-archive-read-independent-replay/receipt.json --output /tmp/rusty-tiles-archive-read-independent-replay/crc-ranges.json
```

Python 3 without `-O` is required. No production decoder or ZIP library is the
reference. Replay requires the pinned earlier source manifest and unchanged
historical snapshot at `/tmp/rusty-tiles-a2-continuation`, for source-observation
bindings; it produces only tiny external `/tmp` archives and JSON receipts.

Candidate command, once the coordinator supplies reviewed source and binary
pins and schedules the run:

```sh
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/archive_read_acceptance/probe.py --work /tmp/rusty-tiles-archive-read-candidate-replay --binary /path/to/pinned-cli --binary-sha256 BINARY_SHA --source-pin /path/to/coordinator-source-pin.json --source-pin-sha256 PIN_SHA
```

The candidate executes all 45 cases, fails on category disagreement, and retains
the complete receipt. The driver sets `RAYON_NUM_THREADS=2` and runs each CLI
at `nice -n 10` with a ten-second timeout, serially and with child reaping.
Full source/build and Rust allocation/I/O acceptance still need independent
review. No further frozen batch is needed; exact historical drivers are retained
separately to preserve execution bindings.
