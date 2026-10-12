//! Resolved configuration, the declared memory bound, the stage-1 scratch bound, the 3TZ member
//! bounds and the memory-aware jobs plan. Every figure here is configuration or a count from
//! the admitted document; none depends on payload bytes.
use super::records::{CornerPosition, CornerRef, NodeRecord, Span, TriangleRecord};
use crate::spill::{Record, SortConfig};
use serde::Serialize;
use std::{num::NonZeroUsize, path::PathBuf};

/// The per-tile member byte budget `B_tile` (design §2.5, E39).
pub(crate) const TILE_BYTES: u64 = 96 << 20;
/// P's hierarchy cap: at most 32 levels (design §2.4 B3).
pub(crate) const HEIGHT_CAP: u32 = 32;
/// The accepted 3TZ member ceiling (U2, design §9.2).
pub(crate) const MEMBER_CEILING_3TZ: u64 = 178_956_970;
/// Members besides tiles that every archive holds at least: one subtree, one document and the
/// three fixed members (design §2.5). The exact subtree and document counts belong to the
/// forest writer; both stages here use these lower terms.
pub(crate) const MEMBERS_BESIDES_TILES: u64 = 1 + 1 + 3;

/// Output profiles. The leaf triangle budget `L` comes from the profile; there is no separate
/// leaf-triangle option (design §8; `--leaf-triangles` is retired).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Profile {
    Fast,
    Balanced,
    Quality,
    Lossless,
}

impl Profile {
    pub(crate) fn leaf_triangles(self) -> u64 {
        match self {
            Self::Fast => 32_768,
            Self::Balanced => 16_384,
            Self::Quality | Self::Lossless => 8_192,
        }
    }
}

/// `--jobs`: `auto` (the default) or an explicit worker count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Jobs {
    #[default]
    Auto,
    Fixed(NonZeroUsize),
}

/// Capture, sort and cell-tree configuration.
#[derive(Clone, Debug)]
pub(crate) struct TreeConfig {
    pub profile: Profile,
    pub jobs: Jobs,
    /// `--scratch-dir`; the system temporary directory when absent.
    pub scratch_dir: Option<PathBuf>,
    /// `R` is the total run budget shared by the two sorters that are live at once (one
    /// streaming into the next); each gets `R / 2`. `F` and `b` apply to each. `jobs` is ignored
    /// here and resolved by the jobs plan.
    pub sort: SortConfig,
    pub(crate) leaf_triangles: u64,
    pub(crate) tile_bytes: u64,
    pub(crate) height_cap: u32,
    pub(crate) member_ceiling: u64,
}

impl TreeConfig {
    pub(crate) fn validate(&self) -> Result<(), crate::JobError> {
        let invalid = |message| crate::JobError::new(crate::JobErrorKind::InvalidRequest, message);
        if self.leaf_triangles == 0 || self.tile_bytes == 0 {
            return Err(invalid(
                "tree triangle and geometry-byte budgets must be positive",
            ));
        }
        if self.height_cap == 0 || self.height_cap > HEIGHT_CAP {
            return Err(invalid("tree height cap must be between 1 and 32"));
        }
        if self.sort.fan_in < 2 || self.sort.buffer_bytes == 0 {
            return Err(invalid(
                "sort fan-in must be at least two and buffers must be positive",
            ));
        }
        if self.sort.run_bytes / 2 < std::mem::size_of::<TriangleRecord>() {
            return Err(invalid(
                "shared sort run budget must hold two triangle records",
            ));
        }
        Ok(())
    }
    pub(crate) fn new(profile: Profile) -> Self {
        Self {
            profile,
            jobs: Jobs::Auto,
            scratch_dir: None,
            sort: SortConfig::default(),
            leaf_triangles: profile.leaf_triangles(),
            tile_bytes: TILE_BYTES,
            height_cap: HEIGHT_CAP,
            member_ceiling: MEMBER_CEILING_3TZ,
        }
    }

    pub(crate) fn leaf_triangles(&self) -> u64 {
        self.leaf_triangles
    }

    pub(crate) fn tile_bytes(&self) -> u64 {
        self.tile_bytes
    }

