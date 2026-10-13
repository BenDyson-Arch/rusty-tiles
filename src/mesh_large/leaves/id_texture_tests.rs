use super::*;
use crate::basisu_ktx2 as codec;
use std::collections::BTreeSet;

const MEMORY: u64 = 1 << 30;

fn lcg(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed >> 33
}

/// Random IDs below each set's null, and a support map with an empty border
/// and a hole, so pages have both islands and gutters.
fn fixture(w: u32, h: u32, sets: &[IdSet], seed: u64) -> (Vec<u32>, Vec<u32>) {
    let mut seed = seed;
    let (w, h) = (w as usize, h as usize);
    let mut ids = Vec::new();
    let mut support = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let border = x == 0 || y == 0 || x + 1 == w || y + 1 == h;
            let hole = (w / 2..w / 2 + 2).contains(&x) && (h / 2..h / 2 + 2).contains(&y);
            support.push(if border || hole {
                0
            } else {
                1 + (lcg(&mut seed) % 3) as u32
            });
            for set in sets {
                ids.push((lcg(&mut seed) % u64::from(set.null())) as u32);
            }
        }
    }
    (ids, support)
}

fn png(bytes: &[u8]) -> mips::Level {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .unwrap()
        .into_rgba8();
    mips::Level {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

fn bake_fixture(w: u32, h: u32, sets: &[IdSet], seed: u64) -> (IdTexture, Vec<u32>, Vec<u32>) {
    let (ids, support) = fixture(w, h, sets, seed);
    let out = bake(&Request {
        width: w,
        height: h,
        sets,
        ids: &ids,
        support: &support,
        memory_limit: MEMORY,
    })
    .unwrap();
    (out, ids, support)
}

/// Independent oracle: the set of values each level-L texel may hold, built
/// by unioning child sets level by level (not by level-0 region arithmetic).
fn allowed_sets(base: &[u8], support: &[u32], w: usize, h: usize) -> Vec<Vec<BTreeSet<[u8; 4]>>> {
    let mut level: Vec<BTreeSet<[u8; 4]>> = (0..w * h)
        .map(|i| {
            let mut s = BTreeSet::new();
            if support[i] > 0 {
                s.insert(base[i * 4..i * 4 + 4].try_into().unwrap());
            }
            s
        })
        .collect();
    let (mut lw, mut lh) = (w, h);
    let mut out = vec![level.clone()];
    while lw > 1 || lh > 1 {
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
        let mut next = vec![BTreeSet::new(); nw * nh];
        for y in 0..lh {
            for x in 0..lw {
                let parent = (y / 2).min(nh - 1) * nw + (x / 2).min(nw - 1);
                next[parent].extend(level[y * lw + x].iter().copied());
            }
        }
        out.push(next.clone());
        level = next;
        (lw, lh) = (nw, nh);
    }
    out
}

fn oracle_accepts(allowed: &[BTreeSet<[u8; 4]>], level: &[u8]) -> bool {
    allowed.iter().enumerate().all(|(i, s)| {
        let got: [u8; 4] = level[i * 4..i * 4 + 4].try_into().unwrap();
        if s.is_empty() {
            got == [255; 4]
        } else {
            s.contains(&got)
        }
    })
}

/// The request page as the production packer must have laid it out.
fn request_page(
    w: u32,
    h: u32,
    sets: &[IdSet],
    ids: &[u32],
    support: &[u32],
    page: usize,
) -> Vec<u8> {
    let (layouts, _) = layout(sets).unwrap();
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    for (t, &s) in support.iter().enumerate() {
        if s == 0 {
            continue;
        }
        for (k, l) in layouts.iter().enumerate().filter(|(_, l)| l.page == page) {
            let bytes = ids[t * sets.len() + k].to_le_bytes();
            for (b, &c) in l.channels.iter().enumerate() {
                rgba[t * 4 + c as usize] = bytes[b];
            }
        }
    }
    rgba
}

#[test]
fn exact_id_round_trip() {
    let sets = [IdSet { bytes: 4 }];
    let (out, ids, support) = bake_fixture(16, 16, &sets, 7);
    assert_eq!(out.pages.len(), 1);
    assert_eq!(out.layouts[0].channels, vec![0, 1, 2, 3]);
    let level = png(&out.pages[0].levels[0]);
    assert_eq!((level.width, level.height), (16, 16));
    for (t, &s) in support.iter().enumerate() {
        let got = read(&level.rgba[t * 4..t * 4 + 4], &[0, 1, 2, 3]);
        let want = if s > 0 { ids[t] } else { u32::MAX };
        assert_eq!(got, want, "texel {t}");
    }
    // One-bit mutant of a decoded ID is caught by the independent level check.
    let base = request_page(16, 16, &sets, &ids, &support, 0);
    check_level(&base, &support, [16, 16], &level).unwrap();
    let mut mutant = level;
    let t = support.iter().position(|&s| s > 0).unwrap();
    mutant.rgba[t * 4] ^= 1;
    assert!(check_level(&base, &support, [16, 16], &mutant).is_err());
    // IDs equal to the null value are refused, not silently made gutters.
    let mut bad = ids.clone();
    bad[t] = u32::MAX;
    let err = bake(&Request {
        width: 16,
        height: 16,
        sets: &sets,
        ids: &bad,
        support: &support,
        memory_limit: MEMORY,
    })
    .unwrap_err();
    assert_eq!(err.kind(), JobErrorKind::InvalidRequest);
}

#[test]
fn every_level_holds_only_source_region_ids_and_mutants_fail() {
    let sets = [IdSet { bytes: 2 }, IdSet { bytes: 2 }];
    let (out, ids, support) = bake_fixture(32, 16, &sets, 11);
    let base = request_page(32, 16, &sets, &ids, &support, 0);
    let allowed = allowed_sets(&base, &support, 32, 16);
    assert_eq!(out.pages[0].levels.len(), allowed.len());
    for (k, bytes) in out.pages[0].levels.iter().enumerate() {
        let level = png(bytes);
        assert_eq!([level.width, level.height], out.pages[0].sizes[k]);
        assert!(oracle_accepts(&allowed[k], &level.rgba), "level {k}");
    }
    // Mutant: the averaging (Data) sparse chain blends IDs.
    let averaged = sparse_mips::build(32, 16, &base, &support, Class::Data, MEMORY).unwrap();
    let blended = averaged
        .levels
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(k, l)| !oracle_accepts(&allowed[*k], &l.rgba))
        .count();
    assert!(blended > 0);
    assert!(averaged.levels[1..].iter().any(|l| {
        let level = mips::Level {
            width: l.width,
            height: l.height,
            rgba: l.rgba.clone(),
        };
        check_level(&base, &support, [32, 16], &level).is_err()
    }));
    // Mutant: linear (bilinear 2x2 box) reduction of the exact level 0.
    let level0 = png(&out.pages[0].levels[0]);
    let linear = mips::build(32, 16, level0.rgba.clone(), Class::Data, None, 1 << 30).unwrap();
    assert!(!oracle_accepts(&allowed[1], &linear.levels[1].rgba));
    assert!(check_level(&base, &support, [32, 16], &linear.levels[1]).is_err());
}

#[test]
fn mode_majority_and_ties_keep_first_row_major() {
    // 2x2 footprints: A,A,A,B -> A; B weighted 3 vs A,A -> B; tie -> first.
    let (a, b) = ([1, 0, 0, 0], [2, 0, 0, 0]);
    let mut rgba = vec![0u8; 4 * 4 * 4];
    let mut support = vec![1u32; 16];
    let mut put = |x: usize, y: usize, p: [u8; 4]| rgba[(y * 4 + x) * 4..][..4].copy_from_slice(&p);
    for (x, y, p) in [(0, 0, a), (1, 0, a), (0, 1, a), (1, 1, b)] {
        put(x, y, p);
    }
    for (x, y, p) in [(2, 0, a), (3, 0, b), (2, 1, a), (3, 1, [9, 0, 0, 0])] {
        put(x, y, p);
    }
    for (x, y, p) in [(0, 2, b), (1, 2, a), (0, 3, a), (1, 3, b)] {
        put(x, y, p);
    }
    for (x, y, p) in [(2, 2, a), (3, 2, a), (2, 3, b), (3, 3, b)] {
        put(x, y, p);
    }
    support[3] = 3; // B at (3,0) outweighs A,A (and the 9) in footprint (1,0).
    support[7] = 0; // (3,1) is gutter.
    let chain = sparse_mips::build(4, 4, &rgba, &support, Class::Id, MEMORY).unwrap();
    let l1 = &chain.levels[1].rgba;
    assert_eq!(&l1[0..4], &a); // majority
    assert_eq!(&l1[4..8], &b); // support-weighted majority
    assert_eq!(&l1[8..12], &b); // 2:2 tie, first row-major is B at (0,2)
    assert_eq!(&l1[12..16], &a); // 2:2 tie, first row-major is A at (2,2)
    assert_eq!(&chain.levels[0].rgba[28..32], &[255; 4]);
    // Dense (#163 API) ID mips: odd and 1-D pages also take the overlap mode.
    let dense: Vec<u8> = (0..15usize)
        .flat_map(|i| [[0, 10, 200, 7][i % 4], 0, 0, 255])
        .collect();
    let chain = mips::build(5, 3, dense.clone(), Class::Id, None, 1 << 20).unwrap();
    let values: BTreeSet<[u8; 4]> = dense.as_chunks::<4>().0.iter().copied().collect();
    for l in &chain.levels {
        assert!(l.rgba.as_chunks::<4>().0.iter().all(|p| values.contains(p)));
    }
    let averaged = mips::build(5, 3, dense, Class::Data, None, 1 << 20).unwrap();
    assert!(averaged.levels[1..].iter().any(|l| l
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| !values.contains(p))));
}

