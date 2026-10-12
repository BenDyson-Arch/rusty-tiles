# F1d3 large-mesh tiler: public surface inventory and dispositions (ML0)

Status: **DRAFT** (ML0, light process), with every user question answered (2026-10-12). It
records, for every public option of the legacy `mesh-to-3tz` family and of today's
`mesh-local-to-3tz`, whether the large-mesh tiler retains, replaces or retires it, and which user
decision or coordinator ruling governs. Nothing is removed by this document; removals happen in ML10,
after ML9 accepts (the legacy command stays advertised until then).

**Base:** `origin/develop` `1b1ff9d99fe5e66b844d2c0453abdbe6ed20dd26`. Sources read: `src/main.rs`
(`MeshArgs` `:467-551`, `LocalMeshArgs` `:553-568`, `RigidPlacementArgs` to `:581`), `bindings/python/src/lib.rs`
(`mesh_local_to_3tz` `:283-296`, `mesh_to_3tz` `:526-575`), `src/tile.rs` (`MeshTo3tzOptions`
`:33-84`, `TextureFormat` `:124-129`, `DEFAULT_*` `:25-30`), `src/mesh_archive.rs`
(`MeshApproximation` `:33-40`, `MeshRequest` `:67-100`), `src/doctor.rs:358-361`. The governing
inventory is [public-surface-inventory.md](public-surface-inventory.md); the contract is
[f1d3-large-mesh-contract.md](f1d3-large-mesh-contract.md).

## User decisions applied

| Ref | Decision (date) | Effect here |
| --- | --- | --- |
| Brief (2026-10-12) | KTX2/Basis only, in process; meshopt kept; performance profiles; **no georeferenced input** | Texture-format, encoder-path, meshopt-toggle and every CRS option retired |
| D3b | `lossless` uses PNG through core glTF, the only exception to KTX2-only | `--profile lossless` |
| D4 | The name stays `mesh-local-to-3tz` / `mesh_local_to_3tz` / `mesh_to_archive` | `mesh-to-3tz` is deleted, not reclaimed; old invocations fail as unknown commands |
| D5 | Default profile `balanced` | `--profile` optional |
| D1 | Exact per-triangle identity at leaves, primitive identity on proxies | `--nodeFeatures` superseded |
| V3 | `--scratch-dir`, default the system temp directory, refusal before starting | New option |
| U1 | Small primitives vanish from coarse tiles | Documented in help and docs; no option |
| U2 | 3TZ member ceiling accepted, typed early refusal | Documented in help and docs; no option |
| U4 = (c) | Small islands vanish and small holes are filled at coarse depths; the output publishes the sets | Documented in help and docs; no option |
| O-U1 (2026-10-12) | Retire `--leaf-triangles`; `L` comes from the profile only | §3 |
| O-U2 (2026-10-12) | Retire `--root-proxy-triangles`, `--max-proxy-error-metres` and the full-detail mode; **supersedes the F1d2 contract's "keep the public FullDetail/RootProxy request" note** | §3 |
| O-U3 (2026-10-12) | `--jobs` defaults to auto: `max(1, min(cores, available memory ÷ per-job need))`; the report states the value and why | §4 |
| O-U4 (coordinator) | Python `validate` gains `max_volume`, for CLI/Python parity | §4 |

## 1. Operations

| Rust / CLI / Python | Disposition | Gate |
| --- | --- | --- |
| `mesh_to_archive`, `MeshRequest`, `MeshReport`, `MeshResult`, placement types / `mesh-local-to-3tz` / `mesh_local_to_3tz` | **Retained and extended** into F1d3 (profiles, scratch, jobs, streaming) | ML4–ML6 deliver; ML9 accepts |
| `tile::{MeshTo3tzOptions, TextureFormat, mesh_to_3tz, mesh_to_3tz_reported, DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_BYTES, DEFAULT_TILE_SIZE, DEFAULT_MAX_TEXEL_DENSITY}`, `MeshTo3tzOptions::set_source_crs` / `mesh-to-3tz` (alias `meshTo3tz`) / `mesh_to_3tz` | **Retired.** Advertised until ML9 accepts; ML10 deletes the subcommand, alias, Python function and Rust exports together, with no wrapper (as `convert-to-implicit` was) | ML10 |
| `doctor` `mesh-to-3tz` entry (general-CRS note, external `basisu`) | **Retired** with the command. A `mesh-local-to-3tz` entry reports the in-process encoder's readiness | ML10 |
| Legacy implementation modules behind the command (`tile`, `hlod`, `texture`, `mesh_crs`, `lossless`, the `mesh` IR, the CRS parts of `georef`, `glb_write`) | **Remove when their last caller goes**, module by module, under the existing inventory's gates. `metadata`, `implicit` and parts of `glb_write` have point and vector callers and stay | ML10 |

