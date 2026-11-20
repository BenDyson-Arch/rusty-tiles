use super::*;

/// xorshift: deterministic fixtures without a dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// `bake`'s wrap of one texel coordinate (CLAMP, MIRRORED_REPEAT, else REPEAT).
fn wrap_one(i: i64, n: u32, mode: u32) -> u32 {
    let n = i64::from(n);
    (match mode {
        33071 => i.clamp(0, n - 1),
        33648 => {
            let m = i.rem_euclid(2 * n);
            if m < n {
                m
            } else {
                2 * n - 1 - m
            }
        }
        _ => i.rem_euclid(n),
    }) as u32
}

#[test]
fn wrapped_spans_are_exactly_the_wrapped_texels() {
    let mut rng = Rng(0x5eed);
    for mode in [33071, 33648, 10497] {
        for n in [1u32, 2, 3, 7, 64, 65, 100] {
            for _ in 0..400 {
                let lo = (rng.next() % 900) as i64 - 450;
                let hi = lo + (rng.next() % 260) as i64;
                let mut want = vec![false; n as usize];
                for i in lo..=hi {
                    want[wrap_one(i, n, mode) as usize] = true;
                }
                let mut got = vec![false; n as usize];
                for &(a, b) in wrapped_spans(lo as f64, hi as f64, n, mode).as_slice() {
                    assert!(a <= b && b < n, "{mode} {n} {lo}..={hi}: ({a}, {b})");
                    for i in a..=b {
                        got[i as usize] = true;
                    }
                }
                assert_eq!(got, want, "mode {mode} n {n} {lo}..={hi}");
            }
        }
    }
    assert!(wrapped_spans(3., 2., 10, 10497).as_slice().is_empty());
}

#[test]
fn chain_dims_are_the_built_chains_levels() {
    for (w, h) in [(1, 1), (1, 9), (300, 170), (64, 64), (129, 3), (8192, 4096)] {
        let dims = chain_dims(w, h);
        assert_eq!(dims.last(), Some(&(1, 1)));
        if w * h <= 300 * 170 {
            let level = mips::Level {
                width: w,
                height: h,
                rgba: vec![7; (w * h * 4) as usize],
            };
            let built = bake::chain_from(level, Class::Colour, &|| Ok(())).unwrap();
            assert_eq!(
                built
                    .iter()
                    .map(|l| (l.width, l.height))
                    .collect::<Vec<_>>(),
                dims
            );
        }
    }
}

fn copy(level: &mips::Level) -> mips::Level {
    mips::Level {
        width: level.width,
        height: level.height,
        rgba: level.rgba.clone(),
    }
}

fn image(w: u32, h: u32, seed: u64) -> mips::Level {
    let mut rng = Rng(seed);
    mips::Level {
        width: w,
        height: h,
        rgba: (0..w * h * 4).map(|_| rng.next() as u8).collect(),
    }
}

/// Stores `marks`' tiles of `level0`'s chain in a scratch log.
fn store(
    scratch: &crate::spill::Scratch,
    level0: &mips::Level,
    class: Class,
    marks: &Marks,
    drop_tile: Option<(usize, usize)>,
) -> Chains {
    let mut writer = crate::spill::ByteWriter::new(scratch, "coarse-test", 1 << 16).unwrap();
    let mut chain = Chain::new(marks);
    for (level, tile, block) in cut(copy(level0), class, marks, &|| Ok(())).unwrap() {
        if Some((level, tile)) == drop_tile {
            continue;
        }
        let offset = writer.len();
        writer.append(&block.bytes).unwrap();
        chain.levels[level].1[tile] = Some(Entry {
            offset,
            len: block.bytes.len() as u32,
            stored: block.stored,
        });
    }
    Chains {
        log: writer.finish().unwrap(),
        chains: BTreeMap::from([(
            (0, crate::mesh_large::leaves::crop_store::class_rank(class)),
            chain,
        )]),
        decodes: Default::default(),
    }
}

/// The full chain's sample: `bake`'s arithmetic over every level.
fn reference(
    levels: &[mips::Level],
    class: Class,
    sampler: SourceSampler,
    uv: [f32; 2],
    lod: f64,
) -> [f64; 4] {
    bake::trilinear(
        levels.len(),
        |i| (levels[i].width, levels[i].height),
        class,
        sampler,
        uv,
        lod,
        |i, x, y| {
            let at = (y * levels[i].width as usize + x) * 4;
            let p = &levels[i].rgba;
            Ok([p[at], p[at + 1], p[at + 2], p[at + 3]])
        },
    )
    .unwrap()
}

/// A barycentric point of `uv` as the coarse bake interpolates it (f32).
fn interpolate(uv: [[f32; 2]; 3], rng: &mut Rng) -> [f32; 2] {
    let (a, b) = (rng.unit(), rng.unit());
    let (a, b) = if a + b > 1. { (1. - a, 1. - b) } else { (a, b) };
    let w = [(1. - a - b) as f32, a as f32, b as f32];
    std::array::from_fn(|k| uv[0][k] * w[0] + uv[1][k] * w[1] + uv[2][k] * w[2])
}

