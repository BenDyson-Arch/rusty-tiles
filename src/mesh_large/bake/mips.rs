//! Proxy frame and terminal-ledger reduction. H8 owns colour/data/support and
//! gutters. One finite base renderer-frame ledger is streamed at every level.
use super::{
    frames::Frame,
    invalid, math, resource,
    types::{NormalSample, SignalLevel, SignalPage},
    unsupported,
};
use crate::{
    gltf_source::source::TextureSlot,
    mesh_large::leaves::{mips::Class, sparse_mips},
    JobError,
};
use std::collections::BTreeMap;
fn quant(v: f64) -> u8 {
    ((v * 0.5 + 0.5) * 255.).clamp(0., 255.).round() as u8
}
/// Aggregate CHART support, rather than selecting the largest contributor texel.
/// Stable first sample supplies that chart's representative decoded frame.
pub(super) fn dominant_frames(
    samples: &[NormalSample],
    width: u32,
    height: u32,
    memory: u64,
    work: &mut u64,
    limit: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<(Vec<Option<Frame>>, u64), JobError> {
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| resource("normal chart tally dimensions"))?;
    let charge = (count as u64)
        .checked_mul(96)
        .and_then(|n| n.checked_add((samples.len() as u64).checked_mul(256)?))
        .ok_or_else(|| resource("normal chart tally charge"))?;
    if charge > memory {
        return Err(resource("normal chart tally lease"));
    }
    let mut tally: BTreeMap<(usize, usize), (f64, Frame)> = BTreeMap::new();
    for s in samples {
        check()?;
        *work = work
            .checked_add(1)
            .ok_or_else(|| resource("normal witness work"))?;
        if *work > limit {
            return Err(resource("normal witness work budget"));
        }
        if !s.weight.is_finite() || s.weight <= 0. || s.uv.iter().any(|v| !v.is_finite()) {
            return Err(invalid("normal chart base witness"));
        }
        let x = (s.uv[0] * f64::from(width))
            .floor()
            .clamp(0., f64::from(width - 1)) as usize;
        let y = (s.uv[1] * f64::from(height))
            .floor()
            .clamp(0., f64::from(height - 1)) as usize;
        tally
            .entry((y * width as usize + x, s.chart))
            .or_insert((0., s.frame))
            .0 += s.weight;
    }
    let mut frames = vec![None; count];
    let mut best = vec![0.; count];
    let mut groups = vec![0u32; count];
    for ((i, _), (weight, frame)) in tally {
        groups[i] += 1;
        if weight > best[i] {
            best[i] = weight;
            frames[i] = Some(frame);
        }
    }
    Ok((frames, groups.into_iter().filter(|n| *n > 1).count() as u64))
}
pub(super) fn build(
    page: &mut SignalPage,
    memory: u64,
    work_limit: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<sparse_mips::Chain, JobError> {
    let class = match page.slot {
        TextureSlot::BaseColor | TextureSlot::Emissive => Class::Colour,
        TextureSlot::Normal => Class::Normal,
        _ => Class::Data,
    };
    if page.levels.len() != 1 {
        return Err(invalid("proxy semantic base chain"));
    }
    let mut work = 0;
    let mut mixed = 0;
    if class == Class::Normal {
        let (frames, count) = dominant_frames(
            &page.normal_samples,
            page.width,
            page.height,
            memory,
            &mut work,
            work_limit,
            check,
        )?;
        mixed += count;
        if page.levels[0].world_normals.len() != page.support.len() {
            return Err(invalid("normal base world reference shape"));
        }
        for (i, frame) in frames.into_iter().enumerate() {
            if page.support[i] == 0 {
                continue;
            }
            let frame =
                frame.ok_or_else(|| invalid("occupied normal texel lacks base frame witness"))?;
            page.levels[0].frames[i] = frame;
            let v = frame.inverse_direction(page.levels[0].world_normals[i])?;
            for a in 0..3 {
                page.rgba[i * 4 + a] = quant(v[a]);
            }
        }
    }
    let mut chain = sparse_mips::build(
        page.width,
        page.height,
        &page.rgba,
        &page.support,
        class,
        memory,
    )?;
    let owners = sparse_mips::refill_gutters(&mut chain.levels[0], class, memory)?;
    for (i, &owner) in owners.iter().enumerate() {
        if page.levels[0].support[i] == 0 && owner != usize::MAX {
            page.levels[0].frames[i] = page.levels[0].frames[owner];
            page.levels[0].samples[i] = page.levels[0].samples[owner];
            page.levels[0].samples[i].weight = 0;
            if class == Class::Normal {
                page.levels[0].world_normals[i] = page.levels[0].world_normals[owner];
            }
        }
    }
    page.levels[0].rgba = chain.levels[0].rgba.clone();
    page.rgba = chain.levels[0].rgba.clone();
    page.samples = page.levels[0].samples.clone();
    let mut cancellations = 0u64;
    for level in chain.levels.iter_mut().skip(1) {
        check()?;
        let selected = if class == Class::Normal {
            let (frames, count) = dominant_frames(
                &page.normal_samples,
                level.width,
                level.height,
                memory,
                &mut work,
                work_limit,
                check,
            )?;
            mixed += count;
            frames
        } else {
            Vec::new()
        };
        let src = page.levels.last().unwrap();
        let count = level.support.len();
        if (count as u64).checked_mul(256).is_none_or(|n| n > memory) {
            return Err(resource("proxy mip ledgers"));
        }
        let mut frames = Vec::with_capacity(count);
        let mut samples = Vec::with_capacity(count);
        let mut world_normals = if class == Class::Normal {
            vec![[0., 0., 1.]; count]
        } else {
            Vec::new()
        };
        for y in 0..level.height as usize {
            for x in 0..level.width as usize {
                check()?;
                let i = y * level.width as usize + x;
                let mut sum = [0.; 3];
                let mut centre = [0.; 3];
                let mut lo = [f64::INFINITY; 3];
                let mut hi = [f64::NEG_INFINITY; 3];
                let mut total = 0u64;
                let mut contributors = 0;
                let mut best = None;
                for sy in y * 2..(y * 2 + 2).min(src.height as usize) {
                    for sx in x * 2..(x * 2 + 2).min(src.width as usize) {
                        let j = sy * src.width as usize + sx;
                        let w = src.support[j];
                        if w == 0 {
                            continue;
                        }
                        contributors += 1;
                        let t = src.samples[j];
                        total += u64::from(w);
                        for a in 0..3 {
                            centre[a] += t.centre[a] * f64::from(w);
                            lo[a] = lo[a].min(t.support_min[a]);
                            hi[a] = hi[a].max(t.support_max[a]);
                        }
                        let world = if class == Class::Normal {
                            src.world_normals[j]
                        } else {
                            [0.; 3]
                        };
                        for a in 0..3 {
                            sum[a] += world[a] * f64::from(w);
                        }
                        if best.is_none_or(|(old, _, _)| w > old) {
                            best = Some((w, j, world));
                        }
                    }
                }
                if total == 0 {
                    frames.push(src.frames[0]);
                    samples.push(src.samples[0]);
                    continue;
                }
                if total != u64::from(level.support[i]) {
                    return Err(invalid("proxy/H8 mip support disagreement"));
                }
                let (_, j, dominant) = best.unwrap();
                let frame = if class == Class::Normal {
                    selected[i].ok_or_else(|| invalid("normal mip has no contributing chart"))?
                } else {
                    src.frames[j]
                };
                let mut t = src.samples[j];
                t.centre = centre.map(|v| v / total as f64);
                t.support_min = lo;
                t.support_max = hi;
                t.weight = level.support[i];
                if contributors > 1 {
                    t.centre_is_centroid = true;
                }
                if class == Class::Normal {
                    let world = math::unit(sum).unwrap_or_else(|| {
                        cancellations += 1;
                        dominant
                    });
                    world_normals[i] = world;
                    let v = frame.inverse_direction(world)?;
                    for a in 0..3 {
                        level.rgba[i * 4 + a] = quant(v[a]);
                    }
                }
                frames.push(frame);
                samples.push(t);
            }
        }
        let owners = sparse_mips::refill_gutters(level, class, memory)?;
        for (i, &owner) in owners.iter().enumerate() {
            if level.support[i] == 0 && owner != usize::MAX {
                frames[i] = frames[owner];
                samples[i] = samples[owner];
                samples[i].weight = 0;
                if class == Class::Normal {
                    world_normals[i] = world_normals[owner];
                }
            }
        }
        page.levels.push(SignalLevel {
            width: level.width,
            height: level.height,
            rgba: level.rgba.clone(),
            support: level.support.clone(),
            frames,
            samples,
            world_normals,
        });
    }
    page.receipt["normalCancellationFallbacks"] = serde_json::json!(cancellations);
    page.receipt["mixedChartTexels"] = serde_json::json!(mixed);
    page.receipt["normalChartWitnessWork"] = serde_json::json!(work);
    page.receipt["semanticLevels"] = serde_json::json!(page.levels.len());
    page.receipt["terminal1x1"] =
        serde_json::json!(page.levels.last().map(|l| [l.width, l.height]));
    Ok(chain)
}
fn taps(width: u32, height: u32, uv: [f64; 2], linear: bool) -> [(usize, f64); 4] {
    let xy = [
        uv[0] * f64::from(width) - 0.5,
        uv[1] * f64::from(height) - 0.5,
    ];
    let index = |x: i64, y: i64| {
        y.clamp(0, i64::from(height) - 1) as usize * width as usize
            + x.clamp(0, i64::from(width) - 1) as usize
    };
    if !linear {
        return [
            (
                index((xy[0] + 0.5).floor() as i64, (xy[1] + 0.5).floor() as i64),
                1.,
            ),
            (0, 0.),
            (0, 0.),
            (0, 0.),
        ];
    }
    let x = xy[0].floor() as i64;
    let y = xy[1].floor() as i64;
    let a = xy[0] - x as f64;
    let b = xy[1] - y as f64;
    [
        (index(x, y), (1. - a) * (1. - b)),
        (index(x + 1, y), a * (1. - b)),
        (index(x, y + 1), (1. - a) * b),
        (index(x + 1, y + 1), a * b),
    ]
}
/// Finite rendering oracle: evaluate each actual emitted fragment frame, rather
/// than substituting the representative frame chosen to store a coarse texel.
/// The 0.01-degree histogram rounds p99 UP, with explicit finite-work bounds.
pub(super) fn normal_floor(
    page: &SignalPage,
    rgba: &[&[u8]],
    work_limit: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<serde_json::Value, JobError> {
    if page.slot != TextureSlot::Normal {
        return Ok(serde_json::Value::Null);
    }
    if rgba.is_empty() || rgba.len() > page.levels.len() {
        return Err(invalid("normal renderer floor missing level"));
    }
    let mut receipts = Vec::new();
    let mut work = 0u64;
    for (mip, (level, pixels)) in page.levels.iter().zip(rgba).enumerate() {
        if pixels.len() != level.rgba.len() || level.world_normals.len() != level.support.len() {
            return Err(invalid("normal renderer floor shape"));
        }
        let linear = if mip == 0 {
            page.sampler["magFilter"] != 9728
        } else {
            matches!(page.sampler["minFilter"].as_u64(), Some(9729 | 9985 | 9987))
        };
        let mut histogram = vec![0f64; 18001];
        let mut bleed_histogram = vec![0f64; 18001];
        let mut bleed_sum = 0.;
        let mut bleed_total = 0.;
        let mut bleed_max = 0f64;
        let mut cancelled_world_weight = 0.;
        let mut sum = 0.;
        let mut total = 0.;
        let mut maximum = 0f64;
        let mut affected = 0.;
        for sample in &page.normal_samples {
            check()?;
            work = work
                .checked_add(1)
                .ok_or_else(|| resource("normal semantic floor work"))?;
            if work > work_limit {
                return Err(resource("normal semantic floor work budget"));
            }
            let mut raw = [0.; 3];
            let mut reference = [0.; 3];
            let mut common_world = [0.; 3];
            for (i, w) in taps(level.width, level.height, sample.uv, linear) {
                if w == 0. {
                    continue;
                }
                let component = level.frames[i].inverse_direction(level.world_normals[i])?;
                for a in 0..3 {
                    raw[a] += (f64::from(pixels[i * 4 + a]) / 127.5 - 1.) * w;
                    reference[a] += component[a] * w;
                    common_world[a] += level.world_normals[i][a] * w;
                }
            }
            let reference = sample.frame.transform(reference)?;
            if let Some(world) = math::unit(common_world) {
                let bleed = math::dot(reference, world)
                    .clamp(-1., 1.)
                    .acos()
                    .to_degrees()
                    .next_up();
                bleed_sum += bleed * sample.weight;
                bleed_total += sample.weight;
                bleed_max = bleed_max.max(bleed);
                bleed_histogram[(bleed * 100.).ceil().min(18000.) as usize] += sample.weight;
            } else {
                cancelled_world_weight += sample.weight;
            }
            let actual = sample.frame.transform(raw)?;
            let angle = math::dot(actual, reference)
                .clamp(-1., 1.)
                .acos()
                .to_degrees()
                .next_up();
            let bin = (angle * 100.).ceil().min(18000.) as usize;
            histogram[bin] += sample.weight;
            total += sample.weight;
            sum += angle * sample.weight;
            maximum = maximum.max(angle);
            if angle > 0.01 {
                affected += sample.weight;
            }
        }
        if total <= 0. || !total.is_finite() {
            return Err(invalid("normal semantic floor has no finite coverage"));
        }
        let mut cumulative = 0.;
        let mut p99 = 0.;
        for (i, w) in histogram.into_iter().enumerate() {
            cumulative += w;
            if cumulative >= total * 0.99 {
                p99 = i as f64 / 100.;
                break;
            }
        }
        let mean = (sum / total).next_up();
        let mut cumulative = 0.;
        let mut bleed_p99 = 0.;
        for (i, w) in bleed_histogram.into_iter().enumerate() {
            cumulative += w;
            if cumulative >= bleed_total * 0.99 {
                bleed_p99 = i as f64 / 100.;
                break;
            }
        }
        receipts.push(serde_json::json!({"mip":mip,"meanAngle":mean,"p99AngleUpper":p99,"maxAngle":maximum,"affectedWeight":affected,"totalWeight":total,"histogramDegrees":0.01,
            "committedMixtureReference":true,"worldMixtureBleed":{"mean":if bleed_total>0.{Some(bleed_sum/bleed_total)}else{None},"p99Upper":bleed_p99,"max":bleed_max,"weight":bleed_total,"cancelledWorldWeight":cancelled_world_weight,"disposition":"measured permitted subtexel mixture; no floor waiver"}}));
        if mean > 3. || p99 > 12. {
            return Err(unsupported(format!(
                "normal renderer-frame mip{mip} semantic mean{mean} p99{p99} exceeds3/12 degrees"
            )));
        }
    }
    Ok(
        serde_json::json!({"rendererFrameFloors":receipts,"finiteBaseWitnesses":page.normal_samples.len(),"work":work}),
    )
}
