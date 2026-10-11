# C2 decoded-content containment contract (draft)

Status: decision-ready preparation, not an implementation acceptance. No production
source was edited and no Cargo/rustc/native/CLI command ran in this lane. Observed source:
`/tmp/rusty-tiles-r2-foundation` HEAD `70d184fb6c98ac5a9853176155f5790149af3a4b`
(R2 PR #155 branch, still OPEN, not in `develop`; another lane has uncommitted edits to
`src/raster.rs` and `docs/architecture/r2-raster-contract.md` only). `origin/develop` is
`6c5ea2acac9fc403bfabfa9581464223b52259d7`. File hashes for every cited source are in
[verify/source-pins.txt](verify/source-pins.txt). The implementation baseline must be freshly
pinned after R2's accepted merge. Line numbers refer to the observed HEAD.

This draft builds on the two earlier preparation packages,
`/tmp/rusty-tiles-c2-domain-preparation-20261011` and
`/tmp/rusty-tiles-c2-math-resources-preparation-20261011`. It was checked against them rather
than inherited. The section *Corrections to the prior preparation* lists where it departs from
them and why.

## Claim

The proposed check is named `decodedContentContainment`. It replaces the existing
`decodedContentBounds` entry in `notInspected` (`src/validate.rs:892`), and only when
the check is proven for the whole archive.

When the check is listed in `checks`, the archive satisfies all of the following:

- Every content reference to a tile payload was decoded from the actual archive
  bytes. "Tile payload" means a content URI that is not a `.json` tileset.
- Each decoded position was taken in its selected static scene, for every core
  node instance and for every drawn index occurrence.
- Each such position, read as an exact real number from its represented source
  values, lies in the closed set of each of these volumes:
  - its own tile's bounding volume;
  - its `content.boundingVolume`, if present;
  - every ancestor tile's bounding volume, including ancestors reached through
    external tilesets.
- Each test is evaluated in the volume's own tile frame.

The claim is about decoded, represented content. It does not cover:

- source fidelity or LOD/simplification error;
- rendered footprint (point size, line width, shaders, draping or clamping);
- GPU f32 dequantization;
- animation, skins or morph targets.

Because box and sphere volumes are convex, vertex containment also contains the core
segments and triangles between those vertices. It says nothing about polygon topology.

The primary text supports this as the normative check. The relevant text is in the
pinned primary copy at
`/tmp/rusty-tiles-c2-domain-preparation-20261011/primary/3d-tiles-core.adoc`:

- line 86 and line 752: "each tile has a bounding volume completely enclosing its
  content, and the content for child tiles are completely inside the parent's bounding
  volume. This does not imply that a child's bounding volume is completely inside its
  parent's bounding volume."
- line 167: `content.boundingVolume` encloses that content.

The existing `hierarchyBounds` check tests declared volume against declared volume
(`validate.rs:643-650`, `validate.rs:679-685`). That is stronger than this normative
requirement and does not substitute for it.

## Supported domain (first slice)

