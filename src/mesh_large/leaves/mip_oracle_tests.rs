//! Independent whole-page semantic reference: continuous overlap integration,
//! source-authored truth and sensitive controls. No production filter helpers.
use super::mips::{self, Class, Mask};

#[derive(Clone)]
struct Image {
    w: usize,
    h: usize,
    pixels: Vec<u8>,
}
fn quantize(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8
}
fn from_srgb(b: u8) -> f64 {
    let s = f64::from(b) / 255.0;
    if s < 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}
fn to_srgb(v: f64) -> u8 {
    quantize(if v < 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    })
}
fn vector(p: &[u8]) -> [f64; 3] {
    [p[0], p[1], p[2]].map(|x| 2.0 * f64::from(x) / 255.0 - 1.0)
}
fn length(v: [f64; 3]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}
fn direction(v: [f64; 3]) -> [f64; 3] {
    let n = length(v);
    assert!(n > 1e-12, "reference authored normal must be nonzero");
    v.map(|x| x / n)
}
fn effective_pass(a: u8, mask: Mask) -> bool {
    f64::from(a) * mask.factor / 255.0 >= mask.cutoff
}
fn pass_fraction(pixels: &[u8], mask: Mask) -> f64 {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| effective_pass(p[3], mask))
        .count() as f64
        / (pixels.len() / 4) as f64
}
fn scaled_alpha(a: u8, scale: f64) -> u8 {
    quantize(f64::from(a) * scale / 255.0)
}
fn preserve_mask(image: &mut Image, mask: Mask, target: f64) -> f64 {
    // Enumerate actual pixel transition candidates rather than the producer's
    // histogram. Actual quantized output defines the attainable coverage.
    let first_pass = (0..=255).find(|&a| effective_pass(a, mask));
    let mut scales = vec![0.0, 1.0];
    if let Some(threshold) = first_pass.filter(|&t| t > 0) {
        for pixel in image.pixels.as_chunks::<4>().0.iter().filter(|p| p[3] > 0) {
            let boundary = (f64::from(threshold) - 0.5) / f64::from(pixel[3]);
            scales.push(f64::from_bits(boundary.to_bits() + 1));
        }
    }
    let evaluate = |s: f64| {
        let passing = image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| effective_pass(scaled_alpha(p[3], s), mask))
            .count();
        (
            (passing as f64 / (image.w * image.h) as f64 - target).abs(),
            (s - 1.0).abs(),
            s,
        )
    };
    let chosen = scales
        .into_iter()
        .map(evaluate)
        .min_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap()
        .2;
    for pixel in image.pixels.as_chunks_mut::<4>().0.iter_mut() {
        pixel[3] = scaled_alpha(pixel[3], chosen);
    }
    chosen
}
fn reference(w: usize, h: usize, pixels: Vec<u8>, class: Class, mask: Option<Mask>) -> Vec<Image> {
    let target = mask.map(|m| pass_fraction(&pixels, m));
    let mut first = Image { w, h, pixels };
    if class == Class::Normal {
        for p in first.pixels.as_chunks_mut::<4>().0.iter_mut() {
            let v = direction(vector(p));
            for c in 0..3 {
                p[c] = quantize((v[c] + 1.0) / 2.0);
            }
        }
    }
    if let Some(m) = mask {
        preserve_mask(&mut first, m, target.unwrap());
    }
    let mut images = vec![first];
    while images.last().unwrap().w > 1 || images.last().unwrap().h > 1 {
        let input = images.last().unwrap();
        let (w, h) = ((input.w / 2).max(1), (input.h / 2).max(1));
        let mut output = Image {
            w,
            h,
            pixels: vec![0; w * h * 4],
        };
        for y in 0..h {
            for x in 0..w {
                let (left, right) = (
                    x as f64 * input.w as f64 / w as f64,
                    (x + 1) as f64 * input.w as f64 / w as f64,
                );
                let (top, bottom) = (
                    y as f64 * input.h as f64 / h as f64,
                    (y + 1) as f64 * input.h as f64 / h as f64,
                );
                let mut sums = [0.0; 4];
                let mut area = 0.0;
                let mut dominant = ([0.0; 3], -1.0);
                // Brute-force every source texel, avoiding integer-overlap loops
                // shared with production. Fixtures deliberately stay small.
                for sy in 0..input.h {
                    for sx in 0..input.w {
                        let overlap_x = (right.min((sx + 1) as f64) - left.max(sx as f64)).max(0.0);
                        let overlap_y = (bottom.min((sy + 1) as f64) - top.max(sy as f64)).max(0.0);
                        let weight = overlap_x * overlap_y;
                        if weight == 0.0 {
                            continue;
                        }
                        let p = &input.pixels[(sy * input.w + sx) * 4..][..4];
                        let rgb = match class {
                            Class::Colour => [from_srgb(p[0]), from_srgb(p[1]), from_srgb(p[2])],
                            Class::Data => [p[0], p[1], p[2]].map(|a| f64::from(a) / 255.0),
                            Class::Normal => direction(vector(p)),
                            Class::Id => unreachable!("ID class has its own oracle in id_texture"),
                        };
                        if class == Class::Normal && weight > dominant.1 {
                            dominant = (rgb, weight);
                        }
                        for c in 0..3 {
                            sums[c] += weight * rgb[c];
                        }
                        sums[3] += weight * f64::from(p[3]) / 255.0;
                        area += weight;
                    }
                }
                let rgb = if class == Class::Normal {
                    if length([sums[0], sums[1], sums[2]]) <= 1e-12 {
                        dominant.0
                    } else {
                        direction([sums[0], sums[1], sums[2]])
                    }
                } else {
                    [sums[0] / area, sums[1] / area, sums[2] / area]
                };
                let p = &mut output.pixels[(y * w + x) * 4..][..4];
                for c in 0..3 {
                    p[c] = match class {
                        Class::Colour => to_srgb(rgb[c]),
                        Class::Data => quantize(rgb[c]),
                        Class::Normal => quantize((rgb[c] + 1.0) / 2.0),
                        Class::Id => unreachable!("ID class has its own oracle in id_texture"),
                    };
                }
                p[3] = quantize(sums[3] / area);
            }
        }
        if let Some(m) = mask {
            preserve_mask(&mut output, m, target.unwrap());
        }
        images.push(output);
    }
    images
}
fn compare(w: usize, h: usize, pixels: Vec<u8>, class: Class, mask: Option<Mask>) -> mips::Chain {
    let expected = reference(w, h, pixels.clone(), class, mask);
    let actual = mips::build(w as u32, h as u32, pixels, class, mask, 1 << 20).unwrap();
    assert_eq!(actual.levels.len(), expected.len());
    for (level, (got, want)) in actual.levels.iter().zip(&expected).enumerate() {
        assert_eq!((got.width as usize, got.height as usize), (want.w, want.h));
        for (byte, (&a, &b)) in got.rgba.iter().zip(&want.pixels).enumerate() {
            assert!(
                a.abs_diff(b) <= 1,
                "level {level} byte {byte}: {a} versus {b}"
            );
        }
        if class == Class::Normal {
            for (p, q) in got
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(want.pixels.as_chunks::<4>().0.iter())
            {
                assert!(
                    (length(vector(p)) - 1.0).abs() <= 0.01,
                    "raw length at mip {level}"
                );
                let (a, b) = (direction(vector(p)), direction(vector(q)));
                let dot = a
                    .iter()
                    .zip(b)
                    .map(|(x, y)| x * y)
                    .sum::<f64>()
                    .clamp(-1.0, 1.0);
                assert!(
                    dot.acos().to_degrees() < 1.0,
                    "angular error at mip {level}"
                );
            }
        }
        if let Some(m) = mask {
            let report = &actual.coverage[level];
            let target = pass_fraction(&expected[0].pixels, m);
            let measured = pass_fraction(&got.rgba, m);
            assert!(
                (measured - pass_fraction(&want.pixels, m)).abs() < 1e-12,
                "independent MASK coverage mip{level}"
            );
            assert!((report.target - target).abs() < 1e-12);
            assert!((report.actual - measured).abs() < 1e-12);
            assert!((report.residual - (measured - target).abs()).abs() < 1e-12);
            assert!(report
                .attainable_counts
                .contains(&((measured * (want.w * want.h) as f64).round() as usize)));
        }
    }
    assert_eq!(
        (
            actual.levels.last().unwrap().width,
            actual.levels.last().unwrap().height
        ),
        (1, 1)
    );
    actual
}

