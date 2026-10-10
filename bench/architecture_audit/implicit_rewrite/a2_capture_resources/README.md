# A2 capture/resource preparation

[Audit](audit.md) defines one captured archive owner, pre-allocation ZIP/JSON
admission, full member/name accounting, one Attempt/one final candidate, typed
failure/cleanup/report semantics and Rust/CLI/installed-Python removal gates.
This is preparation, not implementation or acceptance.

The smallest necessary changes belong to their actual owners: A2 captures one
bounded regular source into an immutable byte owner with member ranges; the
private ZIP/JSON admission helpers accept explicit remaining budgets; the
existing inspector's private Read+Seek core accepts a captured Cursor or the
same final staging File, while public tooling retains its own file binding.
A2 uses F0 Attempt/Staging and private archive serialization directly. No nested
public package/inspect wrapper or new default RunControl enters the operation.

[41 model cases](results.json) execute independent Python admission/accounting/
schedule models and actual POSIX capture/identity operations. They include a
sensitive proof-limit control demonstrating that restored mtime/length can
hide an in-place capture change. [Fourteen final small real-source envelopes](source-envelopes.json)
were independently counted and checked against the originating agent's pinned
portable producer receipt; this agent ran no producer/build.

```sh
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/a2_capture_resources/probe.py /tmp/new-capture-evidence
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/a2_capture_resources/count_source_envelopes.py /path/to/pinned/real-source-artifacts /tmp/new-source-envelopes.json
```

Driver/result identities and evidence limits are explicit in the receipts.
Proposed byte/count/depth/reference/name/output ceilings are independently
motivated by admission/amplification controls but still need final production
RSS, descriptor, scratch and boundary acceptance. Source/provenance resource
semantics and C1's child-volume containment predicate remain contract
dependencies. No legacy operation removal or release gate closes here.
