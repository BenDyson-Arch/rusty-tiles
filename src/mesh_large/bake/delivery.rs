//! Proxy delivery reuses the H8 codec/floor owner without copying its policy.
use super::{
    invalid, mips, resource,
    types::{EncodedPage, SignalPage},
    unsupported,
};
use crate::{
    gltf_source::source::TextureSlot,
    mesh_large::{
        config::Profile,
        leaves::{delivery, mask},
    },
    JobError,
};
pub(super) fn encode(
    page: &mut SignalPage,
    samples: &[mask::SurfaceSample],
    material: &serde_json::Value,
    profile: Profile,
    memory: u64,
    work_limit: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<EncodedPage, JobError> {
    check()?;
    let mut texels = 0u64;
    let (mut w, mut h) = (page.width, page.height);
    loop {
        texels = texels
            .checked_add(u64::from(w) * u64::from(h))
            .ok_or_else(|| resource("proxy delivery chain size"))?;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    // Extra proxy frame/terminal ownership is outside H8's pixel/native lease.
    // Reserve the complete extra chain before any of its allocations occur.
    let extra = texels
        .checked_mul(
            (std::mem::size_of::<super::frames::Frame>()
                + std::mem::size_of::<super::types::TerminalSample>()
                + 32) as u64,
        )
        .and_then(|n| n.checked_add((page.normal_samples.len() as u64).checked_mul(512)?))
        .and_then(|n| n.checked_add(512 * 1024))
        .ok_or_else(|| resource("proxy delivery ledger lease"))?;
    let codec_memory = memory
        .checked_sub(extra)
        .ok_or_else(|| resource("proxy delivery complete ledger lease"))?;
    if delivery::workspace_charge([page.width, page.height], profile)? > codec_memory {
        return Err(resource("proxy delivery codec plus semantic ledger peak"));
    }
    let chain = mips::build(page, memory, work_limit, check)?;
    let count = if profile == Profile::Lossless {
        1
    } else {
        chain.levels.len()
    };
    let authored: Vec<&[u8]> = chain
        .levels
        .iter()
        .take(count)
        .map(|l| l.rgba.as_slice())
        .collect();
    page.receipt["authoredRendererFrameFloors"] =
        mips::normal_floor(page, &authored, work_limit, check)?;
    let context =
        (page.slot == TextureSlot::BaseColor && material["alphaMode"] == "MASK").then(|| {
            delivery::MaskDelivery {
                samples,
                factor: material
                    .pointer("/pbrMetallicRoughness/baseColorFactor/3")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(1.),
                cutoff: material["alphaCutoff"].as_f64().unwrap_or(0.5),
                linear: page.sampler["magFilter"] != 9728,
                min_linear: matches!(page.sampler["minFilter"].as_u64(), Some(9729 | 9985 | 9987)),
            }
        });
    let output = if profile == Profile::Lossless {
        // Proxy PNG is an appearance approximation. Independent base PNG
        // readback remains exact to this authored base; viewer mips are outside
        // the guarantee. Never call encode_authored(Lossless).
        let mut rgba = chain.levels[0].rgba.clone();
        let mut coverage = serde_json::Value::Null;
        if let Some(c) = context {
            let optimum = mask::adjust(
                &mut rgba,
                page.width,
                page.height,
                c.samples,
                c.factor,
                c.cutoff,
                c.linear,
            )?;
            let actual = mask::coverage(
                &rgba,
                page.width,
                page.height,
                c.samples,
                c.factor,
                c.cutoff,
                c.linear,
            )?;
            if (actual - optimum.target).abs() > optimum.optimum_gap + 0.005 {
                return Err(unsupported("proxy PNG MASK optimum/floor"));
            }
            coverage = serde_json::json!({"target":optimum.target,"optimumGap":optimum.optimum_gap,"actual":actual,"scale":optimum.scale});
        }
        // The gate ladder polls cancellation through a shared `Fn`; route it to `check`.
        let cell = std::cell::RefCell::new(&mut *check);
        let poll = || (cell.borrow_mut())();
        let mut output = delivery::encode(
            [page.width, page.height],
            &rgba,
            &page.support,
            page.slot,
            profile,
            codec_memory,
            None,
            delivery::Ladder {
                // A non-mipmapped minification filter reads level 0 only.
                level0_sampled: matches!(page.sampler["minFilter"].as_u64(), Some(9728 | 9729)),
                check: &poll,
            },
        )?;
        let decoded = image::load_from_memory_with_format(&output.bytes, image::ImageFormat::Png)
            .map_err(|e| invalid(format!("proxy PNG independent readback {e}")))?
            .into_rgba8()
            .into_raw();
        if decoded != rgba {
            return Err(invalid("proxy PNG authored base changed"));
        }
        output.receipts.push(mips::normal_floor(
            page,
            &[decoded.as_slice()],
            work_limit,
            check,
        )?);
        output.receipts.push(serde_json::json!({"proxyApproximation":true,"decodedBaseExact":true,"pngProtectedGutterTexels":4,"viewerGeneratedMips":"finite padding cannot isolate terminal1x1","mask":coverage}));
        output
    } else {
        let level0_sampled = matches!(page.sampler["minFilter"].as_u64(), Some(9728 | 9729));
        let cell = std::cell::RefCell::new(&mut *check);
        let poll = || (cell.borrow_mut())();
        delivery::encode_authored(
            &chain,
            page.slot,
            profile,
            codec_memory,
            context,
            delivery::Ladder {
                level0_sampled,
                check: &poll,
            },
            |decoded: &[crate::mesh_large::leaves::mips::Level]| {
                poll()?;
                if decoded.len() != page.levels.len() {
                    return Err(invalid("proxy missing decoded mip"));
                }
                // The mandatory retained H8 floors already cover every meaningful
                // channel and normal length/angle on every emitted level. Preserve
                // their receipts; no caller regenerates mips or exempts 1x1.
                let slices: Vec<&[u8]> = decoded.iter().map(|l| l.rgba.as_slice()).collect();
                let normal = mips::normal_floor(page, &slices, work_limit, &mut || poll())?;
                Ok(
                    serde_json::json!({"proxyDecodedCompleteChain":decoded.len(),"terminal1x1":decoded.last().map(|l|[l.width,l.height]),"sourceSamplerPreserved":page.sampler,"normalRendererFrames":normal}),
                )
            },
        )?
    };
    if output.bytes.len() as u64 > memory {
        return Err(resource("proxy encoded page output lease"));
    }
    Ok(EncodedPage {
        bytes: output.bytes,
        mime: output.mime,
        size: [page.width, page.height],
        slot: page.slot,
        material: page.material,
        sampler: page.sampler.clone(),
        receipt: serde_json::json!({"codec":output.receipts,"semantic":page.receipt,"slot":format!("{:?}",page.slot),"semanticLevels":page.levels.len(),"mipCount":output.mip_count}),
    })
}