#[test]
fn every_colour_mip_linearizes_rgb_but_keeps_alpha_linear() {
    let image = (0..64)
        .flat_map(|i| {
            if (i / 8 + i % 8) % 2 == 0 {
                [0, 255, 0, 0]
            } else {
                [255, 0, 255, 255]
            }
        })
        .collect();
    let chain = compare(8, 8, image, Class::Colour, None);
    for level in &chain.levels[1..] {
        for p in level.rgba.as_chunks::<4>().0.iter() {
            assert_eq!(&p[..3], &[188, 188, 188]);
            assert_eq!(p[3], 128);
        }
    }
    // Sensitive wrong-gamma and alpha-gamma mutants fail distinct channels.
    assert!(188u8.abs_diff(128) > 1);
}
#[test]
fn odd_rectangle_and_one_dimensional_footprints_include_every_edge_channel() {
    for (w, h) in [(5, 3), (1, 7), (7, 1), (3, 5), (1, 1)] {
        let data = (0..w * h)
            .flat_map(|i| {
                [
                    (i * 17 % 256) as u8,
                    (i * 43 % 256) as u8,
                    (i * 71 % 256) as u8,
                    (i * 101 % 256) as u8,
                ]
            })
            .collect();
        compare(w, h, data, Class::Data, None);
    }
    let data = vec![0, 255, 0, 0, 255, 0, 255, 255];
    let chain = compare(2, 1, data, Class::Data, None);
    assert_eq!(chain.levels[1].rgba, [128, 128, 128, 128]);
    // Applying an sRGB conversion to linear data is a sensitive mutant.
    assert!(to_srgb(0.5).abs_diff(128) > 1);
}
#[test]
fn every_normal_mip_has_raw_unit_length_and_independent_direction() {
    let image = (0..64)
        .flat_map(|i| {
            if i % 2 == 0 {
                [218, 128, 218, 255]
            } else {
                [37, 128, 218, 255]
            }
        })
        .collect();
    let chain = compare(8, 8, image, Class::Normal, None);
    for level in &chain.levels[1..] {
        for p in level.rgba.as_chunks::<4>().0.iter() {
            assert!(vector(p)[2] > 0.99);
        }
    }
    // Missing renormalization averages these oblique normals to length~0.71.
    assert!((length(vector(&[128, 128, 218, 255])) - 1.0).abs() > 0.20);
}
fn mask_fixture() -> Vec<u8> {
    (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                let local = (y % 2) * 2 + x % 2;
                let pass = if x < 4 { local < 3 } else { local == 0 };
                [64, 128, 192, if pass { 255 } else { 0 }]
            })
        })
        .collect()
}
#[test]
fn mask_coverage_uses_factor_cutoff_and_reports_terminal_discreteness() {
    for mask in [
        Mask {
            factor: 1.0,
            cutoff: 0.8,
        },
        Mask {
            factor: 0.8,
            cutoff: 0.64,
        },
    ] {
        let chain = compare(8, 8, mask_fixture(), Class::Colour, Some(mask));
        assert!((chain.coverage[0].target - 0.5).abs() < 1e-12);
        assert_eq!(chain.coverage[1].residual, 0.0);
        assert!((chain.coverage.last().unwrap().residual - 0.5).abs() < 1e-12);
        // A plain-alpha reduction has .75/.25 effective alpha and no texels
        // survive cutoff .8. This mutant misses target coverage by .5.
        let ordinary = reference(8, 8, mask_fixture(), Class::Colour, None);
        assert_eq!(pass_fraction(&ordinary[1].pixels, mask), 0.0);
    }
    for mask in [
        Mask {
            factor: 0.0,
            cutoff: 0.0,
        },
        Mask {
            factor: 0.0,
            cutoff: 0.5,
        },
        Mask {
            factor: 0.5,
            cutoff: 1.0,
        },
    ] {
        compare(8, 8, mask_fixture(), Class::Colour, Some(mask));
    }
}

