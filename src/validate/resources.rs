//! Validation resources (#342): one memory budget and one worker count, resolved once at the
//! request boundary, the scratch stores sized from them, and the scratch-disk bound.
//!
//! The budget uses `mesh`'s rule ([`MemoryBudget`]): explicit, or the lesser of 4 GiB and half
//! the detected headroom. Stages plan against `min(budget, detected headroom)`. It bounds the
//! validator's scale stores: external sort runs and merge buffers, the catalog page cache,
//! resident facts and stream-log thresholds, the HLOD delivery store's SQLite page cache and
//! temporary store, and in-flight parallel work. Per-item work (one document, one payload
//! decode) stays bounded by the fixed per-item limits, outside the plan ([`RESERVE_BYTES`]).
//!
//! None of these sizes changes a verdict or a report field outside `resources` and `scratch`:
//! every store returns the same records in the same order whether it stays in memory or spills
//! (the `FORCED_SPILL` parity profile), and parallel work is consumed in input order.
use super::{Error, LimitRefusal};
use crate::archive3tz::CatalogConfig;
use crate::mesh_large::{BudgetSource, MemoryBudget};
use crate::spill::SortConfig;
use serde::Serialize;
use std::num::NonZeroUsize;

/// Memory outside the plan, for per-item work bounded by the fixed per-item limits.
pub(crate) const RESERVE_BYTES: u64 = 256 << 20;
/// Smallest plannable share. The fixed 0.4.0 store profile held about this much.
pub(crate) const MIN_PLAN_BYTES: u64 = 256 << 20;
/// Planned memory of each worker beyond the first: its in-flight items' bytes and workspace.
pub(crate) const JOB_BYTES: u64 = 64 << 20;
/// Largest merge fan-in: open run readers stay well inside common descriptor limits.
pub(crate) const MAX_FAN_IN: usize = 128;
/// Bytes of one SQLite temporary-store row bound: the record plus sorter overhead. The widest
/// sorted rows carry a 128-byte frame or world blob (`placements`, `tiles`); their sorter
/// records are about 160 to 200 bytes (review P2-5).
const TEMP_ROW_BYTES: u64 = 256;

/// How `jobs` was resolved, stated in the report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobsReport {
    /// `auto` or the requested maximum.
    pub requested: String,
    pub cores: u64,
    pub jobs: u64,
    /// `request`, `cores`, `memory`, `floor` or `unknown_memory`.
    pub limited_by: &'static str,
}

/// The resolved memory budget, stated in the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetReport {
    pub requested_bytes: Option<u64>,
    pub resolved_bytes: u64,
    pub source: BudgetSource,
    pub detected_headroom_bytes: Option<u64>,
    /// `min(budget, detected headroom)`: what the stores were sized against.
    pub planned_bytes: u64,
}

/// The store sizes derived from the budget, stated in the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorePlan {
    /// The external sorts of the HLOD delivery store and the catalog's side sorts.
    pub sort_run_bytes: u64,
    pub sort_fan_in: u64,
    pub sort_buffer_bytes: u64,
    /// Sorted bytes one such sort merges in a single pass (`fan-in × run`).
    pub single_pass_bytes: u64,
    pub catalog_cache_bytes: u64,
    pub sqlite_cache_bytes: u64,
    /// `memory` when the delivery store's temporary-store bound fits the plan, else `file`.
    pub sqlite_temp_store: &'static str,
}

/// The start-of-run scratch-disk bound, stated in the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskBound {
    /// Estimated scratch peak: the catalog's terms plus, for an HLOD archive, 256 bytes per
    /// declared payload triangle. Less free space than this is a warning.
    pub estimated_peak_bytes: u64,
    /// The catalog's terms plus, for an HLOD archive, 16 bytes per declared triangle. Less
    /// free space than this is refused.
    pub refusal_bytes: u64,
    /// Free bytes on the scratch volume when the bound was checked.
    pub available_bytes: u64,
}

