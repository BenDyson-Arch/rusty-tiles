# Foundation boundary audit

Status: replacement-first design review for #113, against production baseline
`8dfd74bd87dd23c86278e98d736c3f5912c246cf`. No production changes or full call-graph
analysis. Conditional count is not a quality metric: the concern is repeated,
inconsistent decisions and states that downstream code must keep repairing.
No current implementation earns retention merely by being moved behind a boundary.

## Decisions currently spread across layers

| Decision | Concrete source evidence | Replacement owner |
| --- | --- | --- |
| Coordinate interpretation and placement | `main::tileset_opts` rejects rotation without origin (`main.rs:916`); `main::mesh_opts` reads offsets and rejects geographic offsets (`main.rs:929-970`); Python `placement` validates values (`bindings/python/src/lib.rs:152`); `MeshTo3tzOptions::set_source_crs` interprets definitions according to already-set axes/height (`tile.rs:89-108`); mesh execution branches again (`tile.rs:199-223`); `mesh_crs::bake` rejects missing/conflicting fields (`mesh_crs.rs:16-38`). | One converter request normalizer and pure validator; one source-dependent coordinate preparation step. Adapters translate syntax, not domain rules. |
| Source and runtime capability selection | Doctor constructs every readiness result before filtering selection (`doctor.rs:38-74`); native vector `available` requires all three input drivers (`vector/pipeline.rs:84-108`); portable `Reader::new` chooses format (`vector/portable.rs:72-82`); CRS construction and batch transformation can both select native fallback (`crs.rs:53-78,97-111`). | Request-specific preparation chooses source reader, encoder, and coordinate strategy. Readiness uses those same checks, scoped to the requested operation. |
| Feature failure policy | Native/portable readers classify all acceptance errors except `Environment` as invalid features (`vector/pipeline/source_native.rs:372-380`, `vector/portable.rs:315-323`); store independently maps failures to skipped/invalid (`vector/pipeline/store.rs:653-680`); geometry identifies outline eligibility from a string prefix (`vector/pipeline/geometry.rs:13-18,399-404`). | Typed geometry outcomes and feature rejection, distinct from job failure; vector coordinator applies rejection policy once. |
| Output mode and publication | `force` is carried through domain options; converters choose `Job`/directory/archive entry points; `output::publish_directory` uses exists-then-rename (`output.rs:161-179`) while archive uses persist-noclobber (`output.rs:120-125`); `pack` calls `Job`, which calls `pack` (`pack.rs:2`, `output.rs:116-118`). | Artifact kind plus publication policy resolved before execution; publisher owns platform commit mechanics. Archive serialization has no job dependency. |
| Processing shortcuts and hierarchy | Small mesh routes call a different conversion path (`tile.rs:226-234`); explicit/implicit choices appear in geometry partitioning, manifest construction, report fields, and byte framing (`tile.rs:266,504-515,658`; `vector/pipeline/encoding.rs:514-518`, `vector/pipeline/store.rs:797-870`). | Pipeline plans domain hierarchy once; its format boundary owns layout/framing. Shortcut eligibility is a domain decision under the same validation and result contract. |
| Presentation and error meaning | Core `Error::category` embeds CLI exit policy (`error.rs:55-64`); library reporter defaults to stderr (`report.rs:36-50`); HLOD bypasses it (`hlod.rs:150`); Python checks stored callback failure after conversion (`bindings/python/src/lib.rs:137-144`). | Domain errors and job abort state in core; CLI/Python own presentation and transport mappings; publisher observes one final abort fence. |

The table uses shortened `src/` paths. These are source observations, not proofs
of replacement semantics. [Runtime evidence](../../bench/architecture_audit/runtime/README.md)
separately establishes natural rollback side effects, injected I/O classification,
and deterministic directory conflict interleaving, with narrow cleanup controls.

## Minimal replacement boundaries

1. **Adapter → request.** Parse syntax, aliases, offset-file representation, and
   frontend objects. Construct the same domain request; omitted processing values
   use domain defaults. Remove obsolete knobs and wrappers when callers migrate.
   A compatibility parser, if justified, terminates here and emits an ordinary
   request; no `legacy` mode propagates through runtime or algorithms.
2. **Request → validated configuration.** Pure validation owns cross-field rules,
   finite/range checks, and normalized option meaning. Use a sum type when choices
   genuinely exclude each other: coordinates/placement, texture encoder with its
   required executable, field inclusion/exclusion, imagery display recipe.
   Keep independent booleans independent; do not make a type for every number.
   A single `validate` constructor can produce a private config; builders are not
   a second validator or an order-dependent interpreter.
3. **Validated configuration → prepared operation.** Inspect bounded source
   metadata and resolve the actual reader, encoder, coordinate strategy, and
   supported publication mechanism. Runtime readiness and execution share this
   resolver. Its result owns the selected handles/strategy; execution does not
   rerun a broad Doctor query, rediscover environment defaults, or probe unrelated
   drivers. Data-dependent eligibility remains checked while consuming data.
4. **Domain execution → artifact.** Converter orchestration owns source/model,
   hierarchy, feature policy, and accepted content/resource closure. Readers return
   owned values and typed outcomes; they do not reinterpret errors returned by the
   consuming pipeline. A feature transaction commits rows, counters, and success
   diagnostics together. Geometry reports an explicit alternate outcome; warning
   text never controls whether a feature is kept, repaired, skipped, or replaced.
