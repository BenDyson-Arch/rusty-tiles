# PR B evidence

Draft: https://github.com/BenDyson-Arch/rusty-tiles/pull/312.
Tested implementation: `ca963b780688b4c00660a7ce595e9ed470188eb2`, on
PR A `5c319c9b8b9bbf4b3073dc365bcc7a1daee9587d`.

Bulk evidence was never tracked. It is retained at
`/home/bend/.cache/rusty-tiles-evidence/pr-b-20261102/`:

- `original-ca963b7/FINAL.md`: SHA-256 `90926e85bbb97b8a598212d8c0608f04b3e02e56f5f5a26c46fc8eb3ade7d39d`.
- `original-ca963b7/MANIFEST.sha256`: SHA-256 `96bcd681bc661b91066cc4ebafc111808eacd383f5348438e67683d4a0e54f01`.

The report covers guarded D10 support admission, per-tile triangles/G,
source-floor/achieved appearance, exact PR A leaf comparison, 200-city
comparison, crop3 S5, native archives, three slot-1 CPU pairs, and resource
limits. The manifest binds final logs, scripts, artifacts and references.
No full Walem bake or floor result is claimed. A/R integration and separate
nonauthor review remain open; this evidence does not authorize promotion.

## PR A merge follow-up

Merged source: `6d224ccd5934da26aa40afbe40dc35045b811b0a` (parents `9b848fed` and `b5ef178f`).
Current validation is in the evidence directory's `merge-a-b5ef178f/`:

- `REPORT.md`: SHA-256 `6489e7c88db5508bbf3e662140346def31b8543e1e8c3f65a48e7c118bfe788f`.
- `MANIFEST.sha256`: SHA-256 `5f67cb65c279cf3255a0fb99cdb32761d3b446c0c30b99d0c73056bc989b5d63`.

The requested digest/library/integration/clippy/fmt checks pass; the digest
fixture was not regenerated. Prior crop/S5/CPU results above retain their
original source identity. The Sol review of `9b848fed` predates this merge.

## Develop merge follow-up

Merged source: `40c8308c5c8e243e371d6fd5d9d4434cd7d1c7ad`, incorporating develop `8e010ad9`.
Validation is in the evidence directory's `merge-develop-8e010ad9/`:

- `REPORT.md`: SHA-256 `0abc8136648fdae38a7e515f178d886774f32ac765ddcde07579d6f56c3a333a`.
- `MANIFEST.sha256`: SHA-256 `1067d2a97904c03d7c0f5481f56900f818825006dd7e3be77989e0d79c18b9cf`.

All requested checks were rerun and passed without regenerating the digest
fixture. The HLOD tree is unchanged from `6d224ccd`; prior corpus and timing
measurements retain their original source identities. PR base: `develop`.

## Develop cf43aef8 and 4 GiB follow-up

Tested implementation: `ff03b78165dfc4e6c02a5834fd6e3ee62df9a127`. Develop `cf43aef8` was merged
without rewriting history, including PR A's charges/digests, #316/#317 and
#318's 1% root competition. A sensitive negative control protects selected and
collapsed roots from D10. Chart/page workspace is admitted on demand, with
actual-size pages, protected encoder headroom and bounded waits. No retry,
quality, source-error or topology rule changed.

Current evidence at the directory above:

- `final-f17f9cb7/FINAL.md`: SHA-256 `d6f7a5cce57bb9bf73790291c7ce98be1fb4c4accef3921a9ea23a244fbee3e8`.
- `final-f17f9cb7/MANIFEST.sha256`: SHA-256 `df2b42c60a4bc58e8087a0d394aebcf2c4e385936f9e0ca39b6189d8a84e08e3`.

Three alternating slot-1 pairs per fixture at full 4 GiB / three workers:
mean CPU −1.63% on tex3 and +0.53% on the Walem crop, with substantial paired
variation. Mean wall time remains +8.63% / +7.69%; elapsed-time parity is not
claimed. All pairs and prior unsuccessful sources are retained. All 12 archives
validate, all candidate bytes equal the accepted B output, and all 156 tex3 /
103 Walem leaves equal develop. Coarse geometry and appearance intentionally
differ from develop; per-tile/depth triangles, G, source floors and cap reporting
are in the current report.

Final-source lib (1,254 passed / 25 ignored), --tests including S5 acceptance
and unchanged output digests, workspace/all-target clippy, fmt, tree and changelog
checks pass. The clean working-tree release binary equals the measured binary;
target-pr-b was cargo-cleaned. Earlier full city/crop3 S5 evidence retains its
original identity. No full Walem bake or floor result is claimed. Fresh Sol review
and R integration remain pending; CI was not polled. The PR remains a draft.

## Relief merge and r2 decision report (2026-11-03)

Local tested implementation: `0fd2f65cd4cbf6a7d5ff8daf4f08be08dabe0f4f`, merging
`f1c69172d573aa3b3cf35721fc877d644ea3eb62` without rewriting history (#307/#319).
This update is unpushed pending the owner's requested output-change report.

- `FINAL.md`: SHA-256 `6c32726b53518db4334f4fd71f25c2e71666226cbfff9e73e8e05fdffff0717d`.
- `MANIFEST.sha256`: SHA-256 `36580a5f4666c0a0313ffb7c0c7d10d8f4367fe89310e3de6edda4c0e7934991`.

The root manifest binds the report, the `merge-relief-f1c69172/` and
`r2-root-wall-20261103/` submanifests, and preserved prior reports/manifests.
Submanifests bind source snapshots, scripts, binaries, logs, archives and raw
results; symlink text is recorded separately. Bytecode and live checkpoint/freeze
logs are excluded. Bulk evidence was never tracked.

Output digests (unchanged), lib (1265 passed/25 ignored), tests including S5,
clippy, fmt, tree and both 200-layout city sweeps pass. Separate relieved-sheet
controls admit six D10 reductions; the N3 control preserves four refusals.
No admitted combined N3-relief-handover fixture is claimed. The city has no
relieved cells and uses the reduced test S2 schedule.

The r2 root finding remains: 160/199 payloads change and 53 roots have higher G.
41 of those have no directly G-raising admission; ancestor remeasurement also
changes. Local E0 and honest child-monotone S2 pass, but do not establish
cross-run root invariance. Both tested depth cutoffs retain Walem far-view
regressions and neither is adopted. An ancestor-input preservation design or
explicit acceptance of the measured tradeoff requires an owner decision.

Three profiled 4 GiB slot-1 cycles give mean CPU/wall changes versus develop of
-7.69%/-2.28% (tex3) and 0.00%/+4.06% (Walem). Same-binary traces identify changed
ancestor work and critical-path concentration, with negligible lease waiting.
Fine-profiler-disabled controls retain the ancestor cost shift but do not prove
a fixed elapsed penalty under host variation. All 24 measured/control archives
validate; all 156 tex3 and 103 Walem leaf payloads remain exact. No unprofiled
three-pair throughput claim or full-Walem floor result is made.

The first min4 city result reused a stale executable and is explicitly excluded.
The accepted repeat forcibly compiled B and matched all 200 develop archives.
No staged retry, quality or source/seam rule changed. Target-pr-b was cleaned
(21.1 GiB). Sol r2 remains CHANGES-REQUIRED; fresh nonauthor acceptance and the
owner decision remain open. No guard, output push or release is authorized by
this evidence.
