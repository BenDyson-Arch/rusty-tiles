This is an additive correction to the frozen P2 domain oracle. The original package, its fixtures, expectations, passed phase/frame results and failed domain result remain unchanged. C1 is now accepted and merged at `f5401dd120441b4f9aac5ad229dbfffe03fe4428`; compression P2–P5 remain HELD.

The original selector raises `StopIteration` for `polygon-loops-offsets`: that case has `featureTable`, `indices` and `polygonOffsetAccessor`, with none of the original seven selected association keys. Appending `featureTable` fixes that case without changing other selections. The original `check(case, dir)` source is retained byte-for-byte. The revised self-test requires actual rejection by that same checker, with the expected assertion message, for each changed binary and association. It checks the copied baseline and the restored baseline as well. Controls use ephemeral copies and never mutate the original files. All 15 case inputs and every original fixture SHA are retained explicitly in `external-inputs.json` and the revision integrity file.

`expected-selftest-records.json` records 15 baseline/restored admissions and 33 faulty-control rejections predicted from literal frozen inputs. These are expectations, not executed results. Each binary control flips only bit 0 of its first byte; each association control changes its selected literal. The exact changed-file hashes are frozen. Root must run the revised checker before any result is described as passing. This exercises oracle sensitivity only; it creates no purported codec output and provides no producer, native codec, extractor, consumer, allocation or conformance evidence.

Root-only command, using a new output directory:

```bash
python3 /tmp/rusty-tiles-next-vector-compression-audit/codec/p2-domain-oracle-revision-20261010/coordinator_adapter.py --lane domain --c1-gate /tmp/rusty-tiles-codec-p2-executions/accepted-c1-gate.json --out-dir /tmp/rusty-tiles-codec-p2-executions/domain-revision-20261010
```

Use `--lane models` and a different fresh directory only if a distinct full three-model receipt is wanted. The phase/frame scripts are reused unchanged, with the original root-only coordinator's execution helper (nice 10, at most two allowed CPUs, worker counts two, timeout 60 seconds). The adapter checks all frozen original package inputs and this revision before execution, requires the accepted merge and JSON-owner identity, compares the full actual domain record to frozen expectations, and rechecks both packages afterward. It writes failure or success evidence in the new output directory; the original failed receipt remains intact.

The author has only performed static syntax/source-text, pin and mutation arithmetic checks. No revised oracle, adapter, model, target, native probe or build has been executed by the author. `revision.json` records the precise delta, source pins and retained failure lineage.
