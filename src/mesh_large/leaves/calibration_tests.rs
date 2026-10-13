//! Real-data calibration of the decoded-image gates and encoder settings.
//! Ignored: needs `RT_CALIBRATION_PAGES`, a directory of raw page dumps
//! (`<w>x<h>-<slot>-<id>.rgba` RGBA8 plus `.support` u8 masks), and writes one
//! JSON line per (page, slot class, profile, effort, level, channel) to
//! `RT_CALIBRATION_OUT`. Normal and data witnesses are derived from the real
//! colour pages: data takes the bytes literally, normals take the luma
//! gradient as a height field. Sensitivity controls run on the same pages.
use super::{
    delivery::{channel_gate, channel_stats, check_level, codec_settings, scalar_occlusion},
    mips::Class,
    sparse_mips,
};
use crate::{basisu_ktx2 as codec, gltf_source::source::TextureSlot, mesh_large::config::Profile};
use serde_json::json;
use std::{io::Write, time::Instant};

struct Page {
    name: String,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    support: Vec<u32>,
}

fn pages() -> Vec<Page> {
    let dir = std::env::var_os("RT_CALIBRATION_PAGES").expect("RT_CALIBRATION_PAGES");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rgba"))
        .collect();
    names.sort();
    let limit: usize = std::env::var("RT_CALIBRATION_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    names
        .into_iter()
        .take(limit)
        .map(|path| {
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            let dims = stem.split('-').next().unwrap();
            let (w, h) = dims.split_once('x').unwrap();
            let support = std::fs::read(path.with_extension("support"))
                .unwrap()
                .into_iter()
                .map(u32::from)
                .collect();
            Page {
                name: stem.clone(),
                width: w.parse().unwrap(),
                height: h.parse().unwrap(),
                rgba: std::fs::read(&path).unwrap(),
                support,
            }
        })
        .collect()
}

/// Luma height field to unit tangent-space normals, strength 4.
fn normal_witness(page: &Page) -> Vec<u8> {
    let (w, h) = (page.width as usize, page.height as usize);
    let luma = |x: usize, y: usize| {
        let p = &page.rgba[(y.min(h - 1) * w + x.min(w - 1)) * 4..];
        (0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])) / 255.
    };
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let dx = (luma(x + 1, y) - luma(x.saturating_sub(1), y)) * 4.;
            let dy = (luma(x, y + 1) - luma(x, y.saturating_sub(1))) * 4.;
            let n = [-dx, -dy, 1.];
            let l = n.iter().map(|v| v * v).sum::<f64>().sqrt();
            for v in n {
                out.push(((v / l + 1.) * 127.5).round().clamp(0., 255.) as u8);
            }
            out.push(255);
        }
    }
    out
}

fn class(slot: TextureSlot) -> Class {
    match slot {
        TextureSlot::BaseColor | TextureSlot::Emissive => Class::Colour,
        TextureSlot::Normal => Class::Normal,
        _ => Class::Data,
    }
}

fn encode(
    chain: &sparse_mips::Chain,
    slot: TextureSlot,
    profile: Profile,
    effort: Option<u8>,
) -> (Vec<u8>, Vec<Vec<u8>>, f64) {
    let scalar = scalar_occlusion(slot, profile);
    let delivered: Vec<Vec<u8>> = chain
        .levels
        .iter()
        .map(|l| {
            let mut p = l.rgba.clone();
            if scalar {
                for px in p.as_chunks_mut::<4>().0 {
                    px[1] = px[0];
                    px[2] = px[0];
                }
            }
            p
        })
        .collect();
    let levels: Vec<_> = chain
        .levels
        .iter()
        .zip(&delivered)
        .map(|(l, p)| codec::MipLevel {
            rgba: p,
            width: l.width,
            height: l.height,
        })
        .collect();
    let (mode, mut params) = codec_settings(slot, profile);
    if let Some(effort) = effort {
        params.effort = effort;
    }
    let start = Instant::now();
    let (bytes, decoded) = codec::encode_ktx2_mips_decoded(&levels, mode, params).unwrap();
    (bytes, decoded, start.elapsed().as_secs_f64())
}

fn channels(slot: TextureSlot) -> &'static [usize] {
    match slot {
        TextureSlot::Occlusion => &[0, 3],
        TextureSlot::MetallicRoughness => &[1, 2, 3],
        _ => &[0, 1, 2, 3],
    }
}

