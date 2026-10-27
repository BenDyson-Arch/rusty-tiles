//! D11: bounded parents from immediate children and a common, certified border contract.
//! Boundary reconstruction precedes meshopt reduction. Stalled interiors get bounded
//! topology cleanup and remeshing; only the reported capped frontier remains afterwards.
#[cfg(test)]
mod combined_controls;
mod constrained;
#[cfg(test)]
mod contraction_memory;
#[cfg(test)]
mod fold_probe;
pub(crate) mod folds;
pub(crate) mod locations;
pub(crate) mod measurement;
mod sheets;
mod topology;
pub(crate) use topology::Receipt as TopologyReceipt;

use super::{
    borders::Contract,
    interfaces::{key, InterfaceRow, Key},
    store::{SourcePrimitive, StoppingBound, TileGeometry, TileKey},
};
use crate::{JobError, JobErrorKind};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

type BorderMap<'a> = dyn Fn(Key) -> Result<Option<(Key, Key, f64, u8)>, JobError> + 'a;

/// Protect-and-retry rounds after the first simplification.
pub(crate) const RETRIES: u32 = 3;
// Geometry proposals have no S2/audit write until accepted. This separate fixed
// allowance lets local carrier protection converge without multiplying measured
// hole-opening attempts or their scratch reservation.
const FOLD_RETRIES: u32 = 8;
// Ordinary and independent-sheet strategies share ten reduction passes. Reserve
// one for the second strategy; exhausting this allowance enters constructive
// remeshing, rather than supplying evidence of cap infeasibility.
const REDUCTION_PASSES: u32 = 10;

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// Grow by distinct keys, never by the duplicated endpoint iterator's size hint.
fn interface_points(interface: &[InterfaceRow]) -> HashSet<Key> {
    let mut points = HashSet::default();
    for row in interface {
        points.insert(row.a);
        points.insert(row.b);
    }
    points
}

/// A built parent, before encoding.
#[derive(Debug)]
pub(crate) struct Built {
    pub geometry: TileGeometry,
    /// Per vertex: on the parent's outer interface (locked).
    pub locked: Vec<bool>,
    pub stopping: StoppingBound,
    pub simplifier_error: f64,
    pub children_error: f64,
    pub input_triangles: u32,
    pub protected_faces: u32,
    pub retries: u32,
    pub interface_vertices: u32,
    pub interface_segments: u32,
    /// Historical receipt field; proposals are rejected rather than reoriented.
    pub reoriented: u32,
    pub topology: topology::Receipt,
    pub change: topology::Change,
    pub controls: measurement::Controls,
}

pub(crate) struct Outcome {
    pub built: Option<Built>,
    pub contraction_failed: bool,
    /// All measured proposals, including rejected candidates and the accepted publication.
    pub attempted_topology: TopologyReceipt,
    pub construction_dropped: Vec<[[f32; 3]; 3]>,
}

/// Children merged into one vertex space welded by canonical decoded position, in child
/// (slot) order.
#[derive(Clone)]
struct Merged {
    keys: Vec<Key>,
    ells: Vec<u8>,
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    representative: Vec<SourcePrimitive>,
}

fn merge(children: &[TileGeometry]) -> Merged {
    let _cpu = super::cpu_profile::start(
        super::cpu_profile::Stage::Merge,
        children.iter().map(|c| c.triangles.len()).sum(),
    );
    let triangles: usize = children.iter().map(|c| c.triangles.len()).sum();
    let mut index: HashMap<Key, u32> =
        HashMap::with_capacity_and_hasher(triangles, Default::default());
    let mut merged = Merged {
        keys: Vec::new(),
        ells: Vec::new(),
        positions: Vec::new(),
        triangles: Vec::with_capacity(triangles),
        representative: Vec::with_capacity(triangles),
    };
    for child in children {
        let mut local = Vec::with_capacity(child.positions.len());
        for (v, p) in child.positions.iter().enumerate() {
            let next = merged.keys.len() as u32;
            let id = *index.entry(key(*p)).or_insert(next);
            if id == next {
                merged.keys.push(key(*p));
                merged.ells.push(child.ells[v]);
                merged.positions.push(*p);
            } else {
                // A finer lattice still holds the point exactly.
                let ell = &mut merged.ells[id as usize];
                *ell = (*ell).max(child.ells[v]);
            }
            local.push(id);
        }
        for (t, tri) in child.triangles.iter().enumerate() {
            merged.triangles.push(tri.map(|v| local[v as usize]));
            merged
                .representative
                .push(child.primitives[child.representative[t] as usize]);
        }
    }
    // Triangles whose corners decode onto fewer than three points carry no surface: drop
    // them, unless nothing else is left (then the parent publishes the same zero-area
    // geometry its leaves publish).
    let surface = |t: &[u32; 3]| t[0] != t[1] && t[1] != t[2] && t[0] != t[2];
    if merged.triangles.iter().any(surface) {
        let keep: Vec<bool> = merged.triangles.iter().map(surface).collect();
        let mut i = 0;
        merged.triangles.retain(|_| (keep[i], i += 1).0);
        let mut i = 0;
        merged.representative.retain(|_| (keep[i], i += 1).0);
    }
    merged
}