    /// One sorter's configuration with `jobs` workers.
    pub(crate) fn sorter(&self, jobs: usize) -> SortConfig {
        SortConfig {
            run_bytes: (self.sort.run_bytes / 2).max(1),
            fan_in: self.sort.fan_in,
            buffer_bytes: self.sort.buffer_bytes,
            jobs,
        }
    }
}

/// Test and bench seams for limits that production derives from the profile and format.
#[cfg(test)]
impl TreeConfig {
    pub(crate) fn with_leaf_triangles(mut self, leaf_triangles: u64) -> Self {
        self.leaf_triangles = leaf_triangles;
        self
    }
    pub(crate) fn with_tile_bytes(mut self, tile_bytes: u64) -> Self {
        self.tile_bytes = tile_bytes;
        self
    }
    pub(crate) fn with_height_cap(mut self, height_cap: u32) -> Self {
        self.height_cap = height_cap;
        self
    }
    pub(crate) fn with_member_ceiling(mut self, member_ceiling: u64) -> Self {
        self.member_ceiling = member_ceiling;
        self
    }
}

/// Declared resident-memory bound of the capture, sort and cell-tree passes (design §2.6 terms
/// `M_base` (JSON plan), `M_sort` and `M_tree`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MemoryBound {
    /// `R`, shared by the two live sorters.
    pub run_budget_bytes: u64,
    pub fan_in: u64,
    pub stream_buffer_bytes: u64,
    /// Conservative JSON parse/typed-plan allowance, including allocation amplification.
    pub document_bytes: u64,
    /// Runtime, allocator and source descriptor/metadata reserve.
    pub runtime_bytes: u64,
    /// Bounded run descriptors and their scratch paths, for two sorters.
    pub descriptor_bytes: u64,
    /// Two live sorters at `R / 2` each, excluding their per-job terms.
    pub sort_bytes: u64,
    /// Accessor readers and sequence writers, `4 × b`, plus the O(depth) cell stack.
    pub stream_bytes: u64,
    /// Per-job term: each live sorter's sort scratch plus worker stack reserve.
    pub per_job_bytes: u64,
    pub jobs: u64,
    /// `document + sort + stream + jobs × per_job`.
    pub bound_bytes: u64,
}

/// Stable-sort scratch allowance per sorting thread, as in `SortConfig::memory_bound`.
const SORT_THREAD_BYTES: u64 = 8_000_000;
/// The cell stack: 22 open cells with at most 8 child summaries each, rounded up.
const TREE_STACK_BYTES: u64 = 256 << 10;

impl MemoryBound {
    pub(crate) fn new(config: &TreeConfig, json_bytes: u64, jobs: usize) -> Self {
        let sorter = config.sorter(jobs);
        let per_job = 2 * (SORT_THREAD_BYTES + (8 << 20));
        // `memory_bound` already holds `jobs × 8 MB`; it is moved to the per-job term.
        let sort = sorter
            .memory_bound::<TriangleRecord>()
            .saturating_sub((jobs as u64).saturating_mul(SORT_THREAD_BYTES))
            .saturating_mul(2);
        let b = config.sort.buffer_bytes as u64;
        let stream = b.saturating_mul(8).saturating_add(TREE_STACK_BYTES);
        let document = json_bytes.saturating_mul(128);
        let runtime = 64 << 20;
        let path_bytes = config.scratch_dir.as_ref().map_or_else(
            || crate::spill::Scratch::default_parent().as_os_str().len(),
            |path| path.as_os_str().len(),
        ) as u64;
        let descriptors = (config.sort.fan_in as u64)
            .saturating_mul(256)
            .saturating_mul(path_bytes.saturating_mul(4).saturating_add(1024));
        let bound = document
            .saturating_add(runtime)
            .saturating_add(descriptors)
            .saturating_add(sort)
            .saturating_add(stream)
            .saturating_add((jobs as u64).saturating_mul(per_job));
        Self {
            run_budget_bytes: config.sort.run_bytes as u64,
            fan_in: config.sort.fan_in as u64,
            stream_buffer_bytes: b,
            document_bytes: document,
            runtime_bytes: runtime,
            descriptor_bytes: descriptors,
            sort_bytes: sort,
            stream_bytes: stream,
            per_job_bytes: per_job,
            jobs: jobs as u64,
            bound_bytes: bound,
        }
    }

