//! Constructive cap fit for consistently projected manifold patches. Keep boundary loops
//! and mandatory interface items, discard source interior sampling, and retriangulate.
//! Outside this domain return to the bounded general remesher without changing the mesh.
use super::{topology, Merged};
use crate::mesh_large::hlod::{interfaces::InterfaceRow, store::SourcePrimitive};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use spade::{ConstrainedDelaunayTriangulation, Point2, PositionInTriangulation, Triangulation};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[path = "constrained_fan.rs"]
mod fan;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PatchPlan {
    Charts,
    Planes,
    ChordSpokes,
}

fn edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// A face whose height over its longest edge is below 1/250 of that edge: its normal is
/// rounding noise, so its chart says nothing about the sheet it belongs to. This only
/// steers which faces one CDT rebuilds together; it licenses nothing.
fn sliver(positions: &[[f32; 3]], t: [u32; 3]) -> bool {
    let p = t.map(|v| positions[v as usize].map(f64::from));
    let n = topology::normal(positions, t);
    let twice_area = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let longest = (0..3)
        .map(|k| {
            let (a, b) = (p[k], p[(k + 1) % 3]);
            (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
        })
        .fold(0., f64::max);
    twice_area < longest / 250.
}

pub(super) fn remesh(
    m: &mut Merged,
    interface: &[InterfaceRow],
    cap: usize,
) -> Option<topology::Receipt> {
    let guard = super::folds::Source::new(&m.positions, &m.triangles).with_boundary(interface);
    triangulate(
        m,
        interface,
        cap,
        None,
        &[],
        &guard,
        PatchPlan::Charts,
        false,
        false,
    )
    .or_else(|| fan::remesh(m, interface, cap, &guard))
}

/// Reconstruct contracted patches with all unaffected interior samples. In particular,
/// no moved face is reoriented to conceal an inversion, and no distant support vertex is
/// chosen to invent a replacement face. Nonmanifold edges separate their surface sheets.
pub(super) fn contract(
    m: &mut Merged,
    interface: &[InterfaceRow],
    map: &[u32],
    endpoints: &[u32],
    cap: usize,
    guard: &super::folds::Source,
    fallbacks: bool,
) -> Option<topology::Receipt> {
    // All three constructions consume the same pre-contraction source. A stricter chart
    // partition may separate book pages that reconnect around a nonmanifold spine;
    // it cannot confer any new face or edge license.
    // A deterministic CDT can select the same new ear on opposite sheets.
    // One fixed endpoint-rooted partition changes those interior diagonals,
    // retaining every source sample and the same original-source checks.
    // `fallbacks` (only once the caller's ordinary contraction recovery failed): flip
    // interface diagonals, and when all three plans fail, repeat them with slivers joined
    // to their neighbours' patches. A near-degenerate child face beside a chord otherwise
    // forms a patch of its own, and its rebuild overlaps the neighbouring one.
    [false, true][..1 + usize::from(fallbacks)]
        .iter()
        .find_map(|&slivers| {
            [PatchPlan::Charts, PatchPlan::Planes, PatchPlan::ChordSpokes]
                .into_iter()
                .find_map(|plan| {
                    triangulate(
                        m,
                        interface,
                        cap,
                        Some(map),
                        endpoints,
                        guard,
                        plan,
                        slivers,
                        fallbacks,
                    )
                })
        })
}

#[allow(clippy::too_many_arguments)]
fn triangulate(
    m: &mut Merged,
    interface: &[InterfaceRow],
    cap: usize,
    mapping: Option<&[u32]>,
    endpoints: &[u32],
    guard: &super::folds::Source,
    plan: PatchPlan,
    slivers: bool,
    flips: bool,
) -> Option<topology::Receipt> {
    let components = topology::components(m);
    let mut patches: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    // Freeze the surface beyond the first interior ring. Include both removed vertices
    // and retained chord endpoints: an interior sample on the chord may need its two
    // endpoint faces rebuilt as well. Their unmoved edges form the constrained ring.
    let mut touched = vec![false; m.positions.len()];
    let mut chord_endpoints = if plan == PatchPlan::ChordSpokes {
        vec![false; m.positions.len()]
    } else {
        Vec::new()
    };
    for &v in endpoints {
        if plan == PatchPlan::ChordSpokes {
            chord_endpoints[v as usize] = true;
        } else {
            touched[v as usize] = true;
        }
    }
    if let Some(map) = mapping {
        for (v, &to) in map.iter().enumerate() {
            if v != to as usize {
                touched[v] = true;
                if plan != PatchPlan::ChordSpokes {
                    touched[to as usize] = true;
                }
            }
        }
    }
    // The final construction freezes endpoint-only faces: rebuilding those can
    // move a coincident ear into a neighboring, originally unchanged sheet strip.
    // A root shared with the opposite source chart cannot separate such sheets.
    let mut vertex_charts = if plan == PatchPlan::ChordSpokes {
        vec![0u8; m.positions.len()]
    } else {
        Vec::new()
    };
    if plan == PatchPlan::ChordSpokes {
        for &t in &m.triangles {
            let n = topology::normal(&m.positions, t);
            let axis = (0..3).max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs()))?;
            if n[axis] != 0. {
                let chart = 1 << (2 * axis + usize::from(n[axis] > 0.));
                for v in t {
                    vertex_charts[v as usize] |= chart;
                }
            }
        }
    }
    let affected: Vec<bool> = m
        .triangles
        .iter()
        .map(|t| t.iter().any(|&v| touched[v as usize]))
        .collect();
    let face_patches = if mapping.is_some() {
        manifold_patches(m, &affected, plan == PatchPlan::Planes, slivers)
    } else {
        Vec::new()
    };
    for (i, t) in m.triangles.iter().enumerate() {
        patches
            .entry(if mapping.is_some() {
                face_patches[i]
            } else {
                components[t[0] as usize]
            })
            .or_default()
            .push(i);
    }
    let ids: HashMap<_, _> = m
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u32))
        .collect();
    if patches.len() > cap {
        return rejected("CDT", concat!(file!(), ":", line!()));
    }
    let mut required = Required::default();
    for r in interface {
        let (a, b) = (*ids.get(&r.a)?, *ids.get(&r.b)?);
        if mapping.is_none() && components[a as usize] != components[b as usize] {
            return rejected("CDT", concat!(file!(), ":", line!()));
        }
        required.points.extend([a, b]);
        if a != b {
            required.segments.insert(edge(a, b));
        }
    }
    let mut output = Vec::new();
    let mut representatives = Vec::new();
    let mut count = 0;
    for (_, faces) in patches {
        if mapping.is_some() && faces.iter().all(|&i| !affected[i]) {
            if output.len().checked_add(faces.len())? > cap {
                return rejected("CDT", concat!(file!(), ":", line!()));
            }
            output.extend(faces.iter().map(|&i| m.triangles[i]));
            representatives.extend(faces.iter().map(|&i| m.representative[i]));
            continue;
        }
        let mut edges: BTreeMap<(u32, u32), (u32, i32)> = BTreeMap::new();
        let mut sum = [0f64; 3];
        for &i in &faces {
            let t = m.triangles[i];
            let normal = topology::normal(&m.positions, t);
            for a in 0..3 {
                sum[a] += normal[a];
            }
            for a in 0..3 {
                let (v, w) = (t[a], t[(a + 1) % 3]);
                if v == w {
                    return rejected("CDT", concat!(file!(), ":", line!()));
                }
                let (n, sign) = edges.entry(edge(v, w)).or_default();
                *n += 1;
                *sign += if v < w { 1 } else { -1 };
            }
        }
        if edges
            .values()
            .any(|&(n, sign)| n > 2 || (n == 2 && sign != 0))
        {
            return rejected("CDT", concat!(file!(), ":", line!()));
        }
        let axis = (0..3).max_by(|&a, &b| sum[a].abs().total_cmp(&sum[b].abs()))?;
        if sum[axis] == 0. {
            return rejected("CDT", concat!(file!(), ":", line!()));
        }
        // A joined sliver may face either way in the projection; the CDT replaces it and
        // the source guard and topology comparison still judge the whole proposal.
        if faces.iter().any(|&i| {
            !(slivers && sliver(&m.positions, m.triangles[i]))
                && topology::normal(&m.positions, m.triangles[i])[axis] * sum[axis] <= 0.
        }) {
            return rejected("CDT", concat!(file!(), ":", line!()));
        }
        let mut boundary: BTreeSet<_> = edges
            .iter()
            .filter(|(_, v)| v.0 == 1)
            .map(|(e, _)| *e)
            .collect();
        let mut directed_boundary = Vec::new();
        if let Some(map) = mapping {
            let mut mapped = BTreeMap::<(u32, u32), i32>::new();
            for &(a, b) in &boundary {
                let direction = edges[&(a, b)].1;
                let (a, b) = (map[a as usize], map[b as usize]);
                if a != b {
                    *mapped.entry(edge(a, b)).or_default() +=
                        direction * if a < b { 1 } else { -1 };
                }
            }
            // Opposite incidences cancel when a thin neck collapses to an interior
            // edge. Same-direction multiplicity still cannot be a single sheet.
            if mapped.values().any(|&n| n.abs() > 1) {
                return rejected("CDT", concat!(file!(), ":", line!()));
            }
            if plan == PatchPlan::ChordSpokes {
                directed_boundary.extend(mapped.iter().filter_map(|(&(a, b), &n)| {
                    (n != 0).then_some(if n > 0 { (a, b) } else { (b, a) })
                }));
            }
            boundary = mapped
                .into_iter()
                .filter_map(|(e, n)| (n != 0).then_some(e))
                .collect();
        }
        if boundary.is_empty() {
            // A source strip may collapse completely. Retain no invented support;
            // the unchanged global topology comparison and source guard below still
            // decide whether the entire construction is admissible.
            if mapping.is_some_and(|map| {
                faces.iter().all(|&i| {
                    topology::normal(&m.positions, m.triangles[i].map(|v| map[v as usize]))
                        .iter()
                        .all(|&n| n == 0.)
                })
            }) {
                continue;
            }
            return rejected("CDT", concat!(file!(), ":", line!()));
        }
        let mut adjacent: HashMap<u32, Vec<u32>> = HashMap::default();
        for &(a, b) in &boundary {
            adjacent.entry(a).or_default().push(b);
            adjacent.entry(b).or_default().push(a);
        }
        // A contracted boundary may have several simple loops meeting at one
        // vertex. An even-degree planar boundary still defines its interior by
        // parity; do not walk it as though each vertex had a unique successor.
        // The non-contracting cap fit retains its simple-loop Euler identity.
        if adjacent
            .values()
            .any(|v| v.len() < 2 || v.len() % 2 != 0 || (mapping.is_none() && v.len() != 2))
        {
            return rejected("CDT", concat!(file!(), ":", line!()));
        }
        let mut visited = HashSet::default();
        let mut loops = 0;
        for &start in adjacent.keys() {
            if !visited.insert(start) {
                continue;
            }
            loops += 1;
            let mut stack = vec![start];
            while let Some(at) = stack.pop() {
                for &next in &adjacent[&at] {
                    if visited.insert(next) {
                        stack.push(next);
                    }
                }
            }
        }
        let mut vertices: BTreeSet<u32> = adjacent.keys().copied().collect();
        let mut constraints = boundary.clone();
        required.patch(m, &faces, mapping, &mut vertices, &mut constraints);
        if plan == PatchPlan::ChordSpokes {
            // Change only patches with a proven endpoint-rooted alternative. A
            // longer collinear strip may need its ordinary CDT unchanged while
            // another sheet's new ear is repaired. The original global guard
            // still judges their complete union below.
            let spokes = fan::chord_spokes(
                m,
                &directed_boundary,
                &chord_endpoints,
                &vertex_charts,
                axis,
                sum[axis] > 0.,
            );
            if let Some(spokes) = spokes {
                constraints.extend(spokes);
            }
        }
        // Only degree-two boundaries admit this formula. Touching mapped loops
        // are capped using the actual emitted faces below, including retained input.
        let mut predicted = 2 * (vertices.len() - adjacent.len()) + adjacent.len() + 2 * loops - 4;
        if mapping.is_none() && output.len().checked_add(predicted)? > cap {
            return rejected("CDT", concat!(file!(), ":", line!()));
        }
        let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
        let mut handles = HashMap::default();
        let mut reverse = HashMap::default();
        for v in vertices {
            let p = m.positions[v as usize];
            let handle = cdt
                .insert(Point2::new(
                    f64::from(p[(axis + 1) % 3]),
                    f64::from(p[(axis + 2) % 3]),
                ))
                .ok()?;
            if reverse.insert(handle, v).is_some() {
                return rejected("CDT", concat!(file!(), ":", line!()));
            }
            handles.insert(v, handle);
        }
        for &(a, b) in &constraints {
            if !cdt.can_add_constraint(handles[&a], handles[&b]) {
                return rejected("CDT", concat!(file!(), ":", line!()));
            }
            cdt.add_constraint(handles[&a], handles[&b]);
        }
        if let Some(map) = mapping {
            let interior: BTreeSet<u32> = faces
                .iter()
                .flat_map(|&i| m.triangles[i])
                .filter(|&v| map[v as usize] == v && !handles.contains_key(&v))
                .collect();
            for v in interior {
                let p = m.positions[v as usize];
                let p = Point2::new(f64::from(p[(axis + 1) % 3]), f64::from(p[(axis + 2) % 3]));
                // A collinear interior sample must not split a delivered border chord.
                if let PositionInTriangulation::OnEdge(e) = cdt.locate(p) {
                    let e = cdt.directed_edge(e);
                    let endpoints = edge(reverse[&e.from().fix()], reverse[&e.to().fix()]);
                    if e.is_constraint_edge()
                        && (boundary.contains(&endpoints) || required.segments.contains(&endpoints))
                    {
                        continue;
                    }
                    // An auxiliary spoke is not a delivered chord. Insert a source
                    // sample on it normally, splitting that spoke rather than dropping
                    // the mandatory unmoved interior sample.
                }
                let handle = cdt.insert(p).ok()?;
                if reverse.insert(handle, v).is_some() {
                    return rejected("CDT", concat!(file!(), ":", line!()));
                }
                predicted += 2;
            }
        }
        let mut flags = HashMap::default();
        let mut queue = VecDeque::new();
        for e in cdt.directed_edges().filter(|e| e.face().is_outer()) {
            if let Some(face) = e.rev().face().as_inner() {
                let h = face.fix();
                let inside =
                    boundary.contains(&edge(reverse[&e.from().fix()], reverse[&e.to().fix()]));
                if let Some(old) = flags.insert(h, inside) {
                    if old != inside {
                        return rejected("CDT", concat!(file!(), ":", line!()));
                    }
                } else {
                    queue.push_back(h);
                }
            }
        }
        while let Some(h) = queue.pop_front() {
            let inside = flags[&h];
            for e in cdt.face(h).adjacent_edges() {
                if let Some(f) = e.rev().face().as_inner() {
                    let next = f.fix();
                    let value = inside
                        ^ boundary
                            .contains(&edge(reverse[&e.from().fix()], reverse[&e.to().fix()]));
                    if let Some(old) = flags.insert(next, value) {
                        if old != value {
                            return rejected("CDT", concat!(file!(), ":", line!()));
                        }
                    } else {
                        queue.push_back(next);
                    }
                }
            }
        }
        let start = output.len();
        let mut area = 0.;
        for f in cdt
            .inner_faces()
            .filter(|f| flags.get(&f.fix()) == Some(&true))
        {
            let mut t = f.vertices().map(|v| reverse[&v.fix()]);
            let normal = topology::normal(&m.positions, t)[axis];
            if normal * sum[axis] < 0. {
                t.swap(1, 2);
            }
            area += normal.abs();
            if output.len() == cap {
                return rejected("CDT", concat!(file!(), ":", line!()));
            }
            output.push(t);
            representatives.push(m.representative[faces[0]]);
        }
        if flips {
            flip_interface_edges(
                &m.positions,
                &mut output[start..],
                axis,
                &|v| required.points.contains(&v),
                &|t| guard.licensed(&m.positions, t),
                &|a, b| guard.licensed_edge(&m.positions, a, b),
                &constraints,
            );
        }
        // Positive input Jacobians plus a simple boundary of degree one establish the
        // height-field domain. The area identity detects inconsistent ring orientation.
        if (mapping.is_none() && output.len() - start != predicted)
            || (mapping.is_none() && (area - sum[axis].abs()).abs() > 1e-10 * area.max(1.))
        {
            return rejected("CDT", concat!(file!(), ":", line!()));
        }
        count += 1;
    }
    // The patch iterator and each CDT have finished. Release their shared indexes
    // before missing/topology/Source work adds its own validation workspace.
    drop((
        ids,
        required,
        components,
        affected,
        touched,
        face_patches,
        chord_endpoints,
        vertex_charts,
    ));
    finish(
        m,
        interface,
        output,
        representatives,
        guard,
        count,
        mapping.is_some(),
    )
}