/// The report's `resources` member. Every field depends on the host and the request, never on
/// a verdict; reports compared across budgets and jobs exclude it with `scratch`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResources {
    pub memory_budget: BudgetReport,
    pub jobs: JobsReport,
    pub stores: StorePlan,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<DiskBound>,
}

/// The plan of one validation, resolved before the input is opened.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub(crate) catalog: CatalogConfig,
    /// The HLOD delivery store's external sorts.
    pub(crate) delivery_sort: SortConfig,
    pub(crate) jobs: usize,
    /// SQLite page cache of the delivery store.
    pub(crate) sqlite_cache_bytes: u64,
    /// Temporary-store share: `temp_store=MEMORY` when the row bound fits.
    pub(crate) sqlite_temp_bytes: u64,
    /// In-flight bytes of parallel payload work.
    pub(crate) window_bytes: u64,
    pub(crate) report: ValidationResources,
}

fn invalid_request(message: impl Into<String>) -> Error {
    Error::InvalidRequest(message.into())
}

/// One external sort's configuration within `memory` bytes: a merge buffer `b` from 256 KiB
/// to 1 MiB, a fan-in `F` from 16 to [`MAX_FAN_IN`] with `F·b` at most a quarter of `memory`,
/// at most `threads` run-sorting threads (one per 64 MiB, each an 8 MB margin), and the largest
/// run `R` with `1.5·R + (F + 1)·b + threads·8 MB + heap ≤ memory`
/// ([`SortConfig::memory_bound`]), at least [`MIN_RUN_BYTES`].
fn sort_for(memory: u64, threads: usize) -> SortConfig {
    let buffer = (memory / 1024).clamp(256 << 10, 1 << 20);
    let fan_in = (memory / 4 / buffer).clamp(16, MAX_FAN_IN as u64);
    let threads = threads.min((memory >> 26).max(1) as usize).max(1);
    let fixed = (fan_in + 1) * buffer + threads as u64 * 8_000_000 + fan_in * 96;
    let run = (memory.saturating_sub(fixed) * 2 / 3).max(MIN_RUN_BYTES);
    SortConfig {
        run_bytes: usize::try_from(run).unwrap_or(usize::MAX),
        fan_in: fan_in as usize,
        buffer_bytes: buffer as usize,
        jobs: threads,
        // No background run writers: the plan's bound is `1.5·R`, without `w·1.5·R` more.
        writers: 0,
    }
}

/// The smallest planned sort run.
const MIN_RUN_BYTES: u64 = 2 << 20;

fn host(bytes: u64) -> usize {
    usize::try_from(bytes).unwrap_or(usize::MAX)
}

impl Plan {
    /// Resolves the request's budget and jobs against the detected host.
    pub(crate) fn resolve(
        requested: Option<u64>,
        jobs: Option<NonZeroUsize>,
    ) -> Result<Self, Error> {
        Self::resolve_with(
            requested,
            jobs,
            crate::mesh_large::available_memory(),
            std::thread::available_parallelism().map_or(1, NonZeroUsize::get),
        )
    }

