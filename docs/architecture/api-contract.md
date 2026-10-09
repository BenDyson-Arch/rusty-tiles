# Proposed public API contract for the first mesh slice

Status: design proposal for [#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113), based on develop `8dfd74b`; no production changes implement this document yet. Breaking API changes are acceptable before 0.4.0. Names below are illustrative, while ownership, validation order, and publication guarantees are intended acceptance requirements. See the [architecture plan](README.md) for dependency boundaries and sequencing.

## Current behavior motivating the proposal

These are source observations at the base revision; they do not describe the proposed API.

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

Keep one Rust domain library and the existing Python binding crate. A CLI module owns clap definitions, command spelling, terminal rendering, exit codes, and NDJSON serialization. The Python crate owns Python signatures, exceptions, callback objects, and conversion to Python values. Neither adapter owns domain validation. Splitting the CLI into another package is a separate packaging decision, not a prerequisite.

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
let options = MeshOptions::default()
    .coordinates(MeshCoordinates::LegacyAutomatic { offset: None })
    .placement(Some(placement))
    .texture(TextureEncoding::Lossless);
let request = MeshRequest::new(input_path, output_path, options)
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

`mesh::convert` runs these stages in order. Validation is pure and checks cancellation before proceeding. Preparation performs source inspection and runtime capability resolution without creating output directories or staging files. It resolves the source CRS/backend and chosen encoder. `PreparedSource` owns whatever read-only inspection/loading the current algorithm needs; it need not contain a fully transformed scene. Avoid loading a scene twice. Disk-backed spooling and transformations requiring scratch space belong in execution, after the request/source requirements are known. Recheck source-dependent constraints when data is actually consumed; preparation cannot guarantee that an external source stays unchanged.

Execution starts the existing private `Job`, performs geometry/texture work, stages content/report, and publishes. A prepared operation is consumed once. It must not contain a `Job` or independently publish any resources. Do not generalize this state sequence into a public framework before another converter needs it.

## Coordinate and option invariants

Replace the overlapping `source_crs`, `source_crs_definition`, `source_axes`, and `height_offset` fields with one choice:

```rust,ignore
#[non_exhaustive]
pub enum MeshCoordinates {
    LegacyAutomatic { offset: Option<SourceOffset> },
    LegacyGeographic,
    LegacyWebMercator { offset: Option<SourceOffset> },
    Horizontal {
        definition: String,
        axes: SourceAxes,
        height_offset_metres: f64,
        offset: Option<SourceOffset>,
    },
}
```

This preserves the existing legacy automatic/geographic/Web Mercator conventions. Automatic with an offset retains the existing interpretation as Web Mercator; document that behavior explicitly. A new mode forcing local coordinates is a separate product decision, not required here. `Horizontal` always carries axes and an explicit height offset, including zero. The pure validator rejects an empty definition/nonfinite height offset; preparation resolves the definition through the shared conservative CRS policy. Definition aliases are normalized independently of builder order.

`SourceOffset` has private fields and a fallible constructor enforcing finite E/N/A values. E/N use horizontal CRS units; A uses metres. Geographic legacy mode cannot carry an offset. Adapters must reject a requested legacy geographic offset, rather than omit it when constructing the enum. General geographic coordinates with explicit axes/height use `Horizontal` and may carry a valid offset.

`Cartographic::try_new` enforces finite longitude, latitude, height, longitude in [-180, 180], and latitude in [-90, 90]. `RotationDegrees::try_new` enforces finite angles. `Placement` always contains an origin plus rotation (zero rotation is permitted), so rotation without an origin is unrepresentable. Manual placement is allowed on legacy modes, with existing origin/bake behavior characterized by regression tests; it is rejected with `Horizontal`. Preserve these valid behaviors while centralizing their validation.

Validate positive triangle and byte budgets, power-of-two atlas edges in 64..=4096, and every supported option combination before source/output work. A positive byte budget remains a splitting/leaf target, not a guarantee that an irreducible primitive can fit. If the implementation cannot honor a hard limit, it must report its actual policy rather than imply one. The first slice removes the always-zero `max_texel_density` option; preserving source texels is an invariant of this pipeline. Translate/remove its CLI spelling explicitly during migration. Represent texture encoding as `Lossless`, `Jpeg`, `Webp`, or `Uastc { encoder: PathBuf }`, so the external encoder path is attached to the only choice that uses it. Separate replacement policy from geometry options.

## Defaults and capabilities

Recommended intentional behavior change: lossless texture delivery is the default in Rust, CLI, and Python. Existing Rust/Python defaults are lossless; the current CLI default is JPEG. JPEG remains an explicit opt-in. This can change archive size, runtime, and bytes for existing CLI invocations and must be called out in the release/migration notes. Update CLI help/examples and baseline expectations in the implementation PR. Do not describe the recommendation as already adopted.

Use a single `MeshOptions::default()` as the domain source of defaults: implicit hierarchy, meshopt enabled, node features disabled, 20,000 triangles, 204,800 target bytes, 2048 atlas edge, and legacy automatic coordinates. CLI omitted values and Python omitted keywords use those defaults. Adapter types may still show defaults in help/signatures, but tests compare their resolved configuration to the domain defaults. Default reporting is silent; CLI explicitly installs its terminal/NDJSON observer.

Distinguish compiled support, runtime readiness, and request eligibility. A converter's baseline availability does not imply every codec or CRS operation is available. Mesh baseline conversion is portable; UASTC needs its explicit encoder; general CRS support depends on the requested operation and portable/native tier. Capability inspection describes those requirements without promising source eligibility. Preparation applies the same backend/encoder checks used to construct readiness diagnostics. It never silently substitutes an unavailable codec or weaker CRS operation.

Keep Python's portable wheel policy: it does not promise native raster/terrain or external UASTC execution. For this slice Python explicitly rejects unsupported texture choices with the shared unsupported-capability classification; accepting UASTC plus an external process in the wheel is a separate decision. Rust/CLI may support it when preparation resolves the executable. Doctor should expose option-specific requirements and avoid stating that every mesh option has no dependencies. A full capability schema for every converter is deferred.

## Errors, results, and reports

The facade exposes an error with a non-exhaustive domain kind, stable machine code, human message, optional option/path context, and an underlying source where available. Proposed kinds are invalid request, invalid source, unavailable capability, I/O, output conflict, cancelled, and observer failure. Validation failures name domain options, not CLI flags. Private implementations may use richer internal errors; neither public variants nor public signatures expose GDAL handles, Python exceptions, clap objects, or third-party parser error types.

CLI maps domain errors to its documented categories/exit statuses and translates option names to flag spelling. Python maps them to its documented exception classes. Both expose the same machine code when one exists. Explicitly settle `InputNotFound` as an I/O error, invalid budgets/placement as invalid request, and missing native/encoder capability as unavailable capability. Preserve exact external JSON/exception contracts only where deliberately retained; update contract tests for intentional breaks. Do not repurpose all existing `Message` errors as invalid request: source-data failures need classification at their creation sites.

Successful `ConversionResult` owns a published output descriptor with a path and output kind (`ThreeTz`, `Directory`, or `Manifest`, as needed). It means commit occurred. A boolean archive field may be a temporary Python compatibility accessor, but is not the primary Rust representation. Results should contain a typed, versioned common report envelope plus mesh detail where the mesh algorithm reports it. The typed envelope identifies converter, effective settings, output kind, and report schema version; optional detailed mesh statistics and existing JSON fields can remain a payload during this slice. Serialize the same report object into `conversion.json` and return it; adapters must not infer settings from command names or output extensions.

Do not require reports for unchanged wrapping paths until their existing no-report contract is deliberately migrated. The first mesh slice preserves whether each path currently produces a report, including the small unchanged-model path. Report serialization completes before commit. Warnings have stable codes and readable messages, with optional owned source context; progress remains transient. No report schema for all other converters is frozen here.

## Events, cancellation, and callback publication

Observers receive progress, warning, and note events during preparation/execution; the event interface is independent of JSON transport. Events may borrow short-lived text/detail for the call; observers retain them only by copying into owned storage. Observers can be called on worker threads and must be `Send + Sync`; ordering between workers is not promised. The context records the first abort reason and exposes cheap cooperative cancellation checks. An observer returns success or an observer-failure indication. Callback code executes without locks needed for reentry.

Python stores the first original callback exception in the binding-owned sink, signals observer failure to the context, and suppresses subsequent callback invocations. The core joins in-flight work and checks that failure before publication. The binding rethrows the original callback exception after receiving the observer-failure outcome. Rust observer failures return the structured observer-failure error. Preserve a genuine conversion error when a concurrent observer error would otherwise mask it; document/test precedence, with cancellation originating from that observer classified as observer failure.

The commit boundary is explicit:

1. Complete work, report serialization, and all fallible user callbacks.
2. Join event-producing workers; check observer failure and cancellation.
3. Acquire the context's commit gate. Cancellation/observer abort requested before the gate prevents commit. Once commit begins, a later cancellation request cannot turn a successfully published conversion into a failed call.
4. Publish using `Job`; return its committed result. No user callback runs after step 2, including a completion callback. Completion is represented by the returned result. CLI may render its own completion message after success.

The gate coordinates only commit versus abort state; never hold it while invoking callbacks. Filesystem publication can still fail and return an I/O/conflict error. Existing outputs remain protected by the existing no-clobber/replacement lifecycle; verify rollback for the directory publisher separately. Failed preparation creates no output parents/staging; failed execution removes private work and preserves previously committed output. Cancellation is cooperative between bounded work units, not immediate interruption of a native call. Python signal/interrupt handling must request cancellation before the gate; do not unconditionally call `check_signals` after commit and report an already successful conversion as a failed call. The first slice guarantees callback-abort behavior; responsive interruption across all native loops is a separate follow-up.

## Frontend and release migration

First port Rust mesh internals and their tests through the facade, then have CLI/Python construct the same request. Remove the old mesh options and unit-returning/reported entry-point split when callers are migrated. Other converter entry points remain temporarily; do not pretend they already implement this contract. Move stderr/NDJSON policies out of the domain reporter incrementally, with the CLI preserving its chosen transport. Make fixture helpers test-owned and rewrite integration tests that depended on internal public modules.

Rust API changes require recompiling downstream crates; this proposal makes no stable Rust ABI promise. Python's existing abi3 build policy is independent of Python signature/exception/report compatibility. Rebuild and test the wheel, update its docs/type declarations and wheel acceptance examples, and record the Python API changes in release notes. CLI aliases/spelling can remain even when the domain names change. Document the unified lossless default, newly rejected invalid placement/offset requests, changed error classifications, removed fixed-density option, and callback no-publication guarantee. Do not silently drop unsupported adapter arguments.

## First vertical slice acceptance

The implementation PR should migrate only mesh request validation, preparation, context/error/result boundaries, and CLI/Python mesh adapters. Keep the geometry, tiling, and encoding algorithms intact except where required to stop before commit. Review each intentional artifact change separately from interface cleanup.

- Pure request tests cover NaN/infinity, longitude/latitude bounds, rotation without origin at adapter construction, zero budgets, every atlas edge boundary, empty CRS, missing general axes/height, manual placement with general CRS, all legacy offset combinations, and builder-order independence. Invalid configuration wins before any source/output error and creates neither an output parent nor work directory.
- Rust/CLI/Python parity tests build equivalent requests and compare resolved defaults/effective settings, acceptance/rejection, and machine error codes. Retain existing CLI spellings/aliases unless an explicit migration test describes removal. Assert CLI's new omitted texture choice is lossless.
- Offset regressions prove that legacy geographic requests reject an offset across all adapters and that general geographic/Web Mercator shifts affect placement as documented. Characterize valid legacy manual placement, small-model wrapping, general axes, explicit height zero, and implicit/explicit hierarchy behavior with existing fixtures.
- Capability tests cover portable supported CRS, portable refusal/native fallback where applicable, native requirements, and missing UASTC executable before `Job::begin`. Python's unsupported UASTC policy is checked separately from invalid enum syntax.
- Publication tests inject a callback failure early and at the final precommit event; assert the original Python exception, no output for a new destination, unchanged bytes for a forced existing destination, cleaned work, and joined workers. Test cancellation before/after the commit gate, observer failure versus source-error precedence, and concurrent conversion/callback reentry. After commit, cancellation cannot suppress the returned committed result.
- Report tests compare the returned serialized report with published `conversion.json`, verify schema/effective defaults, and retain no-report wrapping behavior. Existing report/progress transport tests identify deliberate JSON changes. Library-default runs produce no process stderr.
- Run relevant mesh fidelity, resource packaging, implicit hierarchy, machine-result, and output-digest regressions with explicit texture settings. Changing the CLI default has its own expected baseline change; API restructuring must not excuse other byte/fidelity changes. Run portable Rust checks and Python wheel API acceptance; native checks apply to CRS preparation changes. An external-consumer compile/doc test demonstrates the intended facade without importing implementation modules.

Deferred: other converter requests, a universal report payload, separate CLI packaging, a new forced-local mesh mode, Python UASTC execution, and broader responsive signal/cancellation support. Apply lessons from the mesh slice before settling their types.