/// The source guard refuses a new edge joining two interface points (either neighbour could
/// build it) and a new face of interface points only, yet the CDT picks such diagonals
/// where the patch also has an interior sample. Flip each unconstrained interface-to-
/// interface edge that is unlicensed, or borders such a face, where the apex across it on
/// one side is interior and the quad is strictly convex in the patch projection. A flip
/// removes that edge and adds a diagonal with an interior end, so there is at most one
/// flip per edge. A licensed source edge beside such a face is flipped too, with the
/// source face on its other side; the guard judges the result. Linear worklist.
fn flip_interface_edges(
    positions: &[[f32; 3]],
    faces: &mut [[u32; 3]],
    axis: usize,
    boundary: &dyn Fn(u32) -> bool,
    licensed: &dyn Fn([u32; 3]) -> bool,
    licensed_edge: &dyn Fn(u32, u32) -> bool,
    constraints: &BTreeSet<(u32, u32)>,
) {
    let q = |v: u32| {
        let p = positions[v as usize];
        [f64::from(p[(axis + 1) % 3]), f64::from(p[(axis + 2) % 3])]
    };
    let orient = |a: u32, b: u32, c: u32| {
        let (a, b, c) = (q(a), q(b), q(c));
        let d = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        // Three-valued: `f64::signum(0.0)` is 1, which would pass a collinear apex.
        f64::from(i8::from(d > 0.) - i8::from(d < 0.))
    };
    let all = |t: &[u32; 3]| t.iter().all(|&v| boundary(v)) && !licensed(*t);
    let mut directed: HashMap<(u32, u32), usize> = HashMap::default();
    for (i, t) in faces.iter().enumerate() {
        for k in 0..3 {
            directed.insert((t[k], t[(k + 1) % 3]), i);
        }
    }
    let bad = |faces: &[[u32; 3]], directed: &HashMap<(u32, u32), usize>, a: u32, b: u32| {
        boundary(a)
            && boundary(b)
            && !constraints.contains(&edge(a, b))
            && (!licensed_edge(a, b)
                || [(a, b), (b, a)]
                    .iter()
                    .any(|e| directed.get(e).is_some_and(|&f| all(&faces[f]))))
    };
    let mut queue: VecDeque<(u32, u32)> = directed
        .keys()
        .filter(|&&(a, b)| a < b && bad(faces, &directed, a, b))
        .copied()
        .collect();
    while let Some((a, b)) = queue.pop_front() {
        let (Some(&i), Some(&j)) = (directed.get(&(a, b)), directed.get(&(b, a))) else {
            continue;
        };
        if !bad(faces, &directed, a, b) {
            continue;
        }
        let (f, g) = (faces[i], faces[j]);
        let Some(c) = f.into_iter().find(|&v| v != a && v != b) else {
            continue;
        };
        let Some(x) = g.into_iter().find(|&v| v != a && v != b) else {
            continue;
        };
        let s = orient(a, b, c);
        // Strictly convex quad a-x-b-c, both new faces in the patch orientation, and
        // the new diagonal has an interior end.
        if (boundary(c) && boundary(x))
            || s == 0.
            || orient(a, b, x) != -s
            || orient(a, x, c) != s
            || orient(x, b, c) != s
        {
            continue;
        }
        for t in [f, g] {
            for n in 0..3 {
                directed.remove(&(t[n], t[(n + 1) % 3]));
            }
        }
        faces[i] = [a, x, c];
        faces[j] = [x, b, c];
        for (id, t) in [(i, faces[i]), (j, faces[j])] {
            for n in 0..3 {
                directed.insert((t[n], t[(n + 1) % 3]), id);
            }
        }
        for (u, v) in [(a, c), (c, b), (b, x), (x, a)] {
            if bad(faces, &directed, u, v) {
                queue.push_back((u, v));
            }
        }
    }
}

