# Vector acceptance evidence

The independent CLI oracle is `tests/vector_acceptance_oracle.py`. It uses only Python standard-library ZIP, JSON and GLB byte decoding. It does not import the converter, frozen Python vector encoder, or existing test inventory helpers. Expected source identities come from invented fixtures; GeoJSON IDs use their JSON string representation. Full-detail leaves are inspected so parent LOD copies do not inflate accepted identity counts.

Replay with an explicitly selected freshly built binary:

```
python tests/vector_acceptance_oracle.py --binary /absolute/path/rusty-tiles --evidence /tmp/vector-acceptance-results
```

The accepted-only and rejected-first sources deliberately share the layer name `source`. Their accepted content hashes and frame must match. The rejected polygon contains a unique property so leaked schema state changes content bytes and fails the control. A rejected first feature must not select the frame: the selected policy is first accepted geometry, except compatible reuse retains its prior frame. Reports for rejected geometry contain rejection outcomes only. Strict rejection preserves pre-existing output.

Archive inspection compares exact manifest references with all `t/` members, rejects duplicate archive members, checks content-address hashes, decodes structural metadata string columns, and validates expected leaf identities. Encoded-byte parent rejection and unchanged reuse repeat those checks. A two-part MultiLineString with three vertices per part exceeds the four-vertex spool budget; the second part has coordinates outside float32 range. Store SQL tests establish that a first fragment was inserted before the second rejects; the CLI oracle independently checks final content and reports. Forced exact, hardlink and symlink output aliases preserve source bytes, and simultaneous conversions verify separate accepted identities. These checks establish acceptance ownership and resource inventory, not topology, CRS, repair or LOD algorithm correctness.

## Baseline executed observation

Binary SHA256 `61831ded55b94f278ce2557e2de51f5f2b1a6a22b47709d1435e89df6d214461` is identified by the D2 evidence as source commit `5c4bd6d7d012472b35cc360cc188b469cb094ad5`. Vector files have no diff from that commit to baseline `0cdbb72`. Executing the previously retained oversized polygon fixture produced one accepted point and one skipped polygon, while reporting `fragmentedPolygons=1`. Independent structural-metadata decoding found only source ID `"valid"`; JSONL retained a partition-success diagnostic for `"oversize"` followed by its skipped diagnostic. This is an executed report/counter defect. No general orphan-content leak was observed.

## Additional evidence boundaries

The CLI cannot observe private spool SQL, trigger observer CWD changes or inject internal storage failures. Store unit tests must independently inspect SQL rows and vertex ownership after a partial-fragment rejection. Public API integration controls must cover relative paths with observer-triggered CWD changes and source/output/reuse alias overlap. Fatal fault coverage must include source queries, savepoint/insert/rollback/release/commit, report writes and finalization, worker/storage/archive writes, cancellation and observers under `skipInvalid`; original infrastructure causes and existing output hashes must survive. No production environment fault switches are authorized.

Native migration requires its own recorded execution. Concurrent isolation and edited reuse are checked by the CLI oracle; resource measurements must be read with their sampling limitations. Source observations and existing test assertions are not substitute evidence.

Resource replay (Linux, requires `cc`):

```
python bench/architecture_audit/vector_acceptance/measure.py --binary /absolute/path/rusty-tiles --output /tmp/vector-resources.json
```

The runner varies feature count separately from maximum individual geometry size. It uses the existing small C `wait4` launcher to avoid inheriting Python fixture-generation RSS. Scratch totals are sampled and can miss short peaks; no asymptotic bound follows from these runs.

## Portable candidate execution

The independent oracle passed for binary SHA256 `a4faa83ce9a81ea9fda5da1a8706de31d52c779ebbf1ce59c6a5380523737b34`. Artifacts and commands were retained at `/tmp/rt119-independent/portable-candidate` during this audit. Edited reuse holds the first accepted coordinate fixed (so fresh and prior-frame policies agree), changes the second coordinate and metadata, and compares fresh versus reuse rendered identity sets, content hashes and root transform. It requires rebuilding some content and verifies the edited content differs from the original.

The portable resource runner passed five cases. Peak RSS KiB for 100/1,000/10,000 two-vertex features was 31,680/34,196/48,820; for one 1,000/10,000-vertex feature it was 32,196/37,076. Corresponding sampled scratch bytes were 67,836/769,992/10,109,378 and 83,072/701,352. These observations show growth within the exercised fixtures and do not establish constant memory or exact scratch maxima. All runs cleaned scratch directories. Full measurements were retained at `/tmp/rt119-independent/resources-portable.json`.

## Provisional native candidate execution

The independent oracle also passed for native binary SHA256 `326924d88448042320db335d573f1c85d5412b8342f21e9a6305452aadcb5532`. Actual GLB byte maxima, decoded position counts and content-member counts agree with the published conversion report in both candidate builds. Native resource peak RSS KiB for 100/1,000/10,000 two-vertex features was 69,028/71,172/83,096; for one 1,000/10,000-vertex feature it was 69,232/75,032. Sampled scratch bytes were 70,200/769,992/10,109,381 and 83,072/703,229. Scratch directories cleaned. Native evidence is retained at `/tmp/rt119-independent/native-candidate` and `/tmp/rt119-independent/resources-native.json`.

This native build preceded the final source freeze; its observations are provisional candidate behavior and must be replayed against the final frozen source artifact before approval.

## Malformed GeoJSON and independent native driver control

The oracle now checks unsupported geometry type, short Point, four-coordinate Point, nonnumeric Point and missing coordinates. Each malformed feature must reject under strict mode with existing bytes preserved; skip mode must count a rejected feature (not absent geometry) and preserve accepted-only content, frame and schema. This expanded matrix requires replay after the reader migration; prior recorded oracle passes do not prove the added cases.

Pass `--native` to additionally consume a GeoPackage generated directly with stdlib SQLite and hand-built GeoPackage headers/ISO WKB. No OGR fixture writer or converter code supplies expected identities. Native FID 2 is the sole accepted source identity; oversized FID 1 rejects. Skip, strict preservation, unchanged reuse, exact archive inventory and unchanged source SHA are inspected independently. The provisional native binary passed this driver control at `/tmp/rt119-independent/native-gpkg-candidate`; a frozen-source replay is still required.

## Reuse deletion and derived schema controls

The expanded oracle now reuses the two-point archive with an empty collection and a collection containing only null geometry. Both must publish an empty accepted identity/content inventory with zero accepted features/fragments; null geometry contributes exactly one `featuresWithoutGeometry`. Source bytes and the prior archive SHA remain unchanged. A new integer property column exercises schema changes: reused output must rebuild and match fresh accepted identities and exact content hashes.

The shared-reader native candidate passed this expanded oracle including `--native` driver controls at `/tmp/rt119-independent/shared-native`. This remains prefreeze evidence. The earlier native candidate's narrower pass was invalidated as comprehensive acceptance evidence by the subsequently added malformed-type fixture, which executed strict-mode publication of malformed input. It remains evidence only for its originally executed cases. The recorded `portable-final.json` likewise identifies the earlier `1445c503` source and narrower matrix; the revised source needs fresh final replay.
