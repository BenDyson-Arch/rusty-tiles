# P1 independent final review

A Sol 6.1 agent that did not author the implementation reviewed production
commit `12d98ab`, the source/ownership boundaries and actual frozen portable and
native binaries. This records a bounded review, not universal source or algorithm
certification.

The review found a real oracle defect: the duplicate-GeoKey mutation branch also
captured the untyped-ExtraBytes and waveform controls, making their intended
mutations unreachable. Commit `d92acd2` restricts that branch and labels each
typed refusal assertion separately. The reviewer then reran the committed quick
oracle against both binaries: each passed six decoded cases, 30 profile controls
and seven archive negative controls. Binary identities match `validation.json`.

Malformed headers/offsets, duplicate relevant GeoKeys, unsupported FLOAT32
declarations, actual untyped/waveform inputs and source/output aliases were
refused without replacing old outputs. Explicit/implicit source attributes,
multiplicity, rendered placement, representatives, bounds and reachable inventory
passed the independent decoded checks. The recorded production source hashes
match the reviewed files; later oracle/fixture/evidence commits change no producer
or Rust adapter code.

The reviewer reported no confirmed production merge blocker and checked the local
Rust, CLI, installed-wheel and system-Blender validation records. A final artifact
review verified matching binary/source/oracle identities and all 18 LAS cases,
30 profile/reference controls, seven mutations and 14 LAZ cases per adapter.
LAZ covers each admitted format in both layouts and uses the original independent
LAS records as expected truth. Resource records retain fixed options, successful
point counts and empty postexit workspaces; the native probe covers three row
counts and the portable probe also covers two wider schemas. Some short runs have
as few as two 10 ms samples, which cannot establish maximum or asymptotic memory
bounds. This artifact audit does not add those full runs to the reviewer's own
quick replay claim. Remote CI, cross-platform behavior, official Blender matrices,
arbitrary CRS/source breadth and production payload validation (#133) remain
outside this approval.