// A declined construction step; `stage` and `reason` name the rejected case at the call site.
fn rejected<T>(_stage: &str, _reason: &str) -> Option<T> {
    None
}

// All constructive paths use the original source and the same publication checks.
fn finish(
    m: &mut Merged,
    interface: &[InterfaceRow],
    output: Vec<[u32; 3]>,
    representatives: Vec<SourcePrimitive>,
    guard: &super::folds::Source,
    count: u64,
    count_reconstructed: bool,
) -> Option<topology::Receipt> {
    let required: HashSet<_> = interface
        .iter()
        .filter(|r| !r.is_vertex())
        .map(|r| (r.a, r.b))
        .collect();
    let points = super::interface_points(interface);
    let lost = super::missing_owned(&output, &m.keys, required, points);
    if !lost.0.is_empty() || !lost.1.is_empty() {
        return rejected("finish", "missing required interface edge or point");
    }
    drop(lost);
    // Reject an illegal proposal before allocating topology witnesses. Both checks
    // consume the same immutable input and neither grants a source license.
    let invalid = guard.violations(&m.positions, &output);
    if invalid.iter().any(|v| *v) {
        return rejected("finish", "original Source rejected candidate");
    }
    drop(invalid);
    let change = topology::changes(m, &output, interface, 0.)
        .or_else(|| rejected("finish", "topology::changes refused"))?;
    if change.receipt.closed_holes != 0
        || change.receipt.opened_holes != 0
        || change.receipt.dropped_components != 0
    {
        return rejected("finish", "closed/opened hole or dropped component");
    }
    let mut receipt = change.receipt;
    drop(change);
    receipt.constrained_remeshes = count;
    if count_reconstructed {
        // Count new oriented output faces with multiplicity. Rebuilt patches may
        // also contain unchanged faces; neither those nor frozen input are repairs.
        let mut old = HashMap::<[u32; 3], usize>::default();
        for t in &m.triangles {
            *old.entry(super::oriented(t)).or_default() += 1;
        }
        for t in &output {
            match old.get_mut(&super::oriented(t)) {
                Some(n) if *n > 0 => *n -= 1,
                _ => receipt.reconstructed_faces += 1,
            }
        }
    }
    m.triangles = output;
    m.representative = representatives;
    Some(receipt)
}