fn edge(a: Key, b: Key) -> (Key, Key) {
    (a.min(b), a.max(b))
}

/// Required segments and points (canonical keys) missing from `triangles`.
fn missing(
    triangles: &[[u32; 3]],
    keys: &[Key],
    segments: &HashSet<(Key, Key)>,
    points: &HashSet<Key>,
) -> (Vec<(Key, Key)>, Vec<Key>) {
    missing_owned(triangles, keys, segments.clone(), points.clone())
}

/// Consume temporary queries instead of retaining a second copy during the scan.
fn missing_owned(
    triangles: &[[u32; 3]],
    keys: &[Key],
    mut missing_edges: HashSet<(Key, Key)>,
    mut missing_points: HashSet<Key>,
) -> (Vec<(Key, Key)>, Vec<Key>) {
    for t in triangles {
        for k in 0..3 {
            let (a, b) = (keys[t[k] as usize], keys[t[(k + 1) % 3] as usize]);
            missing_edges.remove(&edge(a, b));
            missing_points.remove(&a);
        }
        if missing_edges.is_empty() && missing_points.is_empty() {
            return (Vec::new(), Vec::new());
        }
    }
    let mut edges: Vec<_> = missing_edges.into_iter().collect();
    let mut lost: Vec<_> = missing_points.into_iter().collect();
    edges.sort_unstable();
    lost.sort_unstable();
    (edges, lost)
}

/// meshopt over a subset of the merged triangles, compacted to the vertices they use.
/// Returns triangles in merged vertex indices and the absolute error estimate.
fn simplify(
    merged: &Merged,
    faces: &[[u32; 3]],
    locked: &dyn Fn(u32) -> bool,
    target: usize,
) -> (Vec<[u32; 3]>, f64) {
    let mut local = vec![u32::MAX; merged.positions.len()];
    simplify_mapped(merged, faces, locked, target, &mut local)
}

fn simplify_mapped(
    merged: &Merged,
    faces: &[[u32; 3]],
    locked: &dyn Fn(u32) -> bool,
    target: usize,
    local: &mut [u32],
) -> (Vec<[u32; 3]>, f64) {
    let _cpu = super::cpu_profile::start(super::cpu_profile::Stage::Simplify, faces.len());
    // In pinned meshopt 0.6.2, a satisfied target or all locked vertices leaves
    // the copied index buffer untouched: no collapse, pruning or solve runs for
    // our ErrorAbsolute-only options. Avoid allocating its quadrics/adjacency
    // for these common tiny sheets. A finite extent is needed for its final
    // sqrt(0) * extent to equal zero; unusual coordinates retain the FFI path.
    if !faces.is_empty()
        && (target == faces.len() || faces.iter().flatten().all(|&v| locked(v)))
        && finite_extent(&merged.positions, faces)
    {
        let _fixed =
            super::cpu_profile::start(super::cpu_profile::Stage::FixedSimplify, faces.len());
        return (faces.to_vec(), 0.);
    }
    // Merged indices are dense and bounded to eight capped children. Preserve first-use
    // order without hashing each corner, including when simplifying a protected subset.
    let mut global = Vec::new();
    let mut indices = Vec::with_capacity(faces.len() * 3);
    for f in faces {
        for &v in f {
            let next = global.len() as u32;
            let id = &mut local[v as usize];
            if *id == u32::MAX {
                *id = next;
                global.push(v);
            }
            indices.push(*id);
        }
    }
    let positions: Vec<[f32; 3]> = global
        .iter()
        .map(|&v| merged.positions[v as usize])
        .collect();
    let locks: Vec<u8> = global.iter().map(|&v| u8::from(locked(v))).collect();
    let mut destination = vec![0u32; indices.len()];
    let mut error = 0f32;
    // SAFETY: every pointer covers the lengths passed with it: `indices` and `destination`
    // hold `indices.len()` u32 (meshopt's worst-case output is the input count),
    // `positions` holds `global.len()` tightly packed f32x3 (stride 12), `locks` one byte per
    // vertex, and no attributes are passed. Indices are all below `global.len()`.
    let count = unsafe {
        meshopt::ffi::meshopt_simplifyWithAttributes(
            destination.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr().cast::<f32>(),
            positions.len(),
            12,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            locks.as_ptr(),
            target.saturating_mul(3),
            f32::MAX,
            meshopt::SimplifyOptions::ErrorAbsolute.bits(),
            &mut error,
        )
    };
    destination.truncate(count);
    let triangles = destination
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| t.map(|v| global[v as usize]))
        .collect();
    for &v in &global {
        local[v as usize] = u32::MAX;
    }
    (triangles, f64::from(error))
}