| Class | Disposition | Basis |
| --- | --- | --- |
| Explicit `.3tz` through the existing `inspect` operation, external tilesets included | Supported | Only archive operation (`validate.rs:752-790`); the external root transform joins the ancestor chain |
| Implicit tiling (any `implicitTiling` root) | Not inspected, reason `implicitTiling` | Effective volumes are `TILE_BOUNDING_BOX` semantic rows. Their owner is #126 A2, which is held. See open decision D3 |
| Payloads `.glb`, archive-local `.gltf`, modern b3dm v1 | Supported | Exactly C1's `payload::inspect` kinds (`validate.rs:413-423`) |
| b3dm `RTC_CENTER`, `CESIUM_RTC` | Not inspected (`rtcCenter`, `cesiumRtc`) | No producer writes either (see the inventory). JSON float32 precision is deferred (D6) |
| Scene selection | Explicit `scene`; otherwise the sole scene. No scene, or several scenes with no default: not inspected (`sceneSelection`) | Never default to scene 0. Every current producer writes `"scene":0` (`src/glb.rs:312-321`, terrain, F1c2) |
| Nodes | `matrix` (affine last row required) or TRS; strict tree; exact references; cycles and shared children are InvalidInput | glTF 2.0.1 node rules. Records are not consumed by C1 today (`payload/records.rs:40`) |
| Node rotation | Finite, nonzero quaternion with normalized meaning `H(q)/‖q‖²` (D2) | Exact and rational; no window is needed for soundness |
| Zero or negative node scale, cumulative shear | Supported | Positions are only transformed forward |
| POSITION encodings | Entire C1-admitted domain: FLOAT; KHR_mesh_quantization i8/u8/i16/u16, normalized or not; meshopt NONE views; viewless zero accessors | Exact rationals `n/127` (clamped at −1), `n/255`, `n/32767` (clamped at −1), `n/65535` |
| Sparse accessors | Already C1 Unsupported (`payload.rs:419-421`) | Unchanged |
| Primitive modes | POINTS, LINES, LINE_STRIP (including the C1 restart profile), TRIANGLES | Indexed draws visit referenced indices only. Restart markers are not positions. Repeats are charged |
| Skins, morph targets on a selected primitive, any nonempty `animations`, `EXT_mesh_gpu_instancing`, unknown positional extension on root/node/mesh/primitive/accessor/bufferView | Not inspected (finite reason per class) | No static meaning is invented |
| Volumes: box | Supported in two cases: exact diagonal or signed-permutation boxes, including zero half-lengths; and nonsingular general boxes under the parallelepiped reading (D5) | Every producer writes axis-aligned boxes |
| Rotated degenerate box | Not inspected (`degenerateBox`) | No producer writes one |
| Volumes: sphere | Supported (exact squared distance) | No producer writes one, but the check is cheap |
| Volumes: region | Not inspected (`regionVolume`) | No producer writes one; the geodesy owner belongs to #120 |
| Tile transforms | Any finite affine transform, singular ones included | Forward-only evaluation in the volume frame; no inversion |
| Explicit tile or content `metadata` with `TILE_BOUNDING_*` or `CONTENT_BOUNDING_*` semantics | Not inspected (`metadataVolumeOverride`) | No effective-semantics owner exists for explicit tiles |
| Retained source payloads (`rustyTilesSourceContents` `validate.rs:560-571`, `retainedContentUris` `validate.rs:908-919`) | C2 does not apply | They are not tile content and have no frame. C1 integrity is unchanged |

"Not inspected" applies to the whole archive. The report keeps `decodedContentBounds`
in `notInspected` and adds a finite reason. In-domain references are still streamed,
and a proven Outside verdict still fails (see D1).

## Frame chain

For a drawn position `x` of an instance under the selected node path `n_1 … n_k`:

1. Decode: FLOAT gives exact f64 from f32. Integer types give exact integers.
   Normalized types give exact rationals with the glTF clamp.
2. Node frame: `y = L_1 · … · L_k · x`.
   - `L = T·R·S` for TRS nodes, or the column-major matrix.
   - This is the parent·local order.
3. Up axis: `t = U(y)` with `U(x,y,z) = (x, −z, y)`, applied exactly once.
   - `asset.gltfUpAxis` or any other axis override: not inspected (`upAxisOverride`).
   - The axis is never inferred from coordinates.
4. RTC (later slice only): `t += RTC_CENTER`, added after U and before the tile transform.
5. Own tile and content volume: test `t` directly, because the volume is in the tile frame.
6. Ancestor `A_j`:
   - Apply the transforms of the tile itself and of every intermediate tile below
     `A_j`, child first, including external-tileset roots.
   - `A_j`'s own transform is never applied.
   - The root transform (ENU or ECEF placement) therefore enters no predicate unless an
     external tileset places that root.

Each step is pinned by the primary sources. These citations come from the pinned copies
in the domain-preparation package; this lane did not re-fetch them:

- glTF 2.0.1 node/TRS, `gltf-2.0.1.adoc:680-784`;
- 3D Tiles core transforms and the glTF up-axis;
- modern b3dm RTC, `modern-b3dm.adoc:129-135`.

## Numeric policy: exact, zero tolerance

**The verdict is the exact real predicate over the represented input values. There is no
spatial tolerance, slack, epsilon or relative padding anywhere in C2.** The represented inputs
are:

- f64 JSON numbers after the consuming numeric adapter;
- f32 and integer accessor words;
- the exact normalization rationals.

The verdict is one of:

- **Contained**;
- **Outside**: becomes InvalidInput;
- **beyond the exact-arithmetic magnitude domain** (D7): not inspected, `numericRange`;
- **beyond a work cap**: ResourceLimit.

There is no "NumericalUnresolved" outcome for an in-domain input.

Rationale, executed in this lane (`verify/tight_bounds_probe.py`, output in
`verify/tight-bounds-observed.json`):

- The setup is 20,000 seeded tight producer-style boxes, with `c = (lo+hi)/2` and
  `h = (hi−lo)/2` in f64 over f32 vertices.
- The outward-interval rule proposed earlier returns Unresolved for **40,000 of 40,000**
  extreme vertices.
