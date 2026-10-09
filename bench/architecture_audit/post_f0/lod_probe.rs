use rusty_tiles::{glb_write::TilePrimitive, hlod::two_sided_error};

fn probe(spike: Option<usize>) -> f64 {
    let parent = TilePrimitive {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
        indices: vec![0, 1, 2],
        ..Default::default()
    };
    let mut child = TilePrimitive::default();
    for i in 0..3000 {
        let z = if spike == Some(i) { 100. } else { 0. };
        let base = child.positions.len() as u32;
        child
            .positions
            .extend([[0., 0., 0.], [1., 0., z], [0., 1., z]]);
        child.indices.extend([base, base + 1, base + 2]);
    }
    two_sided_error(&[parent], &[child])
}

fn main() {
    println!(
        "{{\"flat_control\":{},\"spike_at_zero\":{},\"spike_at_one\":{},\"independent_spike_distance_metres\":100}}",
        probe(None),
        probe(Some(0)),
        probe(Some(1))
    );
}