    pub(crate) fn resolve_with(
        requested: Option<u64>,
        jobs: Option<NonZeroUsize>,
        detected: Option<u64>,
        cores: usize,
    ) -> Result<Self, Error> {
        let budget =
            MemoryBudget::resolve_with(requested, detected).map_err(|error| {
                match error.kind() {
                    crate::JobErrorKind::InvalidRequest => invalid_request(error.message()),
                    _ => Error::ResourceLimit(LimitRefusal::from(error.message().to_owned())),
                }
            })?;
        let planned = budget.ceiling(detected);
        // An explicit budget is never raised: one that cannot hold the reserve and the
        // smallest plan is refused before the input is opened.
        if budget.source == BudgetSource::Explicit && planned < RESERVE_BYTES + MIN_PLAN_BYTES {
            return Err(Error::ResourceLimit(LimitRefusal::from(format!(
                "validation needs a memory budget of at least {} bytes (512 MiB); {} bytes are \
                 available to plan against",
                RESERVE_BYTES + MIN_PLAN_BYTES,
                planned
            ))));
        }
        let share = planned.saturating_sub(RESERVE_BYTES).max(MIN_PLAN_BYTES);
        let jobs_plan = crate::mesh_large::plan_jobs(
            match jobs {
                Some(n) => crate::mesh_large::Jobs::Fixed(n),
                None => crate::mesh_large::Jobs::Auto,
            },
            cores,
            Some(share),
            0,
            // Workers take at most half the share; the stores keep the rest.
            (share / 2).max(MIN_PLAN_BYTES),
            JOB_BYTES,
        );
        let workers = usize::try_from(jobs_plan.jobs).unwrap_or(1).max(1);
        // P: the share left after every worker beyond the first is admitted.
        let p = share
            .saturating_sub((workers as u64 - 1) * JOB_BYTES)
            .max(MIN_PLAN_BYTES);
        // Live together at the peak (the delivery stage): catalog (resident P/8 or a P/32
        // page cache), facts P/8, stream log P/32, payload-row sort P/16, one delivery sort
        // P/4, SQLite (page cache P/8 plus a sorter list of at most the same size) and its
        // temporary store P/16: under 0.9·P. No term keeps a fixed 0.4.0 floor: every store
        // returns the same records resident or spilled, so a small plan only spills more.
        // At most four workers sort one delivery run. The catalog's side sorts and the
        // payload-row passes, never live beside them, sort on one thread: an in-memory sort
        // of one chunk then needs no scratch (`SortedStream::into_sorted_vec`).
        let delivery_sort = sort_for(p / 4, workers.min(4));
        let side_sort = sort_for(p / 4, 1);
        let catalog = CatalogConfig {
            // Admission only: the row and extent sorts are live together, nothing else is.
            sort: sort_for(p / 2, 1),
            side_sort,
            rows_sort: sort_for(p / 16, 1),
            resident_bytes: p / 8,
            max_fences: CatalogConfig::DEFAULT.max_fences,
            cache_bytes: p / 32,
            buffer_bytes: CatalogConfig::DEFAULT.buffer_bytes,
            facts_resident_bytes: p / 8,
            stream_log_resident_bytes: host(p / 32),
        };
        let sqlite_cache_bytes = p / 8;
        let sqlite_temp_bytes = p / 16;
        let report = ValidationResources {
            memory_budget: BudgetReport {
                requested_bytes: budget.requested_bytes,
                resolved_bytes: budget.resolved_bytes,
                source: budget.source,
                detected_headroom_bytes: budget.detected_headroom_bytes,
                planned_bytes: planned,
            },
            jobs: JobsReport {
                requested: jobs_plan.requested,
                cores: jobs_plan.cores,
                jobs: workers as u64,
                limited_by: jobs_plan.limited_by,
            },
            stores: StorePlan {
                sort_run_bytes: delivery_sort.run_bytes as u64,
                sort_fan_in: delivery_sort.fan_in as u64,
                sort_buffer_bytes: delivery_sort.buffer_bytes as u64,
                single_pass_bytes: (delivery_sort.run_bytes as u64)
                    .saturating_mul(delivery_sort.fan_in as u64),
                catalog_cache_bytes: catalog.cache_bytes,
                sqlite_cache_bytes,
                // Decided at the delivery store from the resolved work limits.
                sqlite_temp_store: "file",
            },
            disk: None,
        };
        Ok(Self {
            catalog,
            delivery_sort,
            jobs: workers,
            sqlite_cache_bytes,
            sqlite_temp_bytes,
            // Half of each further worker's reservation; the rest is its decode workspace.
            window_bytes: (workers as u64 - 1).saturating_mul(JOB_BYTES / 2),
            report,
        })
    }

