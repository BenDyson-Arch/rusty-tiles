# P implicit layout: retained preparation

Lossless copies of the P-IMPL preparation, made on 2026-10-11 for the combined I1 + P branch
`feat/implicit-forest`. The published contract is
[docs/architecture/p-impl-layout-contract.md](../../../../docs/architecture/p-impl-layout-contract.md).
The frozen PO forest checker and controls are in the parent directory.

| Directory | Source | What it is |
| --- | --- | --- |
| [design-20261011/](design-20261011/) | `/tmp/rusty-tiles-pimpl-design-20261011` | Design v1 to v3 (`design.md` `1c84e8ec…`), change log, open decisions PD-1 to PD-13, ownership plan, `SHA256SUMS`, and the `evidence/` scripts and results cited in design §5 and §9. Nothing was dropped |
| [precode-review-20261011/](precode-review-20261011/) | `/tmp/pimpl-precode-review-20261011` | PN's precode review `review.md` (`16205c1c…`, NO-GO), its scripts, replay copy, regenerated fixtures, cross-check notes, run logs and perf JSON, and the v3 delta review `delta/delta-review.md` (`09044be8…`, GO-WITH-CHANGES) with its reruns |

Each directory has a `RETAINED.json` listing every copied file with its byte length and sha256,
and every file or directory that was not copied. Everything not listed as dropped is
byte-identical to the source.

## Dropped from precode-review-20261011, and how to regenerate it

- `cesium-unmin/Cesium.js` and `cesium-patched/Cesium.js` (15.8 MB each), with their `Assets`,
  `ThirdParty`, `Widgets` and `Workers` symlinks into a local npm cache. `cesium-unmin` is the
  unmodified `cesium` 1.146.0 `Build/CesiumUnminified/Cesium.js` (sha256 `f439e176…`, recorded
  in `RETAINED.json`). The ablation build is that file with
  [cesium-ablation.diff](precode-review-20261011/cesium-ablation.diff) applied: the child-subtree
  placeholder takes the implicit root volume instead of the derived cell.
- `models/{pc-deep-s4,today-pc-deep,today-vec-perf,vec-perf-s4,vec-perf-s6,vec-perf-s8,vec-perf-s12}/`
  (8,413 files, 135 MB): servable forest and today-layout models. They are regenerated from
  the I1 W samples (see `../../implicit_read/preparation/README.md`) by
  `evidence-copy/relayout_forest.py` and the design's `evidence/v2/forest_model.py`. The review
  records that they regenerate byte-identically. Their summaries and view files
  (`models/*.json`) are retained.
- `runs/perf/work-*/` (7,830 files, 100 MB): perf-lane scratch directories written by
  `scripts/abba.sh` through the lane's `cesium_cpus.py --work`. The timed results
  (`runs/perf/*.json`, `*.log`, `*.cpuload`, `*.cpu0`) are retained.
