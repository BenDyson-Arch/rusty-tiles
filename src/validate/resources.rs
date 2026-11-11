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
/// Bytes of one SQLite temporary-store row bound: the record plus sorter overhead.
const TEMP_ROW_BYTES: u64 = 128;

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
    /// Estimated scratch peak: the catalog's terms plus, for an HLOD archive, the delivery
    /// store's (`hlodBytesPerStoredByte × S`).
    pub estimated_peak_bytes: u64,
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

/// One external sort's configuration within `memory` bytes: the fan-in `F` at most
/// [`MAX_FAN_IN`], a merge buffer `b` from 256 KiB to 1 MiB, and the largest run `R` with
/// `1.5·R + (F + 1)·b + jobs·8 MB + heap ≤ memory` ([`SortConfig::memory_bound`]). `floor`
/// keeps the 0.4.0 run size as the smallest run.
fn sort_for(memory: u64, jobs: usize, floor: SortConfig) -> SortConfig {
    let buffer = (memory / 1024).clamp(256 << 10, 1 << 20);
    let fan_in = MAX_FAN_IN as u64;
    let fixed = (fan_in + 1) * buffer + jobs as u64 * 8_000_000 + fan_in * 96;
    let run = (memory.saturating_sub(fixed) * 2 / 3).max(floor.run_bytes as u64);
    SortConfig {
        run_bytes: usize::try_from(run).unwrap_or(usize::MAX),
        fan_in: MAX_FAN_IN.max(floor.fan_in),
        buffer_bytes: buffer as usize,
        jobs,
    }
}

fn host(bytes: u64) -> usize {
    usize::try_from(bytes).unwrap_or(usize::MAX)
}