fn finite_extent(positions: &[[f32; 3]], faces: &[[u32; 3]]) -> bool {
    let mut lo = [f32::MAX; 3];
    let mut hi = [-f32::MAX; 3];
    for &v in faces.iter().flatten() {
        for (i, x) in positions[v as usize].into_iter().enumerate() {
            if !x.is_finite() {
                return false;
            }
            lo[i] = lo[i].min(x);
            hi[i] = hi[i].max(x);
        }
    }
    (0..3).all(|i| (hi[i] - lo[i]).is_finite())
}

/// A triangle rotated to start at its least vertex: equal for the same oriented face.
fn oriented(t: &[u32; 3]) -> [u32; 3] {
    let k = (0..3).min_by_key(|&k| t[k]).unwrap();
    [t[k], t[(k + 1) % 3], t[(k + 2) % 3]]
}
/// Reconstruct the common contracted boundary before ordinary reduction. Only capped
/// immediate children enter this job. After a stalled reduction, explicitly remesh the
/// interior; omission is the reported last resort and propagates upwards in the walk.
pub(crate) fn build_parent(
    tile: TileKey,
    children: &[TileGeometry],
    caps: &[usize],
    contract: &Contract,
    mapping: &BorderMap<'_>,
    assess: &mut dyn FnMut(&mut Built) -> Result<bool, JobError>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Outcome, JobError> {
    check()?;
    if caps.is_empty() {
        return Err(state("a parent build needs at least one cap"));
    }
    #[cfg(test)]
    fold_probe::capture(tile, children, contract, caps);
    let mut merged = merge(children);
    let reference = merged.clone();
    if merged.triangles.is_empty() {
        return Err(state(format!(
            "coarse tile f{}-d{} has no source triangle",
            tile.first, tile.depth
        )));
    }
    let children_error = children
        .iter()
        .map(|c| c.geometric_error)
        .fold(0., f64::max);
    let interface = &contract.interface;
    let mut topology = topology::Receipt::default();
    let mut attempted = topology::Receipt::default();
    {
        // Coarsening only merges owners: every live parent border is a child border.
        // Nested canonical endpoints therefore make the union of child interfaces a
        // complete lookup domain. Interior vertices need no disk-backed source search.
        let mut previous_interface =
            Vec::with_capacity(children.iter().map(|c| c.interface.len()).sum());
        for child in children {
            previous_interface.extend_from_slice(&child.interface);
        }
        super::interfaces::merge_rows(&mut previous_interface);
        previous_interface.shrink_to_fit();
        let child_border = interface_points(&previous_interface);
        let lookup = |k| {
            if child_border.contains(&k) {
                mapping(k)
            } else {
                Ok(None)
            }
        };
        if !topology::contract(
            &mut merged,
            interface,
            &previous_interface,
            &lookup,
            &mut topology,
        )? {
            attempted.add(topology);
            topology = Default::default();
            if !recover_contraction(
                &mut merged,
                &reference,
                interface,
                &previous_interface,
                &lookup,
                caps,
                &mut topology,
                check,
            )? {
                attempted.add(topology);
                return Ok(Outcome {
                    built: None,
                    contraction_failed: true,
                    attempted_topology: attempted,
                    construction_dropped: Vec::new(),
                });
            }
        }
    }
    let contracted_faces = merged.triangles.clone();
    let floor = contract.scale.max(children_error);
    let mut opening_pins = HashSet::default();
    for (i, &cap) in caps.iter().enumerate() {
        if caps[..i].contains(&cap) {
            continue;
        }
        for preserve_holes in [false, true] {
            if let Some(mut b) = assessed_attempt(
                tile,
                &merged,
                interface,
                cap,
                children_error,
                &reference,
                &contracted_faces,
                floor,
                preserve_holes,
                &mut opening_pins,
                &mut attempted,
                assess,
                check,
            )? {
                attempted.add(topology);
                b.topology.add(topology);
                return Ok(Outcome {
                    built: Some(b),
                    contraction_failed: false,
                    attempted_topology: attempted,
                    construction_dropped: Vec::new(),
                });
            }
        }
    }
    // D11: simplifier stagnation alone is not cap infeasibility. Deduplicate and remove
    // eligible interior components, then remesh with increasingly coarse interior cells.
    let construction_dropped = topology::cleanup(&mut merged, interface, floor, &mut topology);
    if let Some(receipt) = constrained::remesh(&mut merged, interface, *caps.last().unwrap()) {
        topology.add(receipt);
    }
    for round in 0..=6 {
        check()?;
        if round != 0 && !topology::remesh(&mut merged, interface, round, &mut topology) {
            continue;
        }
        for preserve_holes in [false, true] {
            if let Some(mut b) = assessed_attempt(
                tile,
                &merged,
                interface,
                *caps.last().unwrap(),
                children_error,
                &reference,
                &contracted_faces,
                floor,
                preserve_holes,
                &mut opening_pins,
                &mut attempted,
                assess,
                check,
            )? {
                // Final comparison already counts actual changes against the reference.
                topology.dropped_components = 0;
                topology.closed_holes = 0;
                topology.opened_holes = 0;
                attempted.add(topology);
                b.topology.add(topology);
                return Ok(Outcome {
                    built: Some(b),
                    contraction_failed: false,
                    attempted_topology: attempted,
                    construction_dropped: Vec::new(),
                });
            }
        }
    }
    attempted.add(topology);
    Ok(Outcome {
        built: None,
        contraction_failed: false,
        attempted_topology: attempted,
        construction_dropped,
    })
}

/// Failed contraction is not an omission decision. Preserve the old border samples
/// while reducing separate sheets at both caps, then try constructive and six bounded
/// interior remeshes. Every final contraction is checked against the original faces.
fn recover_contraction(
    merged: &mut Merged,
    reference: &Merged,
    interface: &[InterfaceRow],
    previous: &[InterfaceRow],
    mapping: &BorderMap<'_>,
    caps: &[usize],
    receipt: &mut TopologyReceipt,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<bool, JobError> {
    let border = interface_points(previous);
    let locked: Vec<_> = merged.keys.iter().map(|k| border.contains(k)).collect();
    drop(border);
    for round in 0..caps.len() + 7 {
        check()?;
        merged.triangles.clone_from(&reference.triangles);
        merged.representative.clone_from(&reference.representative);
        let mut attempt = TopologyReceipt::default();
        if round < caps.len() {
            if caps[..round].contains(&caps[round]) {
                continue;
            }
            let (faces, _) = sheets::simplify(
                merged,
                &merged.triangles,
                &|v| locked[v as usize],
                caps[round],
            );
            if faces == merged.triangles {
                continue;
            }
            merged.triangles = faces;
            merged
                .representative
                .resize(merged.triangles.len(), reference.representative[0]);
        } else if round == caps.len() {
            let Some(r) = constrained::remesh(merged, previous, reference.triangles.len()) else {
                continue;
            };
            attempt.add(r);
        } else if !topology::remesh(merged, previous, (round - caps.len()) as u32, &mut attempt) {
            receipt.rejected_remesh_rounds += attempt.rejected_remesh_rounds;
            continue;
        }
        if topology::contract_from(
            merged,
            interface,
            previous,
            mapping,
            &mut attempt,
            Some(&reference.triangles),
        )? {
            receipt.add(attempt);
            return Ok(true);
        }
        receipt.rejected_remesh_rounds += attempt.rejected_remesh_rounds;
    }
    Ok(false)
}

/// Keep the source face ring around a rejected new hole before considering omission.
/// Pins grow monotonically and retries are bounded; no patch or new boundary is invented.
fn assessed_attempt(
    tile: TileKey,
    merged: &Merged,
    interface: &[InterfaceRow],
    cap: usize,
    children_error: f64,
    reference: &Merged,
    contracted_faces: &[[u32; 3]],
    floor: f64,
    preserve_holes: bool,
    opening_pins: &mut HashSet<Key>,
    attempted: &mut TopologyReceipt,
    assess: &mut dyn FnMut(&mut Built) -> Result<bool, JobError>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Option<Built>, JobError> {
    for round in 0..=RETRIES {
        check()?;
        let proposal = attempt(
            tile,
            merged,
            interface,
            cap,
            children_error,
            reference,
            contracted_faces,
            floor,
            preserve_holes,
            opening_pins,
        );
        let mut b = match proposal {
            Ok(b) => b,
            Err(e) if e.kind() == JobErrorKind::Unsupported => {
                #[cfg(test)]
                if std::env::var_os("RT_HLOD_FOLD_REPLAY").is_some() {
                    eprintln!("replay unsupported cap={cap} preserve_holes={preserve_holes} round={round}: {e}");
                }
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        b.retries += round;
        let accepted = assess(&mut b)?;
        if !accepted {
            b.change.receipt.rejected_hole_closures += b.change.receipt.closed_holes;
            b.change.receipt.rejected_hole_openings += b.change.receipt.opened_holes;
        }
        attempted.add(b.change.receipt);
        if accepted {
            return Ok(Some(b));
        }
        if round == RETRIES || b.change.receipt.opened_holes == 0 {
            break;
        }
        let rim: HashSet<Key> = b
            .change
            .opening_boundary
            .iter()
            .flatten()
            .map(|p| key(*p))
            .collect();
        let before = opening_pins.len();
        for t in &reference.triangles {
            let keys = t.map(|v| reference.keys[v as usize]);
            if keys.iter().any(|k| rim.contains(k)) {
                opening_pins.extend(keys);
            }
        }
        if opening_pins.len() == before {
            break;
        }
    }
    Ok(None)
}

/// One bounded parent proposal. Unsupported means its protected repair did not
/// produce an eligible capped result; it is not a proof of cap infeasibility.
fn attempt(
    tile: TileKey,
    merged: &Merged,
    interface: &[InterfaceRow],
    cap: usize,
    children_error: f64,
    reference: &Merged,
    contracted_faces: &[[u32; 3]],
    floor: f64,
    preserve_holes: bool,
    opening_pins: &HashSet<Key>,
) -> Result<Built, JobError> {
    let mut remaining = REDUCTION_PASSES;
    let ordinary = attempt_mode(
        tile,
        merged,
        interface,
        cap,
        children_error,
        reference,
        contracted_faces,
        floor,
        preserve_holes,
        opening_pins,
        false,
        &mut remaining,
    );
    // An overprotected ring is a failed strategy, not a cap lower bound. Both
    // strategies retain the same source guards and pre-contraction reference.
    match ordinary {
        Err(e) if e.kind() == JobErrorKind::Unsupported => attempt_mode(
            tile,
            merged,
            interface,
            cap,
            children_error,
            reference,
            contracted_faces,
            floor,
            preserve_holes,
            opening_pins,
            true,
            &mut remaining,
        ),
        result => result,
    }
}

fn attempt_mode(
    tile: TileKey,
    merged: &Merged,
    interface: &[InterfaceRow],
    cap: usize,
    children_error: f64,
    reference: &Merged,
    contracted_faces: &[[u32; 3]],
    floor: f64,
    preserve_holes: bool,
    opening_pins: &HashSet<Key>,
    separate_sheets: bool,
    remaining: &mut u32,
) -> Result<Built, JobError> {
    let _cpu =
        super::cpu_profile::start(super::cpu_profile::Stage::Attempt, merged.triangles.len());
    let required = || {
        let segments: HashSet<(Key, Key)> = interface
            .iter()
            .filter(|r| !r.is_vertex())
            .map(|r| (r.a, r.b))
            .collect();
        let vertices: HashSet<Key> = interface
            .iter()
            .filter(|r| r.is_vertex())
            .map(|r| r.a)
            .collect();
        (segments, vertices)
    };
    let (segments, vertices) = required();
    let (interface_segments, interface_vertices) = (segments.len(), vertices.len());
    // By induction every child kept its own outer interface, a superset of these.
    let (lost_edges, lost_points) = missing(&merged.triangles, &merged.keys, &segments, &vertices);
    if !lost_edges.is_empty() || !lost_points.is_empty() {
        return Err(state(format!(
            "children of f{}-d{} lost {} interface segments and {} interface vertices",
            tile.first,
            tile.depth,
            lost_edges.len(),
            lost_points.len()
        )));
    }
    let mut locked: Vec<bool> = if preserve_holes {
        topology::protected(merged, interface, true).0
    } else {
        merged.keys.iter().map(|k| vertices.contains(k)).collect()
    };
    let contact_pins = folds::contact_pins(&merged.keys, &merged.triangles, interface);
    for (v, lock) in locked.iter_mut().enumerate() {
        *lock |= opening_pins.contains(&merged.keys[v]) || contact_pins[v];
    }
    drop(contact_pins);
    drop(segments);
    drop(vertices);
    let input = merged.triangles.len();
    let mut protected = vec![false; input];
    let mut retries = 0;
    let degenerate_only = merged
        .triangles
        .iter()
        .all(|t| t[0] == t[1] || t[1] == t[2] || t[0] == t[2]);
    let mut accepted_change = None;
    let (output, simplifier_error, stopping) = if input <= cap {
        (merged.triangles.clone(), 0., StoppingBound::Input)
    } else if degenerate_only {
        (
            merged.triangles[..cap].to_vec(),
            0.,
            StoppingBound::TriangleBudget,
        )
    } else {
        let mut reduce = |faces: &[[u32; 3]], lock: &dyn Fn(u32) -> bool, target| {
            let reserve = u32::from(!separate_sheets);
            if *remaining <= reserve {
                return Err(JobError::new(
                    JobErrorKind::Unsupported,
                    "protected reduction exhausted its shared strategy budget",
                ));
            }
            *remaining -= 1;
            Ok(if separate_sheets {
                sheets::simplify(merged, faces, lock, target)
            } else {
                simplify(merged, faces, lock, target)
            })
        };
        let (mut output, mut error) = reduce(&merged.triangles, &|v| locked[v as usize], cap)?;
        let mut topology_protected = false;
        let mut fold_retries = 0;
        // Only one immutable post-contraction source per retry sequence. Release
        // the interface query sets before the guard, and all repair sets before
        // meshopt, so these maps fit the same parent workspace charge.
        let mut fold_source = None;
        loop {
            let (segments, vertices) = required();
            let (edges, points) = missing_owned(&output, &merged.keys, segments, vertices);
            let invalid = fold_source
                .get_or_insert_with(|| {
                    folds::Source::new(&merged.positions, contracted_faces).with_boundary(interface)
                })
                .violations(&merged.positions, &output);
            #[cfg(test)]
            if std::env::var_os("RT_HLOD_FOLD_REPLAY").is_some() {
                eprintln!(
                    "replay retry={retries} faces={} invalid={} protected={} missing={}/{}",
                    output.len(),
                    invalid.iter().filter(|v| **v).count(),
                    protected.iter().filter(|v| **v).count(),
                    edges.len(),
                    points.len()
                );
            }
            let valid = edges.is_empty() && points.is_empty() && !invalid.iter().any(|v| *v);
            if valid {
                // Topology comparison has its own tables; do not overlap them
                // with a retained fold source. Rebuild only if it rejects.
                fold_source = None;
                accepted_change = topology::changes(reference, &output, interface, floor);
                if accepted_change.is_some() {
                    break;
                }
            }
            let previously_protected = protected.iter().filter(|p| **p).count();
            if valid || fold_retries == FOLD_RETRIES {
                fold_source = None;
                if topology_protected {
                    return Err(JobError::new(
                        JobErrorKind::Unsupported,
                        format!("coarse tile f{}-d{}: protected reduction exhausted its repair budget for topology and orientation ({input} input triangles, {} protected, cap {cap})",
                            tile.first, tile.depth, protected.iter().filter(|p| **p).count()),
                    ));
                }
                // Keep these carriers in the SAME partition as all previous fold
                // protection. A fresh whole-mesh reduction would discard that evidence.
                for (keep, support) in protected.iter_mut().zip(topology::support_faces(
                    merged,
                    interface,
                    preserve_holes,
                )) {
                    *keep |= support;
                }
                topology_protected = true;
                fold_retries = 0;
            } else {
                fold_retries += 1;
            }
            retries += 1;
            let edges: HashSet<(Key, Key)> = edges.into_iter().collect();
            let points: HashSet<Key> = points.into_iter().collect();
            let mut carried = if separate_sheets {
                vec![false; invalid.len()]
            } else {
                Vec::new()
            };
            for (i, t) in merged.triangles.iter().enumerate() {
                let o = t.map(|v| merged.keys[v as usize]);
                if (0..3)
                    .any(|k| edges.contains(&edge(o[k], o[(k + 1) % 3])) || points.contains(&o[k]))
                {
                    protected[i] = true;
                }
                // A full vertex star can consume the cap even on a two-sided plate.
                // The sheet retry retains one fresh local carrier per offending
                // corner per round; all failures are rechecked on the next proposal.
                let needed = t
                    .iter()
                    .any(|&v| invalid[v as usize] && (!separate_sheets || !carried[v as usize]));
                if needed && (!separate_sheets || !protected[i]) {
                    protected[i] = true;
                    if separate_sheets {
                        for &v in t {
                            carried[v as usize] = true;
                        }
                    }
                }
            }
            drop(carried);
            if protected.iter().filter(|p| **p).count() == previously_protected {
                return Err(JobError::new(
                    JobErrorKind::Unsupported,
                    "protected reduction made no progress",
                ));
            }
            drop(edges);
            drop(points);
            drop(invalid);
            if separate_sheets {
                // Partition incidence workspace and the cached Source must not
                // overlap; the regular path keeps its established retry cache.
                fold_source = None;
            }
            let keep: Vec<[u32; 3]> = merged
                .triangles
                .iter()
                .zip(&protected)
                .filter(|(_, p)| **p)
                .map(|(t, _)| *t)
                .collect();
            let rest: Vec<[u32; 3]> = merged
                .triangles
                .iter()
                .zip(&protected)
                .filter(|(_, p)| !**p)
                .map(|(t, _)| *t)
                .collect();
            let kept: HashSet<u32> = keep.iter().flatten().copied().collect();
            let (mut reduced, e) = if rest.is_empty() {
                (Vec::new(), 0.)
            } else {
                reduce(
                    &rest,
                    &|v| locked[v as usize] || kept.contains(&v),
                    cap.saturating_sub(keep.len()),
                )?
            };
            reduced.extend(keep);
            output = reduced;
            error = e;
        }
        if output.len() > cap {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                format!(
                    "coarse tile f{}-d{}: its locked outer interface ({} segments, {} vertices) \
                     and {} protected faces cannot be simplified to the {cap}-triangle cap \
                     ({input} input triangles, {} after simplification)",
                    tile.first,
                    tile.depth,
                    interface_segments,
                    interface_vertices,
                    protected.iter().filter(|p| **p).count(),
                    output.len()
                ),
            ));
        } else {
            (output, error, StoppingBound::TriangleBudget)
        }
    };
    if matches!(stopping, StoppingBound::Input)
        && folds::Source::new(&merged.positions, contracted_faces)
            .with_boundary(interface)
            .violations(&merged.positions, &output)
            .iter()
            .any(|v| *v)
    {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            "remeshing introduces coincident or folded faces",
        ));
    }
    if output.is_empty() {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            format!(
                "coarse tile f{}-d{} simplified to no triangles",
                tile.first, tile.depth
            ),
        ));
    }
    // The successful loop check used these same immutable inputs. Preserve its
    // witnesses before compaction; input/degenerate-only paths still compare here.
    let topology_change =
        accepted_change.or_else(|| topology::changes(reference, &output, interface, floor));
    let Some(change) = topology_change else {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            "simplification removes a border-connected component or touching boundary loop",
        ));
    };
    if !simplifier_error.is_finite() {
        return Err(state("meshopt reported a nonfinite error"));
    }

    // Compact to the vertices the output uses, in first-use order.
    let mut remap = vec![u32::MAX; merged.keys.len()];
    let mut geometry = TileGeometry {
        key: tile,
        ells: Vec::new(),
        positions: Vec::new(),
        triangles: Vec::with_capacity(output.len()),
        representative: Vec::with_capacity(output.len()),
        primitives: Vec::new(),
        interface: interface.to_vec(),
        geometric_error: 0.,
        lease: None,
    };
    let mut out_locked = Vec::new();
    for t in &output {
        geometry.triangles.push(t.map(|v| {
            let slot = &mut remap[v as usize];
            if *slot == u32::MAX {
                *slot = geometry.positions.len() as u32;
                geometry.ells.push(merged.ells[v as usize]);
                geometry.positions.push(merged.positions[v as usize]);
                out_locked.push(locked[v as usize]);
            }
            *slot
        }));
    }
    geometry.primitives.push(merged.representative[0]);
    geometry.representative.resize(output.len(), 0);
    geometry.interface.shrink_to_fit();
    geometry.ells.shrink_to_fit();
    geometry.positions.shrink_to_fit();
    geometry.triangles.shrink_to_fit();
    geometry.representative.shrink_to_fit();
    geometry.primitives.shrink_to_fit();
    Ok(Built {
        locked: out_locked,
        stopping,
        simplifier_error,
        children_error,
        input_triangles: input as u32,
        protected_faces: protected.iter().filter(|p| **p).count() as u32,
        retries,
        interface_vertices: interface_vertices as u32,
        interface_segments: interface_segments as u32,
        reoriented: 0,
        topology: change.receipt,
        change,
        controls: Default::default(),
        geometry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_sheet_shortcut_matches_pinned_meshopt_with_degeneracies() {
        for seed in 0..32u32 {
            let positions: Vec<_> = (0..12)
                .map(|i| {
                    if seed % 4 == 0 {
                        [0., -0., 0.]
                    } else {
                        [((i * 7 + seed) % 5) as f32, (i % 3) as f32, 0.]
                    }
                })
                .collect();
            // Includes repeated positions/indices, duplicate and reversed faces,
            // nonmanifold shared edges and unused vertices. No cleanup is implied.
            let triangles = vec![
                [1, 2, 3],
                [3, 2, 1],
                [1, 2, 3],
                [1, 2, 4],
                [1, 5, 2],
                [1, 1, 6],
                [7, 8, 9],
            ];
            let merged = Merged {
                keys: positions.iter().copied().map(key).collect(),
                ells: vec![1; positions.len()],
                positions,
                triangles: triangles.clone(),
                representative: vec![],
            };
            for (locked, target) in [(true, 0), (true, 3), (false, triangles.len())] {
                let indices: Vec<_> = triangles.iter().flatten().copied().collect();
                let mut actual = vec![0; indices.len()];
                let locks = vec![u8::from(locked); merged.positions.len()];
                let mut error = -1f32;
                // SAFETY: buffers cover the declared indices/vertices, positions
                // are tightly packed f32x3, and all indices are in range.
                let n = unsafe {
                    meshopt::ffi::meshopt_simplifyWithAttributes(
                        actual.as_mut_ptr(),
                        indices.as_ptr(),
                        indices.len(),
                        merged.positions.as_ptr().cast::<f32>(),
                        merged.positions.len(),
                        12,
                        std::ptr::null(),
                        0,
                        std::ptr::null(),
                        0,
                        locks.as_ptr(),
                        target * 3,
                        f32::MAX,
                        meshopt::SimplifyOptions::ErrorAbsolute.bits(),
                        &mut error,
                    )
                };
                actual.truncate(n);
                assert_eq!(actual, indices);
                assert_eq!(error.to_bits(), 0);
                let mut local = vec![u32::MAX; merged.positions.len()];
                let (output, estimate) =
                    simplify_mapped(&merged, &triangles, &|_| locked, target, &mut local);
                assert_eq!(output, triangles);
                assert_eq!(estimate.to_bits(), 0);
                assert!(local.iter().all(|&i| i == u32::MAX));
            }
        }
        assert!(!finite_extent(
            &[[f32::MAX, 0., 0.], [-f32::MAX, 0., 0.]],
            &[[0, 1, 0]]
        ));
        assert!(!finite_extent(&[[f32::NAN, 0., 0.]], &[[0, 0, 0]]));
    }

    #[test]
    fn parent_mapping_queries_only_child_interfaces_and_keeps_the_contracted_arc() {
        let parent = TileKey {
            first: 0,
            end: 2,
            depth: 0,
            leaf_begin: 0,
            leaf_end: 1,
        };
        let positions = vec![
            [0., 0., 0.],
            [0.5, 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [1., 1., 0.],
        ];
        let [a, middle, b] = [key(positions[0]), key(positions[1]), key(positions[2])];
        let child = TileGeometry {
            key: TileKey { depth: 1, ..parent },
            positions,
            ells: vec![1; 5],
            triangles: vec![[0, 1, 3], [1, 4, 3], [1, 2, 4]],
            representative: vec![0; 3],
            primitives: vec![SourcePrimitive {
                node: 0,
                mesh: 0,
                primitive: 0,
                expanded_ordinal: 0,
                material: u32::MAX,
            }],
            interface: vec![
                InterfaceRow::new(0, a, middle, 0, 1),
                InterfaceRow::new(0, middle, b, 0, 1),
            ],
            geometric_error: 0.,
            lease: None,
        };
        let calls = std::cell::Cell::new(0);
        let built = build_parent(
            parent,
            &[child],
            &[8],
            &Contract {
                depth: 0,
                interface: vec![InterfaceRow::new(0, a, b, 0, 1)],
                certificates: vec![],
                terminal_vertices: vec![],
                scale: 0.,
            },
            &|p| {
                assert!([a, middle, b].contains(&p), "interior source lookup");
                calls.set(calls.get() + 1);
                Ok((p == middle).then_some((a, b, 0.5, 1)))
            },
            &mut |_| Ok(true),
            &|| Ok(()),
        )
        .unwrap()
        .built
        .unwrap();
        assert_eq!(calls.get(), 3);
        let geometry = built.geometry;
        let keys: Vec<_> = geometry.positions.iter().copied().map(key).collect();
        let absent = missing(
            &geometry.triangles,
            &keys,
            &[edge(a, b)].into_iter().collect(),
            &HashSet::default(),
        );
        assert!(absent.0.is_empty());
        assert!(built.topology.contracted_vertices > 0);
    }

    #[test]
    fn thin_tetrahedron_collapse_retries_instead_of_publishing_fresh_twins() {
        let positions = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0.25, 0.25, 0.01],
            [1. / 3., 1. / 3., 0.],
        ];
        let triangles = vec![
            [0, 1, 4],
            [1, 2, 4],
            [2, 0, 4],
            [0, 3, 1],
            [1, 3, 2],
            [2, 3, 0],
        ];
        let merged = Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            positions,
            representative: Vec::new(),
            triangles,
        };
        let source = folds::Source::new(&merged.positions, &merged.triangles);
        let (raw, _) = simplify(&merged, &merged.triangles, &|v| v < 3, 2);
        assert!(
            source
                .violations(&merged.positions, &raw)
                .iter()
                .any(|v| *v),
            "{raw:?}"
        );
        let interface = (0..3)
            .map(|v| InterfaceRow::new(0, merged.keys[v], merged.keys[v], 0, 1))
            .collect::<Vec<_>>();
        let mut merged = merged;
        merged.representative.resize(
            merged.triangles.len(),
            SourcePrimitive {
                node: 0,
                mesh: 0,
                primitive: 0,
                expanded_ordinal: 0,
                material: u32::MAX,
            },
        );
        let tile = TileKey {
            first: 0,
            end: 6,
            depth: 0,
            leaf_begin: 0,
            leaf_end: 1,
        };
        let pins = HashSet::default();
        assert!(attempt(
            tile,
            &merged,
            &interface,
            2,
            0.,
            &merged,
            &merged.triangles,
            1.,
            false,
            &pins
        )
        .is_err());
        let built = attempt(
            tile,
            &merged,
            &interface,
            6,
            0.,
            &merged,
            &merged.triangles,
            1.,
            false,
            &pins,
        )
        .unwrap();
        assert_eq!(built.geometry.triangles.len(), 6);
        assert_eq!(built.reoriented, 0);
    }
}
