#!/usr/bin/env python3
"""Finite design model of job lifetime; does not execute the production runtime."""
from __future__ import annotations

import argparse
from collections import Counter, deque
from dataclasses import asdict, dataclass, replace
from enum import Enum
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import sys


class Phase(str, Enum):
    STAGING = "staging"
    SEALED = "sealed"
    CHECKED = "split_gate_check"  # Negative control only; grants no permission.
    PUBLISHING = "publishing"
    ABORTING = "aborting"
    RESTORING = "restoring_old_destination"
    COMMITTED = "committed"
    FAILED = "precommit_failure"
    RECOVERY = "recovery_required"


class Candidate(str, Enum):
    WRITABLE = "writable_staging"
    SEALED = "closed_checked_candidate"
    NONE = "no_candidate"


class Destination(str, Enum):
    ABSENT = "absent"
    OLD = "previous_destination"
    RIVAL = "competitor_destination"
    OURS = "new_destination"


class Backup(str, Enum):
    NONE = "none"
    HELD = "old_destination_held"
    RETAINED = "old_destination_retained_with_cleanup_notice"


class Cancellation(str, Enum):
    NONE = "not_requested"
    BEFORE = "requested_before_permission"
    AFTER = "requested_after_permission"


class Cause(str, Enum):
    NONE = "none"
    CANCEL = "cancellation"
    OBSERVER = "observer_error"
    ENCODER = "encoder_error"
    PUBLISHER = "publisher_error"


@dataclass(frozen=True)
class State:
    phase: Phase
    candidate: Candidate
    workers: tuple[int, ...]
    events: tuple[int, ...]
    destination: Destination
    backup: Backup = Backup.NONE
    cancellation: Cancellation = Cancellation.NONE
    diagnostic: str = "none"
    primary_cause: Cause = Cause.NONE


@dataclass(frozen=True)
class Case:
    name: str
    mode: str
    policy: str
    callbacks: tuple[str, ...]


