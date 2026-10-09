# Independent vector acceptance evidence

The source-derived oracle is `tests/vector_acceptance_oracle.py`. It imports only Python standard-library ZIP, JSON, SQLite and binary decoding. It does not import the converter, frozen Python vector encoder, OGR fixture writers or prior test helpers. Expected identities come from invented fixtures. GeoJSON IDs use JSON string representation; native GeoPackage IDs use numeric FIDs serialized as strings.

Replay portable and native builds separately:

```
python tests/vector_acceptance_oracle.py --binary /absolute/path/portable --evidence /tmp/portable-oracle
python tests/vector_acceptance_oracle.py --binary /absolute/path/native --evidence /tmp/native-oracle --native
python bench/architecture_audit/vector_acceptance/measure.py --binary /absolute/path/binary --output /tmp/vector-resources.json
```

## Contract and observable inventory

The selected frame policy is first accepted geometry, except compatible reuse retains its prior frame. Accepted-only and rejected-first sources share a layer name; their accepted content hashes and frame must match. Rejected geometry has a unique property so leaked schema state changes content bytes and fails the comparison. Rejected geometry has rejection diagnostics only. Strict rejection preserves existing output bytes.

The oracle compares exact manifest references with all `t/` members, rejects duplicate members, checks content hashes against filenames, decodes structural metadata and rendered `_FEATURE_ID` attributes, and validates expected full-detail leaf identities. Parent LOD copies do not inflate identities. Actual published GLB byte maxima, decoded position counts and content counts must agree with conversion reports.

The fixtures exercise oversized polygon rejection; partial fragmentation with a later float32-range failure; unsupported, short, extra-coordinate, nonnumeric and missing-coordinate GeoJSON geometry; encoded-byte parent rejection; unchanged and edited reuse; new-property schema reuse; deletion of all geometry; all-null reuse; exact/hardlink/symlink source/output aliases; and simultaneous conversions with independent identities. Every source remains byte-identical. Empty reuse has no accepted identities/content and cannot change its prior archive.

With `--native`, the oracle creates a GeoPackage directly with SQLite, GeoPackage binary headers and ISO WKB. Oversized polygon FID 1 rejects and valid point FID 2 alone survives. Skip, strict preservation, exact inventory, unchanged reuse and unchanged source bytes are inspected independently. Native drivers earn evidence separately from the shared GeoJSON reader.

## Executed baseline defects

The pre-change native binary SHA256 `61831ded55b94f278ce2557e2de51f5f2b1a6a22b47709d1435e89df6d214461` is identified in D2 evidence as source `5c4bd6d`; vector files have no diff from that commit to baseline `0cdbb72`. Oversized polygon plus valid point produced accepted identity `"valid"` only, yet retained `fragmentedPolygons=1` and a success-like partition diagnostic for skipped `"oversize"`. The independent oracle fails the counter assertion. No general orphan-content leak was observed.

The earlier native candidate SHA256 `326924d88448042320db335d573f1c85d5412b8342f21e9a6305452aadcb5532` passed the original narrower matrix. The expanded unsupported-type fixture then executed strict-mode publication of malformed geometry. That narrower pass is invalidated as comprehensive acceptance evidence. Raw reproduction is retained at `/tmp/rt119-independent/native-malformed-before`. The shared GeoJSON reader is consequently required in both builds.

## Frozen source execution and provenance

[portable-final.json](portable-final.json) records the expanded portable replay for frozen source `12ce4c6b93625f309877a249a5406aae5a1d4e7c`, immutable binary hash, exact build command/log hash, all case observations and resource measurements. The final lint-clean portable replay passed 27 source-derived cases and five resource runs; compiler paths match the frozen source. The final native replay passed 30 cases, including the separately constructed GeoPackage driver fixture, and five resource runs. [native-final.json](native-final.json) records its immutable binary/build provenance and measurements. Both records require an empty compiler-path diff against the frozen source. The earlier `3b39c310` runs are retained locally as provisional historical observations. Earlier candidate and `1445c503` executions are historical observations, not evidence for this freeze. Raw sources, archives and commands remain outside the repository; the source-derived fixture script recreates them.

The resource runner varies feature count separately from maximum individual geometry size. It uses the existing small C `wait4` launcher to avoid inherited Python fixture-generation RSS. Scratch totals are sampled every >=2ms and can miss short peaks. Runs must clean their scratch directories. These measurements do not establish an asymptotic bound or constant memory.

## Boundaries

These controls establish feature acceptance ownership and final resource inventory; topology, CRS, repair and LOD algorithms remain separately gated. Private SQL rows and vertex ownership after partial rejection are inspected in store unit tests. The public API integration probe isolates observer-triggered CWD changes in a subprocess. Fatal fault tests must separately cover source queries, savepoint/insert/rollback/release/commit, report writes/finalization, storage/archive writes, cancellation and observers under skip-invalid, preserving infrastructure causes and existing output. No production environment fault switches are used.