/// Every sample of a UV inside a marked triangle, at any level of detail, wrap mode and
/// filter, reads only stored tiles and equals the full chain's sample bit for bit; the
/// sparse chain stores fewer tiles than the chain has.
#[test]
fn samples_inside_marked_triangles_equal_the_full_chain() {
    let scratch = crate::spill::Scratch::create(None, 0).unwrap();
    let mut rng = Rng(0xc0a75e);
    for (w, h, mode, class) in [
        (300, 170, 33071, Class::Colour),
        (257, 129, 10497, Class::Colour),
        (96, 200, 33648, Class::Data),
        (5, 3, 10497, Class::Data),
    ] {
        let level0 = image(w, h, rng.next());
        let levels = bake::chain_from(copy(&level0), class, &|| Ok(())).unwrap();
        for filter in [9729, 9728] {
            let sampler = SourceSampler {
                wrap_s: mode,
                wrap_t: if mode == 33071 { 10497 } else { mode },
                min_filter: 9987,
                mag_filter: filter,
            };
            // Small triangles, some across period seams and outside [0, 1].
            let triangles: Vec<[[f32; 2]; 3]> = (0..24)
                .map(|_| {
                    let (u, v) = (rng.unit() * 3. - 1., rng.unit() * 3. - 1.);
                    let s = 0.002 + rng.unit() * 0.05;
                    [
                        [u as f32, v as f32],
                        [(u + s) as f32, v as f32],
                        [u as f32, (v + s * 0.7) as f32],
                    ]
                })
                .collect();
            let mut marks = Marks::new(w, h);
            for uv in &triangles {
                marks.mark(sampler, *uv);
            }
            let chains = store(&scratch, &level0, class, &marks, None);
            let mut reader = Reader::new(&chains);
            for uv in &triangles {
                for _ in 0..200 {
                    let p = interpolate(*uv, &mut rng);
                    let lod = match rng.next() % 5 {
                        0 => 0.,
                        1 => f64::INFINITY,
                        2 => 40.,
                        _ => rng.unit() * levels.len() as f64,
                    };
                    let got = reader.sample(0, class, sampler, p, lod).unwrap();
                    let want = reference(&levels, class, sampler, p, lod);
                    assert_eq!(
                        got.map(f64::to_bits),
                        want.map(f64::to_bits),
                        "{w}x{h} mode {mode} uv {p:?} lod {lod}"
                    );
                }
            }
            // A non-finite coordinate reads texel (0, 0), which every chain keeps.
            let got = reader
                .sample(0, class, sampler, [f32::NAN, 0.5], 0.)
                .unwrap();
            assert_eq!(got, reference(&levels, class, sampler, [f32::NAN, 0.5], 0.));
            if w >= 200 {
                let (stored, _) = marks.marked();
                assert!(
                    stored < marks.tiles(),
                    "{w}x{h}: {stored} of {}",
                    marks.tiles()
                );
            }
        }
    }
}

/// Sensitive control: with one needed tile left out of the store, the sample that reads it
/// is an `InvalidState` error, not a substituted value; an unmarked image is one too.
#[test]
fn a_sample_outside_the_stored_tiles_is_an_error() {
    let scratch = crate::spill::Scratch::create(None, 0).unwrap();
    let level0 = image(300, 300, 3);
    let sampler = SourceSampler {
        wrap_s: 33071,
        wrap_t: 33071,
        min_filter: 9987,
        mag_filter: 9729,
    };
    let uv = [[0.5f32, 0.5], [0.52, 0.5], [0.5, 0.52]];
    let mut marks = Marks::new(300, 300);
    marks.mark(sampler, uv);
    // Level 0 texel (150, 150) is in tile (2, 2) of a 5-tile-wide level.
    let chains = store(
        &scratch,
        &level0,
        Class::Colour,
        &marks,
        Some((0, 2 * 5 + 2)),
    );
    let mut reader = Reader::new(&chains);
    let error = reader
        .sample(0, Class::Colour, sampler, [0.505, 0.505], 0.)
        .unwrap_err();
    assert_eq!(error.kind(), JobErrorKind::InvalidState, "{error}");
    // Coarser levels are still served.
    reader
        .sample(0, Class::Colour, sampler, [0.505, 0.505], 3.)
        .unwrap();
    let error = reader
        .sample(1, Class::Colour, sampler, [0.5, 0.5], 0.)
        .unwrap_err();
    assert_eq!(error.kind(), JobErrorKind::InvalidState);
    // A tile beyond the mark: far from the triangle (and from tile 0) at level 0.
    let error = reader
        .sample(0, Class::Colour, sampler, [0.95, 0.05], 0.)
        .unwrap_err();
    assert_eq!(error.kind(), JobErrorKind::InvalidState);
    drop(reader);
    assert!(chains.decodes() > 0);
}
