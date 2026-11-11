//! A failed protected reduction may still fit when independent surface sheets are
//! simplified separately. This only proposes geometry; the caller retains every
//! incidence, boundary, topology and measured-error check on their union.
use super::{topology, HashMap, Merged};

fn root(parent: &mut [usize], mut a: usize) -> usize {
    while parent[a] != a {
        parent[a] = parent[parent[a]];
        a = parent[a];
    }
    a
}

pub(super) fn simplify(
    m: &Merged,
    faces: &[[u32; 3]],
    locked: &dyn Fn(u32) -> bool,
    target: usize,
) -> (Vec<[u32; 3]>, f64) {
    // Only ordinary, oppositely oriented two-face edges connect a sheet. A dominant
    // normal chart separates back-to-back folds even when they share a spine. This
    // is a partition choice, not a licence to discard or reorient any face.
    let chart: Vec<_> = faces
        .iter()
        .map(|&t| {
            let n = topology::normal(&m.positions, t);
            let axis = (0..3)
                .max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs()))
                .unwrap();
            (axis, n[axis] > 0.)
        })
        .collect();
    let mut parent: Vec<_> = (0..faces.len()).collect();
    let mut boundary = vec![false; m.positions.len()];
    {
        let mut edges = HashMap::<(u32, u32, usize, bool), (usize, usize, u32, i32)>::default();
        for (i, t) in faces.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let e = edges
                    .entry((a.min(b), a.max(b), chart[i].0, chart[i].1))
                    .or_insert((i, i, 0, 0));
                e.1 = i;
                e.2 += 1;
                e.3 += if a < b { 1 } else { -1 };
            }
        }
        for (&(v, w, _, _), &(a, b, count, direction)) in &edges {
            if count != 2 || direction != 0 {
                boundary[v as usize] = true;
                boundary[w as usize] = true;
            }
            if count == 2 && direction == 0 && chart[a] == chart[b] {
                let (a, b) = (root(&mut parent, a), root(&mut parent, b));
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    drop(chart);
    let mut groups = std::collections::BTreeMap::<usize, Vec<[u32; 3]>>::new();
    let mut owner = vec![usize::MAX; m.positions.len()];
    let mut shared = vec![false; m.positions.len()];
    for (i, &t) in faces.iter().enumerate() {
        let group = root(&mut parent, i);
        for &v in &t {
            let o = &mut owner[v as usize];
            if *o == usize::MAX {
                *o = group;
            } else if *o != group && boundary[v as usize] {
                shared[v as usize] = true;
            }
        }
        groups.entry(group).or_default().push(t);
    }
    drop(owner);
    drop(parent);
    let mut local = vec![u32::MAX; m.positions.len()];
    let mut output = Vec::new();
    let mut maximum = 0f64;
    let mut prefix = 0usize;
    let mut allocated = 0usize;
    for (_, group) in groups {
        prefix += group.len();
        let allocation = target.saturating_mul(prefix) / faces.len();
        let budget = allocation - allocated;
        allocated = allocation;
        let (triangles, error) = super::simplify_mapped(
            m,
            &group,
            &|v| locked(v) || shared[v as usize],
            budget,
            &mut local,
        );
        output.extend(triangles);
        maximum = maximum.max(error);
    }
    (output, maximum)
}
