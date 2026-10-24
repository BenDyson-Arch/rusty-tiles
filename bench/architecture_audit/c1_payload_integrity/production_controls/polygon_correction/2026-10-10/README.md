# Additive polygon oracle correction

Read [adjudication](adjudication.md) before interpreting the corrected lane.
The original205 execution remains204/1: its sole failed expectation was an
invalid positive oracle. Original frozen records are unchanged.

[generate.py](generate.py) independently packs the corrected triangle/loop
bytes using Python's standard library and retains203 original archive/control
records exactly. The new corrected205 bundle replaces two original polygon
fixtures: the positive and a formerly masked reference-OOB negative. A separate
five-case bundle supplies sensitivity and the exact original malformed source
with its primary-justified InvalidInput expectation.

Extract the selected bundle into a new external directory using
`python3 -m tarfile -e BUNDLE NEWDIR`. Both dated bundles contain a manifest
compatible with the **unchanged** original [driver](../../driver.py).
Its driverSHA and original referencePins remain identical. The dated generator
and newly retained polygon draft are additionally pinned in manifest fields
and the preparation record. Execute with the same original driver arguments:
`--fixtures NEWDIR --binary BIN --binary-sha256 HASH --source-pin PIN
--source-pin-sha256 HASH --run-dir NEW_RUN_DIR`.

No production source, Cargo, Git or target execution is performed by this lane.
Root owns compilation, actual binary execution and exact source/artifact
binding. Category/report expectations come from supplied literal bytes and
pinned primary rules. Failure categories are typed codes, not diagnostic-text
guesses. The full original failed receipt and raw stdout/stderr are retained
separately, without rewriting historical observations.
