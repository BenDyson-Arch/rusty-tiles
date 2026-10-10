#!/usr/bin/env python3
"""Independent finite DFS accounting model; no production/geometry acceptance."""
import hashlib
import json
from pathlib import Path


def run(roots, targets, cap, depth_limit, accept_depth, cancel_after=None):
    base = roots * targets
    if base > cap:
        return dict(outcome="base_refusal", work=0, base=base, peak_pending=0)
    work = visited = accepted = splits = peak = 0
    checkpoints = []
    for _ in range(roots):
        pending = [0]
        while pending:
            peak = max(peak, len(pending))
            depth = pending.pop()
            visited += 1
            for _ in range(targets):
                if work % 64 == 0:
                    checkpoints.append(work)
                    if cancel_after is not None and work >= cancel_after:
                        return dict(outcome="cancelled", work=work, base=base,
                                    peak_pending=peak, checkpoints=checkpoints)
                if work == cap:
                    return dict(outcome="dynamic_refusal", work=work, base=base,
                                peak_pending=peak, checkpoints=checkpoints)
                work += 1
            if depth >= accept_depth:
                accepted += 1
            elif depth == depth_limit:
                return dict(outcome="depth_refusal", work=work, base=base,
                            peak_pending=peak, checkpoints=checkpoints)
            else:
                splits += 1
                pending.extend([depth + 1] * 4)
    assert visited == roots + 4 * splits
    assert accepted == roots + 3 * splits
    assert peak <= 3 * depth_limit + 1
    return dict(outcome="accepted", work=work, base=base, visited=visited,
                accepted=accepted, splits=splits, peak_pending=peak,
                checkpoints=checkpoints)


cases = {
    "base_refusal_without_evaluations": run(2, 4, 7, 24, 0),
    "exact_cap_acceptance": run(2, 4, 8, 24, 0),
    "complete_depth_one": run(2, 4, 40, 24, 1),
    "dynamic_refusal_after_admitted_base": run(2, 4, 39, 24, 1),
    "depth_refusal_does_not_assert_true_distance": run(2, 4, 100, 0, 1),
    "bounded_checkpoint_cancellation": run(2, 4, 1000, 24, 4, cancel_after=1),
}
assert cases["base_refusal_without_evaluations"]["work"] == 0
assert cases["exact_cap_acceptance"]["outcome"] == "accepted"
assert cases["complete_depth_one"]["accepted"] == 8
assert cases["dynamic_refusal_after_admitted_base"]["work"] == 39
assert cases["depth_refusal_does_not_assert_true_distance"]["outcome"] == "depth_refusal"
assert cases["bounded_checkpoint_cancellation"]["work"] == 64

root = Path(__file__).resolve().parents[4]
sources = ["AGENTS.md", "docs/architecture/README.md", "src/mesh_archive.rs",
           "src/mesh_archive/approximation.rs", "src/mesh_archive/source.rs",
           "src/mesh_archive/binding.rs", "src/mesh_archive/encode.rs",
           "src/main.rs", "bindings/python/src/lib.rs"]
print(json.dumps({
    "kind": "independent_design_accounting_model",
    "production_executed": False,
    "baseline_supplied_by_coordinator": "3b5703231bf3991c2e37caef99989214308cc08d",
    "source_sha256": {p: hashlib.sha256((root / p).read_bytes()).hexdigest() for p in sources},
    "driver_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    "profile_proposal": {"work_limit": 16777216, "depth_limit": 24,
                         "max_pending_per_root": 73},
    "cases": cases,
    "limits": ["Uses scripted acceptance depth rather than geometry.",
               "Does not execute Rust, meshopt, F0, adapters or filesystem publication.",
               "Establishes accounting-model consistency, not production resource guarantees."]
}, indent=2))