def transitions(state: State, case: Case):
    """Candidate design relation. The property checker below is separate."""
    phase = state.phase
    if state.cancellation == Cancellation.NONE:
        if phase in (Phase.STAGING, Phase.SEALED):
            yield "cancel_before_permission", replace(state, phase=Phase.ABORTING, cancellation=Cancellation.BEFORE, primary_cause=Cause.CANCEL)
        elif phase == Phase.CHECKED:
            # Broken split gate ignores the abort request after its earlier check.
            yield "cancel_before_permission", replace(state, cancellation=Cancellation.BEFORE, primary_cause=Cause.CANCEL)
        elif phase == Phase.ABORTING:
            yield "secondary_cancellation", replace(state, cancellation=Cancellation.BEFORE)
        elif phase in (Phase.PUBLISHING, Phase.RESTORING, Phase.COMMITTED):
            yield "cancel_after_permission", replace(state, cancellation=Cancellation.AFTER)

    if case.policy == "no_clobber" and state.destination == Destination.ABSENT and phase in (Phase.STAGING, Phase.SEALED, Phase.CHECKED, Phase.PUBLISHING):
        yield "competitor_creates_destination", replace(state, destination=Destination.RIVAL)

    if phase == Phase.STAGING:
        for worker in state.workers:
            yield f"worker_{worker}_finishes", replace(
                state, workers=tuple(w for w in state.workers if w != worker),
                events=state.events + (worker,),
            )
        if state.events:
            worker, *rest = state.events
            if case.callbacks[worker] == "error":
                yield "observer_error_before_permission", replace(state, phase=Phase.ABORTING, events=tuple(rest), primary_cause=Cause.OBSERVER)
            else:
                yield "deliver_worker_event", replace(state, events=tuple(rest))
        yield "encoder_error", replace(state, phase=Phase.ABORTING, primary_cause=Cause.ENCODER)
        if not state.workers and not state.events:
            yield "seal", replace(state, phase=Phase.SEALED, candidate=Candidate.SEALED)
        if case.mode == "premature_gate":
            yield "grant_publication", replace(state, phase=Phase.PUBLISHING)

    elif phase == Phase.SEALED:
        if case.mode == "split_gate":
            yield "check_abort_separately", replace(state, phase=Phase.CHECKED)
        else:
            # One synchronized transition grants publication permission.
            yield "grant_publication", replace(state, phase=Phase.PUBLISHING)

    elif phase == Phase.CHECKED:
        yield "grant_publication", replace(state, phase=Phase.PUBLISHING)

    elif phase == Phase.PUBLISHING:
        if case.policy == "replace" and state.destination == Destination.OLD:
            yield "hold_old_destination", replace(state, destination=Destination.ABSENT, backup=Backup.HELD)
            yield "publication_error", replace(state, phase=Phase.ABORTING, primary_cause=state.primary_cause if state.primary_cause != Cause.NONE else Cause.PUBLISHER)
        else:
            # Assumed linearizable install-if-absent namespace operation.
            # No assumption about cleanup atomicity or power-loss durability.
            if state.destination != Destination.ABSENT and case.mode != "clobbering_install":
                yield "install_refused_existing_destination", replace(state, phase=Phase.ABORTING, primary_cause=state.primary_cause if state.primary_cause != Cause.NONE else Cause.PUBLISHER)
            else:
                events = (-1,) if case.mode == "callback_after_publish" else state.events
                yield "install_success", replace(
                    state, phase=Phase.COMMITTED, candidate=Candidate.NONE,
                    destination=Destination.OURS, events=events,
                )
            failed_phase = Phase.RESTORING if state.backup == Backup.HELD else Phase.ABORTING
            yield "publication_error", replace(state, phase=failed_phase, primary_cause=state.primary_cause if state.primary_cause != Cause.NONE else Cause.PUBLISHER)

    elif phase == Phase.ABORTING:
        if case.mode == "revive_failed_producer" and state.primary_cause == Cause.ENCODER and not state.workers and not state.events:
            yield "seal", replace(state, phase=Phase.SEALED, candidate=Candidate.SEALED)
        if state.diagnostic == "none":
            cause = Cause.OBSERVER if case.mode == "overwrite_primary" else state.primary_cause
            yield "secondary_abort_error", replace(state, diagnostic="secondary_worker_or_observer_error", primary_cause=cause)
        for worker in state.workers:
            yield f"worker_{worker}_acknowledges_abort", replace(state, workers=tuple(w for w in state.workers if w != worker))
        if state.events:
            yield "discard_undelivered_event", replace(state, events=state.events[1:])
        if not state.workers and not state.events:
            yield "discard_candidate", replace(state, phase=Phase.FAILED, candidate=Candidate.NONE)

    elif phase == Phase.RESTORING:
        yield "restore_success", replace(state, phase=Phase.FAILED, candidate=Candidate.NONE, destination=Destination.OLD, backup=Backup.NONE)
        if case.mode == "lost_recovery":
            yield "restore_error", replace(state, phase=Phase.FAILED, candidate=Candidate.NONE, backup=Backup.NONE)
        else:
            yield "restore_error", replace(state, phase=Phase.RECOVERY, candidate=Candidate.NONE)

    elif phase == Phase.COMMITTED:
        if case.mode == "callback_after_publish" and state.events:
            yield "postcommit_callback_error", replace(state, phase=Phase.FAILED, events=())
        if state.backup == Backup.HELD:
            yield "backup_cleanup_success", replace(state, backup=Backup.NONE)
            if case.mode == "cleanup_reclassifies_commit":
                yield "backup_cleanup_error", replace(state, phase=Phase.FAILED, backup=Backup.RETAINED)
            else:
                yield "backup_cleanup_error", replace(state, backup=Backup.RETAINED)
        if state.diagnostic == "none":
            # A VM/return adapter can observe an error after core installation;
            # that error is diagnostic information on an already committed job.
            yield "external_return_diagnostic", replace(state, diagnostic="postcommit_adapter_error")


@dataclass(frozen=True)
class History:
    """Independent temporal specification monitor, not part of job state."""
    abort_before_permission: bool = False
    saw_commit: bool = False
    selected_primary: Cause = Cause.NONE


INVARIANTS = {
    "abort_before_permission_prevents_commit": "Every accepted fatal cause before publication permission forbids permission and installation.",
    "closed_drained_before_publication": "At permission grant and installation, all workers and events are drained and the candidate is sealed.",
    "committed_outcome_is_sticky": "After installation succeeds, no later error changes the outcome to precommit failure or recovery-required.",
    "no_clobber_preserves_competitor": "Without replace permission, a successful install cannot overwrite an existing destination.",
    "restore_failure_has_recovery": "Failure to restore a held previous destination yields recovery-required with the backup retained.",
    "precommit_failure_preserves_destination": "Precommit failure cannot own a new destination; replace failure preserves the previous destination.",
    "terminal_resources_are_drained": "Committed, failed and recovery outcomes own no candidate, workers or pending observer events.",
    "no_callbacks_after_seal": "Core callback delivery occurs only during staging, before seal/drain completes.",
    "postcommit_destination_survives_cleanup": "Cleanup and return diagnostics after installation preserve the installed destination.",
    "first_gate_cause_remains_primary": "The first abort/fatal cause accepted at the synchronized gate remains primary; later errors are secondary. Publisher errors select the cause after publication permission.",
}