/// Per-level verdict of the production gate (`check_level`).
fn verdicts(
    chain: &sparse_mips::Chain,
    decoded: &[Vec<u8>],
    slot: TextureSlot,
    profile: Profile,
) -> Vec<bool> {
    chain
        .levels
        .iter()
        .zip(decoded)
        .enumerate()
        .map(|(mip, (level, decoded))| {
            check_level(
                &level.rgba,
                decoded,
                &level.support,
                level.width,
                slot,
                profile,
                mip,
            )
            .is_ok()
        })
        .collect()
}

#[test]
#[ignore = "real-data calibration: needs RT_CALIBRATION_PAGES and RT_CALIBRATION_OUT"]
fn real_page_gate_and_encoder_calibration() {
    let out_path = std::env::var_os("RT_CALIBRATION_OUT").expect("RT_CALIBRATION_OUT");
    let mut out = std::io::BufWriter::new(std::fs::File::create(out_path).unwrap());
    let efforts: Vec<Option<u8>> = match std::env::var("RT_CALIBRATION_EFFORTS") {
        Ok(v) => v.split(',').map(|e| Some(e.parse().unwrap())).collect(),
        Err(_) => vec![None],
    };
    let slots: Vec<TextureSlot> = match std::env::var("RT_CALIBRATION_SLOTS") {
        Ok(v) if v == "colour" => vec![TextureSlot::BaseColor],
        _ => vec![
            TextureSlot::BaseColor,
            TextureSlot::Occlusion,
            TextureSlot::MetallicRoughness,
            TextureSlot::Normal,
        ],
    };
    let mut failures = 0usize;
    for page in pages() {
        for &slot in &slots {
            let pixels = if slot == TextureSlot::Normal {
                normal_witness(&page)
            } else {
                page.rgba.clone()
            };
            let chain = sparse_mips::build(
                page.width,
                page.height,
                &pixels,
                &page.support,
                class(slot),
                u64::MAX,
            )
            .unwrap();
            let chain_texels: u64 = chain
                .levels
                .iter()
                .map(|l| u64::from(l.width) * u64::from(l.height))
                .sum();
            for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
                for &effort in &efforts {
                    let (bytes, decoded, seconds) = encode(&chain, slot, profile, effort);
                    let pass = verdicts(&chain, &decoded, slot, profile);
                    for (mip, (level, decoded)) in chain.levels.iter().zip(&decoded).enumerate() {
                        for &channel in channels(slot) {
                            let stats = channel_stats(
                                &level.rgba,
                                decoded,
                                &level.support,
                                level.width as usize,
                                channel,
                            );
                            if stats.texels == 0 {
                                continue;
                            }
                            let gate = channel_gate(slot, profile, channel, stats.texels);
                            writeln!(out, "{}", json!({"kind":"level","page":page.name,"slot":format!("{slot:?}"),"profile":format!("{profile:?}"),"effort":effort.unwrap_or(codec_settings(slot, profile).1.effort),"mip":mip,"width":level.width,"height":level.height,"channel":channel,"psnr":if stats.psnr.is_finite(){stats.psnr}else{99.},"p999":stats.p999,"max":stats.max,"blockRmse":stats.block_rmse,"texels":stats.texels,"gate":{"psnr":gate.psnr,"p999":gate.p999,"blockRmse":gate.block_rmse,"max":gate.max},"levelPass":pass[mip]})).unwrap();
                        }
                    }
                    failures += pass.iter().filter(|p| !**p).count();
                    writeln!(out, "{}", json!({"kind":"encode","page":page.name,"slot":format!("{slot:?}"),"profile":format!("{profile:?}"),"effort":effort.unwrap_or(codec_settings(slot, profile).1.effort),"chainTexels":chain_texels,"seconds":seconds,"bytes":bytes.len(),"secondsPerMegapixel":seconds / (chain_texels as f64 / 1e6),"pass":pass.iter().all(|p| *p)})).unwrap();
                }
            }
        }
        // Sensitivity controls on this real page, balanced profile.
        controls(&page, &mut out);
        out.flush().unwrap();
    }
    println!("calibration level failures: {failures}");
}

fn first_failure(
    chain: &sparse_mips::Chain,
    decoded: &[Vec<u8>],
    slot: TextureSlot,
) -> Option<usize> {
    verdicts(chain, decoded, slot, Profile::Balanced)
        .iter()
        .position(|p| !p)
}