## 2. Legacy `mesh-to-3tz` options

| CLI | Python kwarg | Rust field | Disposition | Replacement or reason |
| --- | --- | --- | --- | --- |
| `-i/--input`, `-o/--output`, `-f/--force` | `input`, `output`, `force` | positional, `force` | **Retained** | `mesh-local-to-3tz` has the same three; `--force` is F0 `OutputPolicy::Replace` |
| `--explicit` | `explicit` | `explicit` | **Retired** | Forest layout only (P); explicit-only output breaks per-document caps |
| `--cartographicPositionDegrees` | `cartographic` | `cartographic` | **Replaced** | `--anchor` lon lat height (F1c1) |
| `--rotationDegrees` (heading, pitch, roll) | `rotation` | `rotation` | **Replaced** | `--orientation-xyzw` (ENU unit quaternion, F1c1). Users convert; the migration note gives the formula |
| `--textureFormat {jpeg, webp, uastc, lossless}` (CLI default `jpeg`; Python and Rust default `lossless`) | `texture_format` (no `uastc`) | `texture_format` | **Retired** | `--profile`. `lossless` → `--profile lossless` (PNG); `uastc` → `--profile quality` (UASTC for every class); `jpeg`, `webp` → no equivalent (KTX2-only); `balanced` is the default. The three adapters' differing defaults disappear with the option |
| `--basisu PATH` | — | `basisu` | **Retired** | Vendored in-process encoder (ruling 6) |
| `--maxTriangles` (20,000) | `max_triangles` | `max_triangles` | **Retired** | Profile `L` |
| `--maxBytes` (204,800) | `max_bytes` | `max_bytes` | **Retired** | Small-model wrapping belongs to W1 `glb-to-3tz`; tiles are bounded by `B_tile` |
| `--tileSize` (2048) | `tile_size` | `tile_size` | **Retired** | Profile `A` |
| `--maxTexelDensity` (must be 0) | — | `max_texel_density` | **Retired** | Leaves are always full resolution |
| `--sourceCrs`, `--sourceAxes`, `--heightOffset`, `--sourceOffset`, `--sourceOffsetFile` | `source_crs`, `source_axes`, `height_offset`, `source_offset` | `source_crs`, `source_crs_definition`, `source_axes`, `height_offset`, `source_offset`, `set_source_crs` | **Retired** | No georeferenced input (user). Users bring the mesh into local metres first and place it with `--anchor` |
| `--noMeshopt` | `meshopt` | `meshopt` | **Retired** | meshopt always on (user: "meshopt is kept") |
| `--nodeFeatures` | `node_features` | `node_features` | **Retired** | Always-on F1c2 identity: node, mesh, primitive and triangle at leaves (with the node name), primitive on proxies |
| — | `callback` | — | **Retained** | F0 observer, as in `mesh_local_to_3tz` |

## 3. Today's `mesh-local-to-3tz` options

| CLI | Python | Rust | Disposition |
| --- | --- | --- | --- |
| `-i`, `-o`, `-f` | `input`, `output`, `force` | `MeshRequest::local_gltf`, `with_policy` | **Retained** |
| `--anchor`, `--orientation-xyzw`, `--scene-offset` | same names | `with_placement(MeshPlacement)` | **Retained** |
| `--leaf-triangles` (required) | `leaf_triangles` (required) | third argument of `local_gltf` | **Retired (O-U1).** `L` comes from the profile only; `local_gltf(input, output)` loses its third argument |
| `--root-proxy-triangles`, `--max-proxy-error-metres` | same names | `MeshApproximation::RootProxy`, `with_approximation` | **Retired (O-U2).** F1d3 always builds the certified multi-level hierarchy, which subsumes F1d1/F1d2's single root proxy. This supersedes the F1d2 contract's note that the FullDetail/RootProxy request and its CLI/Python arguments are kept |
| (default) | (default) | `MeshApproximation::FullDetail`, `MeshApproximationReport` | **Retired (O-U2).** There is no LOD-less output; `MeshCertificateReport` is replaced by the F1d3 report (umbrella §13) |

## 4. New surface

