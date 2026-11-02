//! One post-cap proposal. Exact boundary carriers and component topology are retained;
//! a failed component is restored without another reduction or source query.
use super::{folds, topology, Built, Merged};
use crate::mesh_large::hlod::{interfaces::key, store::TileGeometry};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    pub target: f64,
    pub baseline_triangles: usize,
    pub baseline_g: f64,
    pub proposed_triangles: usize,
    pub measured_g: Option<f64>,
    pub reducer_passes: u32,
    pub source_passes: u32,
    pub restored_components: usize,
    pub accepted: bool,
    pub reason: &'static str,
    #[serde(skip)]
    pub cpu_seconds: f64,
}

fn edges(faces: &[[u32; 3]]) -> HashMap<(u32, u32), usize> {
    let mut result = HashMap::default();
    for t in faces {
        for i in 0..3 {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            *result.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    result
}

// A pinched vertex may have no singular edge (two closed sheets sharing one
// point). Protect its whole star too; edge incidence and Euler alone miss it.
fn singular_vertices(faces: &[[u32; 3]]) -> HashSet<u32> {
    let mut stars = HashMap::<u32, Vec<(u32, u32)>>::default();
    for t in faces {
        for i in 0..3 {
            stars
                .entry(t[i])
                .or_default()
                .push((t[(i + 1) % 3], t[(i + 2) % 3]));
        }
    }
    let mut singular = HashSet::default();
    for (v, links) in stars {
        let mut adjacent = HashMap::<u32, Vec<u32>>::default();
        for (a, b) in links {
            adjacent.entry(a).or_default().push(b);
            adjacent.entry(b).or_default().push(a);
        }
        let mut seen = HashSet::default();
        let mut todo = vec![*adjacent.keys().next().unwrap()];
        while let Some(a) = todo.pop() {
            if seen.insert(a) {
                todo.extend(&adjacent[&a]);
            }
        }
        if seen.len() != adjacent.len() || adjacent.values().any(|n| n.len() > 2) {
            singular.insert(v);
        }
    }
    singular
}

/// Same singular edge incidence, Euler characteristic and one connected component.
/// All singular-edge carriers are pinned before reduction. This is deliberately
/// stronger than the production hole/drop licences: a soft target grants none.
fn preserves(before: &[[u32; 3]], after: &[[u32; 3]]) -> bool {
    if after.is_empty() {
        return false;
    }
    let duplicates = |f: &[[u32; 3]]| {
        let mut counts = HashMap::default();
        for &t in f {
            let mut t = t;
            t.sort_unstable();
            *counts.entry(t).or_insert(0usize) += 1;
        }
        counts
    };
    let old = duplicates(before);
    if duplicates(after)
        .iter()
        .any(|(f, &n)| n > 1 && n > old.get(f).copied().unwrap_or(0))
    {
        return false;
    }
    let signature = |f: &[[u32; 3]]| {
        let vertices: HashSet<_> = f.iter().flatten().copied().collect();
        let mut e = edges(f);
        let chi = vertices.len() as i64 - e.len() as i64 + f.len() as i64;
        e.retain(|_, n| *n != 2);
        (chi, e)
    };
    if signature(before) != signature(after)
        || singular_vertices(before) != singular_vertices(after)
    {
        return false;
    }
    let mut parent = HashMap::default();
    fn root(p: &HashMap<u32, u32>, mut v: u32) -> u32 {
        while p[&v] != v {
            v = p[&v];
        }
        v
    }
    for &v in after.iter().flatten() {
        parent.insert(v, v);
    }
    for t in after {
        for &v in &t[1..] {
            let a = root(&parent, t[0]);
            let b = root(&parent, v);
            parent.insert(a.max(b), a.min(b));
        }
    }
    let first = root(&parent, after[0][0]);
    after.iter().flatten().all(|&v| root(&parent, v) == first)
}

pub(crate) fn propose(
    built: &Built,
    decoded: &[[f32; 3]],
    allowance: f64,
    mut policy: folds::Policy,
    report: &mut Report,
) -> Option<Built> {
    let tile = &built.geometry;
    // CPU/work bounds: one meshopt call on at most one capped tile, no retries;
    // one S2 schedule with the unchanged bounded nearest-query budget afterwards.
    if tile.key.end - tile.key.first > 1_000_000
        || tile.triangles.len() > super::super::TRIANGLE_CAP
        || decoded.len() > 3 * super::super::TRIANGLE_CAP
    {
        report.reason = "work_limit";
        return None;
    }
    let m = Merged {
        positions: decoded.to_vec(),
        keys: decoded.iter().copied().map(key).collect(),
        ells: tile.ells.clone(),
        triangles: tile.triangles.clone(),
        representative: vec![],
    };
    let component = topology::components(&m);
    let mut pins = built.locked.clone();
    // Existing topology licences remain tied to their exact support faces. Pin
    // those carriers and remap their indices; prepare rechecks every source witness.
    let c = &built.change;
    let mut witness_faces: Vec<usize> = c
        .patch_faces
        .iter()
        .chain(&c.closure_carriers)
        .chain(&c.strip.added_faces)
        .copied()
        .collect();
    witness_faces.extend(
        c.opening_faces
            .iter()
            .enumerate()
            .filter_map(|(i, &yes)| yes.then_some(i)),
    );
    witness_faces.sort_unstable();
    witness_faces.dedup();
    for &i in &witness_faces {
        for &v in m.triangles.get(i)? {
            pins[v as usize] = true;
        }
    }
    let incidence = edges(&m.triangles);
    let singular = singular_vertices(&m.triangles);
    let mut supported = HashSet::default();
    for t in &m.triangles {
        // Keep an exact carrier, not merely both endpoints of an open edge.
        if t.iter().any(|v| singular.contains(v))
            || supported.insert(component[t[0] as usize])
            || (0..3).any(|i| {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                incidence[&(a.min(b), a.max(b))] != 2
            })
        {
            for &v in t {
                pins[v as usize] = true;
            }
        }
    }
    let indices: Vec<u32> = m.triangles.iter().flatten().copied().collect();
    let locks: Vec<u8> = pins.iter().map(|&v| u8::from(v)).collect();
    let mut output = vec![0u32; indices.len()];
    let mut estimate = 0f32;
    // SAFETY: tightly packed f32x3 positions; valid input indices; destination has
    // input length (the worst output size); one lock per vertex; no attributes.
    let count = unsafe {
        meshopt::ffi::meshopt_simplifyWithAttributes(
            output.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            decoded.as_ptr().cast(),
            decoded.len(),
            12,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            locks.as_ptr(),
            0,
            allowance as f32,
            meshopt::SimplifyOptions::ErrorAbsolute.bits(),
            &mut estimate,
        )
    };
    report.reducer_passes = 1;
    output.truncate(count);
    let mut old = std::collections::BTreeMap::<usize, Vec<[u32; 3]>>::new();
    let mut new = HashMap::<usize, Vec<[u32; 3]>>::default();
    for &t in &m.triangles {
        old.entry(component[t[0] as usize]).or_default().push(t);
    }
    for t in output.as_chunks::<3>().0 {
        let c = component[t[0] as usize];
        if t.iter().any(|&v| component[v as usize] != c) {
            report.reason = "component_join";
            return None;
        }
        new.entry(c).or_default().push(*t);
    }
    let mut faces = Vec::new();
    for (c, before) in old {
        let after = new.remove(&c).unwrap_or_default();
        if preserves(&before, &after) {
            faces.extend(after);
        } else {
            report.restored_components += 1;
            faces.extend(before);
        }
    }
    report.proposed_triangles = faces.len();
    if faces.len() >= tile.triangles.len() {
        report.reason = "no_reduction";
        return None;
    }
    let used: HashSet<_> = faces.iter().flatten().copied().collect();
    let ee = edges(&faces);
    let ids: HashMap<_, _> = m
        .keys
        .iter()
        .enumerate()
        .map(|(i, &k)| (k, i as u32))
        .collect();
    if tile.interface.iter().any(|r| {
        let (Some(&a), Some(&b)) = (ids.get(&r.a), ids.get(&r.b)) else {
            return true;
        };
        if a == b {
            !used.contains(&a)
        } else {
            !ee.contains_key(&(a.min(b), a.max(b)))
        }
    }) {
        report.reason = "interface";
        return None;
    }
    // A soft target cannot introduce new folds, even on an N2/N3 baseline.
    policy.folds = false;
    let guard = folds::Source::new(decoded, &m.triangles)
        .with_boundary(&tile.interface)
        .with_policy(policy);
    if guard.violations(decoded, &faces).iter().any(|v| *v) {
        report.reason = "fold";
        return None;
    }
    report.reason = "topology";
    let change = topology::changes(&m, &faces, &tile.interface, 0.)?;
    if change.receipt.opened_holes != 0
        || change.receipt.closed_holes != 0
        || change.receipt.dropped_components != 0
    {
        report.reason = "topology";
        return None;
    }
    let mut face_ids = HashMap::<[u32; 3], Vec<usize>>::default();
    for (i, t) in faces.iter().enumerate() {
        face_ids.entry(super::oriented(t)).or_default().push(i);
    }
    let mut remap = HashMap::default();
    for i in witness_faces {
        let Some(j) = face_ids
            .get_mut(&super::oriented(&m.triangles[i]))
            .and_then(Vec::pop)
        else {
            report.reason = "topology_carrier";
            return None;
        };
        remap.insert(i, j);
    }
    let mut change = built.change.clone();
    for i in change
        .patch_faces
        .iter_mut()
        .chain(&mut change.closure_carriers)
        .chain(&mut change.strip.added_faces)
    {
        *i = remap[i];
    }
    let mut opening = vec![false; faces.len()];
    for (i, &yes) in change.opening_faces.iter().enumerate() {
        if yes {
            opening[remap[&i]] = true;
        }
    }
    change.opening_faces = opening;
    // Held tiles are charged by their surviving triangles. Remove unused vertices
    // before publication/parent handoff, retaining first-use order and exact bits.
    let mut vertex_ids = vec![u32::MAX; decoded.len()];
    let (mut positions, mut ells, mut locked) = (Vec::new(), Vec::new(), Vec::new());
    for t in &mut faces {
        for v in t {
            let old = *v as usize;
            let id = &mut vertex_ids[old];
            if *id == u32::MAX {
                *id = positions.len() as u32;
                positions.push(decoded[old]);
                ells.push(tile.ells[old]);
                locked.push(built.locked[old]);
            }
            *v = *id;
        }
    }
    Some(Built {
        geometry: TileGeometry {
            key: tile.key,
            ells,
            positions,
            representative: vec![0; faces.len()],
            primitives: tile.primitives.clone(),
            triangles: faces,
            interface: tile.interface.clone(),
            geometric_error: 0.,
            lease: None,
        },
        locked,
        stopping: super::super::store::StoppingBound::GuardedError,
        simplifier_error: f64::from(estimate),
        children_error: built.children_error,
        input_triangles: built.input_triangles,
        protected_faces: built.protected_faces,
        retries: built.retries,
        last_resort: built.last_resort,
        staged: built.staged,
        last_resort_stage: built.last_resort_stage,
        relief_stuck: built.relief_stuck,
        interface_vertices: built.interface_vertices,
        interface_segments: built.interface_segments,
        reoriented: built.reoriented,
        topology: built.topology,
        change,
        controls: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_proposal_reduces_interior_and_keeps_exact_open_support() {
        use crate::mesh_large::hlod::store::{SourcePrimitive, StoppingBound, TileKey};
        let n = 64u32;
        let positions: Vec<_> = (0..=n)
            .flat_map(|x| (0..=n).map(move |y| [x as f32, y as f32, 0.]))
            .collect();
        let mut triangles = Vec::new();
        for x in 0..n {
            for y in 0..n {
                let a = x * (n + 1) + y;
                triangles.extend([[a, a + 1, a + n + 1], [a + 1, a + n + 2, a + n + 1]]);
            }
        }
        let count = positions.len();
        let built = Built {
            geometry: TileGeometry {
                key: TileKey {
                    first: 0,
                    end: triangles.len() as u64,
                    depth: 3,
                    leaf_begin: 0,
                    leaf_end: 2,
                },
                ells: vec![4; count],
                positions: positions.clone(),
                representative: vec![0; triangles.len()],
                triangles: triangles.clone(),
                primitives: vec![SourcePrimitive {
                    node: 0,
                    mesh: 0,
                    primitive: 0,
                    expanded_ordinal: 0,
                    material: u32::MAX,
                }],
                interface: vec![],
                geometric_error: 0.,
                lease: None,
            },
            locked: vec![false; count],
            stopping: StoppingBound::TriangleBudget,
            simplifier_error: 0.,
            children_error: 0.,
            input_triangles: triangles.len() as u32,
            protected_faces: 0,
            retries: 0,
            last_resort: false,
            staged: false,
            last_resort_stage: 0,
            relief_stuck: 0,
            interface_vertices: 0,
            interface_segments: 0,
            reoriented: 0,
            topology: Default::default(),
            change: Default::default(),
            controls: Default::default(),
        };
        let mut report = Report::default();
        let (reduced, peak) = crate::test_alloc::peak(|| {
            propose(
                &built,
                &positions,
                0.1,
                folds::Policy::default(),
                &mut report,
            )
            .unwrap()
        });
        assert!(peak < 32 << 20, "D10 proposal peak {peak}");
        eprintln!("D10 8192-face proposal peak: {peak} bytes");
        assert!(reduced.geometry.triangles.len() < triangles.len() / 2);
        let original: HashMap<_, _> = positions
            .iter()
            .enumerate()
            .map(|(i, p)| (key(*p), i as u32))
            .collect();
        let mapped: Vec<_> = reduced
            .geometry
            .triangles
            .iter()
            .map(|t| t.map(|v| original[&key(reduced.geometry.positions[v as usize])]))
            .collect();
        assert!(preserves(&triangles, &mapped));
        assert!(reduced.geometry.positions.len() <= 3 * reduced.geometry.triangles.len());
        assert_eq!(report.reducer_passes, 1);
        assert_eq!(report.source_passes, 0);
        let mut changed = built;
        changed.change.patch_faces = vec![usize::MAX];
        assert!(propose(
            &changed,
            &positions,
            0.1,
            folds::Policy::default(),
            &mut report
        )
        .is_none());
    }
    #[test]
    fn pinched_closed_sheets_are_singular_without_singular_edges() {
        let f = [
            [0, 1, 2],
            [0, 3, 1],
            [1, 3, 2],
            [0, 2, 3],
            [0, 4, 5],
            [0, 6, 4],
            [4, 6, 5],
            [0, 5, 6],
        ];
        assert!(edges(&f).values().all(|&n| n == 2));
        assert_eq!(singular_vertices(&f), HashSet::from_iter([0]));
    }
    #[test]
    fn support_refuses_missing_components_and_openings() {
        let fan = [[0, 1, 4], [1, 2, 4], [2, 3, 4], [3, 0, 4]];
        assert!(preserves(&fan, &[[0, 1, 2], [0, 2, 3]]));
        assert!(!preserves(&fan, &[[0, 1, 2]]));
        assert!(!preserves(&fan, &[]));
        let tetra = [[0, 1, 2], [0, 3, 1], [1, 3, 2], [0, 2, 3]];
        assert!(!preserves(&tetra, &tetra[..3]));
        assert!(!preserves(&tetra, &[[0, 1, 2], [0, 1, 2]]));
    }
}