impl Plan {
    /// Resolves the request's budget and jobs against the detected host.
    pub(crate) fn resolve(
        requested: Option<u64>,
        jobs: Option<NonZeroUsize>,
    ) -> Result<Self, Error> {
        // Unit tests compare whole reports across runs: they see one fixed host.
        #[cfg(test)]
        let (detected, cores) = (Some(16 << 30), 4);
        #[cfg(not(test))]
        let (detected, cores) = (
            crate::mesh_large::available_memory(),
            std::thread::available_parallelism().map_or(1, NonZeroUsize::get),
        );
        Self::resolve_with(requested, jobs, detected, cores)
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
        let base = CatalogConfig::DEFAULT;
        // Live together at the peak (the delivery stage): catalog (resident P/8 or a P/32
        // page cache), facts P/8, stream log P/32, payload-row sort P/16, one delivery sort
        // P/4, SQLite (page cache P/8 plus a sorter list of at most the same size) and its
        // temporary store P/16: 0.9·P. At most four workers sort one delivery run (each
        // costs the sort an 8 MB margin). The catalog's side sorts and the payload-row
        // passes, never live beside them, sort on one thread: an in-memory sort of one
        // chunk then needs no scratch (`SortedStream::into_sorted_vec`).
        let delivery_sort = sort_for(p / 4, workers.min(4), base.side_sort);
        let side_sort = sort_for(p / 4, 1, base.side_sort);
        let catalog = CatalogConfig {
            // Admission only: the row and extent sorts are live together, nothing else is.
            sort: sort_for(p / 2, 1, base.sort),
            side_sort,
            rows_sort: sort_for(p / 16, 1, base.rows_sort),
            resident_bytes: (p / 8).max(base.resident_bytes),
            max_fences: base.max_fences,
            cache_bytes: (p / 32).max(base.cache_bytes),
            buffer_bytes: base.buffer_bytes,
            facts_resident_bytes: (p / 8).max(base.facts_resident_bytes),
            stream_log_resident_bytes: host((p / 32).max(base.stream_log_resident_bytes as u64)),
        };
        let sqlite_cache_bytes = (p / 8).max(8 << 20);
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
    /// reference, so `(elements ÷ ROW_UNITS + visits + references) × 128 B` bounds it.
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

/// Scratch bytes of the HLOD delivery store per admitted stored byte: its main database, the
/// external sort runs live beside it and SQLite's own temporary files. Measured on mesh-built
/// HLOD archives: a sampled peak of 2.7 (report 4.4) on Walem 2048 (0.97 GB) and at least
/// 5.5 (143 GB live after 58 min) on a 26 GB archive; the bound keeps a margin above both.
pub(crate) const HLOD_BYTES_PER_STORED_BYTE: u64 = 7;

/// [`HLOD_BYTES_PER_STORED_BYTE`], or a unit test's override on this thread.
pub(crate) fn hlod_bytes_per_stored_byte() -> u64 {
    #[cfg(test)]
    if let Some(factor) = HLOD_FACTOR.with(std::cell::Cell::get) {
        return factor;
    }
    HLOD_BYTES_PER_STORED_BYTE
}
#[cfg(test)]
thread_local! {
    static HLOD_FACTOR: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
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
        for budget in [512 << 20, GIB, 2 * GIB, 4 * GIB, 16 * GIB, 64 * GIB] {
            for jobs in [None, Some(1), Some(2), Some(8), Some(64)] {
                let p = plan(Some(budget), jobs, Some(128 * GIB));
                let c = p.catalog;
                let workers = p.jobs as u64;
                let live = c.resident_bytes.max(c.cache_bytes + 24 * c.max_fences)
                    + c.facts_resident_bytes
                    + c.stream_log_resident_bytes as u64
                    + c.rows_sort.memory_bound::<crate::archive3tz::Row>()
                    + p.delivery_sort.memory_bound::<crate::archive3tz::Row>()
                    + 2 * p.sqlite_cache_bytes
                    + p.sqlite_temp_bytes
                    // The replay window lies inside the further workers' reservation.
                    + (workers - 1) * JOB_BYTES;
                assert!(p.window_bytes <= (workers - 1) * JOB_BYTES);
                let share = (budget - RESERVE_BYTES).max(MIN_PLAN_BYTES);
                // Floors keep the 0.4.0 sizes, so the smallest plan may sit at the floor
                // profile; every larger plan fits its share.
                if budget >= 2 * GIB {
                    assert!(live <= share, "{budget} {jobs:?}: {live} > {share}");
                }
                assert!(c.side_sort.fan_in <= MAX_FAN_IN && c.side_sort.fan_in >= 32);
                assert!(c.side_sort.run_bytes >= CatalogConfig::DEFAULT.side_sort.run_bytes);
                assert!(c.cache_bytes >= CatalogConfig::DEFAULT.cache_bytes);
            }
        }
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

    /// The start-of-run disk bound runs before traversal and refuses a short volume as typed
    /// insufficient space (`io`), naming the validation preflight; with room, the bound is
    /// reported and the archive gets its own verdict.
    #[test]
    fn a_short_scratch_volume_is_refused_before_traversal() {
        use super::super::i1_fixtures::{package, point_glb, tileset, Files};
        use super::super::{inspect_with, ExpectedProfile, ValidationRequest};
        use serde_json::json;
        let files: Files = [
            (
                "tileset.json".to_string(),
                tileset(json!({
                "boundingVolume":{"box":[0,0,0,1,0,0,0,1,0,0,0,1]},
                "geometricError":0,"content":{"uri":"p.glb"}})),
            ),
            ("p.glb".to_string(), point_glb()),
        ]
        .into();
        let (_work, path) = package(&files);
        let run = |factor: Option<u64>| {
            HLOD_FACTOR.with(|f| f.set(factor));
            let phases = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
            struct Phases(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
            impl crate::Observer for Phases {
                fn observe(&self, event: &crate::RunEvent<'_>) -> Result<(), crate::JobError> {
                    if let crate::RunEvent::Progress { phase, done: 0, .. } = event {
                        self.0.lock().unwrap().push((*phase).into());
                    }
                    Ok(())
                }
            }
            let control = crate::RunControl::new(Some(std::sync::Arc::new(Phases(phases.clone()))));
            let result = inspect_with(
                ValidationRequest::new(&path).expect_profile(ExpectedProfile::Hlod),
                &control,
            );
            HLOD_FACTOR.with(|f| f.set(None));
            let phases = phases.lock().unwrap().clone();
            (result, phases)
        };
        let (refused, phases) = run(Some(u64::MAX));
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
        assert_eq!(phases, ["validate.hash"], "refused before traversal");
        // With room, the same archive gets its own verdict: no source identity.
        let (verdict, phases) = run(None);
        let error = verdict.unwrap_err();
        assert!(error.to_string().contains("hlodFeatureIds"), "{error}");
        assert!(phases.iter().any(|p| p == "validate.traverse"));
    }

    #[test]
    fn temp_store_is_memory_only_when_the_row_bound_fits() {
        let p = plan(Some(4 * GIB), Some(1), Some(64 * GIB));
        // The default floor work budget (16 M elements) of a small archive fits.
        assert!(p.temp_store_in_memory(16_000_000, 128, 1 << 18, 1 << 20));
        // A 1 GB archive's scaled budget (about 77 G elements) does not.
        assert!(!p.temp_store_in_memory(77_000_000_000, 128, 1 << 22, 1 << 24));
    }
}
