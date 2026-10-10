# A2 implicit rewrite audit and baseline probes

This is preparation for the next operation under #126, not acceptance or an
implementation of its replacement. The [draft contract](../../../docs/architecture/implicit-rewrite-contract.md)
and [ownership audit](audits/ownership.md) distinguish settled ownership from
unproved admission, numerical, format and resource choices. Existing point/vector
use cases need explicit dispositions before their legacy operation can disappear.

## Fresh continuation audits after accepted F1d2

The A2 continuation at accepted merge
`34a76152d18b02553691f472207225f116307a67` adds three bounded lanes:
[primary semantics and authored frame/availability fixtures](a2_semantics/README.md),
[actual point/vector byte/geometry/resource probes](a2_real_sources/README.md),
and [captured-source/resource/lifecycle models](a2_capture_resources/README.md).
Their individual receipts bind their own executed drivers and source/artifact
identities. They do not overwrite or promote the historical baseline below.

The finite selected design uses source-child ordinals, mandatory effective
bounds/error metadata and decoded content/ancestor coherence. Nominal cell
containment, center uniqueness and provenance labels do not determine truth.
Four authored design fixtures and 22 corruption controls support the finite
addressing/frame model; 41 separate capture/accounting models demonstrate
admission order and explicit snapshot limitations. Actual production rewrite,
terminal external-template interpretation/consumer proof, imported semantic
metadata disposition and maximum Rust resources remain separately gated.

## Executed baseline observations

[Compact receipt](probes/baseline-receipt.json) pins the frozen portable CLI,
authored eight-point GeoJSON, independent replay driver and generated archives.
The CLI is bound by the unchanged 91 production inputs in the retained
[source manifest](../f1c2/receipts/source-artifacts.json); the merged tree at
`e3d222a4c27a86e1f06e1e4b47db4e6a4fa5e24e` retains those inputs. The full
command/stdout receipt is stored verbatim as [baseline-raw.json.gz](probes/baseline-raw.json.gz),
with its uncompressed hash recorded in the compact receipt. No binary or output
archive is committed.

The independently authored source is processed by the current portable vector
producer, then the current explicit-to-implicit operation. The Python oracle
decodes binary subtree headers, LSB availability, ranked bounding-box/error rows,
external child fields/transforms and GLB positions without the production
implicit expander or validator. It observes all eight authored world points with maximum coordinate error
`6.71e-10` metres, exact original non-control members and payload aliases.
Cleared/swapped availability, wrong box/error, changed external transform and
changed payload bytes all fail the oracle. This is one shallow QUADTREE fixture;
it proves no general padding, deeper hierarchy, content-array, OCTREE or viewer
refinement policy.

A separate collision fixture moves payload metadata schemas to a referenced
archive member named `subtrees`. C1 structurally admits the source; conversion
then reports `io`/exit 1 (`File exists`) when creating its generated subtree
directory. The source and forced prior output remain unchanged and no work
directory remains. This executes the ancestor-collision preparation gap identified
by source inspection. C1 admission is supplementary structural evidence; it does
not certify source metadata semantics.

Changing only the explicit source's asset version to 1.0 is accepted and copied
to the implicit output. That is a declaration/admission observation, not a claim
that the mutated vector source is valid 3D Tiles 1.0 or proof of a standards
violation. The implicit-root external-link template interpretation remains a
primary-specification proof question, rather than an executed decoding defect.

## Replay and proof limits

Use the pinned CLI and a new, nonexistent output directory:

```sh
python3 -B bench/architecture_audit/implicit_rewrite/probes/replay_baseline.py /path/to/frozen/rusty-tiles /tmp/new-a2-replay
```

The driver refuses a different binary hash and creates only its requested probe
directory. The generated `receipt.json` retains commands and causal failures.
Paths and resulting receipt hashes differ across replay directories; the fixture,
binary and archive byte identities can be compared separately. Python assertions
are part of this audit driver, so do not run it with `-O`.

The baseline probes do not settle the proposed representation. Before coding,
prove tight local frames, terminal-link format semantics, real producer padding,
resource closure, captured-source identity and measured limits. Replacement
acceptance then adds one-run lifecycle/fault/publication and installed-adapter
checks, plus actual consumer coarse/fine selection. No parent foundation or
release blocker is closed here.