/// Source-edge join, rather than patch x interface-row endpoint matching. Every
/// delivered chord has a carrier among mapped source edges (the crossing of its
/// certified old arc); ordinary remesh segments are already source edges. Points
/// are pinned on each incident patch. Total probes are three edges/vertices per
/// input face, including high-valence shared endpoints and duplicate rows.
#[derive(Default)]
struct Required {
    segments: HashSet<(u32, u32)>,
    points: HashSet<u32>,
    #[cfg(test)]
    probes: usize,
}
impl Required {
    fn patch(
        &mut self,
        m: &Merged,
        faces: &[usize],
        mapping: Option<&[u32]>,
        vertices: &mut BTreeSet<u32>,
        constraints: &mut BTreeSet<(u32, u32)>,
    ) {
        for &i in faces {
            let t = m.triangles[i].map(|v| mapping.map_or(v, |map| map[v as usize]));
            for k in 0..3 {
                #[cfg(test)]
                {
                    self.probes += 1;
                }
                if self.points.contains(&t[k]) {
                    vertices.insert(t[k]);
                }
                let e = edge(t[k], t[(k + 1) % 3]);
                if self.segments.contains(&e) {
                    vertices.extend([e.0, e.1]);
                    constraints.insert(e);
                }
            }
        }
    }
}

