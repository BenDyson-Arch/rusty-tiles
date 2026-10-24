# Independent final-source payload controls

These controls are authored separately from production. They do not use
rusty-tiles producers, its fixture generator, or target self-reported values as
the expected result. Python standard-library packing and pinned primary source
bytes establish the fixture truth. No target execution is claimed by this
preparation record.

[driver.py](driver.py) generates immutable-supplied JSON, GLB, modern b3dm and
stored 3TZ fixtures. Each positive declares the complete public report before
execution: all thirteen checks, fifteen limits, sorted exclusions, entries,
tile/content-reference counts and sorted payload/accessor/primitive/vertex
counters. Independent scalar-component counts are retained separately because
the public report does not expose them. Raw/scalar/normalized extrema use a
separate little-endian decoder. Upstream meshoptimizer expected bytes are
extracted from the pinned source literal and checked against retained goldens;
no project decoder supplies the expected bytes.

The fourteen original primary controls retain their exact literal-source
SHA256 identities. Their expected categories incorporate the six independently
demonstrated corrections. New archive identities deliberately use distinct tile
names and are recorded separately; none rewrites historical receipts or binds
their old artifact to new production. Existing `tests/c1_validation_oracle.py`
and its checked-in corpus are supplemental evidence, not this lane's authority.

The retained [lossless fixture bundle](fixtures.tar.gz) contains its complete
`manifest.json` and every supplied fixture/member/literal byte. Its small
[bundle identity record](fixture-bundle.json) binds compressed/uncompressed tar
and manifest identities. Extract into a new external directory with Python's
standard-library `tarfile` CLI:

```text
python3 -m tarfile -e \
  bench/architecture_audit/c1_payload_integrity/production_controls/fixtures.tar.gz \
  /tmp/new-independent-payload-fixtures
```

The retained fixture manifest pins fixture members,
literal bytes and complete archives, the driver, selected primary text/goldens,
immutable primary records and the approved precode contract/decision. Generate
only into a nonexistent directory. Root owns actual final-artifact execution
and shared build scheduling.

Example final-artifact invocation (hash values must be supplied by the root's
exact source/build record):

```text
python3 bench/architecture_audit/c1_payload_integrity/production_controls/driver.py \
  --fixtures /tmp/new-independent-payload-fixtures \
  --binary /absolute/path/to/frozen/rusty-tiles \
  --binary-sha256 FINAL_BINARY_SHA256 \
  --source-pin /absolute/path/to/final-source-build-record.json \
  --source-pin-sha256 FINAL_SOURCE_RECORD_SHA256 \
  --run-dir /absolute/path/to/nonexistent-run-directory
```

The driver uses nice 10, two CPU affinity and two-worker environment settings
for each supplied CLI invocation. It retains exact commands/stdout/stderr,
source snapshot and supplied source-pin bytes, artifact/driver/manifest hashes,
and before/after fixture/source/artifact identities. It refuses identity drift.
Category comes from typed serialized CLI code and exit status; it never guesses
failure category from diagnostic text. Complete report equality is asserted on
success. It records every failure and exits nonzero if any expectation or
immutability check fails.

Selected authority and limits of interpretation are in the frozen contract and
[component-bounds static decision](2026-10-10-component-bounds-decision.md).
Core authority is published glTF 2.0.1 source revision
`8e798b02d254cea97659a333cfcb20875b62bdd4`, not unpublished main. The manifest
verifies the eight retained extension/b3dm/upstream sources and the exact core
source/accessor schema in the original raw bundle; it does not claim a new
network retrieval.

The independent [private controls](private_controls.rs) are ready for a root-owned
`cfg(test)` include. They use only the real crate-private owner APIs and
explicit small limits. Inputs and expectations independently exercise inclusive
JSON boundaries/compound ordering, owned numeric variants/marker preservation,
outer/JSON payload bytes, complete component/buffer planning before callbacks,
remaining allowance, typed causal/boxed identity, actual resource/image
remainders, padded inline remainder and separate pools, overlapping views and
the real upstream meshopt golden/corrupt decoder sensitivity pair. This source
file is prepared without a Cargo/build execution claim; root owns its include,
compilation and receipts.

The public CLI cannot select small private limits, observe a forbidden decoder
allocation, expose exact logical resource/read charges, inject causal resolver
errors, or directly reveal aggregate component charging. Those require separate
private-boundary controls under coordinator-owned build execution and final
source inspection. Equality/one-over JSON bytes/depth/nodes, payload bytes,
declared/actual buffer/image remainders, overlapping view bytes, meshopt output,
base64 actual padding, pre-resolver components, compound-fault precedence and
causal Io are explicit outstanding probe obligations until their execution
receipts exist. Public fixture success alone does not close them.

Finite profile controls preserve local glTF/GLB/modern b3dm, core modes, actual
ordinary indices, data buffers, viewless values/bounds, quantization, matrix
final padding, required meshopt NONE/placeholders/fallback roles, pinned draft
restart, optional polygon references and counters/cache semantics. Opaque JSON
still receives Unicode/duplicate/grammar admission. Huge opaque numbers do not
force an owned payload DOM; ordinary root/schema/conversion consumers truthfully
refuse unrepresentable `1e400` as Unsupported. Fractional unsigned declarations
are compared mathematically before binary-float rounding, with native/overflow
meaning separately classified by the consuming domain.

None of these controls claims complete glTF/C1 conformance, full polygon
topology, physical reads/CPU work, process RSS, scene/rendered containment,
metadata semantics, full A2 acceptance, #113/#121 closure, or release authority.