#[test]
fn gutters_and_padding_read_no_feature_at_every_level() {
    let sets = [IdSet { bytes: 1 }, IdSet { bytes: 3 }];
    // One island in the top-left quadrant of a 16x16 page.
    let support: Vec<u32> = (0..256)
        .map(|i| u32::from(i % 16 < 6 && i / 16 < 6))
        .collect();
    let ids: Vec<u32> = (0..512).map(|i| (i * 37 % 200) as u32).collect();
    let out = bake(&Request {
        width: 16,
        height: 16,
        sets: &sets,
        ids: &ids,
        support: &support,
        memory_limit: MEMORY,
    })
    .unwrap();
    for (k, bytes) in out.pages[0].levels.iter().enumerate() {
        let level = png(bytes);
        let f = 16 / level.width as usize;
        for y in 0..level.height as usize {
            for x in 0..level.width as usize {
                if x * f >= 6 || y * f >= 6 {
                    let p = &level.rgba[(y * level.width as usize + x) * 4..][..4];
                    assert_eq!(p, &[255; 4], "level {k} ({x},{y})");
                    assert_eq!(read(p, &out.layouts[0].channels), sets[0].null());
                    assert_eq!(read(p, &out.layouts[1].channels), sets[1].null());
                }
            }
        }
    }
    assert_eq!((sets[0].null(), sets[1].null()), (255, 0x00ff_ffff));
    // Mutant: nearest-signal dilation (the Data gutter fill) is refused.
    let base = request_page(16, 16, &sets, &ids, &support, 0);
    let dilated = sparse_mips::build(16, 16, &base, &support, Class::Data, MEMORY).unwrap();
    let level = mips::Level {
        width: 16,
        height: 16,
        rgba: dilated.levels[0].rgba.clone(),
    };
    assert!(check_level(&base, &support, [16, 16], &level).is_err());
}

