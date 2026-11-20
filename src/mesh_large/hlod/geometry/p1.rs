//! P1: one guard-by-construction contraction (`collapse`) of a cell whose primary build
//! failed, before the retry ladder. It contracts the merged children onto the parent
//! contract and simplifies them in one pass that never makes what the fold guard,
//! topology check or interface check refuses; a border vertex that only a face on three
//! shared points could remove is rescued by an interior Steiner vertex on the canonical
//! lattice (`collapse::Problem::steiner`). Its proposal is still assessed like every other
//! (decode validity, S2), so P1 only adds a way to build a cell.
//!
//! Two passes: stage 0 under the default policy; stage 1 also licenses new edges and faces
//! on shared points of the seam segments this cell owns (N1's seam-owner licence, declared
//! as last-resort stage 1).
use super::super::{
    borders::Contract,
    interfaces::{key, Key},
    store::{StoppingBound, TileGeometry, TileKey},
};
use super::{
    collapse, folds, interface_points, merge, missing, topology, BorderMap, Built, Merged,
};
use crate::{mesh_large::leaves::geometry::LatticePlan, JobError};
use rustc_hash::FxHashSet as HashSet;
use std::{borrow::Cow, cell::RefCell};

/// Work units (`collapse::Problem::work_limit`) one P1 pass may spend per merged face before
/// it gives up and the ladder continues. A planar grid spends under 64 per face, the busiest
/// cells of the #344 rock, a 10M-triangle city and a 10M-triangle photogrammetry site at most
/// 187, and built cells of a 5.7M-triangle terrain crop with a base cap up to about 2,200.
/// The limit leaves several times that while still stopping a pathological cell after a
/// bounded multiple of its size: high-valence vertices are requeued amortised
/// (`collapse::retry_wait`), but other quadratic patterns may remain.
const WORK_PER_FACE: u64 = 1 << 14;

fn work_limit(faces: usize) -> u64 {
    u64::try_from(faces)
        .unwrap_or(u64::MAX)
        .saturating_mul(WORK_PER_FACE)
}

/// Upper estimate of what P1 holds at once for `vertices` merged vertices, `faces` faces and
/// `rows` child interface rows: per vertex its merged copy, class, mapping, f64 position,
/// quadric, face list, stamps and heap entries; per face the merged face and representative,
/// the four edge and face hash tables at a load of 7/16, and the face lists' entries; per
/// row the merged child interface and its copy in the contraction guard. The primary build's
/// remaining hand-over is included. Calibrated with margin against the complete-parent
/// controls (`contraction_memory`), whose vertex-dense strips are the worst case measured.
fn workspace_bytes(vertices: usize, faces: usize, rows: usize) -> u64 {
    const PER_VERTEX: u64 = 600;
    const PER_FACE: u64 = 600;
    const PER_ROW: u64 = 100;
    let n = |x: usize| u64::try_from(x).unwrap_or(u64::MAX);
    n(vertices)
        .saturating_mul(PER_VERTEX)
        .saturating_add(n(faces).saturating_mul(PER_FACE))
        .saturating_add(n(rows).saturating_mul(PER_ROW))
}

/// P1 runs within its job's charge: the per-triangle geometry workspace plus the encode
/// allowance, which nothing uses until the published tile is encoded, after P1 has
/// released its state. Beyond it (vertex-dense inputs at full size) the ladder runs as
/// before without P1, N1 included (`Report::stage_one_finished`).
fn fits(vertices: usize, faces: usize, rows: usize) -> bool {
    let faces64 = u64::try_from(faces).unwrap_or(u64::MAX);
    workspace_bytes(vertices, faces, rows)
        <= faces64
            .saturating_mul(super::super::JOB_BYTES_PER_TRIANGLE)
            .saturating_add(super::super::JOB_ENCODE_BYTES)
}