    /// The SQLite temporary-store bound of the delivery store: every row it can sort or keep in
    /// a temporary table is a charged delivery row (`ROW_UNITS` work units each), a tile or a
    /// reference, so `(elements ÷ ROW_UNITS + visits + references) × 256 B` bounds it.
    pub(crate) fn temp_store_in_memory(
        &self,
        elements: u64,
        row_units: u64,
        visits: u64,
        references: u64,
    ) -> bool {
        (elements / row_units.max(1))
            .saturating_add(visits)
            .saturating_add(references)
            .saturating_mul(TEMP_ROW_BYTES)
            <= self.sqlite_temp_bytes
    }
}

/// The delivery store's scratch per declared payload triangle, for the start-of-run disk
/// bound (review P2-1). Its rows are geometry rows (points, segments, faces, requirements,
/// border images), so the bound follows declared triangles, not stored bytes: texture bytes
/// add no rows. Measured peaks (scratch directory plus SQLite temporary files): 165 B on
/// Walem 2048, about 160 B on a 4x Walem archive and 244 B on two 1.3 M-triangle crops.
///
/// Below `FLOOR` bytes per triangle, a tenth of the smallest measured peak, the volume cannot
/// hold the store: refused. Below `ESTIMATE` it may not: a warning, and validation goes on.
/// Neither is a proven bound; the floor's margin covers stores far smaller than measured.
pub(crate) const HLOD_FLOOR_BYTES_PER_TRIANGLE: u64 = 16;
pub(crate) const HLOD_ESTIMATE_BYTES_PER_TRIANGLE: u64 = 256;

/// The HLOD floor and estimate per declared triangle, or a unit test's override on this thread.
pub(crate) fn hlod_bytes_per_triangle() -> (u64, u64) {
    #[cfg(test)]
    if let Some(factors) = HLOD_FACTORS.with(std::cell::Cell::get) {
        return factors;
    }
    (
        HLOD_FLOOR_BYTES_PER_TRIANGLE,
        HLOD_ESTIMATE_BYTES_PER_TRIANGLE,
    )
}
#[cfg(test)]
thread_local! {
    static HLOD_FACTORS: std::cell::Cell<Option<(u64, u64)>> = const { std::cell::Cell::new(None) };
}