#[test]
fn any_number_of_feature_id_sets() {
    for n in 1..=9usize {
        let sets = vec![IdSet { bytes: 1 }; n];
        let (layouts, pages) = layout(&sets).unwrap();
        assert_eq!(pages, n.div_ceil(4));
        assert_eq!(layouts[n - 1].page, (n - 1) / 4);
    }
    let sets = [1, 2, 1, 4, 3, 1, 2].map(|bytes| IdSet { bytes });
    let (layouts, pages) = layout(&sets).unwrap();
    let placed: Vec<_> = layouts
        .iter()
        .map(|l| (l.page, l.channels.clone()))
        .collect();
    assert_eq!(
        placed,
        vec![
            (0, vec![0]),
            (0, vec![1, 2]),
            (0, vec![3]),
            (1, vec![0, 1, 2, 3]),
            (2, vec![0, 1, 2]),
            (2, vec![3]),
            (3, vec![0, 1]),
        ]
    );
    assert_eq!(pages, 4);
    assert!(layout(&[]).is_err());
    assert!(layout(&[IdSet { bytes: 0 }]).is_err());
    assert!(layout(&[IdSet { bytes: 5 }]).is_err());

    // Every set of a 7-set request reads back exactly from its own page.
    let (out, ids, support) = bake_fixture(8, 8, &sets, 23);
    assert_eq!(out.pages.len(), 4);
    let decoded: Vec<_> = out.pages.iter().map(|p| png(&p.levels[0])).collect();
    for (t, &s) in support.iter().enumerate() {
        for (k, l) in out.layouts.iter().enumerate() {
            let got = read(&decoded[l.page].rgba[t * 4..t * 4 + 4], &l.channels);
            let want = if s > 0 {
                ids[t * sets.len() + k]
            } else {
                sets[k].null()
            };
            assert_eq!(got, want, "set {k} texel {t}");
        }
    }
    for (k, counts) in out.feature_counts.iter().enumerate() {
        let distinct: BTreeSet<u32> = (0..64)
            .filter(|&t| support[t] > 0)
            .map(|t| ids[t * sets.len() + k])
            .collect();
        assert_eq!(counts[0] as usize, distinct.len());
        assert!(counts.windows(2).all(|w| w[1] <= w[0]));
    }

    // Wiring: attaching a level adds one PNG page per ID page, one shared
    // NEAREST sampler, and one featureIds entry per set after the two
    // source attribute sets the leaf writer always emits.
    let mut atlas = PreparedAtlas {
        primitives: Vec::new(),
        pages: Vec::new(),
        textures: Vec::new(),
        samplers: vec![json!({"magFilter":9729,"minFilter":9987})],
    };
    let refs = out.append_level(&mut atlas, 0).unwrap();
    assert_eq!(refs.len(), 7);
    assert_eq!(atlas.pages.len(), 4);
    assert_eq!(atlas.samplers.len(), 2);
    for r in &refs {
        assert_eq!(atlas.textures[r.texture]["sampler"], 1);
        let entry = r.feature_id(0, "annotation").unwrap();
        assert_eq!(entry["texture"]["channels"], json!(r.channels));
        assert_eq!(entry["nullFeatureId"], json!(r.null_feature_id));
        assert_eq!(r.property(0)["index"], json!(r.texture));
    }
    let proxy = out.append_level(&mut atlas, 2).unwrap();
    assert_eq!(atlas.samplers.len(), 2);
    assert_eq!(
        png(
            &atlas.pages[atlas.textures[proxy[0].texture]["source"].as_u64().unwrap() as usize]
                .bytes
        )
        .width,
        2
    );
    assert!(out.append_level(&mut atlas, 99).is_err());
}

