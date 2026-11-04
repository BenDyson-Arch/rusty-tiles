// Included by cache_controls.py with source-extracted pruned and unpruned scans.
// No production switch is introduced. Grid construction is prepaid in both runs.
#[test]
fn pruning_never_turns_an_accept_into_a_refusal() {
    type Triangle = [[f64; 3]; 3];
    let near: Triangle = [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]];
    let small: Triangle = [[0.1, 0.1, 0.], [0.2, 0.1, 0.], [0.1, 0.2, 0.]];
    let mut cases = Vec::new();
    for separated in 0..=3 {
        for reverse in [false, true] {
            for z in [0., 0.125] {
                let mut faces = vec![near.map(|p| [p[0], p[1], 0.25]); separated];
                faces.push(near.map(|p| [p[0], p[1], z]));
                if reverse {
                    faces.reverse();
                }
                cases.push((small, faces, 0.18));
            }
        }
    }
    // Geometric refusals, and a region requiring subdivision among four faces.
    for z in [0.25, 2.] {
        cases.push((small.map(|p| [p[0], p[1], z]), vec![near], 0.18));
    }
    let [a, b, c] = near;
    cases.push((
        near,
        vec![
            [a, [1., 0., 0.], [0., 1., 0.]],
            [[1., 0., 0.], b, [1., 1., 0.]],
            [[0., 1., 0.], [1., 1., 0.], c],
            [[1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
        ],
        0.,
    ));
    let (mut old_accepts, mut improved, mut savings, mut refusals) = (0, 0, 0, 0);
    for (case, (triangle, faces, reach)) in cases.iter().enumerate() {
        let faces: Vec<_> = faces
            .iter()
            .map(|t| t.map(|p| p.map(|v| (v as f32).to_bits())))
            .collect();
        let grid = Grid::new(&faces, *reach, 0., &effort_for_tests()).unwrap();
        let run = |unpruned, remaining, cached: Option<usize>| {
            grid.certified
                .set(cached.map(|i| (i as u32, faces[i].map(point))));
            let mut effort = effort_for_tests();
            effort.base = effort.budget.value - remaining;
            let verdict = if unpruned {
                grid.covers_unpruned(*triangle, &faces, &effort)
            } else {
                grid.covers(*triangle, &faces, &effort)
            };
            (
                verdict,
                effort.used.load(Ordering::Relaxed),
                effort.certificate_fast.load(Ordering::Relaxed),
                effort.certificate_slow.load(Ordering::Relaxed),
            )
        };
        let old = run(true, 10_000_000, None);
        let new = run(false, 10_000_000, None);
        assert_eq!(old.0, new.0, "geometric decision, case {case}");
        assert!(new.1 <= old.1, "executed work, case {case}");
        assert!(old.1 < 512, "bounded control corpus");
        savings += usize::from(new.1 < old.1);
        refusals += usize::from(old.0 == Ok(false));
        for remaining in 0..=old.1 + 2 {
            let mut cold = None;
            for cached in [None, Some(0), Some(faces.len() - 1)] {
                let old = run(true, remaining, cached);
                let new = run(false, remaining, cached);
                // Archive and budget determine the result; hidden cache state
                // must change neither verdict nor any charged-work counter.
                if let Some(expected) = &cold {
                    assert_eq!(&new, expected, "cache state, case {case}");
                } else {
                    cold = Some(new.clone());
                }
                if old.0 == Ok(true) {
                    old_accepts += 1;
                    assert_eq!(new.0, Ok(true), "case {case}, budget {remaining}");
                }
                // Strict budget improvements are permitted and required here.
                improved += usize::from(old.0.is_err() && new.0 == Ok(true));
                assert!(new.1 <= old.1, "charged work, case {case}");
            }
        }
    }
    assert!(old_accepts > 0 && improved > 0 && savings > 0 && refusals > 0);
    eprintln!(
        "cases={} old_accepts={old_accepts} budget_improvements={improved} \
         strict_work_savings={savings} geometric_refusals={refusals}",
        cases.len()
    );
}