#[test]
fn normal_cancellation_chooses_source_contributor_not_neutral_vector() {
    let chain = compare(
        2,
        1,
        vec![255, 255, 255, 255, 0, 0, 0, 255],
        Class::Normal,
        None,
    );
    assert_eq!(chain.normal_cancellations[1], 1);
    assert_eq!(&chain.levels[1].rgba[..3], &chain.levels[0].rgba[..3]);
    assert_ne!(&chain.levels[1].rgba[..3], &[128, 128, 255]);
}

/// Historical abandoned ETC colour/MASK recipe: retains the original
/// 16 floor violations for diagnosis; this is not the current production gate.
#[test]
#[ignore = "historical abandoned ETC colour/MASK policy diagnostic: 16 retained failures; not the current production gate"]
fn historical_abandoned_policy_reports_retained_floor_violations() {
    use crate::basisu_ktx2::{encode_ktx2_mips, MipLevel, Mode, Params, Transfer};
    let mut violations: Vec<String> = Vec::new();
    for profile in ["fast", "balanced", "quality"] {
        for (role, class) in [
            ("colour", Class::Colour),
            ("colour-alpha", Class::Colour),
            ("mask", Class::Colour),
            ("data-r", Class::Data),
            ("data-rgb", Class::Data),
            ("normal", Class::Normal),
        ] {
            let source: Vec<u8> = (0..64)
                .flat_map(|i| match class {
                    Class::Colour => {
                        if (i / 8 + i % 8) % 2 == 0 {
                            [0, 255, 0, 0]
                        } else {
                            [255, 0, 255, 255]
                        }
                    }
                    Class::Data => {
                        if i % 2 == 0 {
                            [0, 255, 0, 255]
                        } else {
                            [255, 0, 255, 255]
                        }
                    }
                    Class::Normal => {
                        if i % 2 == 0 {
                            [218, 128, 218, 255]
                        } else {
                            [37, 128, 218, 255]
                        }
                    }
                    Class::Id => unreachable!("ID class has its own oracle in id_texture"),
                })
                .collect();
            let mut source = if role == "mask" {
                mask_fixture()
            } else {
                source
            };
            // Preserve the original alpha checker as colour-alpha. Add the
            // SAME saturated checker with alpha=255 as a separate OPAQUE case.
            if role == "colour" {
                for pixel in source.as_chunks_mut::<4>().0.iter_mut() {
                    pixel[3] = 255;
                }
            }
            let mask = if role == "mask" {
                Some(Mask {
                    factor: 0.8,
                    cutoff: 0.64,
                })
            } else {
                None
            };
            let truth = reference(8, 8, source.clone(), class, mask);
            let chain = mips::build(8, 8, source, class, mask, 1 << 20).unwrap();
            let mode = if profile == "quality" || class == Class::Normal || role == "data-rgb" {
                Mode::Uastc { zstd: true }
            } else {
                Mode::Etc1s {
                    quality: if profile == "fast" && class == Class::Colour {
                        40
                    } else {
                        90
                    },
                }
            };
            let params = Params {
                effort: 0,
                transfer: if class == Class::Colour {
                    Transfer::Srgb
                } else {
                    Transfer::Linear
                },
                mipmaps: false,
            };
            let levels: Vec<_> = chain
                .levels
                .iter()
                .map(|l| MipLevel {
                    rgba: &l.rgba,
                    width: l.width,
                    height: l.height,
                })
                .collect();
            let encoded = match encode_ktx2_mips(&levels, mode, params) {
                Ok(encoded) => encoded,
                Err(error) => {
                    println!("semantic-mip-encode-error profile={profile} role={role} kind={:?} error={}", error.kind(), error.message());
                    violations.push(format!(
                        "{profile} {role}: encode {:?}: {}",
                        error.kind(),
                        error.message()
                    ));
                    continue;
                }
            };
            let decoder = match basisu::Transcoder::new(&encoded) {
                Ok(decoder) => decoder,
                Err(error) => {
                    println!("semantic-mip-independent-error profile={profile} role={role} error={error:?}");
                    violations.push(format!("{profile} {role}: independent decoder {error:?}"));
                    continue;
                }
            };
            assert_eq!(decoder.level_count() as usize, truth.len());
            for (index, expected) in truth.iter().enumerate() {
                let delivered = match decoder.transcode(
                    index as u32,
                    basisu::TargetFormat::Rgba32,
                    basisu::DecodeFlags::NONE,
                ) {
                    Ok(delivered) => delivered,
                    Err(error) => {
                        println!("semantic-mip-independent-error profile={profile} role={role} mip={index} error={error:?}");
                        violations.push(format!(
                            "{profile} {role} mip{index}: independent decode {error:?}"
                        ));
                        continue;
                    }
                };
                assert_eq!(delivered.len(), expected.pixels.len());
                if class == Class::Colour {
                    let take = expected.pixels.len().min(16);
                    println!("semantic-mip-rgba profile={profile} role={role} mip={index} source_first4={:?} truth_first4={:?} delivered_first4={:?}", &truth[0].pixels[..16], &expected.pixels[..take], &delivered[..take]);
                }
                for channel in 0..4 {
                    if role == "data-r" && (channel == 1 || channel == 2) {
                        continue;
                    }
                    let mut sum = 0.0;
                    let mut largest = 0u8;
                    for (a, b) in delivered
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(expected.pixels.as_chunks::<4>().0.iter())
                    {
                        let e = a[channel].abs_diff(b[channel]);
                        sum += f64::from(e).powi(2);
                        largest = largest.max(e);
                    }
                    let psnr = if sum == 0.0 {
                        f64::INFINITY
                    } else {
                        10.0 * (65025.0 * (expected.w * expected.h) as f64 / sum).log10()
                    };
                    let floor = if channel == 3 {
                        if profile == "quality" {
                            34.0
                        } else {
                            30.0
                        }
                    } else if class == Class::Colour {
                        if profile == "quality" {
                            34.0
                        } else {
                            22.0
                        }
                    } else {
                        30.0
                    };
                    let cap = if channel == 3 {
                        match profile {
                            "fast" => 80,
                            "balanced" => 64,
                            _ => 32,
                        }
                    } else if class == Class::Colour {
                        match profile {
                            "fast" => 56,
                            "balanced" => 32,
                            _ => 24,
                        }
                    } else if role == "data-r" {
                        if profile == "quality" {
                            8
                        } else {
                            80
                        }
                    } else if class == Class::Data {
                        32
                    } else {
                        112
                    };
                    println!("semantic-mip-measure profile={profile} role={role} mip={index} width={} height={} channel={channel} psnr={psnr:.9} max_error={largest} psnr_floor={floor} max_error_floor={cap}", expected.w, expected.h);
                    if psnr < floor || largest > cap {
                        violations.push(format!("{profile} {role} mip{index} channel{channel} PSNR{psnr}<{floor} or max{largest}>{cap}"));
                    }
                }
                if let Some(mask) = mask {
                    let coverage = pass_fraction(&delivered, mask);
                    let target = pass_fraction(&truth[0].pixels, mask);
                    let floor = match profile {
                        "fast" => 0.02,
                        "balanced" => 0.01,
                        _ => 0.005,
                    };
                    let allowed = f64::max(floor, 1.0 / (expected.w * expected.h) as f64);
                    println!("semantic-mip-mask profile={profile} role={role} mip={index} width={} height={} coverage={coverage:.9} target={target:.9} residual={:.9} allowed={allowed:.9}", expected.w, expected.h, (coverage-target).abs());
                    if (coverage - target).abs() > allowed {
                        violations.push(format!("MASK {profile} {role} mip{index} coverage{coverage} target{target} residual>{allowed}"));
                    }
                }
                if class == Class::Normal {
                    let mut lengths = Vec::new();
                    let mut angles = Vec::new();
                    for (a, b) in delivered
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(expected.pixels.as_chunks::<4>().0.iter())
                    {
                        let v = vector(a);
                        lengths.push((length(v) - 1.0).abs());
                        let (v, q) = (direction(v), direction(vector(b)));
                        angles.push(
                            v.iter()
                                .zip(q)
                                .map(|(x, y)| x * y)
                                .sum::<f64>()
                                .clamp(-1.0, 1.0)
                                .acos()
                                .to_degrees(),
                        );
                    }
                    angles.sort_by(|a, b| a.total_cmp(b));
                    let mean = lengths.iter().sum::<f64>() / lengths.len() as f64;
                    let max = lengths.iter().copied().fold(0.0, f64::max);
                    let (mean_cap, max_cap) = match profile {
                        "fast" => (0.08, 0.20),
                        "balanced" => (0.05, 0.15),
                        _ => (0.03, 0.10),
                    };
                    let angle_mean = angles.iter().sum::<f64>() / angles.len() as f64;
                    let angle_p99 =
                        angles[((angles.len() * 99).div_ceil(100) - 1).min(angles.len() - 1)];
                    println!("semantic-mip-normal profile={profile} role={role} mip={index} width={} height={} raw_length_error_mean={mean:.9} raw_length_error_max={max:.9} angle_mean={angle_mean:.9} angle_p99={angle_p99:.9} length_mean_floor={mean_cap} length_max_floor={max_cap} angle_mean_floor=3 angle_p99_floor=12", expected.w, expected.h);
                    if mean > mean_cap || max > max_cap {
                        violations.push(format!("normal length {profile} {role} mip{index} mean{mean}>{mean_cap} or max{max}>{max_cap}"));
                    }
                    if angle_mean > 3.0 || angle_p99 > 12.0 {
                        violations.push(format!("normal angle {profile} {role} mip{index} mean{angle_mean}>3 or p99{angle_p99}>12"));
                    }
                }
            }
        }
    }
    println!("semantic-mip-inventory violations={}", violations.len());
    assert!(
        violations.is_empty(),
        "semantic mip quality gates failed: {violations:#?}"
    );
}

