# Legacy mesh retirement ledger

Status: **preparation done, deletion not authorized.** `mesh-to-3tz` /
`mesh_to_3tz` and `mesh-local-to-3tz` / `mesh_local_to_3tz` / `mesh_to_archive`
are deprecated. Removing them is a 0.4.0 gate (owner decision 2026-10-21, in
the [roadmap](04-tiles2-roadmap.md)). The deletion PR may land only after
the gate conditions below hold.

This ledger replaces the static census frozen at `df3f7a5f` (H21 plan,
2026-10-13). It was re-run on develop `0b0a59ae` after #218 (declared source
CRS), H17/H27/H28 and #246. The census is a static reading of imports, calls,
`include_str!` edges, scripts and workflows; it is not a compiler reachability
proof. Re-check the "retained callers" column on the final source before
deleting.

## What the preparation PR changed

- **Shared CRS Jacobians extracted.** #218 made the `mesh` tiler's source-CRS
  capture (`src/mesh_large/source_crs.rs`) call `mesh_crs::{probe_step,
  horizontal_step, vertex_jacobian, to_y_up, inverse_transpose}`. Those helpers,
  and the private `derivatives`, `samples` and `one_sided` they need, now live in
  `src/crs_jacobian.rs`. `src/mesh_crs.rs` keeps only the legacy `bake` and
  `normal`, so it can be deleted whole. The analytic Mercator, Albers
  domain-edge, geographic pole, native epoch longitude-wrap and foot-unit
  checks moved with them: they now go through `vertex_jacobian` rather than the
  legacy bake.
- **Vector reuse fingerprint decoupled.** `src/vector/pipeline/reuse.rs` no
  longer hashes `glb_write.rs`, so deleting that file does not change vector
  reuse identity. The fingerprint hashes whole files, though, and still
  covers `metadata.rs`, `bbox.rs` and `georef.rs`, which the deletion PR edits
  (see its checklist), so that PR invalidates reuse caches a second time. The
  list now covers `src/vector.rs`,
  `vector/pipeline/outline.rs`, `vec3.rs`, `vector_compression/source.rs`,
  `content_integrity/payload{,/ktx2,/numbers,/records,/scene}.rs`,
  `implicit/tileset/stream.rs` and (native) `geospatial/native.rs`. The first
  three close the 2026-10-21 audit finding. A test keeps the list closed over
  `mod` declarations and `crate::` paths. `bench/benchmark_vector.py` reads the
  same list. This PR invalidates every existing vector reuse cache once; the
  changelog says so.
- **Test fixtures decoupled from `glb_write`.** `tests/support::write_glb`
  reproduces the legacy writer's bytes, and the committed `glb-to-3tz`,
  `createTilesetJson` and `convert` digests are unchanged. It is used by
  `tests/support/mod.rs` (`textured_grid_glb`), `tests/implicit_subtree.rs` and
  `tests/demodata_suite.rs`. The committed digests guard only the textured,
  16-bit-index path, so `tests/glb_writer_oracle.rs` also writes 32-bit-index
  and no-normals cases with both writers, asserts identical bytes and pins
  their sha256.
- **Producers migrated to `mesh`.** The digest/CLI-contract recipes (one `mesh`
  lossless recipe replaces `mesh-webp` and `mesh-lossless-nomeshopt`), release
  binary smoke, `scripts/release_acceptance.sh` (README route), the
  `preview_layers.py` mesh layer, `benchmark_runtime.py`, the perf lane `mesh`
  case (`mesh-local` and `mesh-explicit` dropped), scale-gate examples, doctor
  tests that used `mesh-to-3tz` only as a sample command, the `validate` hint,
  and the README, FORMATS and CONTRIBUTING examples. The container smoke already
  used `mesh`.
- **Deprecation.** Both commands print a stderr warning on every run (an NDJSON
  `warning` event with `"code":"deprecated"` under `--progress json`), and
  `--help` says "Deprecated". The Python docstrings say so too. Python does not
  raise a runtime `DeprecationWarning`.

## Gate conditions still open

1. Complete tiler acceptance (roadmap milestone 4): the spilled
   5.26×10^8-triangle, 10^4-primitive/hole/island, procedural-bake and
   2^22-tile / over-4-GiB scale gates are measured against declared budgets.
   The final output auditor and independent validator run on every profile,
   with consumer refinement and picking. Validator scale (VS-B/VS-C/VS-D) is
   closed.
2. Applicable platform CI, installed-wheel and official-Blender acceptance on
   the accepted sources, and a fresh nonauthor review.