    pub(crate) fn base_bytes(&self) -> u64 {
        self.document_bytes
            .saturating_add(self.runtime_bytes)
            .saturating_add(self.descriptor_bytes)
            .saturating_add(self.sort_bytes)
            .saturating_add(self.stream_bytes)
    }
}

/// How `--jobs` was resolved; stated in the report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct JobsPlan {
    /// `"auto"` or the explicit count.
    pub requested: String,
    pub cores: u64,
    /// Memory available to this process, when the platform reports it.
    pub available_memory_bytes: Option<u64>,
    /// Bytes of memory-backed (tmpfs) scratch subtracted from the available memory.
    pub memory_backed_scratch_bytes: u64,
    pub base_memory_bytes: u64,
    pub per_job_memory_bytes: u64,
    pub jobs: u64,
    /// `request`, `cores`, `memory` or `floor` (memory below one job's need: one job).
    pub limited_by: &'static str,
}

/// `auto` is `min(cores, (available − base − tmpfs scratch) ÷ per-job need)`, at least 1. An
/// explicit count is used as given.
pub(crate) fn plan_jobs(
    requested: Jobs,
    cores: usize,
    available_memory: Option<u64>,
    memory_backed_scratch: u64,
    base: u64,
    per_job: u64,
) -> JobsPlan {
    let cores = cores.max(1) as u64;
    let usable = available_memory.map(|m| m.saturating_sub(memory_backed_scratch));
    let (jobs, limited_by, label) = match requested {
        Jobs::Fixed(n) => (n.get() as u64, "request", n.to_string()),
        Jobs::Auto => {
            let by_memory = usable.map(|m| m.saturating_sub(base) / per_job.max(1));
            match by_memory {
                Some(0) => (1, "floor", "auto".into()),
                Some(n) if n < cores => (n, "memory", "auto".into()),
                _ => (cores, "cores", "auto".into()),
            }
        }
    };
    JobsPlan {
        requested: label,
        cores,
        available_memory_bytes: available_memory,
        memory_backed_scratch_bytes: memory_backed_scratch,
        base_memory_bytes: base,
        per_job_memory_bytes: per_job,
        jobs,
        limited_by,
    }
}