- Exact rationals show every one of those vertices is contained (0 exact-outside).
- After an f32 re-centring plus one translation-only parent step, the interval rule leaves
  5,010 exactly-contained cases Unresolved. A further 2,546 are truly outside, which is the
  producer-rounding defect class.
- The interval rule was never wrong. It is useless at tight boundaries, which is exactly
  where producers place extreme vertices.

The probe models rounding patterns only. It is not a measurement of actual producers.

Representation follows from this:

- **Expansion arithmetic.** Use Shewchuk TwoSum, plus Dekker-split TwoProduct with no
  dependence on FMA. Inputs are dyadic, and the polynomial predicates have small fixed
  degree.
- **Point images.** The image of `x` is a short polynomial in the inputs.
- **Normalized integers.** Clear denominators by multiplying through by the positive
  normalization constant.
- **Rotation.** Clear denominators by multiplying through by `‖q‖² > 0`.
- **Box membership.** Use Cramer comparisons `|det(B with column i := d)| ≤ |det B|`, or
  direct per-axis comparison for diagonal and signed-permutation boxes.
- **Sphere membership.** Compare `|d|² ≤ r²`.

A bounded exact-sign primitive already exists in `src/mesh_archive/placement.rs:84-125`
(`within_magnitude_budget`). It is precedent only. C2 must not share producer bound code; see
*Ownership*. An interval filter in front of the exact predicate is an optimization that
decides the same predicate. It is not part of slice 1. Add it only with measured need and a
separate equivalence proof, and never as a fallback ladder.

Owners:

- **Math module** (proposed `src/validate/containment.rs`): exact semantics, the magnitude
  domain and expansion-length caps.
- **Format owner**: the unit-quaternion and local-matrix *validity* windows. These are
  glTF-validity questions, not containment numerics (D2).
- **Operation owner** (`ValidationLimits`): work caps.
- **Adapters**: none.

The C1 rule that declared accessor min/max equal decoded extrema (`payload.rs:736-760`) is
integrity evidence only. It never replaces drawn positions.

## Failure semantics and report

The typed outcomes stay the existing four in `ValidationFailure` (`validate/types.rs:19-29`):

- **InvalidInput.** Raised for:
  - a proven Outside;
  - malformed consumed scene/node/mesh records (bad references, cycles, shared children,
    matrix together with TRS, a non-affine matrix, a zero or nonfinite quaternion);
  - an RTC range overrun, once RTC is supported.

  An Outside message carries finite facts:
  - the archive-local content name;
  - the tile depth;
  - the volume role (`own`, `content` or `ancestor:k`);
  - the node, mesh and primitive indices and the vertex index.

  It never carries interpolated source strings or floats.
- **Unsupported.** Only where C1 already refuses. C2 domain exclusions do not fail the
  operation (D1). They leave `decodedContentBounds` in `notInspected` with a reason from a
  finite enum.
- **ResourceLimit.** Raised when a C2 work counter would exceed its cap, checked before
  the scan or predicate.
- **Io.** Causal resolver/archive I/O keeps its existing typed route
  (`validate.rs:45-62`). A stream-callback failure is distinct from a format failure and
  never yields a partial success.

New counters, resolved once in `ValidationLimits`:

- `positionInstanceVisits`, proposed at 16,000,000;
- `containmentTests`, proposed at 64,000,000 (position × volume);
- `nodeInstanceVisits`, proposed at 262,144;
- `nodeDepth`, proposed at 128.

These are finite engineering refusals, not measured approvals. `ValidationLimits` has
**15** fields today (`validate/types.rs:50-84`). Adding to it is a public report change,
serialized through CLI and Python unchanged (`bindings/python/src/lib.rs:976-981`).

## Ownership of representation

1. **Format stream.** The format layer is `content_integrity/payload.rs` with
   `payload/records.rs`.
   - Its responsibilities:
     - admit `scene`, `scenes`, `nodes`, node fields, `skins`, `animations`, mesh
       `weights` and primitive `targets` as borrowed records, preserving absent-versus-null;
     - preflight the exclusions before decoding;
     - emit a fixed-size event stream while the existing C1 views and accessors are live:
       - `NodeEnter(index, Matrix16 | Trs)`;
       - `DrawPosition(node, mesh, primitive, vertex, raw[3], Normalization)`;
       - `NodeExit`.
   - It allocates no `Vec` of positions and no second decoder.
   - It holds no paths, report, limits policy or math.
   - The `inspect` signature gains a consumer callback whose error type is distinct from
     `FormatError`.