3. The migrations this PR did not do (next section).
4. A re-run of this census on the final source. Nothing new may call a
   "goes" symbol.

## Live dependencies not migrated here

| Dependency | Exact current users | Why not migrated, and what the deletion needs |
| --- | --- | --- |
| Demo corpus | `bench/demodata_manifest.json`: six `mesh-to-3tz` cases (`mesh-milktruck`, `flight-helmet-external`, `mesh-lantern`, `mesh-corset`, `mesh-floreat`, `mesh-delft`); `bench/demodata/runner.rs` command allowlist; `bench/demodata/audit.rs::mesh` (`rusty_tiles::mesh::load`); `tests/demodata_suite.rs::mesh_audit_rejects_changed_winding_even_when_archive_validation_passes` | The exact-float triangle audit cannot apply to the tiler, whose positions are `KHR_mesh_quantization` codes. The expectations (for example the FlightHelmet refusal) must be re-pinned from a real corpus run, which needs the download. Needs a tiler audit (provenance or quantization-bounded) and new expectations. |
| CI F1 oracle drivers (active independent references) | `.github/workflows/ci.yml` runs `tests/f1a_oracle.py`, `f1b_oracle.py`, `f1b2_oracle.py`, `f1b3_oracle.py`, `f1c1_oracle.py`, `f1c2_oracle.py`, `f1d2_oracle.py` (with `f1d2_certificate.py`, `f1d1_oracle.py`), `f1b3_khronos.py`, `f1c2_khronos.py`, `f1d1_khronos.py` and the viewers `f1a/f1b/f1b3/f1c1/f1c2/f1d1_viewer.py`; `f1b/f1b2/f1c1_wheel.py` are wheel drivers | Each runs `mesh-local-to-3tz` as its producer. Keep the independent decoding, source, placement and image reference logic; retarget or retire each executed route deliberately. Import chain: f1d2 → f1d1 → f1c2; f1c1/f1b3 → f1a/f1b/f1b2; f1b2 → f1b → f1a. |
| Validator corpora (active independent references) | `bench/architecture_audit/c2_decoded_containment/corpus/run.py` (`legacy-mesh-explicit`, `legacy-mesh-*implicit`, `mesh-local-to-3tz` families); `bench/architecture_audit/implicit_read/acceptance/scripts/produce.py` (W6, W6d, W7); `tests/native_implicit.rs::deep_implicit_mesh_validates_and_corrupt_subtrees_roots_and_templates_are_rejected` (needs the legacy mixed explicit/implicit forest) | They use legacy outputs as validator fixtures, with pinned verdicts. Replace each with a committed or generated witness of the same shape plus an explicit amendment, never a silent producer swap. |
| Legacy parity test | `src/mesh_large/source_crs_tests.rs::legacy_general_and_web_mercator_bakes_agree_with_the_capture_frame` (`tile::MeshTo3tzOptions`, `mesh::{Scene, Vertex, Triangle, BakeToEnu, bake_to_enu}`, `mesh_crs::bake`) | New since the frozen census (#218). It compares against the legacy bake; delete it with the bake. The capture oracle tests keep the tiler side. |
| Historical reproductions (keep) | `bench/scale_gates/validation_pass.sh` mesh-local cases and the `mesh-local`/`mesh-local-ceiling` budgets; `bench/perf_lane/REPORT.md`; `bench/*_results.json`; `bench/architecture_audit/f1*/resource_probe.py`, `mesh_approximation/placed_proxy_browser.py`; receipts and manifests under `bench/architecture_audit/**` | They record past runs. Leave them as history; do not recast them as acceptance. |

## Goes in the deletion PR

Disclosure: this PR edits `src/metadata.rs`, `src/bbox.rs` and `src/georef.rs`
(below), which the vector reuse fingerprint hashes whole. It therefore
invalidates existing `--reuse-tileset` vector caches a second time (the first
is the retirement-prep PR). State that in its changelog. Deleting `glb_write.rs`
alone does not.

Writer oracle: delete the `legacy_write_glb` comparison in
`tests/glb_writer_oracle.rs` with `glb_write.rs`, but keep its pinned sha256
assertions, so the `tests/support` writer's 32-bit-index and no-normals bytes
stay guarded.

Rust modules, whole: `src/tile.rs`, `src/mesh.rs`, `src/mesh_crs.rs`,
`src/texture.rs`, `src/hlod.rs`, `src/grid.rs`, `src/jpeg.rs`,
`src/gpu_texture.rs`, `src/lossless.rs`, `src/glb_write.rs`, `src/tileset.rs`,
`src/tileset/resources.rs`, `src/mesh_archive/encode.rs`,
`src/mesh_archive/encode/{identity,proxy_identity}.rs`,
`src/mesh_archive/approximation.rs` and
`src/mesh_archive/approximation/certificate.rs`.

Rust symbols, partial:

- `src/mesh_archive.rs`: `MeshApproximation`, `MeshCertificateReport`,
  `MeshApproximationReport`, `MeshRequest`, `MeshReport`, `MeshResult`,
  `RootProxyLimits`, `ValidatedApproximation`, `PreparedMesh`, `PreparedRoot`,
  `CompletedMesh`, `validate`, `prepare`, `produce`,
  `mesh_to_archive[_with_operations]`, the old producer stages and their tests.
- `src/mesh_archive/partition.rs`: `Leaf`, `plan`, `split`, `centroid` and
  their tests. Keep `Bounds`.
- `src/output.rs`: `Job`, `check_output`, `parent_dir`, `tree_members`,
  `check_members`, `temp_archive` and the `Job` tests.
- `src/report.rs`: `Reporter`, `Event`, `EventSink`, `ndjson` and
  `ConversionResult`, once `main.rs`'s progress transport and the Python
  bridge move to `RunEvent`. This is a public API removal; record it.
- `src/metadata.rs::attach_node_features`.
- `src/error.rs`: `TextureProjection`, `NoContent` and `InputNotFound`, plus
  the Python `InputNotFound` match arm.
- `src/georef.rs`: the legacy-only helpers `CrsKind`, `SourceCrs`,
  `EnuFrame::{normal_to_enu_yup[_offset], geog_yup_to_enu_yup,
  mercator_yup[_offset]_to_enu_yup}`, `mercator_to_geodetic`, `looks_*`,
  `geographic_*`, `mercator_*`, `apply_mercator_offset`,
  `parse_metashape_offset` and `geog_yup_to_enu_yup`, after a public-API
  decision.
- `src/fixtures.rs::geographic_glb` (no caller outside legacy tests).
- `src/bbox.rs` glTF-import reader: `bounding_box_from_gltf[_path]` and
  `read_gltf_document`, only after `tests/golden.rs` is dispositioned.
- `src/lib.rs`: the `glb_write`, `grid`, `hlod`, `mesh`, `texture` and `tile`
  modules; the `mesh_crs`, `jpeg`, `gpu_texture`, `lossless` and `tileset`
  declarations; the `tile::*` and legacy `mesh_archive::*` re-exports; and the
  `report` re-exports per the item above.

CLI (`src/main.rs`): `Command::{MeshTo3tz, MeshLocalTo3tz}`, the `meshTo3tz`
alias, `MeshArgs`, `LocalMeshArgs`, `PlacementArgs`, `mesh_opts`,
`legacy_mesh_placement`, `Outcome::Mesh`, `mesh_summary`,
`Command::deprecation`, and the unit tests
`local_mesh_requires_explicit_limit_and_rejects_legacy_options`,
`root_proxy_arguments_are_one_explicit_choice` plus the legacy rows of the
alias tests. Keep `RigidPlacementArgs`, `IoArgs` and the JSON/progress
transport. Old command names must fail with clap's unknown-subcommand error.
Pin that in place of `tests/cli_contract.rs::legacy_mesh_commands_are_deprecated_in_help_and_warn_on_stderr`.

Doctor (`src/doctor.rs`): the `MeshTo3tz` and `MeshLocalTo3tz` variants, their
`ALL` entries, the alias, readiness details and unit-test rows, and
`tests/native_doctor.rs`'s `ALL.len()` expectation (automatic).

Python (`bindings/python/src/lib.rs`): `mesh_to_3tz`, `mesh_local_to_3tz`,
`MeshResult`, `placement`, `run_conversion`, `CallbackSink`, the
`ConversionResult` class and the legacy imports. Keep `rigid_placement` and
the typed observer/error machinery. Tests to remove from
`bindings/python/tests/test_api.py`: `test_certified_root_proxy_installed_api`,
`test_adaptive_certificate_installed_api`,
`test_source_identity_metadata_independent_installed_api`,
`test_explicit_mesh_placement_independent_world_contract`,
`test_local_resource_mesh_independent_profile_oracle`,
`test_local_external_mesh_snapshot_and_typed_overlap`,
`test_local_textured_mesh_independent_profile_oracle`,
`test_core_pbr_mesh_independent_corner_and_resource_oracle`,
`test_local_textured_mesh_forwards_one_prepared_image`, the five
`test_local_mesh_*` tests, `test_mesh_events_and_report`,
`test_general_mesh_crs_requires_axes_and_height_and_places_output`,
`test_mesh_node_features_are_available_from_python`,
`test_callback_exception_is_preserved` and
`test_concurrent_calls_and_callback_reentry`, plus the three `mesh_to_3tz`
lines of `test_typed_errors_and_failed_publication`. The tiler and point tests
already cover typed callbacks, exceptions and concurrency.

Rust integration tests: `tests/mesh_tile.rs`, `mesh_crs.rs`, `fidelity.rs`,
`demo_mesh.rs`, `mesh_archive.rs`, `mesh_local_frontends.rs` and
`f1b_lifecycle.rs`; the `small_mesh` cases of `gltf_resources.rs`;
`shared_metadata.rs::node_features_survive_instancing_partition_and_parent_lods`;
and `native_implicit.rs::implicit_mesh_accepts_case_insensitive_extensions`.
Before deleting a test, move any helper it shares with a retained oracle.

Build and packaging: the `native-jpeg` feature, the `build.rs` probe and link
step, `RUSTY_TILES_DISABLE_NATIVE_JPEG` in the `Dockerfile` and in the
ci/release/wheel workflows, and the `--features native-jpeg` CI legs. Do this
without reducing their portable, native-geospatial, validator or H1 coverage.
`jpeg-decoder` and the `image` WebP feature become candidates once the census
shows no other users. Keep PNG/JPEG source decoding, `basisu_ktx2` and the
vendor notices.

Docs: the CLI.md `mesh-to-3tz` and `mesh-local-to-3tz` sections, the FORMATS
and CONVERTERS legacy sections, the README deprecation notes and the legacy
section of the Python README. Rewrite the MESH.md migration section in the past
tense. Mark the F1 contracts historical and update the public-surface
inventory. Add a changelog fragment for the removal, including the deliberate
loss of automatic CRS detection and the legacy output selectors.

## Retained, with current callers

| Retained | Callers that keep it |
| --- | --- |
| `src/crs_jacobian.rs` | `mesh_large/source_crs.rs` (capture lattice and per-vertex Jacobians) |
| `crs::Transform` incl. `polygon_units` | `crs_jacobian::horizontal_step`, `mesh_large/source_crs.rs`, `vector/portable.rs`, point cloud |
| `georef::{Cartographic, geodetic_to_ecef, ecef_to_cartographic, root_transform, RotationDegrees, EnuFrame::{new, to_enu}, SourceAxes (+ coordinates), SourceOffset}` | `mesh_large/source_crs.rs` and the Python `mesh_to_tileset` (`SourceAxes`, both new since the frozen census); `vector/model.rs`; `vector/pipeline/{source_native,store}.rs`; `vector/portable.rs`; `terrain.rs` |
| `mesh_archive::{model, placement}`, `Workspace`, `GeneratedMember`, `ProducerOperations`, `ProducerStage`, `SystemProducer`, `write_member`, `cleanup_failure`, `codec_error`, `partition::Bounds` | W1 `glb-to-3tz` / `createTilesetJson` (`mesh_archive/model.rs`); `MeshPlacement`/`ResolvedPlacement` also in `mesh_large/{delivery,pipeline,full,source_crs}.rs` |
| `output::write_report` | `vector/pipeline/store.rs` |
| `src/glb.rs` | `mesh_large/leaves/writer.rs`, `terrain/glb.rs`, `vector/pipeline/encoding.rs`, `metadata.rs`, `fixtures.rs`, validator tests |
| `bbox::{BoundingBox, Obb, jacobi_eigenvectors}` | `vector/pipeline/geometry.rs`, `implicit/tileset.rs`, `vec3.rs` tests |
| `fixtures::triangle_glb` | `tests/model_wrapping.rs`, `golden.rs`, `gltf_resources.rs` |
| `error::Error::{MissingTilesetJson, OutputExists, Io, Job, Validation}` | `archive3tz` codec adapter, `vector.rs`, Python error mapping |
| `metadata` public types, `encode_property_table`, `MetadataGlb` | point tiles, vector encoding, the public API |
| `gltf_source/**` | W1 and the `mesh` tiler |
| `tests/support::write_glb` | digest/CLI-contract inputs, `implicit_subtree.rs`, `demodata_suite.rs` |
| `preview --mesh` | serves tiler and W1 output; it is not an exporter |
