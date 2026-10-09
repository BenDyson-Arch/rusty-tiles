# Bounded job-lifetime design model

This is executable evidence for a proposed contract, **not a production runtime
test or proof of filesystem/FFI correctness**. It does not modify a converter,
the normal test suite, or any production source. It proposes a small lifecycle
and checks its contract independently rather than encoding current behavior as
an expectation to preserve.

The first implementation scope (F0) remains one real file-pack consumer through
Rust, CLI and Python. Replacement/restore scenarios below describe a future
contract; they do not expand that first implementation into directory publication,
feature transactions or a universal converter engine.

## Run

From the repository root, with Python's standard library only:

```sh
python3 bench/architecture_audit/foundation/model.py
```

To preserve the checked-in run:

```sh
python3 bench/architecture_audit/foundation/model.py --results /tmp/foundation-model-rerun.json
```

`results.json` records the exact command, model SHA-256, checkout commit,
Python/platform versions, bounds, invariants, reachable-state/edge counts and
shortest counterexample witnesses. The recorded Python 3.14.7/Linux run passes
14 contract cases and detects eight deliberately broken variants, exploring
2,475 state/history pairs and 4,446 edges. The process exits nonzero if any
contract case fails or any negative control misses its designated property.
There is no dependency install, Rust build or production binary invocation.

## Lifecycle and ownership

The consumer finishes or joins its workers, completes its callbacks and closes
event admission. A seal is possible only with no pending workers/events, yielding
a closed, checked candidate. The model does not assign joins or callback-resource
ownership to a generic sealer. The consumer requests publication through a
single synchronized abort/permission gate:

```text
staging -> sealed -> publication permission -> install -> committed
    |          |              |
    +-- abort--+              +-- publisher failure -> precommit failure
           |
      drain/discard -> precommit failure
```

Every accepted fatal cause (including producer failure), cancellation or callback
failure before publication permission prevents permission and installation. The first abort/fatal cause accepted by that gate remains primary;
later causes are secondary. Selection order is gate order, not wall-clock time.
Once publication permission is granted, cancellation does not stop the attempt,
and a publisher error can become primary. Permission is not installation: a
permitted attempt may still return a precommit failure.

Installation creates the committed outcome. Later cleanup errors or VM/return
diagnostics preserve that classification and installed destination. The ordinary
model never invokes core callbacks after seal. A VM/adapter diagnostic is distinct
from core callback delivery; the negative control introduces a final postcommit
callback and converts its exception into a precommit failure.

The no-clobber scenario starts without a destination and allows a competitor to
create one at any position before installation. Installation assumes a
**linearizable install-if-absent namespace operation** that refuses an existing
destination. That is an explicit platform primitive assumption. It is not a
claim that a rename, temporary-name cleanup or whole publisher is atomic, nor a
claim of crash/power-loss durability. Real platform evidence must establish the
chosen primitive separately.

The future replacement scenario starts with an old destination. It models
holding the old destination, installing the new candidate, and cleaning up the
backup. An installation error attempts restore. Restore success yields a
precommit failure with the old destination intact; restore failure yields
recovery-required with the old backup retained. Backup cleanup failure after
successful installation keeps a committed outcome with a retention notice.
Staging deletion is assumed to succeed; deletion failure needs a separate real
ownership/recovery test.

## Independent checks and negative controls

`transitions()` defines the candidate design. `check_edge()` independently
specifies ten temporal/resource invariants using observed actions, destination
ownership and artifact/worker/event state. Its separate history monitor remembers
abort acceptance, commitment and the selected primary cause. Breadth-first
exploration visits the product of job state and this specification history,
checking every reachable edge. Equivalent product states are merged; the model
is finite, acyclic and explored to exhaustion, not stopped at an arbitrary depth.
The safety limit raises an error rather than reporting a truncated run as proof.

| Deliberately broken variant | Designated property that detects it |
| --- | --- |
| Encoder failure followed by invalid reseal/publication | Every pre-permission fatal cause prevents permission and installation |
| Callback invoked after installation and reclassified as failure | Committed outcome remains committed |
| Abort checked separately from publication permission | Abort before permission prevents installation |
| Permission granted before workers/events drain | Closed/drained candidate required before publication |
| Install replaces competitor without replace permission | No-clobber preserves the competitor |
| Failed restore drops the backup and returns ordinary failure | Restore failure retains explicit recovery ownership |
| Backup cleanup error changes committed to failure | Committed outcome remains committed |
| Later observer/worker error overwrites first abort cause | First gate-selected cause remains primary |

The callback and split-gate controls resemble the *shape* of observed baseline
callback-after-publication and publication-race issues. They do not execute the
baseline or establish that its exact thread interleavings match these models.
Each negative control must violate its designated invariant, so merely passing
the proposed design cannot make a disconnected property checker appear useful.

## Bounds and limits

Every contract scenario runs with zero, one or two workers, one event per worker,
and all success/error callback combinations in that range. It allows one
cancellation, competitor creation, publication attempt, restore attempt,
postcommit diagnostic and cleanup attempt. The model explores cancellation
before and after permission, failure before seal, successful/refused/failed
publication, event ordering, restore success/failure and postcommit cleanup.

Callback delivery is modeled as atomic completion. Active, reentrant and FFI/GIL
callbacks, exception identity and actual admission-lock synchronization require
implementation tests. No liveness claim survives an unfair scheduler. Multiple
publication attempts/consumed-handle enforcement, concurrent replacement
publishers, process signals, crash recovery, durability, arbitrary retries,
larger worker counts, staging deletion failure and format/report correctness
are not proved. In particular, the one-attempt bound is not proof that a real
implementation prevents double publication.

## Independent review correction

The original `e54c32b` monitor recorded encoder failure as the first cause but
failed to remember it as a pre-permission abort. An externally supplied invalid
encoder-error → seal → permission → commit trace therefore escaped detection.
The transition generator prevented that trace; this was a checker gap, not an
observed production defect. The corrected monitor covers every accepted
pre-permission fatal cause and rejects permission itself after abort. Publisher
errors after permission remain distinct. The eighth negative control exercises
the exact producer-failure revival defect. Earlier seven-control evidence does
not establish this newly tested property.
