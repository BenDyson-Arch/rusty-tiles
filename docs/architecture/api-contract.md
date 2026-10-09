# Proposed public API contract for the subsequent mesh slice

Status: design proposal for [#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113), based on develop `8dfd74b`; no production changes implement this document yet. Breaking API changes are acceptable before 0.4.0. Names below are illustrative, while ownership, validation order, and publication guarantees are intended acceptance requirements. See the [architecture plan](README.md) for dependency boundaries and sequencing.

The design is replacement-first. Treat current code as an untrusted reference implementation, not the definition of correct behavior. Existing algorithms, defaults, coordinate heuristics, report omissions, and packaging helpers must prove that they satisfy an independently stated requirement before retention. Rewrite them when they fail that gate; a mechanical refactor or agreement with current output is not sufficient evidence.

The [post-F0 scopes](next-foundation-scopes.md) narrow the next mesh milestone.
The broad configuration/context/result sketches here are not an instruction to
build a universal framework or to include every proposed capability in F1a.

## Current behavior motivating the proposal

These are source observations at the base revision; they neither define the proposed API nor establish correct behavior. Source links locate claims to investigate, not trusted oracles.

| Observation | Evidence |
| --- | --- |
| Rust/Python mesh default to lossless; CLI defaults to JPEG | [Rust options](../../src/tile.rs):67; [Python signature](../../bindings/python/src/lib.rs):186; [CLI default](../../src/main.rs):438 |
| Python validates finite/bounded placement, while core construction accepts arbitrary floats | [Python placement](../../bindings/python/src/lib.rs):156; [Cartographic constructor](../../src/georef.rs):18 |
| CLI rejects rotation without placement; the GLB wrapper omits rotation when placement is absent | [CLI options](../../src/main.rs):916; [manifest generation](../../src/tileset.rs):64 |
| CLI rejects legacy geographic offsets; core geographic bake does not apply the supplied offset | [CLI options](../../src/main.rs):967; [geographic bake](../../src/mesh.rs):269,334 |
| Mesh creates its job before validating some coordinate/encoder requirements | [mesh entry point](../../src/tile.rs):183,199; [job contract](../../src/output.rs):63 |
| Missing input maps to CLI data and Python I/O; domain errors embed CLI exit policy | [error classification](../../src/error.rs):55; [Python error mapping](../../bindings/python/src/lib.rs):36 |
| Callback exceptions are rethrown after conversion returns; tests do not assert no publication | [binding runner](../../bindings/python/src/lib.rs):137; [callback test](../../bindings/python/tests/test_api.py):390 |
| Broad public modules include global timings and in-place worker utilities | [library exports](../../src/lib.rs):6; [timing counters](../../src/hlod.rs):34; [compression worker](../../src/vector_encoding.rs):18 |

## Scope and public ownership

Use one Rust domain library and a Python binding crate initially to keep ownership changes reviewable; this is a packaging choice, not endorsement of their current internals. A CLI module owns argument syntax, terminal rendering, exit codes, and NDJSON serialization. The Python crate owns Python signatures, exceptions, callback objects, and conversion to Python values. Neither adapter owns domain validation. Splitting the CLI into another package is a separate packaging decision, not a prerequisite.

Expose an intentional facade with converter requests/options, placement values, conversion results, errors, events, and supported inspection operations. Implementations live behind private modules. In particular, tests and adapters must not require public atlas plans, spatial hashes, simplifiers, scene internals, global timing counters, test fixtures, or compression worker protocols. Public metadata/implicit-format models may remain public where they are intentional supported building blocks; inventory them individually rather than hiding them indiscriminately.

Do not introduce a universal request enum, plugin registry, converter trait hierarchy, or per-converter crates in this slice. Requests remain specific to their converter. Shared concepts should earn their shared representation through a second consumer. Remove compatibility-only entry points when their callers migrate, rather than supporting an indefinite parallel API.

## Request, validated configuration, prepared operation

The public mesh entry point consumes an owned request and borrows a conversion context. Owned input/output paths and CRS definitions avoid tying worker lifetimes to adapter arguments. Options are immutable during a running operation. The context owns its observer/cancellation state through shared handles; it contains no clap or Python types and does not discover settings from process environment.

```rust,ignore
// Proposed usage; not a compilable API in the current tree.
let placement = Placement::new(
    Cartographic::try_new(153.02, -27.47, 0.0)?,
    RotationDegrees::try_new(5.0, 0.0, 0.0)?,
);
let coordinates = MeshCoordinates::LocalGltf { placement: Some(placement) };
let options = MeshOptions::default().texture(TextureEncoding::Lossless);
let request = MeshRequest::new(input_path, output_path, coordinates, options)
    .output_policy(OutputPolicy::CreateNew);
let result = mesh::convert(request, &ConversionContext::default())?;
```

`MeshRequest` and `MeshOptions` have private fields, builders, and read-only accessors. Builders collect choices; they do not interpret previously assigned fields or perform I/O. The following types describe the ownership model, not an additional public execution API:

```rust,ignore
// Internal types. No caller can manufacture validated/prepared states.
struct ValidatedMeshRequest {
    input: PathBuf,
    output: PathBuf,
    output_policy: OutputPolicy,
    config: ValidatedMeshConfig,
}

struct PreparedMesh {
    request: ValidatedMeshRequest,
    source: PreparedSource,
    coordinates: ResolvedMeshCoordinates,
    encoder: ResolvedTextureEncoder,
}

fn validate(request: MeshRequest) -> Result<ValidatedMeshRequest, Error>;
fn prepare(request: ValidatedMeshRequest, context: &ConversionContext)
    -> Result<PreparedMesh, Error>;
fn execute(prepared: PreparedMesh, context: &ConversionContext)
    -> Result<ConversionResult, Error>;
```

`mesh::convert` runs these stages in order. Validation is pure; the context checks cancellation before proceeding. Preparation performs source inspection and runtime capability resolution without creating output directories or staging files. It resolves the source CRS/backend, supported source semantics, and chosen encoder. `PreparedSource` owns bounded metadata/reader state; it need not contain a fully transformed scene. Full scene materialization must have an explicit measured resource policy and normally belongs in execution, rather than moving unbounded work into preflight. Avoid loading a scene twice. Disk-backed spooling and transformations requiring scratch space belong in execution, after the request/source requirements are known. Recheck source-dependent constraints when data is actually consumed; preparation cannot guarantee that an external source stays unchanged.

Execution starts a private publication job satisfying the runtime contract, performs geometry/texture work, stages content/report, and publishes. Existing `Job` code is only a candidate implementation and must pass the publication evidence gate. A prepared operation is consumed once. It must not contain a publication job or independently publish any resources. Do not generalize this state sequence into a public framework before another converter needs it.

## Coordinate and option invariants

Require a coordinate interpretation when constructing a mesh request; there is no guessed CRS default. Replace overlapping options with one choice:

```rust,ignore
#[non_exhaustive]
pub enum MeshCoordinates {
    LocalGltf { placement: Option<Placement> },
    Horizontal {
        definition: String,
        axes: SourceAxes,
        height_offset_metres: f64,
        offset: Option<SourceOffset>,
    },
}
```

`LocalGltf` explicitly interprets node-transformed source coordinates as right-handed glTF local metres with Y up. It never tries to identify geographic degrees or a projected CRS from coordinate magnitudes. Optional placement maps this local frame into a declared WGS84 frame; local +X maps to east, +Y to up, and -Z to north before the declared rotation. Test both the source-to-local-frame mapping and glTF/3D Tiles axis conversion so the mapping is applied exactly once. Without placement, output remains local and the report identifies it as such.

`Horizontal` always carries a source definition, axes, and an explicit offset to ellipsoidal metre heights, including zero. It supports geographic and projected definitions through the same contract. Axes describe node-transformed positions; `Xyz` maps X/Y/Z to east/north/height, while `YUp` maps X/-Z/Y to east/north/height. The pure validator rejects an empty definition/nonfinite height offset; preparation resolves the operation under independently verified accuracy/eligibility requirements. The current CRS resolver and fallback policy must also prove themselves. Definition aliases are normalized independently of builder order, without inventing a datum, grid, epoch, or height interpretation.

CLI must require an explicit coordinate mode, such as `--coordinates local-gltf` or a horizontal CRS with axes/height options. Python must require the equivalent keyword; Rust has no constructor overload omitting coordinates. Magnitudes that resemble longitude/latitude or Web Mercator do not alter a local request. If an independently justified compatibility adapter is added, it is isolated, opt-in, reports the resolved interpretation, and proves its semantics with independent references. Automatic guessing and legacy offset behavior are not requirements of the domain API.

`SourceOffset` has private fields and a fallible constructor enforcing finite E/N/A values. E/N use the declared horizontal CRS units; A uses metres. Proposed ordering applies offsets after axis interpretation and before horizontal transformation; the height offset is added to source height plus A. A geographic offset therefore has angular E/N units and metre A, and must not be ignored. This ordering is a proposed contract requiring independent unit/transform fixtures, not an established standard inferred from legacy code. Local mode does not accept a CRS offset; a future local translation would be a separate explicitly defined option.

`Cartographic::try_new` enforces finite longitude, latitude, ellipsoidal metre height, longitude in [-180, 180], and latitude in [-90, 90]. `RotationDegrees::try_new` enforces finite angles. `Placement` always contains an origin plus rotation (zero rotation is permitted), so rotation without an origin is unrepresentable. Proposed rotation uses the ENU frame with heading about -up, pitch about -north, and roll about +east, composed as heading * pitch * roll. This sign/order choice is a proposed convention requiring independent references and fixtures, not a claim that the current implementation establishes a standard. Independently test signs, order, and noncommuting rotations. Manual placement belongs only to `LocalGltf`; `Horizontal` derives placement from its resolved coordinate operation and cannot also carry a manual origin.

Validate positive configured triangle/byte targets and atlas dimensions, checked arithmetic, and supported option combinations before source/output work. Encoder dimension/range constraints must have a documented technical reason; do not inherit 64..=4096 or power-of-two limits solely from current code. A target budget is explicitly a target: irreducible over-budget content either returns an actionable error under a requested hard limit or records the excess under a target policy. Do not imply bounded process memory. Numeric defaults and hard/target distinctions must be selected from measured representative cases before implementation acceptance.

The fidelity contract requires source-faithful leaf geometry and supported attributes/material semantics, with no unrequested lossy texture encoding or resampling. Define coordinate-storage and transformation tolerances independently of the candidate algorithm and verify them in decoded output; a geometric split must not excuse missing/duplicated source geometry. Coarse LODs may reduce geometry/resolution only under an explicit error policy with measured/reportable deviation. Unsupported source semantics are rejected before publication rather than discarded, including on small-input paths. A fixed-zero option adds no choice and does not belong in the new API. Represent texture encoding as `Lossless`, `Jpeg`, `Webp`, or `Uastc { encoder: PathBuf }`, so the external encoder path is attached to the only choice that uses it. Separate replacement policy from geometry options.

## Defaults and capabilities

Recommend lossless texture delivery by default in Rust, CLI, and Python because the source-fidelity contract excludes unrequested encoding loss. This decision does not derive authority from existing Rust/Python defaults. JPEG and other lossy choices require explicit opt-in and reported effective settings. The new contract can change archive size, runtime, and bytes for existing invocations; describe that migration explicitly. Do not describe the recommendation as already implemented.

Use a single `MeshOptions::default()` as the domain source of reviewed processing defaults; coordinates remain mandatory and are not part of those defaults. Select hierarchy/compression defaults from target viewer interoperability and independently checked decoded fidelity. Select numeric resource/LOD defaults from declared workloads and measurements, not copied constants. No lossy optimization or dropping of source semantics is enabled merely because the old implementation did so. CLI omitted processing values and Python omitted processing keywords resolve to the domain defaults. Help/signatures and tests must agree with those effective settings. Library reporting is silent unless the caller supplies an observer; CLI explicitly installs its presentation observer.

Distinguish compiled support, runtime readiness, and request eligibility. A converter's baseline availability does not imply every codec or CRS operation is available. Target a portable local/lossless mesh path whose implementation proves it has no native/external dependency. UASTC needs its explicit encoder; general CRS support depends on the requested operation and proven portable/native support. Capability inspection describes those requirements without promising source eligibility. Preparation applies the same backend/encoder checks used to construct readiness diagnostics. It never silently substitutes an unavailable codec or weaker CRS operation.

Choose and declare supported distributions independently of the existing wheel's omissions. A portable Python distribution may intentionally omit native raster/terrain and external UASTC execution, but its capability descriptor and tests must establish that policy; do not derive it from today's signatures. Equivalent supported requests share domain validation across Rust/CLI/Python. A distribution rejects unavailable execution choices with the shared unsupported-capability classification. Doctor exposes option-specific requirements and avoids stating that every mesh option has no dependencies. A full capability schema for every converter is deferred.

## Errors, results, and reports

The facade exposes an error with a non-exhaustive domain kind, stable machine code, human message, optional option/path context, and an underlying source where available. Proposed kinds are invalid request, invalid source, unavailable capability, I/O, output conflict, cancelled, and observer failure. Validation failures name domain options, not CLI flags. Private implementations may use richer internal errors; neither public variants nor public signatures expose GDAL handles, Python exceptions, clap objects, or third-party parser error types.

CLI maps domain errors to chosen documented categories/exit statuses and translates option names to flag spelling. Python maps them to chosen documented exception classes. Both expose the same machine code when one exists. Classify missing input as I/O, invalid budgets/placement as invalid request, and missing native/encoder capability as unavailable capability. Exact existing JSON/exception shapes require an independently justified compatibility requirement before retention; update contract tests for intentional breaks. Do not repurpose all existing `Message` errors as invalid request: source-data failures need classification at their creation sites.

Successful `ConversionResult` owns a published output descriptor with a path and output kind (`ThreeTz`, `Directory`, or `Manifest`, as needed). It means core commit occurred. Results contain a typed, versioned common report envelope plus optional mesh detail. The envelope identifies converter, explicit/resolved coordinate interpretation, effective settings, output kind, report schema version, capability/backend decisions, and any deviation from requested targets. It describes every successful mesh path, including a small-source optimization. Serialize the same report object into `conversion.json` and return it; adapters must not infer settings from command names or output extensions. Existing JSON fields are candidates only if useful to this reporting requirement.

There is no grandfathered no-report mesh path. Report serialization completes before commit. Warnings have stable codes and readable messages, with optional owned source context; progress remains transient. A distinct byte-preserving packaging operation may be designed if unchanged content/resource preservation is an independently established product requirement. Its resource closure, URI semantics, and byte guarantees must be specified and tested independently; the current wrapper function or its output is not the requirement. No report schema for all other converters is frozen here.

## Events, cancellation, and callback publication

Observers receive progress, warning, and note events during preparation/execution; the event interface is independent of JSON transport. Events may borrow short-lived text/detail for the call; observers retain them only by copying into owned storage. Observers can be called on worker threads and must be `Send + Sync`; ordering between workers is not promised. The context records the first abort reason and exposes cheap cooperative cancellation checks. An observer returns success or an observer-failure indication. Callback code executes without locks needed for reentry.

Python stores the original callback exception in the binding-owned sink, requests observer-failure abort through the same synchronized gate used for every fatal producer/backend cause and cancellation, and suppresses subsequent callback invocations. The first cause accepted by that gate is primary; gate order is authoritative, not inferred wall-clock order or a source/observer priority ladder. Later failures become secondary diagnostics. The core joins in-flight work before publication. The binding rethrows the stored original callback exception only when observer failure is the selected primary outcome; it must not replace another primary core failure. Rust observer failures use the same domain rule. See [F0 arbitration](foundation-contracts.md) for the foundation shared by these adapters.

The commit boundary is explicit:

1. Complete work, report serialization, and all fallible user callbacks.
2. Join event-producing workers, drain observer calls and close event admission; check observer failure and cancellation.
3. Atomically request publication permission from the run-control gate, arbitrating against accepted abort causes. Release the gate lock before filesystem work. Permission is not installation: the publisher can still fail. A late cooperative cancellation request cannot revoke granted permission or replace a committed core result with a pre-commit failure.
4. Publish using the verified publisher; return its committed core result. No user callback runs after step 2, including a completion callback. Completion is represented by the returned result. CLI may render its own completion message after success.

The gate coordinates only commit versus abort state; never hold it while invoking callbacks. Filesystem publication can still fail and return an I/O/conflict or recovery error. The publisher must prove no-clobber semantics and replacement behavior per artifact/platform; current helpers are not trusted to provide them. Failed preparation creates no output parents/staging. Failed execution preserves the destination before publication and attempts private-work cleanup; filesystem failures require actionable retained-path information, and recovery backups must not be deleted. Directory replacement may have a documented visibility/recovery window and is not promised crash-atomic; a failed restore retains the previous output backup and reports its location. A private-work cleanup failure after commit is a committed diagnostic, not a pre-commit conversion error.

Cancellation is cooperative between bounded work units, not immediate interruption of a native call. Adapter-controlled signal checks request cancellation before the gate and do not deliberately replace a committed core result with a cancellation error. This guarantee is scoped to core outcomes and adapter-controlled checks: the Python VM may deliver external asynchronous exceptions, Python result materialization may fail, and process termination is outside library control. Publication can have occurred even when caller-side execution is interrupted. The first slice guarantees callback-abort behavior before commit; broader responsive interruption needs separate evidence.

## Frontend and release migration

Build the mesh slice against the new contract, replacing internals whenever necessary, then have CLI/Python construct the same request. Existing modules may serve as comparison candidates after passing evidence gates. Remove old mesh options and redundant entry points when callers migrate. Other converter APIs remain explicitly unreviewed until their own slices prove the contract. Adapter-owned transport can retain established spellings/schema only where independently justified; compatibility is not a general correctness requirement. Make fixture helpers test-owned and rewrite tests that depended on internal public modules.

Rust API changes require recompiling downstream crates; this proposal makes no stable Rust ABI promise. Python binary compatibility policy is separate from Python signature/exception/report compatibility and must be tested for the chosen distribution. Rebuild/test the wheel and update docs/type declarations and installed-artifact examples. Document required coordinate modes, lossless fidelity policy, changed defaults/errors/reporting, removed options, compatibility adapters if any, and scoped callback/publication semantics. CLI aliases may remain where useful, but do not silently drop arguments or perpetuate guessed coordinate interpretation.

## Evidence gate for retaining existing code

For each candidate module/algorithm, state its intended input domain, units, supported semantics, accuracy/fidelity/resource targets, side effects, and failure policy. Then demonstrate those claims with fixtures and oracles independent of that candidate. Record failures and replace the code or narrow the declared supported domain; current output is never the sole expected answer. A passing neighboring converter does not prove this module.

Independent evidence includes hand-derived axis/transform cases, independently generated CRS reference values with tool/version/provenance, decoded geometry/texture/material comparisons against source, direct archive/URI inventory checks, specification fixtures, fault-injected publication/rollback checks, and measured resource scaling. Writer and validator must not share the only implementation of the invariant they test. Verify that an oracle covers the same source semantics and units instead of treating another tool's output as automatically authoritative.

Existing tests, digests, and captured outputs are useful test inputs and differential observations. Classify each expectation as a product requirement, independent correctness check, or historical behavior. Only the first two can justify retention. Content-byte equality is required only where the product contract demands it; a rewrite may change hierarchy/layout/bytes while proving equivalent or improved declared semantics. Publish that comparison rather than blindly copying or regenerating golden outputs.

## Subsequent mesh slice acceptance

The foundation-first plan puts the bounded package/job slice F0 before this work; see [foundation-contracts.md](foundation-contracts.md). The subsequent mesh slice delivers request validation, preparation, execution/publication, context/error/result boundaries, and CLI/Python adapters against the desired contract. It may rewrite geometry, tiling, source interpretation, encoding, or job code when current implementations fail evidence. There is no algorithm-change prohibition. Keep replacements reviewable and compare correctness/resource effects using independent evidence rather than requiring historical artifacts.

- Pure request tests cover missing explicit coordinate interpretation, NaN/infinity, bounds, rotation without origin, positive targets/dimension constraints justified by the chosen encoders, empty CRS, missing axes/height, contradictory local/CRS options, offset units, checked arithmetic, and builder-order independence. Invalid configuration wins before source/output errors and creates no output parent/work directory.
- Rust/CLI/Python parity tests compare equivalent supported requests, reviewed defaults/effective settings, acceptance/rejection, and domain machine codes. Omitting coordinates fails; explicitly local coordinates resembling geographic/projected values stay local. Optional compatibility translation is isolated and independently tested, not part of default parity.
- Independently derived coordinate fixtures verify glTF node transforms, local-to-ENU axes, rotation signs/order, ellipsoidal heights, both horizontal axis choices, geographic/projected offsets, and explicit height zero. CRS references record provenance and accuracy tolerances. Test out-of-domain requests and prevent silent fallback to another interpretation.
- Source-fidelity tests compare decoded leaves against source geometry, supported attributes/materials and represented texels, test nontrivial node transforms and alpha, and verify reported coarse-LOD error against independent measurements. Unsupported semantics fail on both small and large inputs. Retaining any old algorithm requires this evidence; historically matching bytes is insufficient.
- Capability tests cover every declared distribution/backend/codec combination, independently checked portable/native CRS behavior, and missing executables before publication job creation. Unavailable choices are distinct from invalid syntax. Numeric defaults, hierarchy/compression choices, and resource-target policies require workload/viewer measurements before acceptance.
- Publication tests inject callback failure early and at the final precommit event; assert the original Python exception when it is the selected observer-failure outcome, no new output, unchanged pre-existing output, joined workers, and cleanup attempts. Inject cleanup/restore failure and check retained recovery paths. Test platform no-replace races, cancellation around the commit gate, first-accepted primary failure with secondary diagnostics, and callback reentry. A late cooperative cancellation cannot replace a committed core result; tests do not claim control over VM-level external exceptions.
- Every successful mesh path returns the report published as `conversion.json`, including small inputs. Verify schema, coordinate mode, effective settings/capabilities and target deviations. Library-default execution emits no domain stderr; callbacks and third-party process behavior are explicitly scoped.
- Use existing regressions only after classifying their expectations under the evidence gate. Run independent decoded hierarchy/resource/metadata checks, portable Rust and installed-wheel acceptance, and relevant native cases on final sources. Explain artifact changes against the desired contract, with a separate rationale for any required byte preservation. An external-consumer compile/doc test exercises only the intended facade.

Deferred: other converter requests, a universal report payload, separate CLI packaging, optional compatibility coordinate adapters, additional distribution/external-codec choices, and broader responsive signal/cancellation support. Explicit local coordinates and independently verified execution are part of this slice, not deferred compatibility refinements. Apply evidence from the mesh slice before settling other types.
