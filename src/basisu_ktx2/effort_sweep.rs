//! Ignored, release-mode measurement of exact captured tex3 caller chains.
//! Raw encoding is measured separately; mandatory verification is recorded,
//! then every mip is independently decoded even if a codec floor refused it.
use super::super::*;
use rayon::prelude::*;
use serde_json::{json, Value};
use std::{fs, path::Path};

fn cpu() -> f64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: writable timespec, current-thread CPU clock has no external state.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) },
        0
    );
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

fn psnr(sse: f64, weight: f64) -> Option<f64> {
    (sse > 0.).then(|| 10. * (65025. * weight / sse).log10())
}

fn page(path: &Path, mode: Mode, params: Params) -> Value {
    let metadata: Value =
        serde_json::from_slice(&fs::read(path.join("page.json")).unwrap()).unwrap();
    let dims = metadata["levels"].as_array().unwrap();
    let pixels: Vec<_> = (0..dims.len())
        .map(|i| fs::read(path.join(format!("{i}.rgba"))).unwrap())
        .collect();
    let levels: Vec<_> = dims
        .iter()
        .zip(&pixels)
        .map(|(d, p)| MipLevel {
            width: d["width"].as_u64().unwrap() as u32,
            height: d["height"].as_u64().unwrap() as u32,
            rgba: p,
        })
        .collect();
    check_mips(&levels, mode, params).unwrap();
    let start = cpu();
    let encoded = encode_mips_unverified(&levels, mode, params).unwrap();
    let seconds = cpu() - start;
    let verification = verify_ktx2_mips(&encoded, &levels, mode, params);
    let mut measurements = Vec::new();
    let (mut all_sse, mut all_weight, mut base_sse, mut base_weight) = (0., 0., 0., 0.);
    for (mip, level) in levels.iter().enumerate() {
        let actual = decode_level_rgba(&encoded, mip as u32, level.width, level.height).unwrap();
        let expected = fs::read(path.join(format!("{mip}.expected"))).unwrap();
        let support: Vec<u32> = fs::read(path.join(format!("{mip}.support")))
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| u32::from_le_bytes(*v))
            .collect();
        assert_eq!(expected.len(), actual.len());
        assert_eq!(support.len() * 4, actual.len());
        let texels = support.iter().filter(|&&v| v > 0).count();
        for channel in 0..4 {
            let (mut sse, mut weight, mut maximum, mut histogram) = (0., 0., 0u8, [0f64; 256]);
            let mut blocks = vec![
                (0., 0.);
                (level.width as usize).div_ceil(4)
                    * (level.height as usize).div_ceil(4)
            ];
            for (i, &w) in support.iter().enumerate().filter(|(_, w)| **w > 0) {
                let error = expected[i * 4 + channel].abs_diff(actual[i * 4 + channel]);
                let squared = f64::from(error).powi(2) * f64::from(w);
                sse += squared;
                weight += f64::from(w);
                maximum = maximum.max(error);
                histogram[usize::from(error)] += f64::from(w);
                let block = (i / level.width as usize / 4) * (level.width as usize).div_ceil(4)
                    + (i % level.width as usize / 4);
                blocks[block].0 += squared;
                blocks[block].1 += f64::from(w);
            }
            let mut cumulative = 0.;
            let p999 = histogram
                .iter()
                .position(|w| {
                    cumulative += w;
                    cumulative >= weight * 0.999
                })
                .unwrap();
            let block_rmse = blocks
                .iter()
                .filter(|(_, w)| *w > 0.)
                .map(|(s, w)| (s / w).sqrt())
                .fold(0., f64::max);
            if channel < 3 {
                all_sse += sse;
                all_weight += weight;
                if mip == 0 {
                    base_sse += sse;
                    base_weight += weight;
                }
            }
            measurements.push(json!({"mip":mip,"channel":channel,"texels":texels,"area":u64::from(level.width)*u64::from(level.height),"sse":sse,"weight":weight,"psnr":psnr(sse,weight),"max":maximum,"p999":p999,"blockRmse":block_rmse}));
        }
    }
    json!({"page":path.file_name().unwrap().to_str().unwrap(),"bytes":encoded.len(),"cpu_seconds":seconds,"base_sse":base_sse,"base_weight":base_weight,"all_sse":all_sse,"all_weight":all_weight,"codec_pass":verification.is_ok(),"codec_error":verification.err().map(|e|e.message().to_owned()),"levels":measurements})
}

#[test]
#[ignore = "release measurement: RT_SWEEP_PAGES/OUT/CODEC/QUALITY/EFFORT"]
fn measure_tex3_efforts() {
    let root = std::env::var_os("RT_SWEEP_PAGES").expect("RT_SWEEP_PAGES");
    let quality: u8 = std::env::var("RT_SWEEP_QUALITY").unwrap().parse().unwrap();
    let effort: u8 = std::env::var("RT_SWEEP_EFFORT").unwrap().parse().unwrap();
    let mode = if std::env::var("RT_SWEEP_CODEC").unwrap() == "etc1s" {
        Mode::Etc1s { quality }
    } else {
        Mode::Uastc { zstd: true }
    };
    let params = Params {
        effort,
        transfer: Transfer::Srgb,
        mipmaps: false,
    };
    let mut paths: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("page.json").exists())
        .collect();
    paths.sort();
    assert!(!paths.is_empty());
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    let rows: Vec<Value> =
        pool.install(|| paths.par_iter().map(|p| page(p, mode, params)).collect());
    let sum = |key: &str| rows.iter().map(|r| r[key].as_f64().unwrap()).sum::<f64>();
    let summary = json!({"mode":format!("{mode:?}"),"effort":effort,"pages":rows.len(),"bytes":rows.iter().map(|r|r["bytes"].as_u64().unwrap()).sum::<u64>(),"cpu_seconds":sum("cpu_seconds"),"base_psnr_rgb":psnr(sum("base_sse"),sum("base_weight")),"all_mip_psnr_rgb":psnr(sum("all_sse"),sum("all_weight")),"codec_refusals":rows.iter().filter(|r|r["codec_pass"]==false).count()});
    fs::write(
        std::env::var_os("RT_SWEEP_OUT").unwrap(),
        serde_json::to_vec(&json!({"summary":summary,"pages":rows})).unwrap(),
    )
    .unwrap();
    println!("etc1s-sweep {summary}");
}