// Explicit bounded policy probes. Baseline source/reference/floors stay intact;
// these tests do not select any production profile or fallback.
fn probe_decode(
    images: &[Image],
    mode: crate::basisu_ktx2::Mode,
    transfer: crate::basisu_ktx2::Transfer,
) -> Vec<Vec<u8>> {
    probe_decode_effort(images, mode, transfer, 0)
}
fn probe_decode_effort(
    images: &[Image],
    mode: crate::basisu_ktx2::Mode,
    transfer: crate::basisu_ktx2::Transfer,
    effort: u8,
) -> Vec<Vec<u8>> {
    use crate::basisu_ktx2::{encode_ktx2_mips, MipLevel, Params};
    let levels: Vec<_> = images
        .iter()
        .map(|im| MipLevel {
            rgba: &im.pixels,
            width: im.w as u32,
            height: im.h as u32,
        })
        .collect();
    let encoded = encode_ktx2_mips(
        &levels,
        mode,
        Params {
            effort,
            transfer,
            mipmaps: false,
        },
    )
    .unwrap();
    let independent = basisu::Transcoder::new(&encoded).unwrap();
    assert_eq!(independent.level_count() as usize, images.len());
    images
        .iter()
        .enumerate()
        .map(|(i, im)| {
            let out = independent
                .transcode(
                    i as u32,
                    basisu::TargetFormat::Rgba32,
                    basisu::DecodeFlags::NONE,
                )
                .unwrap();
            assert_eq!(out.len(), im.pixels.len());
            out
        })
        .collect()
}
fn probe_channel(truth: &Image, delivered: &[u8], channel: usize) -> (f64, u8) {
    let mut squared = 0u64;
    let mut maximum = 0;
    for (a, b) in truth
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(delivered.as_chunks::<4>().0.iter())
    {
        let error = a[channel].abs_diff(b[channel]);
        maximum = maximum.max(error);
        squared += u64::from(error).pow(2);
    }
    (
        if squared == 0 {
            f64::INFINITY
        } else {
            10.0 * (65025.0 * (truth.w * truth.h) as f64 / squared as f64).log10()
        },
        maximum,
    )
}
fn opaque_saturated_checker() -> Vec<u8> {
    (0..64)
        .flat_map(|i| {
            if (i / 8 + i % 8) % 2 == 0 {
                [0, 255, 0, 255]
            } else {
                [255, 0, 255, 255]
            }
        })
        .collect()
}
#[test]
fn policy_probe_a_colour_uastc_keeps_saturated_checker_floors() {
    use crate::basisu_ktx2::{Mode, Transfer};
    let truth = reference(8, 8, opaque_saturated_checker(), Class::Colour, None);
    for (profile, quality, cap) in [("fast", 40, 56), ("balanced", 90, 32)] {
        let baseline = probe_decode(&truth, Mode::Etc1s { quality }, Transfer::Srgb);
        let (old_psnr, old_max) = probe_channel(&truth[0], &baseline[0], 0);
        println!("semantic-policy-a-baseline profile={profile} fixture=opaque-saturated-checker mip=0 channel=R psnr={old_psnr} max_error={old_max} psnr_floor=22 max_error_floor={cap}");
        assert!(
            old_psnr < 22.0 || old_max > cap,
            "baseline failure must remain independently observed"
        );
        let candidate = probe_decode(&truth, Mode::Uastc { zstd: true }, Transfer::Srgb);
        for (mip, (expected, out)) in truth.iter().zip(&candidate).enumerate() {
            for channel in 0..3 {
                let (psnr, maximum) = probe_channel(expected, out, channel);
                println!("semantic-policy-a profile={profile} proposed=UASTC-effort0 mip={mip} channel={channel} psnr={psnr} max_error={maximum} psnr_floor=22 max_error_floor={cap}");
                assert!(psnr >= 22.0 && maximum <= cap);
            }
        }
    }
}
#[test]
fn policy_probe_b_scalar_r_grayscale_preserves_original_r() {
    use crate::basisu_ktx2::{Mode, Transfer};
    let original: Vec<u8> = (0..64)
        .flat_map(|i| {
            if i % 2 == 0 {
                [0, 255, 0, 255]
            } else {
                [255, 0, 255, 255]
            }
        })
        .collect();
    let truth = reference(8, 8, original, Class::Data, None);
    let baseline = probe_decode(&truth, Mode::Etc1s { quality: 90 }, Transfer::Linear);
    let (old_psnr, old_max) = probe_channel(&truth[0], &baseline[0], 0);
    println!("semantic-policy-b-baseline fixture=scalar-R-with-unused-G-B mip=0 channel=R psnr={old_psnr} max_error={old_max} psnr_floor=30 max_error_floor=80");
    assert!(
        old_psnr < 30.0 || old_max > 80,
        "original scalar R failure must remain observed"
    );
    let mut prepared = truth.clone();
    for image in &mut prepared {
        for pixel in image.pixels.as_chunks_mut::<4>().0.iter_mut() {
            pixel[1] = pixel[0];
            pixel[2] = pixel[0];
        }
    }
    let candidate = probe_decode(&prepared, Mode::Etc1s { quality: 90 }, Transfer::Linear);
    println!("semantic-policy-b mapping=RGB-from-original-R alpha=unchanged meaningful-channel=R unused-channels=G,B mode=ETC1S-quality90 transfer=linear");
    for (mip, (expected, out)) in truth.iter().zip(&candidate).enumerate() {
        for channel in 0..3 {
            let (psnr, maximum) = probe_channel(expected, out, channel);
            println!("semantic-policy-b mip={mip} channel={channel} meaningful={} original_truth_psnr={psnr} original_truth_max_error={maximum}",channel==0);
            if channel == 0 {
                assert!(psnr >= 30.0 && maximum <= 80);
            }
        }
    }
}
#[test]
fn policy_probe_c_mask_one_lsb_guard_preserves_fixed_gates() {
    use crate::basisu_ktx2::{Mode, Transfer};
    let mask = Mask {
        factor: 0.8,
        cutoff: 0.64,
    };
    let truth = reference(8, 8, mask_fixture(), Class::Colour, Some(mask));
    let target = pass_fraction(&truth[0].pixels, mask);
    let baseline = probe_decode(&truth, Mode::Uastc { zstd: true }, Transfer::Srgb);
    let mut baseline_failures = 0;
    for (mip, (expected, out)) in truth.iter().zip(&baseline).enumerate() {
        let coverage = pass_fraction(out, mask);
        let allowed = f64::max(0.005, 1.0 / (expected.w * expected.h) as f64);
        println!("semantic-policy-c-baseline mip={mip} coverage={coverage} target={target} residual={} allowed={allowed}",(coverage-target).abs());
        if (coverage - target).abs() > allowed {
            baseline_failures += 1;
        }
    }
    assert!(
        baseline_failures > 0,
        "original quality MASK coverage failure must remain observed"
    );
    let mut prepared = truth.clone();
    let mut guarded = 0usize;
    for image in &mut prepared[1..] {
        for pixel in image.pixels.as_chunks_mut::<4>().0.iter_mut() {
            if effective_pass(pixel[3], mask) && pixel[3] < 255 {
                pixel[3] += 1;
                guarded += 1;
            }
        }
    }
    assert_eq!(
        prepared[0].pixels, truth[0].pixels,
        "base must remain unchanged"
    );
    assert!(guarded > 0);
    for (expected, input) in truth.iter().zip(&prepared) {
        for (&a, &b) in expected.pixels.iter().zip(&input.pixels) {
            assert!(a.abs_diff(b) <= 1, "fixed pre-encode one-byte floor");
        }
    }
    let candidate = probe_decode(&prepared, Mode::Uastc { zstd: true }, Transfer::Srgb);
    for (mip, (expected, out)) in truth.iter().zip(&candidate).enumerate() {
        let (psnr, maximum) = probe_channel(expected, out, 3);
        let coverage = pass_fraction(out, mask);
        let allowed = f64::max(0.005, 1.0 / (expected.w * expected.h) as f64);
        println!("semantic-policy-c proposed=one-LSB-passing-nonbase-alpha-guard mip={mip} alpha_psnr={psnr} alpha_max_error={maximum} alpha_psnr_floor=34 alpha_max_error_floor=32 coverage={coverage} target={target} residual={} allowed={allowed} guarded_texels={guarded}",(coverage-target).abs());
        assert!(psnr >= 34.0 && maximum <= 32);
        assert!((coverage - target).abs() <= allowed);
    }
}