/// What P1 did in one cell (`p1Records` in the stage report). `outcome`: `built`,
/// `refused` (assessment refused its proposal), `noProposal` (a pass finished but its
/// result failed the cap, a contract segment or point, the fold guard or the topology
/// floor), `exhausted` (a pass spent its work budget), `stalled` (border removals left
/// in both passes) or `skipped` (the contract alone exceeds the cap, or the workspace
/// estimate exceeds the job's charge). Per pass (stage 0, 1): work units spent, removals
/// left and Steiner rescues made and refused (`collapse::Stats::steiner_why`), the work spent
/// rescuing and in Steiner rescues, the main loop's work by site (`siteWork`,
/// `collapse::SiteWork`) and its refused checks by reason (`refusedBy`, `collapse::Stats::why`),
/// and the faces of a proposal the contraction fold guard refused, by rule (`guardRefused`,
/// in `folds::Refusal` order) and how many of them have a Steiner corner (`guardSteiner`).
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    pub outcome: &'static str,
    pub faces: u64,
    pub work_limit: u64,
    pub passes: u8,
    pub work: [u64; 2],
    pub removals_left: [u64; 2],
    pub steiner: [u64; 2],
    pub steiner_refused: [[u64; 6]; 2],
    /// Of `work`: units spent rescuing stuck removals, and of those in Steiner rescues.
    pub rescue_work: [u64; 2],
    pub steiner_work: [u64; 2],
    /// The main loop's work by site (`collapse::SiteWork`).
    pub site_work: [collapse::SiteWork; 2],
    /// Collapse checks refused, by reason (`collapse::Stats::why`). The reason is the first
    /// test that refused, so a high-valence vertex's sampled faces can shift a refusal to
    /// another reason than a full scan would report; the totals are those of the decisions.
    pub refused_by: [[u64; 13]; 2],
    pub guard_refused: [[u64; 6]; 2],
    pub guard_steiner: [u64; 2],
}

impl Report {
    /// Whether P1's second pass, with N1's seam-owner licence, ran to its end: N1 then has
    /// nothing to add (`build_parent` skips it). Not when P1 was skipped (the contract alone
    /// exceeds the cap, or its workspace estimate exceeds the job's charge), spent its work
    /// budget, or stopped at a first-pass proposal, published or refused.
    pub(crate) fn stage_one_finished(&self) -> bool {
        self.passes >= 2 && self.outcome != "exhausted"
    }

    /// What `attempt` reports when its workspace estimate refuses the cell.
    #[cfg(test)]
    pub(crate) fn unfit(faces: usize) -> Self {
        Self::new("skipped", faces)
    }

    /// What `attempt` reports when its first pass spends the work budget (it returns
    /// before the second pass).
    #[cfg(test)]
    pub(crate) fn exhausted_first_pass(faces: usize) -> Self {
        let mut report = Self::new("exhausted", faces);
        report.passes = 1;
        report
    }

    fn new(outcome: &'static str, faces: usize) -> Self {
        Self {
            outcome,
            faces: u64::try_from(faces).unwrap_or(u64::MAX),
            work_limit: work_limit(faces),
            passes: 0,
            work: [0; 2],
            removals_left: [0; 2],
            steiner: [0; 2],
            steiner_refused: [[0; 6]; 2],
            rescue_work: [0; 2],
            steiner_work: [0; 2],
            site_work: [collapse::SiteWork::default(); 2],
            refused_by: [[0; 13]; 2],
            guard_refused: [[0; 6]; 2],
            guard_steiner: [0; 2],
        }
    }

    fn pass(&mut self, stage: u8, stats: &collapse::Stats) {
        let i = usize::from(stage.min(1));
        self.passes = self.passes.max(stage + 1);
        self.work[i] = stats.work;
        self.removals_left[i] = stats.removed_left;
        self.steiner[i] = stats.steiner;
        self.steiner_refused[i] = stats.steiner_why;
        self.rescue_work[i] = stats.rescue_work;
        self.steiner_work[i] = stats.steiner_work;
        self.site_work[i] = stats.site;
        self.refused_by[i] = stats.why;
    }
}

/// Whether P1 runs (`build_parent`); in tests also the two ways it does not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Run {
    Off,
    On,
    /// Test: the record of a cell whose workspace estimate refuses P1, as for a vertex-dense
    /// cell at full size (`Report::unfit`).
    #[cfg(test)]
    Unfit,
    /// Test: the record of a cell whose first pass spends its work budget
    /// (`Report::exhausted_first_pass`).
    #[cfg(test)]
    Exhausted,
}

impl From<bool> for Run {
    fn from(on: bool) -> Self {
        if on {
            Self::On
        } else {
            Self::Off
        }
    }
}

