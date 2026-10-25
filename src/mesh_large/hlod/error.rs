//! D2 sampled source error, policy v2. This is deliberately not certification.
//! G=max(child G, 1.05 M). The 5% sampling margin exceeds the prototype's
//! ~1% independent maximum exceedance; it is not a covering-radius bound.
//! Source controls cover bounded patch boundaries, one vertex per patch component
//! and sliver vertices/midpoints. Indexed proxy vertices/edges are checked once.
use super::{
    resource,
    source_index::{self, Budget, Frontier, Hit, SourceQuery, Surface, Triangle, V3},
    state,
    store::{CoarseTile, TileError, TileKey},
};
use crate::{
    mesh_large::{full::FullCaptured, records::NodeKind},
    JobError,
};
use rustc_hash::FxHashSet as HashSet;
use serde::{Deserialize, Serialize};
pub(crate) const POLICY: &str = "hlod-s2-decoded-source-two-sided-v2";
pub(crate) const MIN_TILE: u64 = 2048;
pub(crate) const MIN_DEPTH: u64 = 200_000;
/// Histograms, vertex controls, stream buffers, coarse BVH, summaries and slack.
pub(crate) const WORK_BYTES: u64 = 4 << 20;
const BINS: usize = 16384;
/// Per-query work budget of every S2 nearest search (node visits plus triangle tests; see
/// `source_index::Budget`). Exact search is Θ(R) per query against R source triangles whose
/// boxes all contain it; budgeted, a query costs O(B + depth + block). On exhaustion the
/// best distance found is used: an overestimate, so M and G stay conservative. Exhaustions
/// are counted in the report. A scale-gate tuning constant, chosen by measurement
/// (2026-10-25): tex3/crop3 searches take at most 501 steps (none exhausted at 1024); the
/// adversarial chords input costs 28.0/17.6/9.6 S2 CPU-s at B = 4096/1024/512 with G
/// unchanged, and G grows 30% at 256.
pub(crate) const NEAREST_STEPS: u64 = 1024;
/// The searches' two frontiers and the paged entries for a budget of `steps`, each at most
/// `2B + 1` entries (two pushes per budgeted step), at up to twice that after doubling.
pub(crate) fn search_bytes(steps: u64) -> u64 {
    steps
        .saturating_mul(2)
        .saturating_add(1)
        .saturating_mul(2 * (16 + 16 + 128))
}

/// D2's sampling schedule: area samples per direction owed to each occupied encoded depth
/// and, at least, to each tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub per_depth: u64,
    pub per_tile: u64,
}
impl Default for Schedule {
    fn default() -> Self {
        Self::PRODUCTION
    }
}
impl Schedule {
    /// What every production bake uses and delivery requires.
    pub(crate) const PRODUCTION: Self = Self {
        per_depth: MIN_DEPTH,
        per_tile: MIN_TILE,
    };
    /// Delivery admits only D2's schedule. Library tests that do not exercise S2's
    /// measurement publish a reduced one (`Settings::sampling`), never production.
    pub(crate) fn admitted(self) -> bool {
        self == Self::PRODUCTION || (cfg!(test) && self.per_depth > 0 && self.per_tile > 0)
    }
    /// Samples per direction owed to `range` source triangles of a depth scheduled over
    /// `total`: its share of `per_depth`, at least `per_tile`. A depth whose tiles are all
    /// published gets at least `per_depth` in total. Omitted cells (D1) are not measured,
    /// so a depth with omissions keeps the same density over the source it publishes
    /// (`required` of the published ranges' sum), which delivery checks.
    pub(crate) fn required(self, range: u64, total: u64) -> Result<u64, JobError> {
        if total == 0 || range > total {
            return Err(resource("sampling range outside its depth's schedule"));
        }
        let count = (u128::from(self.per_depth) * u128::from(range)).div_ceil(u128::from(total));
        Ok(u64::try_from(count)
            .map_err(|_| resource("sampling count overflow"))?
            .max(self.per_tile))
    }
}

/// Counts of a tile's budgeted nearest searches (deterministic: a pure function of the tile).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchStats {
    pub searches: u64,
    /// Searches whose budget ran out (their best distance so far was used).
    pub exhausted: u64,
    /// Searches that began from their greedy seed (no candidate yet).
    pub seeded: u64,
    /// Most steps one search took (its seed included).
    pub max_steps: u64,
    /// Steps of every search (node visits and triangle tests, seeds included).
    pub steps: u64,
}
impl SearchStats {
    pub(crate) fn add(&mut self, o: Self) {
        self.searches += o.searches;
        self.steps += o.steps;
        self.exhausted += o.exhausted;
        self.seeded += o.seeded;
        self.max_steps = self.max_steps.max(o.max_steps);
    }
}