#[test]
fn sampler_is_nearest_and_linear_or_mipmapped_mutants_fail() {
    let s = sampler();
    assert_eq!(s["magFilter"], 9728);
    assert_eq!(s["minFilter"], 9728);
    check_sampler(&s).unwrap();
    for (field, value) in [
        ("magFilter", json!(9729)),
        ("minFilter", json!(9729)),
        ("minFilter", json!(9984)),
        ("minFilter", json!(9985)),
        ("minFilter", json!(9986)),
        ("minFilter", json!(9987)),
        ("magFilter", Value::Null),
    ] {
        let mut mutant = s.clone();
        if value.is_null() {
            mutant.as_object_mut().unwrap().remove(field);
        } else {
            mutant[field] = value;
        }
        assert!(check_sampler(&mutant).is_err(), "{mutant}");
    }
    // Emulated sampling: NEAREST reproduces texels, LINEAR blends IDs.
    let sets = [IdSet { bytes: 2 }];
    let (out, _, _) = bake_fixture(16, 16, &sets, 5);
    let level = png(&out.pages[0].levels[0]);
    check_sampled(&level, &s).unwrap();
    for field in ["magFilter", "minFilter"] {
        let mut linear = s.clone();
        linear[field] = json!(9729);
        assert!(check_sampled(&level, &linear).is_err(), "{field}");
    }
}

#[test]
fn basis_ktx2_is_refused_and_would_change_ids() {
    let sets = [IdSet { bytes: 4 }];
    let (ids, support) = fixture(16, 16, &sets, 3);
    let base = request_page(16, 16, &sets, &ids, &support, 0);
    let chain = sparse_mips::build(16, 16, &base, &support, Class::Id, MEMORY).unwrap();
    for profile in [
        crate::mesh_large::config::Profile::Fast,
        crate::mesh_large::config::Profile::Balanced,
        crate::mesh_large::config::Profile::Quality,
    ] {
        let err = delivery::encode_authored(
            &chain,
            crate::gltf_source::source::TextureSlot::BaseColor,
            profile,
            MEMORY,
            None,
            |_| Ok(Value::Null),
        )
        .unwrap_err();
        assert_eq!(err.kind(), JobErrorKind::InvalidRequest);
    }
    // Why: UASTC (the least lossy Basis mode) does not round-trip IDs.
    let level = [codec::MipLevel {
        rgba: &base,
        width: 16,
        height: 16,
    }];
    let params = codec::Params {
        effort: 0,
        transfer: codec::Transfer::Linear,
        mipmaps: false,
    };
    // 128 bits per 4x4 block cannot carry 32 random bits per texel; if the
    // codec's own verification does not already refuse it, IDs change.
    if let Ok((_, decoded)) =
        codec::encode_ktx2_mips_decoded(&level, codec::Mode::Uastc { zstd: true }, params)
    {
        assert_ne!(decoded[0], base);
        let decoded = mips::Level {
            width: 16,
            height: 16,
            rgba: decoded[0].clone(),
        };
        assert!(check_level(&base, &support, [16, 16], &decoded).is_err());
    }
}

#[test]
fn featureless_levels_emit_no_feature_id_entry() {
    let sets = [IdSet { bytes: 1 }];
    let out = bake(&Request {
        width: 4,
        height: 4,
        sets: &sets,
        ids: &[0; 16],
        support: &[0; 16],
        memory_limit: MEMORY,
    })
    .unwrap();
    assert!(out.feature_counts[0].iter().all(|&n| n == 0));
    let mut atlas = PreparedAtlas {
        primitives: Vec::new(),
        pages: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
    };
    let refs = out.append_level(&mut atlas, 0).unwrap();
    assert_eq!(refs[0].feature_id(0, "empty"), None);
    assert_eq!(png(&atlas.pages[0].bytes).rgba, vec![255; 64]);
}