fn controls(page: &Page, out: &mut impl Write) {
    let slot = TextureSlot::BaseColor;
    let chain = sparse_mips::build(
        page.width,
        page.height,
        &page.rgba,
        &page.support,
        Class::Colour,
        u64::MAX,
    )
    .unwrap();
    let (bytes, decoded, _) = encode(&chain, slot, Profile::Balanced, None);
    let mut record = |name: &str, refused: Option<usize>| {
        writeln!(
            out,
            "{}",
            json!({"kind":"control","page":page.name,"control":name,"refusedAtMip":refused})
        )
        .unwrap();
    };
    // 1. One corrupted 4x4 block of the base level (codec payload damage that
    // survives container parsing), placed on the first fully supported block.
    let w = page.width as usize;
    let block = (0..page.height as usize / 4)
        .flat_map(|by| (0..w / 4).map(move |bx| (bx, by)))
        .find(|&(bx, by)| {
            (0..4).all(|dy| (0..4).all(|dx| page.support[(by * 4 + dy) * w + bx * 4 + dx] > 0))
        })
        .unwrap();
    let mut corrupt = decoded.clone();
    for dy in 0..4 {
        for dx in 0..4 {
            let i = ((block.1 * 4 + dy) * w + block.0 * 4 + dx) * 4;
            for c in 0..3 {
                corrupt[0][i + c] = corrupt[0][i + c].wrapping_add(128);
            }
        }
    }
    record(
        "corrupted_block_base",
        first_failure(&chain, &corrupt, slot),
    );
    // 2. Corrupted payload bytes: flip bytes inside the last level's data
    // region of the container and decode what survives.
    let mut damaged = bytes.clone();
    let n = damaged.len();
    for i in (n * 3 / 4..n).step_by(97) {
        damaged[i] ^= 0x5a;
    }
    let levels: Result<Vec<_>, _> = chain
        .levels
        .iter()
        .enumerate()
        .map(|(i, l)| codec::decode_level_rgba(&damaged, i as u32, l.width, l.height))
        .collect();
    record(
        "corrupted_payload_bytes",
        match levels {
            Err(_) => Some(usize::MAX),
            Ok(levels) => first_failure(&chain, &levels, slot),
        },
    );
    // 3. Wrong colour space: levels authored with linear (data) filtering of
    // sRGB bytes, checked against the correct linear-light chain.
    let wrong = sparse_mips::build(
        page.width,
        page.height,
        &page.rgba,
        &page.support,
        Class::Data,
        u64::MAX,
    )
    .unwrap();
    let (_, wrong_decoded, _) = encode(&wrong, slot, Profile::Balanced, None);
    record(
        "wrong_colour_space_chain",
        first_failure(&chain, &wrong_decoded, slot),
    );
    // 4. Swapped red/blue channels.
    let mut swapped = decoded.clone();
    for level in &mut swapped {
        for px in level.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
    }
    record("swapped_red_blue", first_failure(&chain, &swapped, slot));
    // 5. Mip levels shifted by one (each level replaced by the next coarser,
    // nearest-upsampled), i.e. a wrong level order.
    let mut shifted = decoded.clone();
    for i in 0..chain.levels.len() - 1 {
        let (lw, lh) = (
            chain.levels[i].width as usize,
            chain.levels[i].height as usize,
        );
        let nw = chain.levels[i + 1].width as usize;
        let next = &decoded[i + 1];
        for y in 0..lh {
            for x in 0..lw {
                let s = ((y / 2).min(chain.levels[i + 1].height as usize - 1) * nw
                    + (x / 2).min(nw - 1))
                    * 4;
                shifted[i][(y * lw + x) * 4..(y * lw + x) * 4 + 4].copy_from_slice(&next[s..s + 4]);
            }
        }
    }
    record("shifted_mip_levels", first_failure(&chain, &shifted, slot));
    // 6. Normals: unnormalized RGB averaging (wrong normal handling) checked
    // against the renormalized class chain.
    let normals = normal_witness(page);
    let right = sparse_mips::build(
        page.width,
        page.height,
        &normals,
        &page.support,
        Class::Normal,
        u64::MAX,
    )
    .unwrap();
    let raw = sparse_mips::build(
        page.width,
        page.height,
        &normals,
        &page.support,
        Class::Data,
        u64::MAX,
    )
    .unwrap();
    let (_, raw_decoded, _) = encode(&raw, TextureSlot::Normal, Profile::Balanced, None);
    record(
        "unnormalized_normal_chain",
        verdicts(&right, &raw_decoded, TextureSlot::Normal, Profile::Balanced)
            .iter()
            .position(|p| !p),
    );
    // 7. Normal map decoded with the green channel flipped (handedness).
    let (_, normal_decoded, _) = encode(&right, TextureSlot::Normal, Profile::Balanced, None);
    let mut flipped = normal_decoded.clone();
    for level in &mut flipped {
        for px in level.as_chunks_mut::<4>().0 {
            px[1] = 255 - px[1];
        }
    }
    record(
        "flipped_normal_green",
        verdicts(&right, &flipped, TextureSlot::Normal, Profile::Balanced)
            .iter()
            .position(|p| !p),
    );
}