/// S2's budgeted nearest searches (representatives and both directed distances) with their
/// reused frontiers, charged within [`SEARCH_BYTES`].
pub(crate) struct Nearest {
    steps: u64,
    frontier: Frontier,
    inner: Frontier,
    pub stats: SearchStats,
}
impl Nearest {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_steps(NEAREST_STEPS)
    }
    pub(crate) fn with_steps(steps: u64) -> Self {
        Self {
            steps,
            frontier: Frontier::new(),
            inner: Frontier::new(),
            stats: SearchStats::default(),
        }
    }
    fn record(&mut self, budget: Budget) {
        self.stats.add(SearchStats {
            searches: 1,
            exhausted: u64::from(budget.exhausted),
            seeded: u64::from(budget.seeded),
            max_steps: budget.steps,
            steps: budget.steps,
        });
    }
    /// Nearest source triangle of the query's descendant range.
    pub(crate) fn source(&mut self, query: &mut SourceQuery<'_>, p: V3) -> Result<Hit, JobError> {
        let mut budget = Budget::new(self.steps);
        let hit = query.nearest_budget(p, &mut budget, &mut self.frontier, &mut self.inner)?;
        self.record(budget);
        Ok(hit)
    }
    /// Nearest triangle of a resident surface (the decoded coarse tile), from `hint`.
    pub(crate) fn surface(&mut self, surface: &Surface, p: V3, hint: &mut Option<usize>) -> Hit {
        let mut budget = Budget::new(self.steps);
        let hit = surface.nearest_budget_hint(p, all_key(), hint, &mut budget, &mut self.frontier);
        self.record(budget);
        hit
    }
}

/// Fixed-size scheduling by encoded depth, excluding collapsed single-coarse-child
/// cells. Each occupied encoded depth receives >=200k samples per direction.
pub(crate) struct SamplingPlan {
    totals: [u64; 32],
    schedule: Schedule,
}
impl SamplingPlan {
    pub(crate) fn new(captured: &FullCaptured, schedule: Schedule) -> Result<Self, JobError> {
        Self::from_nodes(
            captured.tree.nodes.reader()?.map(|n| n.map_err(Into::into)),
            schedule,
        )
    }
    fn from_nodes(
        nodes: impl IntoIterator<Item = Result<crate::mesh_large::records::NodeRecord, JobError>>,
        schedule: Schedule,
    ) -> Result<Self, JobError> {
        struct Frame {
            node: crate::mesh_large::records::NodeRecord,
            children: u32,
            only_internal: bool,
        }
        let mut plan = Self {
            totals: [0; 32],
            schedule,
        };
        let mut stack: Vec<Frame> = Vec::with_capacity(32);
        let mut close = |f: Frame| -> Result<(), JobError> {
            // Exactly matches walk::close: a single coarse child stands for this cell.
            if f.children != 1 || !f.only_internal {
                plan.add(f.node.depth(), f.node.end - f.node.first)?;
            }
            Ok(())
        };
        for n in nodes {
            let n = n?;
            if n.depth() >= 32 {
                return Err(resource("sampling depth outside 0..32"));
            }
            while stack.last().is_some_and(|f| f.node.end <= n.first) {
                close(stack.pop().unwrap())?;
            }
            if let Some(parent) = stack.last_mut() {
                parent.children += 1;
                parent.only_internal = n.kind == NodeKind::Internal;
            }
            if n.kind == NodeKind::Internal {
                stack.push(Frame {
                    node: n,
                    children: 0,
                    only_internal: false,
                });
            }
        }
        while let Some(frame) = stack.pop() {
            close(frame)?;
        }
        Ok(plan)
    }
    fn add(&mut self, depth: u32, count: u64) -> Result<(), JobError> {
        let total = self
            .totals
            .get_mut(depth as usize)
            .ok_or_else(|| resource("sampling depth outside 0..32"))?;
        *total = total
            .checked_add(count)
            .ok_or_else(|| resource("sampling schedule overflow"))?;
        Ok(())
    }
    pub(crate) fn count(&self, key: TileKey) -> Result<Count, JobError> {
        let total = *self
            .totals
            .get(key.depth as usize)
            .filter(|n| **n > 0)
            .ok_or_else(|| resource("sampling depth outside populated schedule"))?;
        let range = key
            .end
            .checked_sub(key.first)
            .filter(|n| *n > 0)
            .ok_or_else(|| resource("empty sampling source range"))?;
        Ok(Count {
            samples: self.schedule.required(range, total)?,
            depth_source_triangles: total,
            schedule: self.schedule,
        })
    }
}

/// One tile's schedule: its area samples per direction and the source triangles of every
/// encoded tile at its depth that the schedule shares out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Count {
    pub samples: u64,
    pub depth_source_triangles: u64,
    pub schedule: Schedule,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Distribution {
    pub samples: u64,
    pub mean: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}