/// Memory available to this process: `MemAvailable`, capped by a cgroup v2 `memory.max`
/// headroom when one applies. `None` where the platform does not report it.
pub(crate) fn available_memory() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        let available = meminfo
            .lines()
            .find_map(|line| line.strip_prefix("MemAvailable:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())?
            .saturating_mul(1024);
        Some(cgroup_headroom().map_or(available, |headroom| available.min(headroom)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn cgroup_headroom() -> Option<u64> {
    let membership = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?
        .trim();
    let root = std::path::Path::new("/sys/fs/cgroup");
    inherited_headroom(root, &root.join(path.trim_start_matches('/')))
}

#[cfg(target_os = "linux")]
fn inherited_headroom(root: &std::path::Path, leaf: &std::path::Path) -> Option<u64> {
    let mut result = None;
    let mut directory = leaf;
    loop {
        if !directory.starts_with(root) {
            return None;
        }
        if let Some(max) = std::fs::read_to_string(directory.join("memory.max"))
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
        {
            // Missing current usage cannot be treated as zero usage.
            let current = std::fs::read_to_string(directory.join("memory.current"))
                .ok()?
                .trim()
                .parse::<u64>()
                .ok()?;
            let headroom = max.saturating_sub(current);
            result = Some(result.map_or(headroom, |prior: u64| prior.min(headroom)));
        }
        if directory == root {
            return result;
        }
        directory = directory.parent()?;
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn inherited_cgroup_limits_cap_an_unlimited_child() {
        let root = tempfile::tempdir().unwrap();
        let leaf = root.path().join("child");
        std::fs::create_dir(&leaf).unwrap();
        std::fs::write(root.path().join("memory.max"), "1000").unwrap();
        std::fs::write(root.path().join("memory.current"), "750").unwrap();
        std::fs::write(leaf.join("memory.max"), "max").unwrap();
        assert_eq!(inherited_headroom(root.path(), &leaf), Some(250));
        std::fs::write(leaf.join("memory.max"), "100").unwrap();
        std::fs::write(leaf.join("memory.current"), "40").unwrap();
        assert_eq!(inherited_headroom(root.path(), &leaf), Some(60));
        std::fs::remove_file(leaf.join("memory.current")).unwrap();
        assert_eq!(inherited_headroom(root.path(), &leaf), None);
    }
}

/// Upper bound on the tile count from counts alone. With `X = ⌊N/(L+1)⌋ + ⌊ΣB/(B_tile+1)⌋`,
/// at most `X` disjoint cells per depth exceed a budget, so there are at most `22X` internal
/// cells and `8·22X + 1` leaves. Adjacent degenerate runs together exceed a budget, so there
/// are at most `3X + 4` runs, and at most `n/7 + 11` synthetic nodes above `n` runs per cell.
/// `256X + 16` covers the sum.
pub(crate) fn node_bound(triangles: u64, total_bytes: u64, config: &TreeConfig) -> u64 {
    let over = u128::from(triangles) / (u128::from(config.leaf_triangles) + 1)
        + u128::from(total_bytes) / (u128::from(config.tile_bytes) + 1);
    (over * 256 + 16).min(u128::from(u64::MAX)) as u64
}

/// Stage-1 scratch need (design §2.7): the largest set of spill files live at once over the
/// capture, sort and cell-tree passes, from declared counts. Each sort holds two generations
/// of its runs while merging and one while it streams into the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ScratchNeed {
    pub corner_sort_bytes: u64,
    pub position_sort_bytes: u64,
    pub triangle_sort_bytes: u64,
    pub tree_output_bytes: u64,
    pub weld_sort_bytes: u64,
    pub need_bytes: u64,
}

pub(crate) fn scratch_need(triangles: u64, total_bytes: u64, config: &TreeConfig) -> Option<u64> {
    scratch_need_parts(triangles, total_bytes, config).map(|need| need.need_bytes)
}

pub(crate) fn scratch_need_parts(
    triangles: u64,
    total_bytes: u64,
    config: &TreeConfig,
) -> Option<ScratchNeed> {
    let corners = triangles.checked_mul(3)?;
    let a = corners.checked_mul(CornerRef::SIZE as u64)?;
    let p = corners.checked_mul(CornerPosition::SIZE as u64)?;
    let t = triangles.checked_mul(TriangleRecord::SIZE as u64)?;
    let nodes = node_bound(triangles, total_bytes, config);
    // Cover all generations concurrently, rather than relying on iterator drop timing.
    // Four node/span inventories cover postorder, preorder sort generations and run spans.
    let tree = nodes
        .checked_mul((NodeRecord::SIZE + Span::SIZE) as u64)?
        .checked_mul(4)?;
    let weld = corners
        .checked_mul(22)?
        .checked_mul(2)?
        .checked_add(corners.checked_mul(13)?)?;
    let need = a
        .checked_mul(2)?
        .checked_add(p.checked_mul(2)?)?
        .checked_add(t.checked_mul(2)?)?
        .checked_add(tree)?
        .checked_add(weld)?;
    Some(ScratchNeed {
        corner_sort_bytes: a.checked_mul(2)?,
        position_sort_bytes: p.checked_mul(2)?,
        triangle_sort_bytes: t.checked_mul(2)?,
        tree_output_bytes: t.checked_add(tree)?,
        weld_sort_bytes: weld,
        need_bytes: need,
    })
}

/// Stage-1 sound lower bound on tiles: `⌈N/L⌉` leaves, plus the root when it must be internal.
pub(crate) fn tile_lower_bound(triangles: u64, leaf_triangles: u64) -> u64 {
    triangles
        .div_ceil(leaf_triangles)
        .saturating_add(u64::from(triangles > leaf_triangles))
}