/// The P1 proposal for `tile`, with the last-resort stage that licensed it (0 or 1), or
/// none. `caps` are the build's caps (target first, hard cap last).
pub(super) fn attempt(
    tile: TileKey,
    children: &[TileGeometry],
    caps: &[usize],
    contract: &Contract,
    mapping: &BorderMap<'_>,
    lattice: &LatticePlan,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(Option<(u8, Built)>, Report), JobError> {
    let rows: usize = children.iter().map(|c| c.interface.len()).sum();
    let vertices: usize = children.iter().map(|c| c.positions.len()).sum();
    let faces: usize = children.iter().map(|c| c.triangles.len()).sum();
    let mut report = Report::new("skipped", faces);
    let (Some(&target), Some(&cap)) = (caps.first(), caps.last()) else {
        return Ok((None, report));
    };
    // A contract that alone needs more than the cap cannot be kept (production never
    // builds one; `build_tile` filters it out).
    if contract.minimum_triangles() > cap {
        return Ok((None, report));
    }
    if !fits(vertices, faces, rows) {
        return Ok((None, report));
    }
    let m = merge(children);
    let interface = &contract.interface;
    let children_error = children
        .iter()
        .map(|c| c.geometric_error)
        .fold(0., f64::max);
    let floor = contract.scale.max(children_error);
    let (mut prob, mapped0, previous) =
        collapse::problem(&m, children, interface, mapping, target, 0.)?;
    // The guard counts only exterior child rows as path witnesses.
    let leaves = [tile.leaf_begin, tile.leaf_end];
    prob.limit = cap;
    prob.steiner = true;
    // The guard's own directed licences between shared points: P1 and the contraction
    // fold guard below count one source (literal edges and the contract's chord paths).
    let shared = {
        let ids = collapse::key_index(&m.keys);
        folds::Source::for_contraction(
            &m.positions,
            &m.triangles,
            interface,
            &previous,
            leaves,
            &mapped0,
        )
        .boundary_edges()
        .into_iter()
        .filter_map(|(a, b, n)| Some(((*ids.get(&a)?, *ids.get(&b)?), n)))
        .collect()
    };
    prob.shared = Some(shared);
    prob.work_limit = work_limit(m.triangles.len());
    // A Steiner position is the canonical lattice point at the inherited depth of the
    // vertex it replaces, or none (an invalid depth or an out-of-range code refuses it).
    let snap = |p: [f32; 3], r: u32| {
        let ell = *m.ells.get(r as usize)?;
        lattice.canonical(p, ell).ok()
    };
    prob.snap = Some(&snap);
    // A key on a border anywhere in the source is protected; a lookup failure is the job's.
    let failure: RefCell<Option<JobError>> = RefCell::new(None);
    let protected = |p: [f32; 3]| match mapping(key(p)) {
        Ok(found) => found.is_some(),
        Err(e) => {
            failure.borrow_mut().get_or_insert(e);
            true
        }
    };
    prob.protected = Some(&protected);
    // The job's cancellation, polled inside the pass; its error is returned after the pass.
    let cancel = || match check() {
        Ok(()) => false,
        Err(e) => {
            failure.borrow_mut().get_or_insert(e);
            true
        }
    };
    prob.cancel = Some(&cancel);
    let policy = folds::Policy::last_resort(1, tile.leaf_begin, tile.leaf_end, &contract.holders);
    let owns_segment = |a: u32, b: u32| match (m.keys.get(a as usize), m.keys.get(b as usize)) {
        (Some(&a), Some(&b)) => policy.owns_segment(a, b),
        _ => false,
    };
    let owns_face = |f: [u32; 3]| {
        let keys = f.map(|v| m.keys.get(v as usize).copied());
        match keys {
            [Some(a), Some(b), Some(c)] => policy.owns_face([a, b, c]),
            _ => false,
        }
    };
    let segments: HashSet<(Key, Key)> = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .map(|r| (r.a, r.b))
        .collect();
    let points = interface_points(interface);
    // Merged vertices; Steiner vertices follow them.
    let inputs = m.positions.len();
    // The fold guard's refusals of the last proposal `publishable` assessed.
    let guard_seen: RefCell<Option<([u64; 6], u64)>> = RefCell::new(None);
    // `faces` over `m` (the merged children with any Steiner vertices) as a proposal, if it
    // keeps every contract segment and point, passes the contraction fold guard under
    // `policy` and changes topology only within `floor`.
    let publishable = |m: &Merged,
                       faces: &[[u32; 3]],
                       mapped: &[Option<(Key, Key, f64, u8)>],
                       policy: folds::Policy|
     -> Option<Built> {
        let (lost_edges, lost_points) = missing(faces, &m.keys, &segments, &points);
        if !lost_edges.is_empty() || !lost_points.is_empty() {
            return None;
        }
        let guard = folds::Source::for_contraction(
            &m.positions,
            &m.triangles,
            interface,
            &previous,
            leaves,
            mapped,
        )
        .with_policy(policy);
        let refusals = guard.refusals(&m.positions, faces);
        if refusals.iter().any(|&r| r != 0) {
            let mut seen = ([0u64; 6], 0u64);
            for (t, &r) in faces.iter().zip(&refusals) {
                if r != 0 {
                    seen.0[usize::from(r - 1)] += 1;
                    seen.1 += u64::from(t.iter().any(|&v| v as usize >= inputs));
                }
            }
            *guard_seen.borrow_mut() = Some(seen);
            return None;
        }
        let change = topology::changes_noting(m, faces, interface, floor, &mut false)?;
        let mut remap = vec![u32::MAX; m.keys.len()];
        let mut geometry = TileGeometry {
            key: tile,
            ells: Vec::new(),
            positions: Vec::new(),
            triangles: Vec::with_capacity(faces.len()),
            representative: vec![0; faces.len()],
            primitives: m.representative.first().copied().into_iter().collect(),
            interface: interface.to_vec(),
            geometric_error: 0.,
            published_error: 0.,
            surface_error: 0.,
            decoded_override: Vec::new(),
            texture: Default::default(),
            lease: None,
        };
        let mut locked = Vec::new();
        let mut generated = Vec::new();
        for t in faces {
            let mut out = [0u32; 3];
            for (o, &v) in out.iter_mut().zip(t) {
                let slot = remap.get_mut(v as usize)?;
                if *slot == u32::MAX {
                    *slot = u32::try_from(geometry.positions.len()).ok()?;
                    if v as usize >= inputs {
                        generated.push(*slot);
                    }
                    geometry.ells.push(*m.ells.get(v as usize)?);
                    geometry.positions.push(*m.positions.get(v as usize)?);
                    locked.push(points.contains(m.keys.get(v as usize)?));
                }
                *o = *slot;
            }
            geometry.triangles.push(out);
        }
        Some(Built {
            locked,
            generated,
            stopping: StoppingBound::TriangleBudget,
            simplifier_error: 0.,
            children_error,
            input_triangles: u32::try_from(m.triangles.len()).unwrap_or(u32::MAX),
            protected_faces: 0,
            retries: 0,
            last_resort: true,
            staged: false,
            last_resort_stage: 0,
            rejection: None,
            primary: false,
            p1: true,
            relief_stuck: 0,
            interface_vertices: u32::try_from(points.len()).unwrap_or(u32::MAX),
            interface_segments: u32::try_from(segments.len()).unwrap_or(u32::MAX),
            reoriented: 0,
            topology: change.receipt,
            change,
            controls: Default::default(),
            search: Default::default(),
            geometry,
        })
    };
    for stage in [0u8, 1] {
        if stage == 1 {
            prob.owns_segment = Some(&owns_segment);
            prob.owns_face = Some(&owns_face);
        }
        let (out, stats, added) = collapse::simplify_added(&prob);
        if let Some(e) = failure.borrow_mut().take() {
            return Err(e);
        }
        report.pass(stage, &stats);
        if stats.exhausted {
            // Its work bound is spent: the second pass would meet the same mesh.
            report.outcome = "exhausted";
            return Ok((None, report));
        }
        let Some(faces) = out else {
            report.outcome = if stats.removed_left > 0 {
                "stalled"
            } else {
                "noProposal"
            };
            continue;
        };
        report.outcome = "noProposal";
        if faces.len() > cap || faces.is_empty() {
            continue;
        }
        // Only a Steiner rescue adds vertices; otherwise the merged mesh is used as is.
        let (with_added, mapped) = if added.is_empty() {
            (Cow::Borrowed(&m), Cow::Borrowed(&mapped0))
        } else {
            let (mut m, mut mapped) = (m.clone(), mapped0.clone());
            collapse::extend_added(&mut m, &mut mapped, &added);
            (Cow::Owned(m), Cow::Owned(mapped))
        };
        let stage_policy = if stage == 1 {
            policy.clone()
        } else {
            folds::Policy::default()
        };
        if let Some(built) = publishable(&with_added, &faces, &mapped, stage_policy) {
            return Ok((Some((stage, built)), report));
        }
        if let Some((rules, steiner)) = guard_seen.borrow_mut().take() {
            report.guard_refused[usize::from(stage)] = rules;
            report.guard_steiner[usize::from(stage)] = steiner;
        }
    }
    Ok((None, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N1 is skipped only where P1's second pass ran to its end without spending its budget.
    #[test]
    fn stage_one_finishes_only_when_the_second_pass_ran_to_its_end() {
        let finished = |outcome: &'static str, passes: u8| {
            let mut report = Report::new(outcome, 10);
            report.passes = passes;
            report.stage_one_finished()
        };
        for outcome in ["noProposal", "stalled", "refused", "built"] {
            assert!(finished(outcome, 2), "{outcome}");
        }
        assert!(!finished("exhausted", 2));
        assert!(!finished("exhausted", 1));
        assert!(!finished("refused", 1));
        assert!(!finished("skipped", 0));
        assert!(!Report::unfit(10).stage_one_finished());
        assert!(!Report::exhausted_first_pass(10).stage_one_finished());
    }
}