#[test]
fn policy_probe_d_all_profile_efforts_opaque_colour() {
    use crate::basisu_ktx2::{Mode, Transfer};
    let truth = reference(8, 8, opaque_saturated_checker(), Class::Colour, None);
    let mut failures = Vec::new();
    for (profile, effort, floor, cap) in [
        ("fast", 0, 22.0, 56),
        ("balanced", 1, 22.0, 32),
        ("quality", 2, 34.0, 24),
    ] {
        let output =
            probe_decode_effort(&truth, Mode::Uastc { zstd: true }, Transfer::Srgb, effort);
        for (mip, (expected, delivered)) in truth.iter().zip(&output).enumerate() {
            for channel in 0..3 {
                let (psnr, maximum) = probe_channel(expected, delivered, channel);
                println!("semantic-policy-d profile={profile} effort={effort} mip={mip} channel={channel} psnr={psnr} max_error={maximum} psnr_floor={floor} max_error_floor={cap}");
                if psnr < floor || maximum > cap {
                    failures.push(format!(
                        "{profile} effort{effort} mip{mip} channel{channel}"
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "effort color floors missed: {failures:?}"
    );
}
fn passing_alpha_guard(truth: &[Image], mask: Mask) -> Vec<Image> {
    let mut prepared = truth.to_vec();
    for image in &mut prepared {
        for pixel in image.pixels.as_chunks_mut::<4>().0.iter_mut() {
            if effective_pass(pixel[3], mask) && pixel[3] < 255 {
                pixel[3] += 1;
            }
        }
    }
    for (expected, image) in truth.iter().zip(&prepared) {
        for (&a, &b) in expected.pixels.iter().zip(&image.pixels) {
            assert!(
                a.abs_diff(b) <= 1,
                "pre-encode one-byte floor including base"
            );
        }
    }
    prepared
}
#[test]
fn policy_probe_e_base_near_threshold_mask_guard() {
    use crate::basisu_ktx2::{Mode, Transfer};
    let mask = Mask {
        factor: 0.8,
        cutoff: 0.64,
    };
    let original: Vec<u8> = (0..8)
        .flat_map(|y| (0..8).flat_map(move |x| [64, 128, 192, 203 + (y / 4) * 2 + x / 4]))
        .collect();
    let truth = reference(8, 8, original, Class::Colour, Some(mask));
    let target = pass_fraction(&truth[0].pixels, mask);
    assert_eq!(target, 0.75);
    let prepared = passing_alpha_guard(&truth, mask);
    assert_ne!(
        prepared[0].pixels, truth[0].pixels,
        "base guard must actually be exercised"
    );
    let mut failures = Vec::new();
    for (profile, effort, floor, cap, coverage_floor) in [
        ("fast", 0, 30.0, 80, 0.02),
        ("balanced", 1, 30.0, 64, 0.01),
        ("quality", 2, 34.0, 32, 0.005),
    ] {
        let baseline =
            probe_decode_effort(&truth, Mode::Uastc { zstd: true }, Transfer::Srgb, effort);
        let output = probe_decode_effort(
            &prepared,
            Mode::Uastc { zstd: true },
            Transfer::Srgb,
            effort,
        );
        for (mip, (expected, delivered)) in truth.iter().zip(&output).enumerate() {
            let (psnr, maximum) = probe_channel(expected, delivered, 3);
            let source = pass_fraction(&expected.pixels, mask);
            let guarded = pass_fraction(&prepared[mip].pixels, mask);
            let old = pass_fraction(&baseline[mip], mask);
            let actual = pass_fraction(delivered, mask);
            let allowed = f64::max(coverage_floor, 1.0 / (expected.w * expected.h) as f64);
            println!("semantic-policy-e profile={profile} effort={effort} mip={mip} source_coverage={source} base_target={target} guarded_preencode_coverage={guarded} baseline_decoded_coverage={old} decoded_coverage={actual} residual={} allowed={allowed} alpha_psnr={psnr} alpha_max={maximum} psnr_floor={floor} max_floor={cap}",(actual-target).abs());
            if psnr < floor || maximum > cap || (actual - target).abs() > allowed {
                failures.push(format!("{profile} effort{effort} mip{mip}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "base MASK guard failed unchanged gates: {failures:?}"
    );
}
fn contextual_coverage(alpha: u8, contexts: &[f64], factor: f64, cutoff: f64) -> f64 {
    contexts
        .iter()
        .filter(|&&c| f64::from(alpha) / 255.0 * factor * c >= cutoff)
        .count() as f64
        / contexts.len() as f64
}
fn sampled_context_coverage(
    pixels: &[u8],
    width: usize,
    height: usize,
    contexts: &[f64],
    mask: Mask,
) -> f64 {
    assert_eq!(contexts.len(), 64);
    assert_eq!(pixels.len(), width * height * 4);
    let passing = contexts
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            // Source grid cell centres forward-map to the selected NEAREST mip.
            let x = ((2 * (i % 8) + 1) * width / 16).min(width - 1);
            let y = ((2 * (i / 8) + 1) * height / 16).min(height - 1);
            f64::from(pixels[(y * width + x) * 4 + 3]) / 255.0 * mask.factor * **c >= mask.cutoff
        })
        .count();
    passing as f64 / contexts.len() as f64
}
#[test]
fn policy_probe_f_variable_color_alpha_separate_contexts() {
    use crate::basisu_ktx2::{Mode, Transfer};
    let mask = Mask {
        factor: 0.8,
        cutoff: 0.64,
    };
    let contexts: Vec<f64> = (0..64).map(|i| if i < 32 { 1.0 } else { 0.5 }).collect();
    let truth = reference(8, 8, [64, 128, 192, 204].repeat(64), Class::Colour, None);
    let target = contextual_coverage(204, &contexts, mask.factor, mask.cutoff);
    assert_eq!(target, 0.5);
    let prepared = passing_alpha_guard(&truth, mask);
    let mut failures = Vec::new();
    for (profile, effort, floor, cap, coverage_floor) in [
        ("fast", 0, 30.0, 80, 0.02),
        ("balanced", 1, 30.0, 64, 0.01),
        ("quality", 2, 34.0, 32, 0.005),
    ] {
        let baseline =
            probe_decode_effort(&truth, Mode::Uastc { zstd: true }, Transfer::Srgb, effort);
        let output = probe_decode_effort(
            &prepared,
            Mode::Uastc { zstd: true },
            Transfer::Srgb,
            effort,
        );
        for (mip, (expected, delivered)) in truth.iter().zip(&output).enumerate() {
            // Fixed independent surface samples retain their COLOR alpha at
            // every mip, including 1x1. They are not texture texel counts.
            let source =
                sampled_context_coverage(&expected.pixels, expected.w, expected.h, &contexts, mask);
            let guarded = sampled_context_coverage(
                &prepared[mip].pixels,
                expected.w,
                expected.h,
                &contexts,
                mask,
            );
            let old =
                sampled_context_coverage(&baseline[mip], expected.w, expected.h, &contexts, mask);
            let actual =
                sampled_context_coverage(delivered, expected.w, expected.h, &contexts, mask);
            let allowed = f64::max(coverage_floor, 1.0 / contexts.len() as f64);
            let (psnr, maximum) = probe_channel(expected, delivered, 3);
            println!("semantic-policy-f profile={profile} effort={effort} mip={mip} contexts=1,0.5 samples=64 source_coverage={source} base_target={target} guarded_preencode_coverage={guarded} baseline_decoded_coverage={old} decoded_coverage={actual} residual={} allowed={allowed} alpha_psnr={psnr} alpha_max={maximum}",(actual-target).abs());
            if psnr < floor || maximum > cap || (actual - target).abs() > allowed {
                failures.push(format!("{profile} effort{effort} mip{mip}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "separate COLOR-alpha contexts failed gates: {failures:?}"
    );
}
#[test]
#[ignore = "root-owned coupled COLOR alpha counterexample proves guard policy limitation"]
fn policy_probe_g_coupled_color_alpha_guard_limitation() {
    use crate::basisu_ktx2::{Mode, Transfer};
    let mask = Mask {
        factor: 0.8,
        cutoff: 0.64,
    };
    let contexts = [1.0, 0.999];
    let truth = reference(8, 8, [64, 128, 192, 204].repeat(64), Class::Colour, None);
    let prepared = passing_alpha_guard(&truth, mask);
    let target = contextual_coverage(204, &contexts, mask.factor, mask.cutoff);
    assert_eq!(target, 0.5);
    // Two material/COLOR contexts sample THE SAME texel. One stored value
    // cannot independently preserve both cutoff decisions under this guard.
    assert_eq!(
        contextual_coverage(205, &contexts, mask.factor, mask.cutoff),
        1.0
    );
    for effort in [0, 1, 2] {
        let baseline =
            probe_decode_effort(&truth, Mode::Uastc { zstd: true }, Transfer::Srgb, effort);
        let output = probe_decode_effort(
            &prepared,
            Mode::Uastc { zstd: true },
            Transfer::Srgb,
            effort,
        );
        for mip in 0..truth.len() {
            let source =
                contextual_coverage(truth[mip].pixels[3], &contexts, mask.factor, mask.cutoff);
            let guarded =
                contextual_coverage(prepared[mip].pixels[3], &contexts, mask.factor, mask.cutoff);
            let old = contextual_coverage(baseline[mip][3], &contexts, mask.factor, mask.cutoff);
            let actual = contextual_coverage(output[mip][3], &contexts, mask.factor, mask.cutoff);
            println!("semantic-policy-g limitation=coupled-single-texel contexts=1,0.999 effort={effort} mip={mip} source_coverage={source} base_target={target} guarded_preencode_coverage={guarded} baseline_decoded_coverage={old} decoded_coverage={actual} guarded_residual={} decoded_residual={}",(guarded-target).abs(),(actual-target).abs());
            assert!(
                (guarded - target).abs() > 0.005,
                "counterexample must establish pre-encode policy limitation"
            );
        }
    }
}

// A bounded large witness uses the independent overlap integrator on each
// disjoint 2x2 cell. POT cells partition the image exactly; this avoids its
// intentionally quadratic all-image scan without importing production helpers.
fn bounded_pot_reference(edge: usize, pixels: Vec<u8>, class: Class) -> Vec<Image> {
    let mut base = Image {
        w: edge,
        h: edge,
        pixels,
    };
    if class == Class::Normal {
        for p in base.pixels.as_chunks_mut::<4>().0 {
            let unit = direction(vector(p));
            for c in 0..3 {
                p[c] = quantize((unit[c] + 1.) / 2.);
            }
        }
    }
    let mut levels = vec![base];
    while levels.last().unwrap().w > 1 {
        let previous = levels.last().unwrap();
        let edge = previous.w / 2;
        let mut pixels = Vec::with_capacity(edge * edge * 4);
        for y in 0..edge {
            for x in 0..edge {
                let mut cell = Vec::with_capacity(16);
                for dy in 0..2 {
                    for dx in 0..2 {
                        let offset = ((2 * y + dy) * previous.w + 2 * x + dx) * 4;
                        cell.extend_from_slice(&previous.pixels[offset..offset + 4]);
                    }
                }
                let integrated = reference(2, 2, cell, class, None);
                pixels.extend_from_slice(&integrated[1].pixels);
            }
        }
        levels.push(Image {
            w: edge,
            h: edge,
            pixels,
        });
    }
    levels
}

#[test]
fn current_production_policy_nonconstant_128_every_class_profile_and_level() {
    use crate::{gltf_source::source::TextureSlot, mesh_large::config::Profile};
    for slot in [
        TextureSlot::BaseColor,
        TextureSlot::Emissive,
        TextureSlot::Occlusion,
        TextureSlot::MetallicRoughness,
        TextureSlot::Normal,
    ] {
        let class = match slot {
            TextureSlot::BaseColor | TextureSlot::Emissive => Class::Colour,
            TextureSlot::Normal => Class::Normal,
            _ => Class::Data,
        };
        let pixels: Vec<_> = (0..128 * 128)
            .flat_map(|i| {
                let (x, y) = (i % 128, i / 128);
                match class {
                    Class::Colour => {
                        if (x + y) % 2 == 0 {
                            [0, 255, 0, 255]
                        } else {
                            [255, 0, 255, 255]
                        }
                    }
                    Class::Data => [
                        if x % 2 == 0 { 0 } else { 255 },
                        (100 + (x / 16) % 8) as u8,
                        (140 + (y / 16) % 8) as u8,
                        255,
                    ],
                    Class::Normal => {
                        if (x / 4 + y / 8) % 2 == 0 {
                            [218, 128, 218, 255]
                        } else {
                            [37, 128, 218, 255]
                        }
                    }
                    Class::Id => unreachable!("ID class has its own oracle in id_texture"),
                }
            })
            .collect();
        let truth = bounded_pot_reference(128, pixels.clone(), class);
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            let output = super::delivery::encode(
                [128, 128],
                &pixels,
                &vec![1; 128 * 128],
                slot,
                profile,
                128 * 1024 * 1024,
                None,
                super::delivery::LADDER,
            )
            .unwrap();
            let decoder = basisu::Transcoder::new(&output.bytes).unwrap();
            assert_eq!(decoder.level_count() as usize, truth.len());
            for (mip, expected) in truth.iter().enumerate() {
                let actual = decoder
                    .transcode(
                        mip as u32,
                        basisu::TargetFormat::Rgba32,
                        basisu::DecodeFlags::NONE,
                    )
                    .unwrap();
                for channel in 0..4 {
                    if (slot == TextureSlot::Occlusion && matches!(channel, 1 | 2))
                        || (slot == TextureSlot::MetallicRoughness && channel == 0)
                    {
                        continue;
                    }
                    // The production gate, applied to the independent truth.
                    let stats = super::delivery::channel_stats(
                        &expected.pixels,
                        &actual,
                        &vec![1; expected.w * expected.h],
                        expected.w,
                        channel,
                    );
                    let gate = super::delivery::channel_gate(slot, profile, channel, stats.texels);
                    assert!(
                        gate.accepts(&stats),
                        "{slot:?}/{profile:?}/mip{mip}/channel{channel}: {stats:?} {gate:?}"
                    );
                }
                if class == Class::Normal {
                    let mut lengths = Vec::new();
                    let mut angles = Vec::new();
                    for (a, b) in actual
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(expected.pixels.as_chunks::<4>().0)
                    {
                        let raw = vector(a);
                        lengths.push((length(raw) - 1.).abs());
                        let (a, b) = (direction(raw), direction(vector(b)));
                        angles.push(
                            a.iter()
                                .zip(b)
                                .map(|(x, y)| x * y)
                                .sum::<f64>()
                                .clamp(-1., 1.)
                                .acos()
                                .to_degrees(),
                        );
                    }
                    angles.sort_by(f64::total_cmp);
                    let mean = lengths.iter().sum::<f64>() / lengths.len() as f64;
                    let max = lengths.iter().copied().fold(0., f64::max);
                    let (mean_cap, max_cap) = match profile {
                        Profile::Fast => (0.08, 0.20),
                        Profile::Balanced => (0.05, 0.15),
                        _ => (0.03, 0.10),
                    };
                    assert!(mean <= mean_cap && max <= max_cap);
                    assert!(angles.iter().sum::<f64>() / angles.len() as f64 <= 3.);
                    assert!(angles[(angles.len() * 99).div_ceil(100) - 1] <= 12.);
                }
            }
        }
    }
}

#[test]
fn independent_data_filter_control_rejects_colour_transfer() {
    // The large MR witness has bounded low variance to meet the unchanged
    // codec floors. This separate full-range control pins linear data math;
    // interpreting its meaningful G/B channels as colour must fail.
    let source = vec![
        0, 0, 255, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 255, 0, 255,
    ];
    let data = bounded_pot_reference(2, source.clone(), Class::Data);
    let wrong = bounded_pot_reference(2, source.clone(), Class::Colour);
    // Sparse pages require >=4 on each axis. Repeat the same 2x2 cell
    // into a 4x4 page; every first-mip footprint has the identical truth.
    let page: Vec<u8> = (0..16)
        .flat_map(|i| {
            let offset = ((i / 4 % 2) * 2 + i % 2) * 4;
            source[offset..offset + 4].iter().copied()
        })
        .collect();
    let production =
        super::sparse_mips::build(4, 4, &page, &[1; 16], Class::Data, 1024 * 1024).unwrap();
    for pixel in production.levels[1].rgba.as_chunks::<4>().0 {
        assert_eq!(&pixel[1..3], &[128, 128]);
        assert_eq!(pixel.as_slice(), data[1].pixels.as_slice());
    }
    assert_eq!(&data[1].pixels[1..3], &[128, 128]);
    assert_eq!(&wrong[1].pixels[1..3], &[188, 188]);
    for channel in [1, 2] {
        let (psnr, max) = probe_channel(&data[1], &wrong[1].pixels, channel);
        assert!(psnr < 30. && max > 32);
        // The production data gate refuses the colour-transfer mutant.
        let stats =
            super::delivery::channel_stats(&data[1].pixels, &wrong[1].pixels, &[1], 1, channel);
        let gate = super::delivery::channel_gate(
            crate::gltf_source::source::TextureSlot::MetallicRoughness,
            crate::mesh_large::config::Profile::Fast,
            channel,
            stats.texels,
        );
        assert!(stats.psnr < gate.psnr);
    }
}