def check_edge(before: State, action: str, after: State, history: History, case: Case):
    """Specification checks observe events/resources, not transition helpers."""
    committed = history.saw_commit or action == "install_success"
    primary = history.selected_primary
    accepted = {
        "cancel_before_permission": Cause.CANCEL,
        "observer_error_before_permission": Cause.OBSERVER,
        "encoder_error": Cause.ENCODER,
        "publication_error": Cause.PUBLISHER,
        "install_refused_existing_destination": Cause.PUBLISHER,
    }
    # Publisher failures happen after permission; they do not establish a
    # pre-permission abort. All accepted producer-side causes do.
    abort = history.abort_before_permission or (
        action in accepted and accepted[action] != Cause.PUBLISHER
    )
    if primary == Cause.NONE and action in accepted:
        primary = accepted[action]
    violations = []
    if abort and (action in ("grant_publication", "install_success") or after.phase in (Phase.PUBLISHING, Phase.COMMITTED)):
        violations.append("abort_before_permission_prevents_commit")
    if action in ("grant_publication", "install_success") and (before.workers or before.events or before.candidate != Candidate.SEALED):
        violations.append("closed_drained_before_publication")
    if history.saw_commit and after.phase != Phase.COMMITTED:
        violations.append("committed_outcome_is_sticky")
    if action == "install_success" and case.policy == "no_clobber" and before.destination != Destination.ABSENT:
        violations.append("no_clobber_preserves_competitor")
    if action == "restore_error" and (after.phase != Phase.RECOVERY or after.backup != Backup.HELD or after.destination != Destination.ABSENT):
        violations.append("restore_failure_has_recovery")
    if after.phase == Phase.FAILED and (after.destination == Destination.OURS or (case.policy == "replace" and after.destination != Destination.OLD)):
        violations.append("precommit_failure_preserves_destination")
    if after.phase in (Phase.COMMITTED, Phase.FAILED, Phase.RECOVERY) and (after.workers or after.events or after.candidate != Candidate.NONE):
        violations.append("terminal_resources_are_drained")
    if action in ("deliver_worker_event", "observer_error_before_permission", "postcommit_callback_error") and before.phase != Phase.STAGING:
        violations.append("no_callbacks_after_seal")
    if history.saw_commit and after.destination != Destination.OURS:
        violations.append("postcommit_destination_survives_cleanup")
    if after.primary_cause != primary:
        violations.append("first_gate_cause_remains_primary")
    return History(abort, committed, primary), violations


def snapshot(state):
    return asdict(state)