5. **Complete artifact → publication.** Runtime receives complete staged files or
   directory, declared output kind/policy, and job abort state. It does not know
   mesh/vector/raster identities, CRS modes, topology, or hierarchy. Required
   serialization, close, flush, report, and observer work finishes before commit.
   Platform publishers implement tested create/replace guarantees and recovery.
6. **Outcome → frontend.** Return committed output and its report, or a structured
   failure retaining causes/recovery paths. Adapters format messages and translate
   option names and exception/exit conventions. No result is inferred from an
   extension or converter name. A report may carry an opaque converter identifier;
   that identifier never selects behavior inside runtime.

These are ownership seams, not six generic pipeline stages that every converter
must implement. Separate functions and private structs are sufficient initially.
Feature transactions belong to vector; a mesh conversion need not have one.

## Necessary branching remains local and justified

Source formats, geometry classes, topology/degeneracy, numerical domains, and
projection/grid eligibility require decisions. Put them in source/geometry/spatial
owners and audit each against independent semantics; do not export their flags
to runtime or move them into a universal normalizer. A local/projected coordinate
choice and a format version choice can remain explicit matches at their owners.

Compile-time native support and platform publication differences also remain.
Use backend/platform modules with one selected operation, rather than sprinkling
feature probes or OS cases through converters. A prepared coordinate strategy may
contain a validated fallback transition for point-dependent eligibility: resolving
once does not mean all points can be certified from headers. The spatial owner
controls typed transition reasons, whole-batch retry, actual-backend reporting,
and refusal of weaker accuracy. Existing fallback behavior is still unproven.

Validation during data consumption is essential; repeating request interpretation
is accidental. Rechecking a destination at commit is essential; trusting an earlier
`exists()` as permission to overwrite is incorrect. Rechecking source-derived
values is essential; repeatedly constructing their policy from optional flags is
an avoidable ownership problem.

## Tighten the current proposal before implementation

The API proposal's explicit coordinates, converter-specific requests, private
validated/prepared values, and core/adapter split address repeated decisions.
Do not expand these into public lifecycle types, a universal request enum,
converter registry, trait hierarchy, generic feature system, or runtime plugins.
Public builders plus private request/config/prepared copies must each own a real
invariant or resource; avoid three copies of the same settings and pass-through
wrappers with no additional meaning.

Keep the initial capability result small and request-specific. The proposal
already defers a complete capability schema; do not replace Doctor's hardcoded
lists with a larger configurable rules engine. Similarly, one structured error
kind/code with a causal source suffices initially; no catalogue of string codes
for every internal branch and no parsing of human messages to recover kinds.
Choose hard/target budget semantics per converter before exposing a policy knob;
the distinction is a contract question, not automatically another user-selectable mode.

The report envelope does not require runtime to import every pipeline's details.
Pipeline-owned report data is finalized before publication; runtime carries the
artifact/receipt without patching converter-specific JSON. Do not add serializer
switches solely to preserve old numeric spelling, report omissions, or byte forms.
An independently justified interoperability requirement needs its own evidence.

Foundation work precedes the full mesh slice and full numerical/algorithm audit.
The existing README migration step that starts with a mesh vertical slice should
be narrowed to a small foundation consumer first, not interpreted as authorization
to rebuild mesh before its plumbing is stable. Mesh-specific settings/sign/order
contracts remain proposals requiring the later independent domain audit.

## First implementation acceptance and dependency enforcement

First implement only structured errors, request/preparation seams, job-owned
events/abort state, artifact publication, and one small real consumer sufficient
to exercise those contracts. Prefer packaging or a minimal declared source path;
choose explicitly and reject unimplemented modes. This consumer proves foundation
integration, not the correctness of its existing format codec or geometry.
Then migrate converter slices and audit their algorithms independently.

- Runtime imports no pipeline or frontend modules and has no converter-identity
  switch, feature-skip policy, coordinate interpretation, or encoder discovery.
- One request-specific capability resolution supplies execution and readiness;
  execution does not rediscover that policy. Controlled domain fallback stays
  private to its selected strategy and reports its actual operation.
- Equivalent Rust/CLI/Python requests produce the same validated configuration,
  defaults, and error kind; builder order cannot change their interpretation.
- Feature rejection and infrastructure/observer/cancellation failures are distinct;
  only the explicit rejection policy can skip a feature. Text changes cannot change
  control flow. Reports/counters reflect committed state or explicitly named attempts.
- Domain-neutral runtime tests cover create/replace conflicts, abort around commit,
  cleanup/recovery, and repeated jobs, without fake converter names triggering paths.
- Default-private modules and deliberate re-exports restrict external consumers.
  Add a small AST import/path-reference check with an explicit allowed layer graph;
  cover `crate::`, `super::`, aliases, re-exports, and feature-gated modules. Treat it
  as a dependency guard, not a complete call graph or semantic proof. Validate it
  using deliberate forbidden edges. A grep-only import count is insufficient.
- Compile portable/native configurations and external facade examples. Use a
  targeted internal crate only if module/privacy/import checks cannot enforce a
  concrete required boundary; avoid a workspace explosion as a substitute for design.

Acceptance concerns where decisions live and what they guarantee, not a ban on
`if` or a statement-count target. New special cases need a named owning boundary,
an independently justified domain/platform requirement, and evidence for that case.