/// Triangles a GLB member declares: index (or position) count ÷ 3 of every triangle-list
/// primitive in its JSON chunk, read from the member's first block (`head`). A member whose
/// chunk is not entirely in `head`, or does not parse, declares none: the count only sizes the
/// disk bound, and an undercount can only lower the refusal floor.
pub(crate) fn declared_triangles(head: &[u8]) -> u64 {
    let Some(length) = head
        .get(..20)
        .filter(|h| h[..4] == *b"glTF" && h[16..20] == *b"JSON")
        .map(|h| u32::from_le_bytes([h[12], h[13], h[14], h[15]]) as usize)
    else {
        return 0;
    };
    let Some(Ok(doc)) = head
        .get(20..20usize.saturating_add(length))
        .map(serde_json::from_slice::<serde_json::Value>)
    else {
        return 0;
    };
    let accessors = &doc["accessors"];
    let mut triangles = 0u64;
    for primitive in doc["meshes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|mesh| mesh["primitives"].as_array())
        .flatten()
    {
        if primitive.get("mode").is_some_and(|m| m != 4) {
            continue;
        }
        let accessor = primitive
            .get("indices")
            .or_else(|| primitive["attributes"].get("POSITION"))
            .and_then(serde_json::Value::as_u64);
        let count = accessor
            .and_then(|a| accessors[a as usize]["count"].as_u64())
            .unwrap_or(0);
        triangles = triangles.saturating_add(count / 3);
    }
    triangles
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    fn plan(budget: Option<u64>, jobs: Option<usize>, detected: Option<u64>) -> Plan {
        Plan::resolve_with(budget, jobs.and_then(NonZeroUsize::new), detected, 16).unwrap()
    }

    #[test]
    fn default_budget_follows_the_mesh_rule_and_is_reported() {
        let p = plan(None, None, Some(64 * GIB));
        let b = p.report.memory_budget;
        assert_eq!(
            (b.resolved_bytes, b.source),
            (4 * GIB, BudgetSource::DefaultCeiling)
        );
        assert_eq!(b.planned_bytes, 4 * GIB);
        let p = plan(None, None, Some(6 * GIB));
        assert_eq!(
            (
                p.report.memory_budget.resolved_bytes,
                p.report.memory_budget.source
            ),
            (3 * GIB, BudgetSource::DefaultHeadroom)
        );
        let p = plan(Some(12 * GIB), None, Some(8 * GIB));
        assert_eq!(p.report.memory_budget.planned_bytes, 8 * GIB);
        let p = plan(None, None, None);
        assert_eq!(
            p.report.memory_budget.source,
            BudgetSource::DefaultUndetected
        );
    }

    #[test]
    fn explicit_budgets_below_the_floor_and_zero_are_refused() {
        let error = Plan::resolve_with(Some(0), None, Some(GIB), 4).unwrap_err();
        assert_eq!(error.category().0, "usage");
        let error = Plan::resolve_with(Some(511 << 20), None, Some(64 * GIB), 4).unwrap_err();
        assert_eq!(error.category().0, "resource_limit");
        // The budget is planned against the detected headroom, which may be lower.
        let error = Plan::resolve_with(Some(4 * GIB), None, Some(300 << 20), 4).unwrap_err();
        assert_eq!(error.category().0, "resource_limit");
        assert!(Plan::resolve_with(Some(512 << 20), None, Some(64 * GIB), 4).is_ok());
        // A default budget on a small host uses the smallest plan rather than failing.
        assert!(Plan::resolve_with(None, None, Some(256 << 20), 4).is_ok());
    }

    /// Every store's declared memory at the delivery peak fits the planned share.
    #[test]
    fn stores_fit_the_plan_at_every_budget_and_job_count() {
        for budget in [
            512 << 20,
            640 << 20,
            768 << 20,
            GIB,
            3 * GIB / 2,
            2 * GIB,
            4 * GIB,
            16 * GIB,
        ] {
            for jobs in [None, Some(1), Some(2), Some(3), Some(8), Some(64)] {
                let p = plan(Some(budget), jobs, Some(128 * GIB));
                let c = p.catalog;
                let workers = p.jobs as u64;
                // The replay window charges each job its stored bytes and twice the job decode
                // workspace; a job alone may exceed it by that workspace (review P2-6).
                let replay = if workers > 1 {
                    p.window_bytes + 2 * super::super::hlod_delivery::JOB_DECODE_BYTES as u64
                } else {
                    0
                };
                assert!(replay <= (workers - 1) * JOB_BYTES, "{budget} {jobs:?}");
                let stores = c.resident_bytes.max(c.cache_bytes + 24 * c.max_fences)
                    + c.facts_resident_bytes
                    + c.stream_log_resident_bytes as u64
                    + c.rows_sort.memory_bound::<crate::archive3tz::Row>()
                    + p.delivery_sort.memory_bound::<crate::archive3tz::Row>()
                    + 2 * p.sqlite_cache_bytes
                    + p.sqlite_temp_bytes;
                let share = budget - RESERVE_BYTES;
                let live = stores + (workers - 1) * JOB_BYTES;
                assert!(live <= share, "{budget} {jobs:?}: {live} > {share}");
                // Admission: the catalog's two sorts, nothing else.
                let admission = c.sort.memory_bound::<crate::archive3tz::Row>()
                    + c.side_sort.memory_bound::<crate::archive3tz::Row>();
                assert!(admission <= share, "{budget} {jobs:?}");
                assert!((16..=MAX_FAN_IN).contains(&c.side_sort.fan_in));
            }
        }
        // From 4 GiB every store is at least its 0.4.0 fixed size.
        let p = plan(Some(4 * GIB), Some(1), Some(128 * GIB));
        let old = CatalogConfig::DEFAULT;
        assert!(p.catalog.resident_bytes >= old.resident_bytes);
        assert!(p.catalog.facts_resident_bytes >= old.facts_resident_bytes);
        assert!(p.catalog.cache_bytes >= old.cache_bytes);
        assert!(p.delivery_sort.run_bytes >= old.side_sort.run_bytes);
    }

    /// A few-GiB budget sorts the delivery rows of a 26 GB HLOD archive (about 150 GB of
    /// 61-byte rows) in one merge pass per 64 GiB of its largest sort; 4 GiB covers > 60 GiB.
    #[test]
    fn single_pass_capacity_grows_with_the_budget() {
        let small = plan(Some(GIB), Some(1), Some(64 * GIB)).report.stores;
        let large = plan(Some(4 * GIB), Some(1), Some(64 * GIB)).report.stores;
        assert!(large.single_pass_bytes > 60 * GIB, "{large:?}");
        assert!(small.single_pass_bytes > 10 * GIB, "{small:?}");
        assert!(large.single_pass_bytes > 3 * small.single_pass_bytes);
        // The 0.4.0 profile merged 32 runs of 24 MiB in one pass.
        let old = CatalogConfig::DEFAULT.side_sort;
        assert!(small.single_pass_bytes > 10 * (old.run_bytes * old.fan_in) as u64);
    }

    #[test]
    fn jobs_are_capped_by_cores_and_memory_and_reported() {
        let p = plan(Some(4 * GIB), None, Some(64 * GIB));
        assert_eq!((p.jobs, p.report.jobs.limited_by), (16, "cores"));
        let p = plan(Some(4 * GIB), Some(3), Some(64 * GIB));
        assert_eq!(
            (
                p.jobs,
                p.report.jobs.limited_by,
                p.report.jobs.requested.as_str()
            ),
            (3, "request", "3")
        );
        let p = plan(Some(GIB), Some(64), Some(64 * GIB));
        assert_eq!(p.report.jobs.limited_by, "memory");
        assert!(p.jobs < 16);
        // The smallest budget leaves no room for a further worker.
        let p = plan(Some(512 << 20), None, Some(64 * GIB));
        assert_eq!((p.jobs, p.report.jobs.limited_by), (1, "floor"));
    }

    /// One triangle, three positions and no indices.
    fn triangle_glb() -> Vec<u8> {
        let doc = serde_json::json!({
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "buffers":[{"byteLength":36}],"bufferViews":[{"buffer":0,"byteLength":36}],
            "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":3,
                "min":[0,0,0],"max":[1,1,0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]
        });
        let positions: Vec<u8> = [0f32, 0., 0., 1., 0., 0., 0., 1., 0.]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        crate::glb::encode_glb(&doc, &positions).unwrap()
    }

    #[test]
    fn declared_triangles_read_the_json_chunk_only() {
        assert_eq!(declared_triangles(&triangle_glb()), 1);
        let mut points = triangle_glb();
        // Mode 0 (points) declares no triangles; neither do truncated or foreign heads.
        let at = points
            .windows(14)
            .position(|w| w == b"\"attributes\":{")
            .unwrap();
        points.splice(at..at, b"\"mode\":0,".iter().copied());
        assert_eq!(declared_triangles(&points), 0);
        assert_eq!(declared_triangles(&triangle_glb()[..30]), 0);
        assert_eq!(declared_triangles(b"PK\x03\x04 not a glb at all"), 0);
    }

    /// The start-of-run disk bound runs before traversal (review P2-1): below the floor a
    /// short volume is refused as typed insufficient space (`io`) naming the validation
    /// preflight; between the floor and the estimate a warning is emitted and the archive
    /// gets its own verdict; texture bytes declare no triangles and add nothing.
    #[test]
    fn a_short_scratch_volume_is_refused_before_traversal_and_a_tight_one_warned() {
        use super::super::i1_fixtures::{package, tileset, Files};
        use super::super::{inspect_with, ExpectedProfile, ValidationRequest};
        use serde_json::json;
        let files: Files = [
            (
                "tileset.json".to_string(),
                tileset(json!({
                "boundingVolume":{"box":[0,0,0,1,0,0,0,1,0,0,0,1]},
                "geometricError":0,"content":{"uri":"p.glb"}})),
            ),
            ("p.glb".to_string(), triangle_glb()),
        ]
        .into();
        let (_work, path) = package(&files);
        type Seen = std::sync::Arc<std::sync::Mutex<Vec<String>>>;
        struct Events(Seen);
        impl crate::Observer for Events {
            fn observe(&self, event: &crate::RunEvent<'_>) -> Result<(), crate::JobError> {
                match event {
                    crate::RunEvent::Progress { phase, done: 0, .. } => {
                        self.0.lock().unwrap().push((*phase).into())
                    }
                    crate::RunEvent::Warning { code, .. } => {
                        self.0.lock().unwrap().push(format!("warning:{code}"))
                    }
                    _ => {}
                }
                Ok(())
            }
        }
        let run = |factors: Option<(u64, u64)>| {
            HLOD_FACTORS.with(|f| f.set(factors));
            let seen: Seen = Default::default();
            let control = crate::RunControl::new(Some(std::sync::Arc::new(Events(seen.clone()))));
            let result = inspect_with(
                ValidationRequest::new(&path).expect_profile(ExpectedProfile::Hlod),
                &control,
            );
            HLOD_FACTORS.with(|f| f.set(None));
            let seen = seen.lock().unwrap().clone();
            (result, seen)
        };
        let (refused, seen) = run(Some((u64::MAX, u64::MAX)));
        let error = refused.unwrap_err();
        assert_eq!(error.category(), ("io", 1), "{error}");
        assert!(
            matches!(
                &error,
                Error::Scratch(super::super::ScratchRefusal::InsufficientSpace { .. })
            ),
            "{error:?}"
        );
        assert!(
            error.to_string().contains("validation preflight"),
            "{error}"
        );
        assert_eq!(seen, ["validate.hash"], "refused before traversal");
        // Room for the floor, not the estimate: warned, then the archive's own verdict.
        let (verdict, seen) = run(Some((0, u64::MAX)));
        let error = verdict.unwrap_err();
        assert!(error.to_string().contains("hlodFeatureIds"), "{error}");
        assert_eq!(
            &seen[..3],
            [
                "validate.hash",
                "warning:scratch_space",
                "validate.traverse"
            ]
        );
        // The production factors: no warning on this volume.
        let (verdict, seen) = run(None);
        assert!(verdict.unwrap_err().to_string().contains("hlodFeatureIds"));
        assert!(!seen.iter().any(|s| s.starts_with("warning:")));
    }

    #[test]
    fn temp_store_is_memory_only_when_the_row_bound_fits() {
        let p = plan(Some(16 * GIB), Some(1), Some(64 * GIB));
        // At 256 B a row, the floor work budget (16 M elements) of a small archive fits 16 GiB.
        assert!(p.temp_store_in_memory(16_000_000, 128, 1 << 18, 1 << 20));
        // A 1 GB archive's scaled budget (about 77 G elements) does not.
        assert!(!p.temp_store_in_memory(77_000_000_000, 128, 1 << 22, 1 << 24));
        // ... but not the 4 GiB default's P/16.
        let p = plan(Some(4 * GIB), Some(1), Some(64 * GIB));
        assert!(!p.temp_store_in_memory(16_000_000, 128, 1 << 18, 1 << 20));
    }
}