def explore(case: Case):
    initial = State(
        Phase.STAGING, Candidate.WRITABLE, tuple(range(len(case.callbacks))), (),
        Destination.OLD if case.policy == "replace" else Destination.ABSENT,
    )
    start = (initial, History())
    pending = deque([start])
    paths = {start: []}
    violations = {}
    terminals = Counter()
    terminal_causes = Counter()
    actions = Counter()
    counts = Counter()
    while pending:
        current = pending.popleft()
        state, history = current
        successors = list(transitions(state, case))
        if not successors:
            terminals[state.phase.value] += 1
            terminal_causes[state.primary_cause.value] += 1
        for action, after in successors:
            counts["explored_edges"] += 1
            actions[action] += 1
            updated, errors = check_edge(state, action, after, history, case)
            trace = paths[current] + [{"action": action, "before": snapshot(state), "after": snapshot(after)}]
            for invariant in errors:
                if invariant not in violations:
                    violations[invariant] = {"violating_edges": 0, "shortest_witness": trace}
                violations[invariant]["violating_edges"] += 1
            next_state = (after, updated)
            if next_state not in paths:
                paths[next_state] = trace
                pending.append(next_state)
                if len(paths) > 20_000:
                    raise RuntimeError("Exploration limit exceeded: no partial result is a proof")
    return {
        "case": asdict(case), "reachable_state_monitor_pairs": len(paths),
        "explored_edges": counts["explored_edges"],
        "max_shortest_path_length": max(map(len, paths.values())),
        "terminal_state_monitor_counts": dict(sorted(terminals.items())),
        "terminal_primary_cause_counts": dict(sorted(terminal_causes.items())),
        "action_edge_counts": dict(sorted(actions.items())),
        "exhaustive_within_bounds": True, "invariants_hold": not violations,
        "violations": violations,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, default=Path(__file__).with_name("results.json"))
    args = parser.parse_args()
    cases = []
    for policy in ("no_clobber", "replace"):
        for callbacks in ((), ("ok",), ("error",), ("ok", "ok"), ("error", "ok"), ("ok", "error"), ("error", "error")):
            name = f"{policy}_{'_'.join(callbacks) or 'no_workers'}"
            cases.append(Case(name, "contract", policy, callbacks))
    controls = [
        Case("producer_failure_then_publication", "revive_failed_producer", "no_clobber", ()),
        Case("callback_after_publish", "callback_after_publish", "no_clobber", ("ok",)),
        Case("split_abort_check_and_permission", "split_gate", "no_clobber", ("ok",)),
        Case("publish_before_worker_event_drain", "premature_gate", "no_clobber", ("ok", "ok")),
        Case("replace_competitor_without_permission", "clobbering_install", "no_clobber", ("ok",)),
        Case("discard_failed_restore_backup", "lost_recovery", "replace", ("ok",)),
        Case("cleanup_error_reclassifies_commit", "cleanup_reclassifies_commit", "replace", ("ok",)),
        Case("later_abort_overwrites_primary", "overwrite_primary", "no_clobber", ("error",)),
    ]
    results = [explore(case) for case in cases + controls]
    valid = results[:len(cases)]
    negative = results[len(cases):]
    expected_control_properties = {
        "producer_failure_then_publication": "abort_before_permission_prevents_commit",
        "callback_after_publish": "committed_outcome_is_sticky",
        "split_abort_check_and_permission": "abort_before_permission_prevents_commit",
        "publish_before_worker_event_drain": "closed_drained_before_publication",
        "replace_competitor_without_permission": "no_clobber_preserves_competitor",
        "discard_failed_restore_backup": "restore_failure_has_recovery",
        "cleanup_error_reclassifies_commit": "committed_outcome_is_sticky",
        "later_abort_overwrites_primary": "first_gate_cause_remains_primary",
    }
    controls_detected = all(expected_control_properties[item["case"]["name"]] in item["violations"] for item in negative)
    success = all(item["invariants_hold"] for item in valid) and controls_detected
    report = {
        "schema_version": 1, "artifact_kind": "bounded_design_model_not_production_runtime_test",
        "checkout_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "model_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "toolchain": {"python": sys.version, "platform": platform.platform()},
        "command": [sys.executable, *sys.argv],
        "bounds": {"workers": [0, 1, 2], "events_per_worker": 1, "cancellation_requests": 1,
                   "competitor_creates": 1, "external_return_diagnostics": 1,
                   "publication_attempts": 1, "restore_attempts": 1, "cleanup_attempts": 1},
        "invariants": INVARIANTS, "expected_negative_control_properties": expected_control_properties,
        "all_contract_cases_pass": all(item["invariants_hold"] for item in valid),
        "all_negative_controls_detected": controls_detected, "success": success,
        "cases": results,
        "assumptions": [
            "Permission grant is synchronized with pre-permission abort acceptance.",
            "Install-if-absent is a linearizable namespace operation preserving an existing destination.",
            "No assumed atomic temp-name cleanup, filesystem crash durability or FFI callback correctness.",
            "Replacement restore scenarios are a future contract, not an F0 implementation commitment.",
            "Workers eventually finish or acknowledge abort, and sealed candidates have no open writers.",
            "Callback delivery completes atomically; event admission is closed before seal, not modeled as a real lock.",
            "Failures are represented at modeled boundaries, not at every filesystem instruction.",
        ],
        "not_proven": ["Production behavior", "Filesystem/OS primitives", "Crash recovery", "Power-loss durability",
                       "Thread synchronization implementation", "Python FFI/GIL", "Unbounded workers or retries",
                       "Active/reentrant callbacks or actual event-admission synchronization",
                       "Duplicate publication and consumed-handle enforcement (one-attempt bound)",
                       "Exception object identity", "Liveness under unfair scheduling",
                       "Process signals", "Concurrent replace publishers", "Failed staging deletion", "Format correctness"],
    }
    args.results.parent.mkdir(parents=True, exist_ok=True)
    args.results.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"Contract cases: {sum(item['invariants_hold'] for item in valid)}/{len(valid)} pass")
    print(f"Negative controls: {sum(not item['invariants_hold'] for item in negative)}/{len(negative)} detected")
    print(f"Explored {sum(item['reachable_state_monitor_pairs'] for item in results)} state/monitor pairs and {sum(item['explored_edges'] for item in results)} edges")
    print(f"Evidence: {args.results}")
    return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())