fn manifold_patches(
    m: &Merged,
    affected: &[bool],
    separate_sheets: bool,
    slivers: bool,
) -> Vec<usize> {
    let faces = &m.triangles;
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut edges = HashMap::<(u32, u32), (usize, usize, usize, i32)>::default();
    for (i, t) in faces.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let e = edges.entry(edge(a, b)).or_insert((i, i, 0, 0));
            e.1 = i;
            e.2 += 1;
            e.3 += if a < b { 1 } else { -1 };
        }
    }
    let mut parent: Vec<_> = (0..faces.len()).collect();
    for &(a, b, count, direction) in edges.values() {
        // A folded book can be manifold along a two-page spine, yet cannot be one
        // projected patch. Dominant-axis/sign charts split such folds deterministically.
        let chart = |i: usize| {
            let n = topology::normal(&m.positions, faces[i]);
            let axis = (0..3)
                .max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs()))
                .unwrap();
            // The first pass is the original dominant-axis chart. The second
            // keeps coplanar source panels separate, even if a curved connection
            // elsewhere would reunite pages around a spine. Adjacent faces with
            // the same normal ray share a plane. No angular tolerance, position
            // snapping, or extra source license is introduced by this partition.
            let panel = if separate_sheets && n[axis] != 0. {
                std::array::from_fn(|a| {
                    let ratio = n[a] / n[axis].abs();
                    if ratio == 0. {
                        0
                    } else {
                        ratio.to_bits()
                    }
                })
            } else {
                [0; 3]
            };
            (axis, n[axis] > 0., panel)
        };
        // A sliver's normal is noise: in the sliver pass it joins its neighbour's patch.
        let sliver = |i: usize| slivers && sliver(&m.positions, faces[i]);
        if count == 2
            && direction == 0
            && affected[a]
            && affected[b]
            && (chart(a) == chart(b) || sliver(a) || sliver(b))
        {
            let (a, b) = (root(&mut parent, a), root(&mut parent, b));
            parent[a.max(b)] = a.min(b);
        }
    }
    (0..faces.len()).map(|i| root(&mut parent, i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::hlod::{interfaces::key, store::SourcePrimitive};

    fn contract_path(
        m: &mut Merged,
        interface: &[InterfaceRow],
        map: &[u32],
        endpoints: &[u32],
        path: &[u32],
    ) -> Option<topology::Receipt> {
        let (a, b) = (m.keys[path[0] as usize], m.keys[*path.last()? as usize]);
        let mut mapping = vec![None; m.keys.len()];
        for (i, &v) in path.iter().enumerate().skip(1).take(path.len() - 2) {
            mapping[v as usize] = Some((a, b, i as f64 / (path.len() - 1) as f64, 0));
        }
        let previous: Vec<_> = path
            .windows(2)
            .map(|pair| {
                InterfaceRow::new(0, m.keys[pair[0] as usize], m.keys[pair[1] as usize], 0, 1)
            })
            .collect();
        let guard = super::super::folds::Source::for_contraction(
            &m.positions,
            &m.triangles,
            interface,
            &previous,
            &mapping,
        );
        let cap = m.triangles.len();
        contract(m, interface, map, endpoints, cap, &guard, false)
    }
    fn sheet(n: u32, hole: bool) -> Merged {
        let positions: Vec<_> = (0..=n)
            .flat_map(|y| (0..=n).map(move |x| [x as f32, y as f32, 0.]))
            .collect();
        let mut triangles = Vec::new();
        for y in 0..n {
            for x in 0..n {
                if hole && x == n / 2 && y == n / 2 {
                    continue;
                }
                let a = y * (n + 1) + x;
                triangles.extend([[a, a + 1, a + n + 2], [a, a + n + 2, a + n + 1]]);
            }
        }
        Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            positions,
            representative: vec![
                SourcePrimitive {
                    node: 0,
                    mesh: 0,
                    primitive: 0,
                    expanded_ordinal: 0,
                    material: u32::MAX
                };
                triangles.len()
            ],
            triangles,
        }
    }
    #[test]
    fn required_rows_join_source_edges_once_even_with_shared_spine_vertices() {
        for n in [64, 256, 1024, 4096] {
            // Every patch shares both spine endpoints. Endpoint/patch joins scan
            // n rows per patch even though only one non-spine row belongs to it.
            let mut m = sheet(1, false);
            m.positions = vec![[0., 0., 0.]; n + 2];
            m.triangles = (0..n).map(|i| [0, 1, (i + 2) as u32]).collect();
            let mut required = Required::default();
            required.segments.insert((0, 1));
            for i in 0..n {
                required.segments.insert((1, (i + 2) as u32));
                required.points.insert((i + 2) as u32);
            }
            for i in 0..n {
                let mut vertices = BTreeSet::new();
                let mut constraints = BTreeSet::new();
                required.patch(&m, &[i], None, &mut vertices, &mut constraints);
                assert_eq!(constraints, BTreeSet::from([(0, 1), (1, (i + 2) as u32)]));
                assert_eq!(vertices, BTreeSet::from([0, 1, (i + 2) as u32]));
            }
            assert_eq!(required.probes, 3 * n);
        }
    }

    #[test]
    fn contraction_cap_includes_frozen_faces_and_failure_is_atomic() {
        let positions = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [3., 0., 0.],
            [0.5, 0.1, 0.],
            [2., 1., 0.],
            [1., 2., 3.],
        ];
        let triangles = vec![[0, 1, 3], [1, 4, 3], [1, 2, 4], [3, 4, 5]];
        let mut m = Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            representative: vec![sheet(1, false).representative[0]; triangles.len()],
            positions,
            triangles,
        };
        let before = m.triangles.clone();
        let representatives = m.representative.clone();
        let guard = super::super::folds::Source::new(&m.positions, &m.triangles);
        // Two reconstructed faces plus the unchanged curved face need three slots.
        assert!(contract(&mut m, &[], &[0, 0, 2, 3, 4, 5], &[0, 2], 2, &guard, false).is_none());
        assert_eq!(m.triangles, before);
        assert_eq!(m.representative, representatives);
        let result = contract(&mut m, &[], &[0, 0, 2, 3, 4, 5], &[0, 2], 3, &guard, false).unwrap();
        assert_eq!(m.triangles.len(), 3);
        assert_eq!(result.reconstructed_faces, 2);
    }

    #[test]
    fn back_to_back_contracted_strips_keep_separate_source_sheets() {
        let base = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [3., 0., 0.],
            [0.5, 0.1, 0.],
            [2., 1., 0.],
            [1., 2., 3.],
        ];
        let mut positions = base.clone();
        positions.extend(base.iter().map(|p| [p[0], p[1], p[2] + 0.001]));
        let mut triangles = vec![[0, 1, 3], [1, 4, 3], [1, 2, 4], [3, 4, 5]];
        let back: Vec<_> = triangles
            .iter()
            .map(|t| [t[0] + 6, t[2] + 6, t[1] + 6])
            .collect();
        triangles.extend(back);
        let mut m = Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            representative: vec![sheet(1, false).representative[0]; triangles.len()],
            positions,
            triangles,
        };
        let guard = super::super::folds::Source::new(&m.positions, &m.triangles);
        let r = contract(
            &mut m,
            &[],
            &[0, 0, 2, 3, 4, 5, 6, 6, 8, 9, 10, 11],
            &[0, 2, 6, 8],
            8,
            &guard,
            false,
        )
        .unwrap();
        assert_eq!(r.reconstructed_faces, 4);
        assert_eq!(m.triangles.len(), 6);
        assert!(m
            .triangles
            .iter()
            .all(|t| t.iter().all(|&v| v < 6) || t.iter().all(|&v| v >= 6)));
        assert!(!guard
            .violations(&m.positions, &m.triangles)
            .iter()
            .any(|v| *v));
    }

    #[test]
    fn contracted_opposite_sheets_use_a_fixed_alternate_diagonal_without_new_twins() {
        for (interior, long_strip) in [(false, false), (true, false), (false, true)] {
            // Two local six-corner strips share their upper three source vertices.
            // Their distinct old arc midpoints keep the source faces apart. The
            // nearest-endpoint collapse and ordinary CDT create the new twin 2/3/6.
            let mut positions = vec![
                [2., 0., 0.],
                [0., 0., 1.],
                [0., 0., 2.],
                [0., 0., 3.],
                [2., 0., 1.],
                [2., 0., 2.],
                [2., 0., 3.],
                [0., 0.125, 1.],
                [2., 0.125, 0.],
                [2., 0.125, 1.],
                [2., 0.125, 2.],
            ];
            let mut triangles = vec![
                [1, 4, 2],
                [0, 4, 1],
                [4, 5, 2],
                [2, 5, 3],
                [5, 6, 3],
                [7, 2, 9],
                [8, 7, 9],
                [9, 2, 10],
                [2, 3, 10],
                [10, 3, 6],
            ];
            if interior {
                // This unmoved sample lies on the auxiliary 4--3 line. It must
                // survive either in the frozen ring or by splitting a spoke;
                // it cannot be suppressed like a delivered-border sample.
                positions.push([0.5, 0., 2.5]);
                triangles[3] = [2, 5, 11];
                triangles.extend([[5, 3, 11], [3, 2, 11]]);
            } else if long_strip {
                // Extra certified lower arcs leave several retained collinear
                // border edges beyond the best root. They must survive exactly;
                // zero wedges may select spokes but never emit zero-area faces.
                positions.extend([
                    [2., 0., 0.5],
                    [2., 0.125, 0.5],
                    [2., 0., 0.25],
                    [2., 0.125, 0.25],
                ]);
                triangles[1] = [0, 13, 1];
                triangles[6] = [8, 7, 14];
                triangles.extend([[13, 11, 1], [11, 4, 1], [14, 7, 12], [12, 7, 9]]);
            }
            let mut m = Merged {
                keys: positions.iter().copied().map(key).collect(),
                ells: vec![0; positions.len()],
                representative: vec![sheet(1, false).representative[0]; triangles.len()],
                positions,
                triangles,
            };
            let before = m.triangles.clone();
            // InterfaceRow::new consumes canonical items; it does not sort keys.
            // Height puts key(9) after key(6), unlike their vertex-index order.
            let row = |a: usize, b: usize| {
                let (a, b) = (m.keys[a], m.keys[b]);
                InterfaceRow::new(0, a.min(b), a.max(b), 0, 1)
            };
            let mut interface = vec![row(4, 6), row(9, 6)];
            let mut previous: Vec<_> = [(4, 5), (5, 6), (9, 10), (10, 6)]
                .map(|(a, b)| row(a, b))
                .into();
            let mut endpoints = vec![4, 6, 9];
            if long_strip {
                interface.extend([row(0, 11), row(8, 12)]);
                previous.extend([(0, 13), (13, 11), (8, 14), (14, 12)].map(|(a, b)| row(a, b)));
                endpoints.extend([0, 11, 8, 12]);
            }
            let mut labels = vec![None; m.positions.len()];
            labels[5] = Some((m.keys[6], m.keys[4], 0.5, 0));
            labels[10] = Some((m.keys[6], m.keys[9], 0.5, 0));
            if long_strip {
                labels[13] = Some((m.keys[11], m.keys[0], 0.5, 0));
                labels[14] = Some((m.keys[12], m.keys[8], 0.5, 0));
            }
            let guard = super::super::folds::Source::for_contraction(
                &m.positions,
                &m.triangles,
                &interface,
                &previous,
                &labels,
            );
            let mut map: Vec<_> = (0..m.positions.len() as u32).collect();
            map[5] = 6;
            map[10] = 6;
            if long_strip {
                map[13] = 11;
                map[14] = 12;
            }
            let cap = m.triangles.len();
            let receipt = if interior {
                triangulate(
                    &mut m,
                    &interface,
                    cap,
                    Some(&map),
                    &endpoints,
                    &guard,
                    PatchPlan::ChordSpokes,
                    false,
                    false,
                )
                .unwrap()
            } else {
                assert!(triangulate(
                    &mut m,
                    &interface,
                    cap,
                    Some(&map),
                    &endpoints,
                    &guard,
                    PatchPlan::Charts,
                    false,
                    false,
                )
                .is_none());
                assert_eq!(m.triangles, before);
                contract(&mut m, &interface, &map, &endpoints, cap, &guard, false).unwrap()
            };
            assert!(receipt.reconstructed_faces > 0);
            assert!(m.triangles.len() <= cap);
            for t in before
                .iter()
                .filter(|t| t.iter().all(|&v| map[v as usize] == v))
            {
                assert!(
                    m.triangles
                        .iter()
                        .any(|out| super::super::oriented(out) == super::super::oriented(t)),
                    "endpoint-only source face was reconstructed"
                );
            }
            assert!(!guard
                .violations(&m.positions, &m.triangles)
                .iter()
                .any(|v| *v));
            // Independent face-key census: neither duplicated nor reversed output
            // triples are allowed, even though all directional chord counts fit.
            let mut faces = BTreeSet::new();
            for mut t in m.triangles.iter().copied() {
                t.sort_unstable();
                assert!(faces.insert(t), "new coincident face pair");
            }
            for a in [4, 9] {
                assert!(m.triangles.iter().any(|t| t.contains(&a) && t.contains(&6)));
            }
            if interior {
                assert!(
                    m.triangles.iter().any(|t| t.contains(&11)),
                    "source sample was dropped"
                );
            } else if long_strip {
                assert_eq!(m.triangles.len(), 10);
                for (a, b) in [(0, 11), (11, 4), (8, 12), (12, 9)] {
                    assert!(
                        m.triangles.iter().any(|t| t.contains(&a) && t.contains(&b)),
                        "collinear border subdivision lost"
                    );
                }
                assert!(
                    !m.triangles
                        .iter()
                        .any(|t| (t.contains(&0) && t.contains(&4))
                            || (t.contains(&8) && t.contains(&9))),
                    "border subdivision spanned"
                );
            } else {
                assert_eq!(m.triangles.len(), 8);
            }
        }
    }

    #[test]
    fn contracted_strip_workspace_fits_the_parent_charge() {
        let mut m = sheet(181, false); // 65,522 faces: almost eight capped children.
        let input = m.triangles.len();
        let mut map: Vec<_> = (0..m.positions.len() as u32).collect();
        for (v, target) in map.iter_mut().enumerate().take(181).skip(1) {
            *target = if v <= 90 { 0 } else { 181 };
        }
        let interface = [InterfaceRow::new(0, m.keys[0], m.keys[181], 0, 1)];
        let (receipt, peak) = crate::test_alloc::peak(|| {
            contract_path(
                &mut m,
                &interface,
                &map,
                &[0, 181],
                &(0..=181).collect::<Vec<_>>(),
            )
        });
        assert!(receipt.is_some());
        let charge = input as u64 * super::super::super::JOB_BYTES_PER_TRIANGLE;
        eprintln!("CONTRACTION workspace {peak} bytes / {input} faces, charge {charge}");
        assert!(peak as u64 <= charge);
    }

    #[test]
    fn interior_point_on_new_chord_does_not_make_a_sliver_or_split_the_chord() {
        let positions = vec![
            [0., 0., 0.],
            [1., 0.1, 0.],
            [2., 0., 0.],
            [1., 0., 0.],
            [1., -1., 0.],
        ];
        let triangles = vec![[0, 1, 3], [1, 2, 3], [0, 3, 4], [3, 2, 4]];
        let mut m = Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            representative: vec![sheet(1, false).representative[0]; triangles.len()],
            positions,
            triangles,
        };
        let interface = [InterfaceRow::new(0, m.keys[0], m.keys[2], 0, 1)];
        contract_path(&mut m, &interface, &[0, 0, 2, 3, 4], &[0, 2], &[0, 1, 2]).unwrap();
        assert_eq!(m.triangles.len(), 1);
        let t = m.triangles[0];
        assert!(t.contains(&0) && t.contains(&2) && t.contains(&4));
        assert!(topology::normal(&m.positions, t)[2] < 0.);
    }

    #[test]
    fn contracted_strip_retriangulates_a_fold_and_freezes_the_interior_ring() {
        // Snapping m to a in (m,q,p) reverses its sign. Flipping that face would
        // double-cover the same side of a-p. The constrained strip has area 1.35.
        let positions = vec![
            [0., 0., 0.],   // a
            [1., 0., 0.],   // m
            [3., 0., 0.],   // b
            [0.5, 0.1, 0.], // p
            [2., 1., 0.],   // q
            [1., 2., 3.],   // unchanged curved interior beyond p-q
        ];
        let triangles = vec![[0, 1, 3], [1, 4, 3], [1, 2, 4], [3, 4, 5]];
        let mut m = Merged {
            keys: positions.iter().copied().map(key).collect(),
            ells: vec![0; positions.len()],
            representative: vec![sheet(1, false).representative[0]; triangles.len()],
            positions,
            triangles,
        };
        let area: f64 = m.triangles[..3]
            .iter()
            .map(|&t| topology::normal(&m.positions, t)[2])
            .sum();
        let interface = [InterfaceRow::new(0, m.keys[0], m.keys[2], 0, 1)];
        let receipt =
            contract_path(&mut m, &interface, &[0, 0, 2, 3, 4, 5], &[0, 2], &[0, 1, 2]).unwrap();
        assert!(receipt.constrained_remeshes > 0);
        assert_eq!(
            receipt.reconstructed_faces, 2,
            "frozen interior was counted"
        );
        assert!(m.triangles.contains(&[3, 4, 5]), "curved interior moved");
        let strip: Vec<_> = m
            .triangles
            .iter()
            .copied()
            .filter(|t| !t.contains(&5))
            .collect();
        assert_eq!(strip.len(), 2);
        assert!(strip
            .iter()
            .all(|&t| topology::normal(&m.positions, t)[2] > 0.));
        let after: f64 = strip
            .iter()
            .map(|&t| topology::normal(&m.positions, t)[2])
            .sum();
        assert!((area - after).abs() < 1e-12);
        assert!(strip.iter().any(|t| t.contains(&0) && t.contains(&2)));
        let mut counts = BTreeMap::<(u32, u32), [u32; 2]>::new();
        for t in &m.triangles {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                counts.entry(edge(a, b)).or_default()[usize::from(a > b)] += 1;
            }
        }
        assert!(counts.values().all(|c| c[0] <= 1 && c[1] <= 1));
    }

    #[test]
    fn planar_cap_fit_depends_on_boundary_and_holes_not_interior_density() {
        for hole in [false, true] {
            let mut m = sheet(32, hole);
            let p = m.keys[(16 * 33 + 16) as usize];
            let interface = vec![InterfaceRow::new(0, p, p, 0, 1)];
            let result = remesh(&mut m, &interface, 140).unwrap();
            assert_eq!(result.constrained_remeshes, 1);
            assert_eq!(result.closed_holes, 0);
            assert_eq!(m.triangles.len(), if hole { 132 } else { 128 });
            assert!(m
                .triangles
                .iter()
                .flatten()
                .any(|v| m.keys[*v as usize] == p));
        }
    }
    #[test]
    fn insufficient_cap_and_folded_projection_leave_input_unchanged() {
        let mut m = sheet(8, false);
        let before = m.triangles.clone();
        assert!(remesh(&mut m, &[], 2).is_none());
        assert_eq!(m.triangles, before);
        m.triangles[0].swap(0, 1);
        let before = m.triangles.clone();
        assert!(remesh(&mut m, &[], 8192).is_none());
        assert_eq!(m.triangles, before);
    }

    fn flip(p: &[[f32; 3]], faces: &mut [[u32; 3]], interface: &dyn Fn(u32) -> bool) {
        let no = BTreeSet::new();
        flip_interface_edges(p, faces, 2, interface, &|_| false, &|_, _| false, &no);
    }
    fn area(p: &[[f32; 3]], t: [u32; 3]) -> f64 {
        topology::normal(p, t)[2]
    }

    /// Review r1 P2: apex b of edge a-b is collinear with x and c in the projection, so
    /// the quad is not strictly convex and the flip (which would emit a vertical fin
    /// x, b, c) must not happen.
    #[test]
    fn a_collinear_quad_is_not_flipped() {
        let p = [[0., 0., 0.], [2., 0., 1.], [3., 1., 0.], [1., -1., 0.]];
        let mut faces = [[0, 1, 2], [1, 0, 3]];
        flip(&p, &mut faces, &|v| v < 2);
        assert_eq!(faces, [[0, 1, 2], [1, 0, 3]]);
    }

    /// Flips on CDTs of a coarse lattice (many collinear and cocircular cases) keep every
    /// face positive and the area, and never add an interface-to-interface edge.
    #[test]
    fn flips_keep_orientation_and_area_on_lattice_triangulations() {
        let mut rng = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for _ in 0..500 {
            let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
            for _ in 0..4 + next() % 40 {
                let (x, y) = ((next() % 9) as f64, (next() % 9) as f64);
                cdt.insert(Point2::new(x, y)).unwrap();
            }
            let p: Vec<[f32; 3]> = cdt
                .vertices()
                .map(|v| [v.position().x as f32, v.position().y as f32, 0.])
                .collect();
            let mut faces: Vec<[u32; 3]> = cdt
                .inner_faces()
                .map(|f| f.vertices().map(|v| v.fix().index() as u32))
                .collect();
            let interface: Vec<bool> = p.iter().map(|_| next() % 3 != 0).collect();
            let pairs = |faces: &[[u32; 3]]| -> BTreeSet<(u32, u32)> {
                let both = |&(a, b): &(u32, u32)| interface[a as usize] && interface[b as usize];
                let edges = faces
                    .iter()
                    .flat_map(|t| (0..3).map(|k| edge(t[k], t[(k + 1) % 3])));
                edges.filter(both).collect()
            };
            let (before, total) = (
                pairs(&faces),
                faces.iter().map(|&t| area(&p, t)).sum::<f64>(),
            );
            flip(&p, &mut faces, &|v| interface[v as usize]);
            assert!(faces.iter().all(|&t| area(&p, t) > 0.), "{faces:?}");
            assert!(pairs(&faces).is_subset(&before));
            assert!((faces.iter().map(|&t| area(&p, t)).sum::<f64>() - total).abs() < 1e-9);
        }
    }
}