struct Stats {
    bins: Vec<u64>,
    sum: f64,
    correction: f64,
    count: u64,
    max: f64,
}
impl Stats {
    fn new() -> Self {
        Self {
            bins: vec![0; BINS],
            sum: 0.,
            correction: 0.,
            count: 0,
            max: 0.,
        }
    }
    fn add(&mut self, d: f64) -> Result<(), JobError> {
        if !d.is_finite() || d < 0. {
            return Err(state("nonfinite sampled distance"));
        }
        self.count += 1;
        let y = d - self.correction;
        let total = self.sum + y;
        self.correction = (total - self.sum) - y;
        self.sum = total;
        self.max = self.max.max(d);
        let i = if d == 0. {
            0
        } else {
            ((d.log2() + 256.) * 32.)
                .ceil()
                .clamp(1., (BINS - 1) as f64) as usize
        };
        self.bins[i] += 1;
        Ok(())
    }
    fn summary(&self) -> Distribution {
        let quantile = |percent: u64| {
            let rank = self.count.saturating_mul(percent).div_ceil(100);
            let mut n = 0;
            for (i, c) in self.bins.iter().enumerate() {
                n += c;
                if n >= rank {
                    return if i == 0 {
                        0.
                    } else {
                        2f64.powf(i as f64 / 32. - 256.).min(self.max)
                    };
                }
            }
            self.max
        };
        Distribution {
            samples: self.count,
            mean: if self.count == 0 {
                0.
            } else {
                self.sum / self.count as f64
            },
            p95: quantile(95),
            p99: quantile(99),
            max: self.max,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Direction {
    pub area: Distribution,
    pub controls: Distribution,
    pub surface_area: f64,
}
impl Direction {
    fn max(self) -> f64 {
        self.area.max.max(self.controls.max)
    }
    fn p99(self) -> f64 {
        self.area.p99.max(self.controls.p99)
    }
}
/// S3 uses geometric_error for max(G/16,1e-5) chart spacing and keeps decoded
/// positions/locked vertices fixed. q is diagnostic only; no D10 calibration is applied.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub version: u32,
    pub seed: u64,
    pub area_samples_per_direction: u64,
    /// The schedule's source triangles at this tile's encoded depth (see
    /// [`Schedule::required`]).
    pub depth_source_triangles: u64,
    pub schedule: Schedule,
    pub source_to_proxy: Direction,
    pub proxy_to_source: Direction,
    pub sampled_max: f64,
    pub margin: f64,
    pub q: f64,
}
/// Bounded depth summaries in CLI diagnostics and the deterministic leaf manifest.
/// Quantiles below are worst tile quantiles, not pooled depth quantiles.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepthReport {
    pub tiles: u64,
    pub geometric_error: f64,
    pub sampled_max: f64,
    pub maximum_margin: f64,
    pub q: f64,
    pub source_area_samples: u64,
    pub proxy_area_samples: u64,
    pub source_controls: u64,
    pub proxy_controls: u64,
    pub source_mean: f64,
    pub proxy_mean: f64,
    pub source_p95_worst_tile: f64,
    pub proxy_p95_worst_tile: f64,
    pub source_p99_worst_tile: f64,
    pub proxy_p99_worst_tile: f64,
    pub nearest: SearchStats,
}
impl DepthReport {
    pub(crate) fn add(&mut self, s: Summary, g: f64, nearest: SearchStats) {
        self.nearest.add(nearest);
        let source = s.source_to_proxy;
        let proxy = s.proxy_to_source;
        self.source_mean = (self.source_mean * self.source_area_samples as f64
            + source.area.mean * source.area.samples as f64)
            / (self.source_area_samples + source.area.samples) as f64;
        self.proxy_mean = (self.proxy_mean * self.proxy_area_samples as f64
            + proxy.area.mean * proxy.area.samples as f64)
            / (self.proxy_area_samples + proxy.area.samples) as f64;
        self.tiles += 1;
        self.geometric_error = self.geometric_error.max(g);
        self.sampled_max = self.sampled_max.max(s.sampled_max);
        self.maximum_margin = self.maximum_margin.max(s.margin);
        self.q = self.q.max(s.q);
        self.source_area_samples += source.area.samples;
        self.proxy_area_samples += proxy.area.samples;
        self.source_controls += source.controls.samples;
        self.proxy_controls += proxy.controls.samples;
        self.source_p95_worst_tile = self.source_p95_worst_tile.max(source.area.p95);
        self.proxy_p95_worst_tile = self.proxy_p95_worst_tile.max(proxy.area.p95);
        self.source_p99_worst_tile = self.source_p99_worst_tile.max(source.area.p99);
        self.proxy_p99_worst_tile = self.proxy_p99_worst_tile.max(proxy.area.p99);
    }
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorReport {
    pub policy: &'static str,
    pub seed_rule: &'static str,
    /// Per-query budget of the nearest searches ([`NEAREST_STEPS`] in production).
    pub nearest_steps: u64,
    pub schedule: Schedule,
    pub nearest: SearchStats,
    pub by_depth: Vec<DepthReport>,
    pub root: Option<Summary>,
}
impl ErrorReport {
    pub(crate) fn new(nearest_steps: u64, schedule: Schedule) -> Self {
        Self {
            policy: POLICY,
            seed_rule: "SplitMix64; 0x5332202610240001 xor first*0x9e3779b97f4a7c15 xor \
                        rotl(end,23) xor depth; P-to-S xor 0xa5a5a5a5a5a5a5a5",
            nearest_steps,
            schedule,
            ..Default::default()
        }
    }
}

/// Diagnostic CPU shares: index uses the process clock before workers start; each
/// other term sums worker-thread user+system CPU. Unavailable clocks report 0 here
/// and None in the enclosing total, never worker wall seconds.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct CpuReport {
    pub index: f64,
    pub representatives: f64,
    pub proxy_preparation: f64,
    pub source_scan: f64,
    pub source_to_proxy: f64,
    pub proxy_to_source: f64,
}
impl CpuReport {
    pub(crate) fn add(&mut self, c: Self) {
        self.index += c.index;
        self.representatives += c.representatives;
        self.proxy_preparation += c.proxy_preparation;
        self.source_scan += c.source_scan;
        self.source_to_proxy += c.source_to_proxy;
        self.proxy_to_source += c.proxy_to_source;
    }
}
fn elapsed(start: Option<f64>) -> f64 {
    thread_cpu().zip(start).map_or(0., |(b, a)| b - a)
}

struct Rng(u64);
impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        ((z ^ (z >> 31)) >> 11) as f64 * (1. / ((1u64 << 53) as f64))
    }
}
pub(crate) fn seed(key: TileKey) -> u64 {
    0x5332_2026_1024_0001
        ^ key.first.wrapping_mul(0x9e3779b97f4a7c15)
        ^ key.end.rotate_left(23)
        ^ u64::from(key.depth)
}
fn all_key() -> TileKey {
    TileKey {
        first: 0,
        end: u64::MAX,
        depth: 0,
        leaf_begin: 0,
        leaf_end: 0,
    }
}