2. **Math.** Pure fixed-size functions with no serde, `Value` or I/O:
   - an exact affine chain builder;
   - point-in-box and point-in-sphere predicates;
   - a rotation-from-quaternion constructor in homogeneous form;
   - the magnitude-domain check.
3. **Operation.** `validate.rs` owns:
   - the traversal ancestor chain: a bounded arena of `(volume, transform, parent)`;
     depth is at most 128 and visits at most 65,536;
   - re-streaming per content reference: the URI cache keeps integrity facts only and never
     a frame verdict;
   - work counters;
   - the report label.

## Disposition of existing code

| Component (observed) | Decision | Reason / evidence needed |
| --- | --- | --- |
| `StoredArchive` reads, `read_member`, the reference and read budgets | Retain | Accepted A/C1 owners. C2 re-reads are charged to `totalBytesRead` |
| `content_integrity/json.rs` admission and exact numbers | Retain | One parser. Node and scene integers go through `payload/numbers.rs` |
| `payload::inspect` (`payload.rs:371-1112`), `value`/`scalar`/`normalized` (`:109-140`) | Rework | Add the live stream and the records preflight. `normalized()` returns rounded f64. C2 consumes the raw integer plus a normalization tag, never the rounded quotient |
| `payload/records.rs:40` `Root` (no scenes or nodes), `Mesh`, `Primitive` | Rework | Add only the consumed fields, each with one owner |
| `unwrap_b3dm` (`payload.rs:292-337`) | Retain in slice 1 | Its RTC facts are needed only when D6 lands |
| `check_extensions` (`payload.rs:340-369`) and the `supported` list (`:396-401`) | Retain; add a separate positional allowlist consumed by C2 | Optional unknown extensions currently only reach `not_inspected` (`:447-451`) |
| `Check::payload` URI cache (`validate.rs:402-404`) | Retain for integrity only | Must not short-circuit per-reference containment |
| `ValidationTask` parent `Option<Volume>` (`validate.rs:385-400`) | Rework into an ancestor chain | C2 needs every ancestor and every transform, not only the parent |
| `transform`, `point`, `scale_bound`, `Volume`, `inside`, `contains` (`validate.rs:197-338`) | Do not reuse for C2 | Relative slack `1e-6·(1+…)` (`:274`, `:278`), sphere inflation (`:211-219`), and the f64-rounded `point` (`:208-210`). The prior model executes the false-admission controls. These helpers keep serving `hierarchyBounds` until D4 decides its fate |
| `hierarchyBounds` child-volume rejection (`validate.rs:643-650`) | Unchanged in slice 1; D4 decides | Rejects spec-valid tilesets (`3d-tiles-core.adoc:752`) |
| Box-axis orthogonality admission (`validate.rs:240-249`, `1e-8`) | Retain as C1 well-formedness | C2 does not need it for soundness (D5) |
| Implicit expansion `implicit::expand_tileset_bounded` (`validate.rs:575-608`, `implicit/tileset.rs:619-791`, metadata box `:1133-1135`) | Do not build C2 on it | It is the old untrusted owner that A2 is replacing. It honours only the private `rustyTile` schema (`:718-733`) |
| `decodedContentBounds` in `notInspected` (`validate.rs:892`) | Remove only on proven full coverage | Honest label |
| Producer-side bound code (`original_bounds.rs`, `partition.rs`, `terrain.rs:322-331`, `tileset_node.rs`) | Not a C2 dependency | The checker must stay independent of the writer it checks (#125) |

Nothing is removed in slice 1 except the conditional `notInspected` entry. Dead
arithmetic is removed only once D4 retires `hierarchyBounds`.

## Corrections to the prior preparation

1. **Interval-only verdicts.** "Outward intervals + NumericalUnresolved → Unsupported"
   would leave essentially every tight producer boundary unresolved (probe above). The
   prior exact shortcuts cover only integers of magnitude ≤ 2^26 and ≤ 2^52.
   Replacement: exact expansion predicates.
2. **Singular tile chains.** The prior draft marked them Unsupported. They need no
   disposition when containment is evaluated forward in each volume's frame; nothing is
   inverted.
3. **ValidationLimits size.** The math package's "retain existing 16 ValidationLimits" is
   wrong: there are 15 fields.
4. **Quaternion and matrix windows.** The 2^-20 quaternion window and the 2^-40
   column-orthogonality window are not needed for containment soundness. As proposed, both
   are stricter than W1's own source admission:
   - quaternion `|‖q‖−1| ≤ 1e-6` at `mesh_archive/source.rs:715`;
   - matrix `|cos| ≤ 1e-6` at `source.rs:684-692`.

   A strict C2 default would refuse W1 outputs that W1 accepted. See the producer inventory
   and D2.
5. **W13.** The witness encodes the raw-polynomial quaternion meaning: "normalize source
   quaternion" is listed as a fault. Under the normalized meaning its expected point
   differs by about 1.04e-15. W13 is decision-dependent.
6. **Witness re-derivation.** The other 12 literal witness expectations, and W13 under the
   raw meaning, were re-derived exactly by an independent evaluator in this lane
   (`verify/witness_check.py` → `verify/witness-check.json`). The single false flag W13
   still shows (`selected_nodes`) is a definitional difference in this lane's evaluator:
   it records mesh-bearing nodes only. It is not a witness error.

   The 17 context recipes remain unmaterialized and unchecked.
7. **Prior math model.** Re-executed from a copy: 16 cases plus 5 controls. The
   `observed.json` is byte-identical (SHA256 `9d46031b…`). The 4-case normalized addendum
   was not re-run.
8. **Implicit and producer impact.** The prior "active metadata override → Unsupported"
   rule, combined with implicit being the **default** for point clouds and vectors (see the
   inventory), means no default P1/vector output could ever receive the claim. This is the
   concrete "implicit P1/vector" blocker. It is scoped explicitly in D3.
9. **RTC and regions.** No producer writes RTC or regions. The prior RTC f32-conversion
   and region dispositions are not on the slice-1 critical path.
10. **Memory claim.** The prior `workingBytes` 512 MiB joint claim is a whole-validation
    memory guarantee. C2 slice 1 adds only O(node depth + hierarchy depth) fixed state and
    re-reads charged to existing counters. It should not introduce that claim; it stays held.

## Independent acceptance plan

1. **Oracle.** The independent oracle (`bench/architecture_audit/c2_decoded_containment/oracle/`)
   is pure Python 3 with `Fraction`. It has its own GLB, glTF and b3dm reader, accessor
   decoder, normalization and meshopt handling.
   - Meshopt: the oracle either decodes via the pinned upstream golden bytes or treats
     meshopt fixtures as out of oracle scope. It must never use rusty-tiles code.
   - It has its own tileset traversal, frame chain and exact predicates.
   - It must not import, copy or share helpers with `src/`. A separate author writes it.
   - The A2 implicit audit (`implicit-rewrite-contract.md:272`) and the F1 fidelity
     obligations (#125) need the same oracle, so build it first.
2. **Witnesses.**
   - Materialize the 13 literal and 17 context witnesses once D2 fixes W13's meaning.
   - Add:
     - tight non-integer-centre boundaries, exactly on and one ulp outside;
     - an f32 re-centring escape;
     - a vector u16 `q=65535` with f64 `lo`/`scale`;
     - an all-f32-exact point cloud with `rounding = 0`;
     - a near-window W1 quaternion and matrix;
     - a child volume escaping its parent while the content is inside (`hierarchyBounds`
       versus C2);
     - repeated URIs under different transforms;
     - a singular tile transform;
     - a zero node scale;
     - a degenerate diagonal box.
3. **Sensitive controls.** Each control must flip the production verdict *and* the
   oracle's verdict. Expected values must never be edited instead:
   - omit U, double U, or reverse the tile order;
   - reuse the URI cache;
   - default to scene 0;
   - visit undrawn vertices;
   - use rounded normalization;
   - add `1e-6` slack;
   - treat the f64-rounded transformed point as exact;
   - skip the ancestor check;
   - skip the external root transform.
4. **Producer corpus.** Small outputs from every `.3tz` producer, in default and `--explicit`
   modes:
   - vector: plain, `--quantize` and `--meshopt`;
   - point cloud;
   - F1c2/F1d mesh;
   - W1 local and Wgs84;
   - legacy `mesh-to-3tz`;
   - `convert-to-implicit`.

   The oracle reports per-producer Contained, Outside or OutOfDomain. Production must
   agree on every case.
5. **Limits.** Equal and one-over cases for each new counter. Show refusal happens before
   the scan.
6. **Adapters.** Rust/CLI/Python report and error parity; installed-wheel and official
   Blender checks as applicable.
7. **Final acceptance.** A fresh nonauthor review of the final source and evidence, then
   exact-head CI.

## Proof limits and remaining gates

The following are not provided by this contract:

- implicit coverage (A2), regions and geodesy (#120), RTC (D6);
- deformation;
- rendered or GPU extent;
- whole-process memory or RSS.

Parents #113, #120, #125 and #126 stay open. There is no authority to merge to `main`,
tag or publish.
