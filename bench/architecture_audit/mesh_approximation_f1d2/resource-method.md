# F1d2 fresh optimized resource observations

Prepared resource method for the new schema7/profile
`f1d2-adaptive-root-proxy-gltf-v1`. This document and
[resource_probe.py](resource_probe.py) are a fresh method, not a replay of the
F1d1 resource receipt. No converter execution is claimed here until a frozen
optimized F1d2 CLI and its receipt are recorded. The coordinator owns builds,
final source identity and acceptance.

## Fixed workloads and acceptance meaning

The independent fixture writer builds one authored indexed 64x64 planar grid:
8192 source faces, 4225 original positions, one primitive/region, and exact
untextured PBR factors. All runs request 2048-triangle full-detail leaves.
They execute serially with separate fresh case directories and no force/retry.

1. `accepted-near-base-cap`: proxy ceiling1024, declared budget100m. Historical
   proposal evidence suggests1023 actual candidate faces and compulsory root
   baseline16760832 tests, about99.90% of the16777216 ceiling. That historical
   count is a planning observation, not a required future exact candidate digest.
   Require actual reduction and independently decoded root baseline>=90% of
   the cap. The wide budget admits the root patches, so require depth0, actual
   tests equal decoded baseline and accepted leaves equal source+proxy faces.
2. `refused-above-base-cap`: proxy ceiling4096, budget100m. Require actual typed
   Unsupported with the base face-pair work-limit reason, and an absent output
   parent. A proposal refusal, timeout, successful conversion or different
   failure cannot stand in for base admission evidence.
3. `dynamic-cap-control`: proxy ceiling1024, budget6m. The old grid proposal's
   whole-face bound was about7m; a tighter threshold is expected to force enough
   subdivisions to exhaust the tiny remaining work allowance. Require actual
   typed Unsupported with the patch-face work-limit reason and an absent output
   parent. A depth/proposal/base refusal cannot stand in for dynamic admission.
   This is a feasibility assumption awaiting the frozen F1d2 run. If it fails,
   retain the failed receipt and revise the bounded authored workload explicitly;
   do not relabel a different outcome or add a production fallback.

The success checker independently decodes source/root/leaf supports, exact leaf
identity/material/resource meaning, complete proxy provenance membership,
unquantized original-position membership, schema/profile and CLI/published report
agreement. It checks exact archive inventory and absence of staging after success.
It independently derives source/root counts and the root baseline. All schema7
certificate fields are typed and bounded, with no legacy field compatibility.

The large artifact's independent geometric reference is deliberately elementary:
both decoded nonempty triangle supports lie within the same closed64m square.
Their symmetric whole-support distance is therefore <=sqrt(8192)<100m. Exact
rational containment and squared-budget comparisons establish the declared100m
budget. This does **not** independently prove the smaller published certificate
scalar. The receipt explicitly marks that smaller scalar unproved in this resource
case. The separate small final-artifact exact canonical-cover checker establishes
the actual tighter certificate on bounded acceptance workloads. Reported operational
counts here receive arithmetic plausibility/control checks; exact runtime counts
also require source and unit controls.

## Measurement and supervision

The Linux-only driver samples child `/proc/<pid>/status`, descriptors and output
scratch files every10ms by default. It records sampled RSS, thread, descriptor,
scratch-file and scratch-byte peaks separately from `wait4` ru_maxrss, CPU and
elapsed process observations. Sampled peaks may miss short events. Scratch
sampling excludes the final `result.3tz`; a zero sampled scratch peak is not a
proof that staging never existed.

Each child starts a new session. The cooperative supervisor checks a fixed
180-second limit, sends SIGKILL to the child's process group on expiration and
uses wait4 to reap the main child synchronously. Timeout, signal/exit status and
reap truth remain in the receipt, and the driver exits nonzero rather than certify
the case. This is process supervision, not a hard timeout or cancellation latency
guarantee. It does not claim to reap arbitrary descendants or measure their total
RSS. Sampling exceptions also trigger group kill and main-child reap before
propagating failure; such a failure cannot become a successful receipt.

All three children launch and finish before large artifact decoding to reduce
inherited parent-decoder residency bias. Nevertheless wait4 peak RSS may include
Python launcher residency before exec; it is not pure converter allocation or
steady-state RSS. Measurements include source decode, meshopt proposal, proof,
encoding and publication. They establish observations for three serial workloads
on one Linux host, not total-memory boundedness, constant memory, a universal
time guarantee or one meshopt call's responsiveness.

The receipt pins supplied final source commit, exact production file hashes,
frozen optimized binary hash/path, driver and independent helper hashes, fixture
hashes, commands, host/Python metadata, supervisor limits, logs and output members.
It checks binary identity before every launch and after execution and verifies
production file hashes stayed unchanged. The supplied commit is recorded as
coordinator provenance; the driver performs no Git operation and cannot itself
prove that the commit describes the working tree. Large decoding must not occur
while a coordinator build is running or from a binary being overwritten.

## Invocation after the coordinator freezes the optimized CLI

Use absolute paths and a new artifact directory:

```sh
python3 bench/architecture_audit/mesh_approximation_f1d2/resource_probe.py \
  --binary /absolute/frozen/rusty-tiles-f1d2-release \
  --expected-binary-sha256 FROZEN_BINARY_SHA256 \
  --production-source FINAL_PRODUCTION_COMMIT \
  --artifact-dir /absolute/new/f1d2-resource-cases \
  --json-output /absolute/f1d2-resource-release.json
```

The driver's Python syntax was checked without importing or running a converter.
Fresh process measurements, any adjusted dynamic workload, final receipt review
and separate nonauthor review remain pending. Failed/supervised execution receipts
must remain distinct from accepted observations and from artifact math acceptance.