#[derive(Clone, Copy)]
struct Controls<'a> {
    proxy: bool,
    initial: &'a [f64],
}
impl Controls<'_> {
    const SOURCE: Self = Self {
        proxy: false,
        initial: &[],
    };
    const PROXY: Self = Self {
        proxy: true,
        initial: &[],
    };
}
fn directed<I>(
    triangles: I,
    total: f64,
    count: u64,
    seed: u64,
    policy: Controls<'_>,
    mut distance: impl FnMut(V3) -> Result<f64, JobError>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Direction, JobError>
where
    I: Iterator<Item = Result<Triangle, JobError>>,
{
    let (mut samples, mut controls) = (Stats::new(), Stats::new());
    let mut rng = Rng(seed);
    let mut vertices = HashSet::default();
    let mut seen_edges = HashSet::default();
    for &d in policy.initial {
        controls.add(d)?;
    }
    let mut triangles = triangles.peekable();
    let (mut cumulative, mut next, mut i) = (0., rng.unit() / count as f64 * total, 0u64);
    let mut last = None;
    let mut last_positive = None;
    let mut area_correction = 0.;
    let mut ordinal = 0usize;
    while let Some(t) = triangles.next() {
        let t = t?;
        if ordinal.is_multiple_of(256) {
            check()?;
            if !policy.proxy {
                vertices.clear();
                seen_edges.clear();
            }
        }
        let p = t.positions();
        last = Some(p);
        let area = t.area();
        let y = area - area_correction;
        let added = cumulative + y;
        area_correction = (added - cumulative) - y;
        cumulative = added;
        if area > 0. {
            last_positive = Some(p);
        }
        // Cached subtree sums can differ from stream accumulation by roundoff.
        // Correct only that bounded endpoint uncertainty. A genuinely incomplete
        // schedule still refuses; never assign its remainder to the last face.
        let endpoint = if triangles.peek().is_none()
            && (total - cumulative).abs() <= 1024. * f64::EPSILON * total
        {
            total
        } else {
            cumulative
        };
        let sample_triangle = if area > 0. {
            p
        } else {
            last_positive.unwrap_or(p)
        };
        while i < count && total > 0. && next < endpoint {
            if i.is_multiple_of(256) {
                check()?;
            }
            let r = rng.unit().sqrt();
            let s = rng.unit();
            let point = source_index::bary(sample_triangle, [1. - r, r * (1. - s), r * s]);
            samples.add(distance(point)?)?;
            i += 1;
            if i < count {
                next = ((i as f64 + rng.unit()) / count as f64 * total).min(total.next_down());
            }
        }
        for (corner, bits) in t.bits.into_iter().enumerate() {
            if (policy.proxy || t.controls & (1 << corner) != 0) && vertices.insert(bits) {
                controls.add(distance(bits.map(|x| f64::from(f32::from_bits(x))))?)?;
            }
        }
        let edges = [(0, 1), (1, 2), (2, 0)];
        let max_edge = edges
            .iter()
            .map(|&(a, b)| source_index::distance2(p[a], p[b]))
            .fold(0., f64::max);
        for (a, b) in edges {
            let (ka, kb) = (t.bits[a], t.bits[b]);
            if (policy.proxy
                || (area <= max_edge * 1e-4 && source_index::distance2(p[a], p[b]) == max_edge))
                && seen_edges.insert((ka.min(kb), ka.max(kb)))
            {
                controls.add(distance(std::array::from_fn(|k| {
                    0.5 * (p[a][k] + p[b][k])
                }))?)?;
            }
        }
        ordinal += 1;
    }
    let last = last.ok_or_else(|| state("empty directed sample stream"))?;
    // A zero-area surface is still a set of points/segments. Keep the scheduled count,
    // use barycentric samples of its last triangle, and retain every corner above.
    if total == 0. {
        while i < count {
            if i.is_multiple_of(256) {
                check()?;
            }
            let r = rng.unit();
            let s = rng.unit();
            samples.add(distance(source_index::bary(
                last,
                [1. - r, r * (1. - s), r * s],
            ))?)?;
            i += 1;
        }
    }
    if i != count {
        return Err(resource(format!(
            "incomplete required sampling: {i}/{count}"
        )));
    }
    Ok(Direction {
        area: samples.summary(),
        controls: controls.summary(),
        surface_area: total,
    })
}

pub(crate) fn measure(
    tile: &CoarseTile<'_>,
    query: &mut SourceQuery<'_>,
    nearest: &mut Nearest,
    count: Count,
    centroid_checks: &[f64],
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(TileError, Summary, CpuReport), JobError> {
    let mut cpu = CpuReport::default();
    let started = thread_cpu();
    let seed = seed(tile.key);
    let proxy_triangles: Vec<_> = tile
        .triangles
        .iter()
        .enumerate()
        .map(|(i, t)| Triangle::new(t.map(|v| tile.decoded[v as usize]), i as u64))
        .collect();
    let proxy_area = proxy_triangles.iter().map(|t| t.area()).sum::<f64>();
    let proxy = Surface::new(proxy_triangles)?;
    cpu.proxy_preparation = elapsed(started);
    let started = thread_cpu();
    let source_area = query.area()?;
    cpu.source_scan = elapsed(started);
    let started = thread_cpu();
    let mut hint = None;
    let source_to_proxy = directed(
        query.surface()?,
        source_area,
        count.samples,
        seed,
        Controls::SOURCE,
        |p| Ok(nearest.surface(&proxy, p, &mut hint).distance2.sqrt()),
        check,
    )?;
    cpu.source_to_proxy = elapsed(started);
    let started = thread_cpu();
    let proxy_to_source = directed(
        proxy.triangles.iter().copied().map(Ok),
        proxy_area,
        count.samples,
        seed ^ 0xa5a5a5a5a5a5a5a5,
        Controls {
            proxy: true,
            initial: centroid_checks,
        },
        |p| Ok(nearest.source(query, p)?.distance2.sqrt()),
        check,
    )?;
    cpu.proxy_to_source = elapsed(started);
    publish(
        tile.children_error,
        count,
        seed,
        source_to_proxy,
        proxy_to_source,
    )
    .map(|(g, s)| (g, s, cpu))
}
fn publish(
    children_error: f64,
    count: Count,
    seed: u64,
    source_to_proxy: Direction,
    proxy_to_source: Direction,
) -> Result<(TileError, Summary), JobError> {
    let sampled_max = source_to_proxy.max().max(proxy_to_source.max());
    let margin = sampled_max * 0.05;
    let geometric_error = (sampled_max * 1.05).max(children_error);
    if !geometric_error.is_finite() || !children_error.is_finite() || children_error < 0. {
        return Err(state("nonfinite or negative measured G"));
    }
    Ok((
        TileError {
            geometric_error,
            policy: POLICY,
        },
        Summary {
            version: 2,
            seed,
            area_samples_per_direction: count.samples,
            depth_source_triangles: count.depth_source_triangles,
            schedule: count.schedule,
            source_to_proxy,
            proxy_to_source,
            sampled_max,
            margin,
            q: source_to_proxy.p99().max(proxy_to_source.p99()),
        },
    ))
}

impl Summary {
    /// Validate persisted facts at the delivery boundary, before G is copied into
    /// explicit/implicit/external metadata. Quantiles are upper logarithmic-bin
    /// endpoints (<=2^(1/32) relative slack); maxima/means/counts are exact samples.
    /// `range` is the row's source range: its samples must cover its scheduled share.
    pub(crate) fn validate(&self, g: f64, children: f64, range: u64) -> Result<(), JobError> {
        let distribution = |d: Distribution| {
            d.samples > 0
                && [d.mean, d.p95, d.p99, d.max]
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.)
                && d.mean <= d.max + 1e-12 * d.max
                && d.p95 <= d.p99
                && d.p99 <= d.max
        };
        let direction = |d: Direction| {
            distribution(d.area)
                && distribution(d.controls)
                && d.surface_area.is_finite()
                && d.surface_area >= 0.
                && d.area.samples == self.area_samples_per_direction
        };
        if self.version != 2
            || !self.schedule.admitted()
            || self.area_samples_per_direction
                < self.schedule.required(range, self.depth_source_triangles)?
            || !direction(self.source_to_proxy)
            || !direction(self.proxy_to_source)
        {
            return Err(state("invalid S2 sample counts or distributions"));
        }
        let (expected, summary) = publish(
            children,
            Count {
                samples: self.area_samples_per_direction,
                depth_source_triangles: self.depth_source_triangles,
                schedule: self.schedule,
            },
            self.seed,
            self.source_to_proxy,
            self.proxy_to_source,
        )?;
        if !g.is_finite()
            || g < expected.geometric_error
            || self.sampled_max != summary.sampled_max
            || self.margin != summary.margin
            || self.q != summary.q
        {
            return Err(state("understated or inconsistent S2 geometric error"));
        }
        Ok(())
    }
}

/// Worker-thread CPU (sum across jobs is a stage share, unlike worker wall time).
pub(crate) fn thread_cpu() -> Option<f64> {
    #[cfg(unix)]
    {
        let mut t = std::mem::MaybeUninit::<libc::timespec>::zeroed();
        // SAFETY: clock_gettime initializes timespec on success.
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, t.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: initialized by the successful call above.
        let t = unsafe { t.assume_init() };
        Some(t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn triangles(p: &[[[f32; 3]; 3]]) -> Vec<Triangle> {
        let mut result: Vec<_> = p
            .iter()
            .enumerate()
            .map(|(i, &p)| Triangle::new(p, i as u64))
            .collect();
        source_index::mark_controls(&mut result);
        result
    }
    fn pair(a: &[[[f32; 3]; 3]], b: &[[[f32; 3]; 3]], seed: u64) -> (TileError, Summary) {
        pair_count(a, b, seed, MIN_TILE)
    }
    fn pair_count(
        a: &[[[f32; 3]; 3]],
        b: &[[[f32; 3]; 3]],
        seed: u64,
        count: u64,
    ) -> (TileError, Summary) {
        let (a, b) = (triangles(a), triangles(b));
        let (ba, bb) = (
            Surface::new(a.clone()).unwrap(),
            Surface::new(b.clone()).unwrap(),
        );
        let direction = |t: Vec<Triangle>, bv: &Surface, seed, policy| {
            let area = t.iter().map(|t| t.area()).sum();
            directed(
                t.into_iter().map(Ok),
                area,
                count,
                seed,
                policy,
                |p| {
                    let mut h = source_index::Hit::empty();
                    bv.nearest(p, all_key(), &mut h);
                    Ok(h.distance2.sqrt())
                },
                &|| Ok(()),
            )
            .unwrap()
        };
        publish(
            0.,
            Count {
                samples: count,
                depth_source_triangles: 1,
                schedule: Schedule::PRODUCTION,
            },
            seed,
            direction(a, &bb, seed, Controls::SOURCE),
            direction(b, &ba, seed + 1, Controls::PROXY),
        )
        .unwrap()
    }
    fn quad(z: impl Fn(f32, f32) -> f32) -> Vec<[[f32; 3]; 3]> {
        let p = |x, y| [x, y, z(x, y)];
        vec![
            [p(0., 0.), p(1., 0.), p(0., 1.)],
            [p(1., 0.), p(1., 1.), p(0., 1.)],
        ]
    }
    #[test]
    fn analytic_tilted_plane_and_offset_are_within_margin() {
        let source = quad(|_, _| 0.);
        for (proxy, truth) in [(quad(|x, _| 0.25 * x), 0.25), (quad(|_, _| 0.5), 0.5)] {
            let (g, s) = pair(&source, &proxy, 71);
            assert!((s.sampled_max - truth).abs() < 1e-12, "{s:?}");
            assert!(g.geometric_error >= truth && g.geometric_error <= truth + s.margin + 1e-12);
            assert_eq!(s.source_to_proxy.area.samples, 2048);
            assert_eq!(s.proxy_to_source.area.samples, 2048);
            let (g2, s2) = pair(&source, &proxy, 199);
            assert_eq!(s2.sampled_max, s.sampled_max);
            assert!((g2.geometric_error - g.geometric_error).abs() < 1e-12);
        }
    }
    #[test]
    fn area_samples_find_maximum_inside_one_source_triangle() {
        let inner = [[0f32, 1., 0.], [-1., -1., 0.], [1., -1., 0.]];
        let outer = inner.map(|p| p.map(|v| v * 2.));
        let mut ring = Vec::new();
        for i in 0..3 {
            let j = (i + 1) % 3;
            ring.push([outer[i], outer[j], inner[i]]);
            ring.push([inner[i], outer[j], inner[j]]);
        }
        // The triangular gap's inradius is attained at its incenter, strictly
        // inside the single source face. Every source vertex/control is on proxy.
        let truth = 2. / (1. + 5f64.sqrt());
        let (g, s) = pair_count(&[outer], &ring, 177, MIN_DEPTH);
        assert!(s.source_to_proxy.controls.max < 1e-12);
        assert!(s.proxy_to_source.area.max < 1e-12);
        assert!(s.source_to_proxy.area.max > truth / 1.05 && s.source_to_proxy.area.max <= truth);
        assert!(g.geometric_error >= truth && g.geometric_error <= truth * 1.05);
    }
    #[test]
    fn source_controls_use_components_and_proxy_edges_are_unique() {
        let [a, b, c, d] = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        let tetra = [[a, b, c], [a, d, b], [a, c, d], [b, d, c]];
        let (_, s) = pair(&tetra, &tetra, 33);
        assert_eq!(s.source_to_proxy.controls.samples, 1);
        assert_eq!(s.proxy_to_source.controls.samples, 10); // four vertices, six edges
    }
    #[test]
    fn analytic_polygonal_cone_against_disc() {
        // Polygonal cone against its flat base: height 1-.8, attained at the apex.
        let mut a = Vec::new();
        let mut b = Vec::new();
        for i in 0..64 {
            let point = |j| {
                let angle = j as f64 * std::f64::consts::TAU / 64.;
                [0.6 * angle.cos() as f32, 0.6 * angle.sin() as f32, 0.8]
            };
            let x = point(i);
            let y = point(i + 1);
            a.push([[0., 0., 1.], x, y]);
            b.push([[0., 0., 0.8], x, y]);
        }
        let (g, s) = pair(&a, &b, 119);
        let truth = 1. - f64::from(0.8f32);
        assert!((s.sampled_max - truth).abs() < 1e-12, "{s:?}");
        assert!(g.geometric_error >= truth && g.geometric_error <= truth + s.margin + 1e-12);
    }
    #[test]
    fn tiny_component_degenerate_and_boundary_controls_survive_area_sampling() {
        let mut a = quad(|_, _| 0.);
        a.push([[4., 0., 10.]; 3]);
        a.push([[0., 0., 0.], [0., 0., 0.], [1., 0., 0.]]);
        let b = quad(|_, _| 0.);
        let (g, s) = pair(&a, &b, 17);
        let truth = 109f64.sqrt();
        assert!((s.source_to_proxy.controls.max - truth).abs() < 1e-12);
        assert!(g.geometric_error >= truth);
        assert_eq!(s.source_to_proxy.area.samples, MIN_TILE);
        let (g, s) = pair(&[[[0., 0., 0.]; 3]], &[[[0., 0., 2.]; 3]], 18);
        assert_eq!(s.sampled_max, 2.);
        assert_eq!(g.geometric_error, 2.1);
    }
    #[test]
    fn child_and_leaf_floor_and_depth_counts() {
        let (_, s) = pair(&quad(|_, _| 0.), &quad(|_, _| 0.), 73);
        let count = Count {
            samples: MIN_TILE,
            depth_source_triangles: 1,
            schedule: Schedule::PRODUCTION,
        };
        let (g, _) = publish(3., count, s.seed, s.source_to_proxy, s.proxy_to_source).unwrap();
        assert_eq!(g.geometric_error, 3.);
        assert!(publish(
            f64::NAN,
            count,
            s.seed,
            s.source_to_proxy,
            s.proxy_to_source
        )
        .is_err());
        let plan = SamplingPlan {
            totals: [1000; 32],
            schedule: Schedule::PRODUCTION,
        };
        let mut sum = 0;
        for i in 0..333 {
            sum += plan
                .count(TileKey {
                    first: i * 3,
                    end: (i + 1) * 3,
                    depth: 23,
                    leaf_begin: 0,
                    leaf_end: 1,
                })
                .unwrap()
                .samples;
        }
        sum += plan
            .count(TileKey {
                first: 999,
                end: 1000,
                depth: 23,
                leaf_begin: 0,
                leaf_end: 1,
            })
            .unwrap()
            .samples;
        assert!(sum >= MIN_DEPTH);
        // An omitted cell's share is not measured; the rest keep the depth's density.
        assert_eq!(
            Schedule::PRODUCTION.required(500, 1000).unwrap(),
            MIN_DEPTH / 2
        );
        assert_eq!(Schedule::PRODUCTION.required(1, 1000).unwrap(), MIN_TILE);
        let p = Schedule::PRODUCTION;
        assert!(p.required(1001, 1000).is_err() && p.required(0, 0).is_err());
        let bad = TileKey {
            first: 0,
            end: 1,
            depth: 32,
            leaf_begin: 0,
            leaf_end: 1,
        };
        assert_eq!(
            plan.count(bad).unwrap_err().kind(),
            crate::JobErrorKind::ResourceLimit
        );
        let mut plan = plan;
        assert!(plan.add(32, 1).is_err());
    }
    #[test]
    fn schedule_uses_only_encoded_tiles_after_chain_collapse() {
        use crate::mesh_large::records::{Bounds, NodeRecord};
        let node = |depth, first, end, kind| {
            Ok(NodeRecord {
                cell_depth: depth,
                cell: 0,
                path_len: 0,
                path: 0,
                last_octant: 0,
                kind,
                children: 0,
                first,
                end,
                bytes: 0,
                bounds: Bounds {
                    min: [0.; 3],
                    max: [1.; 3],
                },
            })
        };
        // Branch A has an internal chain at depths 1..3; branch B at depth 1
        // directly owns a leaf. Only branch A's depth-3 tile is actually encoded.
        let plan = SamplingPlan::from_nodes(
            [
                node(0, 0, 100, NodeKind::Internal),
                node(1, 0, 80, NodeKind::Internal),
                node(2, 0, 80, NodeKind::Internal),
                node(3, 0, 80, NodeKind::Internal),
                node(4, 0, 80, NodeKind::Leaf),
                node(1, 80, 100, NodeKind::Internal),
                node(2, 80, 100, NodeKind::Leaf),
            ],
            Schedule::PRODUCTION,
        )
        .unwrap();
        assert_eq!(&plan.totals[..5], &[100, 20, 0, 80, 0]);
        for (depth, first, end) in [(0, 0, 100), (1, 80, 100), (3, 0, 80)] {
            assert_eq!(
                plan.count(TileKey {
                    depth,
                    first,
                    end,
                    leaf_begin: 0,
                    leaf_end: 1
                })
                .unwrap(),
                Count {
                    samples: MIN_DEPTH,
                    depth_source_triangles: plan.totals[depth as usize],
                    schedule: Schedule::PRODUCTION,
                }
            );
        }
        assert!(SamplingPlan::from_nodes(
            [node(32, 0, 1, NodeKind::Internal)],
            Schedule::PRODUCTION
        )
        .is_err());
    }
    #[test]
    fn incomplete_area_schedule_is_a_typed_refusal() {
        let t = Triangle::new([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], 0);
        let error = directed(
            std::iter::once(Ok(t)),
            t.area() * 2.,
            MIN_TILE,
            55,
            Controls::SOURCE,
            |_| Ok(0.),
            &|| Ok(()),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), crate::JobErrorKind::ResourceLimit);
        directed(
            std::iter::once(Ok(t)),
            t.area() * (1. + 2. * f64::EPSILON),
            MIN_TILE,
            55,
            Controls::SOURCE,
            |_| Ok(0.),
            &|| Ok(()),
        )
        .unwrap();
    }
    #[test]
    fn cancellation_is_checked_inside_large_and_zero_area_strata() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for t in [
            Triangle::new([[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], 0),
            Triangle::new([[0., 0., 0.]; 3], 0),
        ] {
            let calls = AtomicUsize::new(0);
            let result = directed(
                std::iter::once(Ok(t)),
                t.area(),
                MIN_DEPTH,
                1,
                Controls::SOURCE,
                |_| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    Ok(0.)
                },
                &|| {
                    if calls.load(Ordering::Relaxed) >= 256 {
                        Err(state("cancel test"))
                    } else {
                        Ok(())
                    }
                },
            );
            assert!(result.is_err());
            assert!(calls.load(Ordering::Relaxed) <= 260);
        }
    }
    #[test]
    fn constant_distance_mean_uses_compensated_accumulation() {
        let d = 3f64.sqrt();
        let mut stats = Stats::new();
        for _ in 0..MIN_DEPTH {
            stats.add(d).unwrap();
        }
        let s = stats.summary();
        assert!((s.mean - d).abs() <= f64::EPSILON * d);
        assert_eq!(s.p99, d);
    }
    #[test]
    fn histogram_quantiles_are_upper_within_one_bin() {
        let mut s = Stats::new();
        for i in 0..1000 {
            s.add(i as f64 / 1000.).unwrap();
        }
        let d = s.summary();
        assert!(d.p95 >= 0.949 && d.p95 <= 0.949 * 2f64.powf(1. / 32.));
        assert!(d.p99 >= 0.989 && d.p99 <= 0.989 * 2f64.powf(1. / 32.));
    }
}