| CLI | Python | Rust | Notes |
| --- | --- | --- | --- |
| `--profile {fast, balanced, quality, lossless}` | `profile="balanced"` | `MeshRequest::with_profile(MeshProfile)` | D5 default; unknown values are InvalidRequest |
| `--scratch-dir PATH` | `scratch_dir=None` | `with_scratch_dir` | V3; default the system temp directory; the report says when scratch is memory-backed |
| `--jobs N` (default `auto`) | `jobs=None` (auto) | `with_jobs(Jobs::Auto \| Jobs::Fixed(n))` | **O-U3:** auto = `max(1, min(cores, ⌊available memory ÷ per-job need⌋))`, where the per-job need is the profile's `jobs ×` term of the memory bound (umbrella §9; about 0.6–1.3 GiB) and available memory is read once at start. An explicit `N ≥ 1` is used as given. The report states the chosen value, the cores and memory it saw and which limit decided. Output bytes do not depend on it (umbrella §11) |
| (report) | (report) | `MeshReport` | Profile and constants, per-level counts, seam and certificate statistics, `appearance`, preflight stage fields, vanished primitives (contract §13) |
| `validate --max-volume KEY=N` (repeatable) | `validate(path, max_volume={"KEY": N})` (O-U4) | the validation request takes the same keys; `ValidationLimits` volume and budget fields become `Option<u64>`, serialized as `null` when uncapped (O-C1-4) | A-C1-4 and A-C2-1 keys. The I1 subtree cap stays internal (O-I1-1). Rust `ValidationReport::payloads` becomes `PayloadRows` (ML7a); Python `report["payloads"]` becomes a lazy sequence |

**Help and docs text (U1, U2).** The command help and `docs/` say: small primitives may vanish
from coarse levels, never from leaves, and the vanished geometry is inside the published
`geometricError`; an archive holds at most 178,956,970 members (about 1.7×10^12 triangles at
`balanced`), and larger inputs are refused before work starts; at coarse levels small islands
may vanish and small holes may be filled (U4), and each coarse tile lists what it removed or
filled (umbrella §6.7).

## 5. Tile content formats: glTF only (USER decision 2026-10-12, decided)

| Surface | Disposition |
| --- | --- |
| Supported tile content | **glTF 2.0 only:** GLB, plus archive-local `.gltf` |
| b3dm, i3dm, pnts, cmpt | **Dropped as legacy.** `validate` reports such content as Unsupported (C1 amendment A-C1-6). This is a breaking change and the CHANGELOG notes it |
| b3dm table and `RTC_CENTER` paths | Removed from the C1 profile; no ML amendment and no ML slice relies on them |
| Vector fill output (today written as b3dm, `vector/pipeline/encoding.rs:939`) | Moves to plain glTF. A spike first settles how fills ship as plain glTF in Cesium |
| Implementation | A **separate glTF-only slice**, after that spike. Not part of ML0 or any ML slice |

## 6. Output and compatibility changes

- Textures are always re-encoded (KTX2, or PNG in `lossless`). F1b1's source-faithful forwarding
  does not apply to this command.
- Output is always a multi-level certified hierarchy in the bounded forest.
- `validate` admits `KHR_texture_basisu` (A-C1-1); before ML1 item 4, textured large-mesh output
  is Unsupported by `validate`.
- **Reuse identity:** earlier vector archives stop serving as `--reuse-tileset` baselines at ML1
  (`Cargo.lock`, `vector_encoding/*`) and again at ML3 (`implicit*`). Release notes say "re-run
  the build".
- **Rust API breaks:** `local_gltf`'s signature (O-U1); `MeshApproximation`,
  `with_approximation`, `MeshApproximationReport` and `MeshCertificateReport` (O-U2);
  `ValidationLimits` field types (O-C1-4); `ValidationReport::payloads`.
- **Python breaks:** `mesh_local_to_3tz` loses `leaf_triangles`, `root_proxy_triangles` and
  `max_proxy_error_metres`, and gains `profile`, `scratch_dir` and `jobs`; `validate` gains
  `max_volume`.
- **Docs to update** (ML9 and ML10): `README.md`, `docs/CLI.md`, `docs/CONVERTERS.md`,
  `docs/DEMOS.md`, `docs/FORMATS.md`, `docs/INSTALL.md`, `docs/VALIDATION.md`, `CHANGELOG.md`, and
  the `tile` and `mesh_archive` rows of `public-surface-inventory.md`.

## Open questions

None for the user. The coordinator schedules the C1 meshopt-header defect fix (C1 amendment
A-C1-3) as its own small PR.
