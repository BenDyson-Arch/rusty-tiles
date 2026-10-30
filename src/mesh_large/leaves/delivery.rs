//! Explicit profile delivery. PNG is raw level0; KTX owns a complete class chain.
//! ID pages ([`Class::Id`]) are PNG-only in every profile: one exact PNG per
//! authored level ([`encode_id_chain`]); Basis ETC1S/UASTC are refused.
use super::{
    mips::{self, Class},
    sparse_mips,
};
use crate::stage_timing::{self as timing, Counter, Stage};
use crate::{
    basisu_ktx2 as codec, gltf_source::source::TextureSlot, mesh_large::config::Profile, JobError,
    JobErrorKind,
};
use image::ImageEncoder;
use serde_json::{json, Value};

#[derive(Debug)]
pub(in crate::mesh_large) struct Delivery {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub mip_count: u32,
    pub receipts: Vec<Value>,
    /// Set when the profile's own encoder setting missed a gate and a later
    /// step of the ladder delivered the page.
    pub escalation: Option<Escalation>,
}

/// One step of the per-page encoder ladder of a lossy profile. A page starts
/// at its profile's setting; a miss of any decoded-image gate (per-level
/// channel and normal floors, MASK coverage, source-sampler check) moves it
/// one step up, and every step is checked again by the same gates of the run
/// profile. Gates are never loosened. The ladder depends only on the page, so
/// output is the same for any worker count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    /// The profile's own codec setting.
    Profile,
    /// UASTC pack level 2 (Basis effort 5, the library default; RDO is never
    /// used). Above every profile's own level; scalar ETC1S occlusion moves to
    /// full UASTC, the Quality profile's mode for it.
    UastcLevel2,
    /// UASTC pack level 3 (effort 8). Level 4 (effort 10) was measured at
    /// about 16 times the cost of level 3 and 40 times level 2, for about
    /// 0.5 dB, so the ladder goes to exact PNG instead.
    UastcLevel3,
    /// Exact level-0 PNG, the last step of every page. A mip-sampled page keeps its
    /// sampler and the viewer generates its coarser levels, outside the decoded
    /// guarantee, as in the lossless profile.
    LosslessPng,
}
impl Step {
    pub(crate) const LOSSY: [Step; 3] = [Step::Profile, Step::UastcLevel2, Step::UastcLevel3];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Step::Profile => "profile",
            Step::UastcLevel2 => "uastc_level2",
            Step::UastcLevel3 => "uastc_level3",
            Step::LosslessPng => "lossless_png",
        }
    }
}
/// Why a ladder step missed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MissReason {
    /// A decoded-image gate of this module (channel, normal, MASK, source sampler).
    Gate,
    /// The codec's own mandatory verification: a decoded level missed its error
    /// or detail floor (`basisu_ktx2::FloorMiss`). Structural failures stay hard.
    CodecVerify,
}
impl MissReason {
    pub(crate) fn name(self) -> &'static str {
        match self {
            MissReason::Gate => "gate",
            MissReason::CodecVerify => "codec_verify",
        }
    }
}
/// One check a ladder step missed: the step, the mip level when the check is
/// per level (the source-sampler check spans all levels), why, and its text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GateMiss {
    pub step: Step,
    pub mip: Option<usize>,
    pub reason: MissReason,
    pub gate: String,
}
impl GateMiss {
    pub(crate) fn to_json(&self) -> Value {
        json!({"step":self.step.name(),"mip":self.mip,"reason":self.reason.name(),"gate":self.gate})
    }
}
/// The step that delivered a page and every miss before it, in ladder order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Escalation {
    pub step: Step,
    pub misses: Vec<GateMiss>,
}
/// Page-owned ladder facts, resolved by the atlas.
#[derive(Clone, Copy)]
pub(in crate::mesh_large) struct Ladder<'a> {
    /// Run cancellation, polled before every re-encode.
    pub check: &'a dyn Fn() -> Result<(), JobError>,
}
#[cfg(test)]
fn no_check() -> Result<(), JobError> {
    Ok(())
}
/// Tests: a page that is never cancelled.
#[cfg(test)]
pub(super) const LADDER: Ladder<'static> = Ladder { check: &no_check };
/// A typed gate refusal becomes a ladder miss; every other error stays hard.
fn missed(error: JobError, step: Step, mip: Option<usize>) -> Result<GateMiss, JobError> {
    if error.kind() == JobErrorKind::Unsupported {
        Ok(GateMiss {
            step,
            mip,
            reason: MissReason::Gate,
            gate: error.message().to_owned(),
        })
    } else {
        Err(error)
    }
}
#[derive(Clone, Copy)]
pub(in crate::mesh_large) struct MaskDelivery<'a> {
    pub samples: &'a [super::mask::SurfaceSample],
    pub factor: f64,
    pub cutoff: f64,
    pub linear: bool,
    /// Explicit source minification texel filter; base keeps magnification.
    pub min_linear: bool,
}
fn error(kind: JobErrorKind, message: impl Into<String>) -> JobError {
    JobError::new(kind, message.into())
}
fn resource() -> JobError {
    error(
        JobErrorKind::ResourceLimit,
        "texture delivery workspace/size budget",
    )
}
fn fidelity(message: impl Into<String>) -> JobError {
    error(JobErrorKind::Unsupported, message)
}
fn class(slot: TextureSlot) -> Class {
    match slot {
        TextureSlot::BaseColor | TextureSlot::Emissive => Class::Colour,
        TextureSlot::Normal => Class::Normal,
        _ => Class::Data,
    }
}
/// Support-weighted error statistics of one channel of one decoded level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::mesh_large) struct ChannelStats {
    /// Weighted PSNR in dB; infinite when exact.
    pub psnr: f64,
    /// Weighted 99.9th-percentile absolute error.
    pub p999: u8,
    pub max: u8,
    /// Largest RMSE of any aligned 4x4 codec block (supported texels only).
    pub block_rmse: f64,
    /// Texels with positive support.
    pub texels: u64,
}
pub(in crate::mesh_large) fn channel_stats(
    expected: &[u8],
    decoded: &[u8],
    support: &[u32],
    width: usize,
    channel: usize,
) -> ChannelStats {
    let mut histogram = [0f64; 256];
    let mut sse = 0.;
    let mut count = 0.;
    let mut max = 0;
    let mut texels = 0u64;
    let height = support.len() / width.max(1);
    let mut block_rmse = 0f64;
    for by in (0..height).step_by(4) {
        for bx in (0..width).step_by(4) {
            let (mut block_sse, mut block_weight) = (0., 0.);
            for y in by..(by + 4).min(height) {
                for x in bx..(bx + 4).min(width) {
                    let i = y * width + x;
                    let weight = support[i];
                    if weight == 0 {
                        continue;
                    }
                    let e = expected[i * 4 + channel].abs_diff(decoded[i * 4 + channel]);
                    let w = f64::from(weight);
                    let squared = f64::from(e).powi(2) * w;
                    histogram[usize::from(e)] += w;
                    sse += squared;
                    count += w;
                    block_sse += squared;
                    block_weight += w;
                    max = max.max(e);
                    texels += 1;
                }
            }
            if block_weight > 0. {
                block_rmse = block_rmse.max((block_sse / block_weight).sqrt());
            }
        }
    }
    let mut p999 = 0;
    let mut cumulative = 0.;
    for (e, w) in histogram.iter().enumerate() {
        cumulative += w;
        if cumulative >= count * 0.999 {
            p999 = e as u8;
            break;
        }
    }
    ChannelStats {
        psnr: if sse == 0. {
            f64::INFINITY
        } else {
            10. * (65025. * count / sse).log10()
        },
        p999,
        max,
        block_rmse,
        texels,
    }
}

/// Per-channel decoded-image floors of one slot class and profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::mesh_large) struct ChannelGate {
    pub psnr: f64,
    /// 99.9th-percentile cap (calibrated colour channels of large levels).
    pub p999: Option<u8>,
    /// Worst 4x4-block RMSE cap (colour channels).
    pub block_rmse: Option<f64>,
    /// Maximum absolute error cap (alpha, data and normal channels).
    pub max: Option<u8>,
}
impl ChannelGate {
    pub(in crate::mesh_large) fn accepts(&self, stats: &ChannelStats) -> bool {
        stats.psnr >= self.psnr
            && self.p999.is_none_or(|cap| stats.p999 <= cap)
            && self.block_rmse.is_none_or(|cap| stats.block_rmse <= cap)
            && self.max.is_none_or(|cap| stats.max <= cap)
    }
}
/// Below this many positive-support texels a 99.9th percentile is
/// the maximum of a few hundred samples, and mips this small are sampled only
/// at extreme minification. Small colour levels of packed island pages mix
/// neighbouring islands inside codec blocks; they retain the block corruption
/// guard and a size-banded PSNR floor without a percentile cap.
pub(in crate::mesh_large) const SMALL_LEVEL_TEXELS: u64 = 1024;
/// Colour reconstruction floor shared by decoded texels and source sampling.
/// ETC1S's shared palette is least accurate on tiny/coarse levels; these bands
/// are calibrated on photo, alpha, odd-sized, flat and high-contrast controls.
/// Delivery passes the positive-support count; source sampling passes selected
/// level area, since its finite sample set does not carry the support ledger.
pub(super) fn colour_psnr(profile: Profile, texels: u64) -> f64 {
    if profile == Profile::Quality {
        if texels < SMALL_LEVEL_TEXELS {
            23.
        } else {
            27.
        }
    } else if profile == Profile::Lossless {
        // PNG exactness is checked separately; preserve the historical helper
        // limits for callers that inspect a lossless receipt.
        if texels < SMALL_LEVEL_TEXELS {
            20.
        } else {
            24.
        }
    } else if texels < 64 {
        13.
    } else if texels < SMALL_LEVEL_TEXELS {
        16.
    } else {
        20.
    }
}

/// Tiny ETC1S levels need a joint RGB guard as well as a channel floor: a
/// shared palette can shift one channel more than all three together. Both
/// delivery (positive support) and decoded sampling (level area) use this.
pub(super) fn tiny_colour_rgb_floor(profile: Profile, texels: u64) -> Option<f64> {
    (matches!(profile, Profile::Fast | Profile::Balanced) && texels < 64).then_some(16.)
}

/// Colour channels: PSNR bounds mean quality, the 99.9th percentile bounds
/// the error tail without being decided by one outlier texel, and the worst
/// 4x4 codec-block RMSE catches one localized corrupted block that neither
/// can see on a large page. They are calibrated on every level of the real
/// original 10M-triangle mesh and the ETC1S fixture/tex3 campaign
/// (bench/architecture_audit/etc1s_default/README.md). Alpha, data and normal
/// channels have no real-data calibration yet and retain their previous
/// PSNR floors and maximum caps.
pub(in crate::mesh_large) fn channel_gate(
    slot: TextureSlot,
    profile: Profile,
    channel: usize,
    texels: u64,
) -> ChannelGate {
    let quality = profile == Profile::Quality;
    let fast = profile == Profile::Fast;
    let legacy = |psnr: f64, max: u8| ChannelGate {
        psnr,
        p999: None,
        block_rmse: None,
        max: Some(max),
    };
    if channel == 3 {
        return legacy(
            if quality { 34. } else { 30. },
            if fast {
                80
            } else if quality {
                32
            } else {
                64
            },
        );
    }
    match class(slot) {
        Class::Colour => {
            // Measured ETC1S photo families require separate palette/mip size
            // bands. Strong local caps still reject gross damage. Quality keeps
            // its UASTC floors. Derivation and controls: docs/MESH.md.
            let small = texels < SMALL_LEVEL_TEXELS;
            ChannelGate {
                psnr: colour_psnr(profile, texels),
                p999: (!small).then_some(if quality {
                    64
                } else if fast {
                    112
                } else {
                    80
                }),
                block_rmse: Some(if quality {
                    48.
                } else if fast {
                    96.
                } else {
                    80.
                }),
                max: None,
            }
        }
        Class::Normal => legacy(30., 112),
        Class::Data if slot == TextureSlot::Occlusion => legacy(30., if quality { 8 } else { 80 }),
        Class::Data => legacy(30., 32),
        // Defensive: IDs admit no error at all.
        Class::Id => ChannelGate {
            psnr: f64::INFINITY,
            p999: None,
            block_rmse: None,
            max: Some(0),
        },
    }
}
pub(in crate::mesh_large) fn check_level(
    expected: &[u8],
    decoded: &[u8],
    support: &[u32],
    width: u32,
    slot: TextureSlot,
    profile: Profile,
    mip: usize,
) -> Result<Value, JobError> {
    if expected.len() != decoded.len()
        || expected.len() != support.len() * 4
        || width == 0
        || !support.len().is_multiple_of(width as usize)
    {
        return Err(error(JobErrorKind::InvalidState, "delivery decoded shape"));
    }
    let mut metrics = Vec::new();
    let mut rgb_inverse_snr = 0.;
    let mut rgb_texels = 0;
    for &channel in gated_channels(slot) {
        let stats = channel_stats(expected, decoded, support, width as usize, channel);
        let gate = channel_gate(slot, profile, channel, stats.texels);
        if channel < 3 {
            rgb_inverse_snr += 10f64.powf(-stats.psnr / 10.);
            rgb_texels = stats.texels;
        }
        metrics.push(json!({"channel":channel,"psnr":if stats.psnr.is_finite(){Some(stats.psnr)}else{None},"exact":stats.psnr.is_infinite(),"p999":stats.p999,"max":stats.max,"blockRmse":stats.block_rmse,"texels":stats.texels,"floor":gate.psnr,"p999Cap":gate.p999,"blockRmseCap":gate.block_rmse,"maxCap":gate.max}));
        if !gate.accepts(&stats) {
            return Err(fidelity(format!(
                "texture {slot:?} {profile:?} mip{mip} channel{channel} PSNR{} p99.9 {} blockRMSE{} max{} over {} texels (gate {gate:?})",
                stats.psnr, stats.p999, stats.block_rmse, stats.max, stats.texels
            )));
        }
    }
    let rgb = if class(slot) == Class::Colour {
        tiny_colour_rgb_floor(profile, rgb_texels).map(|floor| {
            let psnr = -10. * (rgb_inverse_snr / 3.).log10();
            (psnr, floor)
        })
    } else {
        None
    };
    if let Some((psnr, floor)) = rgb {
        if psnr < floor {
            return Err(fidelity(format!(
                "texture {slot:?} {profile:?} mip{mip} combined RGB PSNR{psnr} below {floor} over {rgb_texels} texels"
            )));
        }
    }
    let mut normals = Value::Null;
    if slot == TextureSlot::Normal {
        let mut sum = 0.;
        let mut weight_sum = 0.;
        let mut max: f64 = 0.;
        let mut angle_sum = 0.;
        let mut angles = Vec::new();
        angles
            .try_reserve_exact(support.len())
            .map_err(|_| resource())?;
        for (i, &weight) in support.iter().enumerate() {
            if weight == 0 {
                continue;
            }
            let vector = |p: &[u8]| {
                [
                    f64::from(p[0]) / 127.5 - 1.,
                    f64::from(p[1]) / 127.5 - 1.,
                    f64::from(p[2]) / 127.5 - 1.,
                ]
            };
            let v = vector(&decoded[i * 4..]);
            let q = vector(&expected[i * 4..]);
            let length = |v: [f64; 3]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
            let vl = length(v);
            let ql = length(q);
            if vl <= 1e-12 || ql <= 1e-12 || !vl.is_finite() {
                return Err(fidelity("decoded zero/nonfinite normal"));
            }
            let e = (vl - 1.).abs();
            let angle = (v.iter().zip(q).map(|(a, b)| a * b).sum::<f64>() / (vl * ql))
                .clamp(-1., 1.)
                .acos()
                .to_degrees();
            let w = f64::from(weight);
            sum += e * w;
            weight_sum += w;
            max = max.max(e);
            angle_sum += angle * w;
            angles.push((angle, weight));
        }
        if weight_sum == 0. {
            return Err(error(
                JobErrorKind::InvalidRequest,
                "normal page has no signal",
            ));
        }
        angles.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut cumulative = 0.;
        let mut p99 = 0.;
        for (angle, weight) in angles {
            cumulative += f64::from(weight);
            p99 = angle;
            if cumulative >= weight_sum * 0.99 {
                break;
            }
        }
        let mean = sum / weight_sum;
        let angular = angle_sum / weight_sum;
        let (mean_cap, max_cap) = normal_length_caps(profile);
        normals = json!({"length_mean":mean,"length_max":max,"angle_mean":angular,"angle_p99":p99});
        if mean > mean_cap || max > max_cap || angular > 3. || p99 > 12. {
            return Err(fidelity(format!(
                "normal mip{mip} length{mean}/{max} angle{angular}/{p99}"
            )));
        }
    }
    let mut receipt = json!({"mip":mip,"channels":metrics,"normal":normals});
    if let Some((psnr, floor)) = rgb {
        receipt["rgb"] = json!({"psnr":if psnr.is_finite(){Some(psnr)}else{None},"exact":psnr.is_infinite(),"floor":floor});
    }
    Ok(receipt)
}

fn gated_channels(slot: TextureSlot) -> &'static [usize] {
    match slot {
        TextureSlot::Occlusion => &[0, 3],
        TextureSlot::MetallicRoughness => &[1, 2, 3],
        _ => &[0, 1, 2, 3],
    }
}
/// Mean and maximum caps on a decoded normal's length error.
fn normal_length_caps(profile: Profile) -> (f64, f64) {
    match profile {
        Profile::Fast => (0.08, 0.20),
        Profile::Quality => (0.03, 0.10),
        _ => (0.05, 0.15),
    }
}
/// The block-local caps of one level: a channel's maximum error and worst-block RMSE
/// and the normal length cap. One complete 4x4 block over any of them misses the
/// level's gate whatever every other block decodes to.
struct BlockCaps {
    gates: Vec<(usize, ChannelGate)>,
    normal_length: Option<f64>,
}
impl BlockCaps {
    fn new(level: &sparse_mips::Level, slot: TextureSlot, profile: Profile) -> Self {
        let texels = level.support.iter().filter(|&&s| s > 0).count() as u64;
        Self {
            gates: (gated_channels(slot).iter())
                .map(|&c| (c, channel_gate(slot, profile, c, texels)))
                .collect(),
            normal_length: (slot == TextureSlot::Normal).then(|| normal_length_caps(profile).1),
        }
    }
    /// Whether the block whose texel `t` (row-major) is `(support, expected, decoded)`
    /// misses, with `channel_stats`'s and `check_level`'s arithmetic.
    fn over<'a>(&self, texel: impl Fn(usize) -> (u32, &'a [u8], &'a [u8])) -> bool {
        let length = |d: &[u8]| {
            let l = d[..3]
                .iter()
                .map(|&v| (f64::from(v) / 127.5 - 1.).powi(2))
                .sum::<f64>()
                .sqrt();
            // check_level refuses a zero or non-finite decoded normal outright.
            self.normal_length
                .is_some_and(|cap| l <= 1e-12 || !l.is_finite() || (l - 1.).abs() > cap)
        };
        let texel_over = (0..16).any(|t| {
            let (w, e, d) = texel(t);
            let max =
                |&(c, g): &(usize, ChannelGate)| g.max.is_some_and(|m| e[c].abs_diff(d[c]) > m);
            w > 0 && (self.gates.iter().any(max) || length(d))
        });
        texel_over
            || self.gates.iter().any(|&(c, g)| {
                let Some(cap) = g.block_rmse else {
                    return false;
                };
                let (mut sse, mut weight) = (0., 0.);
                for t in 0..16 {
                    let (w, e, d) = texel(t);
                    if w > 0 {
                        sse += f64::from(e[c].abs_diff(d[c])).powi(2) * f64::from(w);
                        weight += f64::from(w);
                    }
                }
                weight > 0. && (sse / weight).sqrt() > cap
            })
    }
    /// Complete 4x4 blocks (row-major index) of a decoded level that miss.
    fn witnesses(&self, level: &sparse_mips::Level, expected: &[u8], decoded: &[u8]) -> Vec<u32> {
        let w = level.width as usize;
        (0..(w / 4 * (level.height as usize / 4)) as u32)
            .filter(|&b| {
                self.over(|t| {
                    let i = block_texel(w, b, t);
                    (level.support[i], &expected[i * 4..], &decoded[i * 4..])
                })
            })
            .take(WITNESS_BLOCKS)
            .collect()
    }
}
/// Blocks kept per missed level for the next step's block probe.
const WITNESS_BLOCKS: usize = 16;

/// Whether a slot/profile stores occlusion as scalar ETC1S (R replicated).
/// Quality is excluded on purpose: its occlusion gate (max <= 8, against 80
/// elsewhere) is far tighter than ETC1S can meet, so Quality keeps UASTC.
pub(in crate::mesh_large) fn scalar_occlusion(slot: TextureSlot, profile: Profile) -> bool {
    slot == TextureSlot::Occlusion && profile != Profile::Quality
}
/// Encoder mode and parameters of one slot and lossy profile.
pub(in crate::mesh_large) fn codec_settings(
    slot: TextureSlot,
    profile: Profile,
) -> (codec::Mode, codec::Params) {
    let scalar = scalar_occlusion(slot, profile);
    // Quality's effort of 2 is deliberately not pinned by a test: lowering it
    // only changes encoder time and size, while every output is still held to
    // the per-profile decoded-image floors in `check_level`, which are the
    // contract. Mutating 2 to 0 therefore survives on purpose.
    let effort = match profile {
        Profile::Fast => 0,
        Profile::Balanced => 1,
        _ => 2,
    };
    let colour =
        class(slot) == Class::Colour && matches!(profile, Profile::Fast | Profile::Balanced);
    let mode = if colour {
        codec::Mode::Etc1s {
            quality: if profile == Profile::Fast { 50 } else { 90 },
        }
    } else if scalar {
        codec::Mode::Etc1s { quality: 90 }
    } else {
        codec::Mode::Uastc { zstd: true }
    };
    (
        mode,
        codec::Params {
            // Native ETC1S level 0 retains the measured byte saving while
            // avoiding the costly refinement/selector ordering of level 2.
            effort: if colour || scalar { 0 } else { effort },
            transfer: if class(slot) == Class::Colour {
                codec::Transfer::Srgb
            } else {
                codec::Transfer::Linear
            },
            mipmaps: false,
        },
    )
}
/// Encoder mode and parameters of one lossy ladder step.
pub(super) fn step_settings(
    slot: TextureSlot,
    profile: Profile,
    step: Step,
) -> Result<(codec::Mode, codec::Params), JobError> {
    let (mode, params) = codec_settings(slot, profile);
    let effort = match step {
        Step::Profile => return Ok((mode, params)),
        Step::UastcLevel2 => 5,
        Step::UastcLevel3 => 8,
        Step::LosslessPng => {
            return Err(error(
                JobErrorKind::InvalidState,
                "lossless ladder step has no codec setting",
            ))
        }
    };
    Ok((
        codec::Mode::Uastc { zstd: true },
        codec::Params { effort, ..params },
    ))
}

pub(in crate::mesh_large) fn workspace_charge(
    dimensions: [u32; 2],
    profile: Profile,
) -> Result<u64, JobError> {
    let [width, height] = dimensions;
    let base = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(resource)?;
    let mut texels = 0u64;
    let (mut w, mut h) = (width, height);
    loop {
        texels = texels
            .checked_add(u64::from(w) * u64::from(h))
            .ok_or_else(resource)?;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    let borrowed = base.checked_mul(8).ok_or_else(resource)?;
    let charge = if profile == Profile::Lossless {
        base.checked_mul(32)
            .and_then(|n| n.checked_add(64 * 1024 * 1024))
            .ok_or_else(resource)?
    } else {
        texels
            .checked_mul(132)
            .and_then(|n| n.checked_add(64 * 1024 * 1024))
            .and_then(|n| n.checked_add(borrowed))
            .ok_or_else(resource)?
    };
    Ok(charge)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mesh_large) fn encode(
    dimensions: [u32; 2],
    rgba: &[u8],
    support: &[u32],
    slot: TextureSlot,
    profile: Profile,
    memory_limit: u64,
    mask: Option<MaskDelivery<'_>>,
    ladder: Ladder<'_>,
) -> Result<Delivery, JobError> {
    encode_islands(
        dimensions,
        rgba,
        support,
        None,
        0,
        slot,
        profile,
        memory_limit,
        mask,
        ladder,
    )
}

/// The levels a lossy page delivers, resolved by the owner of its published sampler.
#[derive(Clone, Copy)]
enum Levels<'a> {
    /// Level 0 only: the sampler has no mipmap minification filter, so it never reads a
    /// reduced level (S3 review P3-2).
    Base,
    /// The complete chain to 1x1, whose islands (`labels`, one per texel, 0 outside every
    /// allocation) stay isolated through level `isolated` ([`sparse_mips::build_islands`]).
    Complete {
        labels: Option<&'a [u32]>,
        isolated: u32,
    },
}

/// [`encode`] for a packed island page: a complete chain whose islands (`labels`, one
/// per texel, 0 outside every allocation) stay isolated through level `isolated`
/// ([`sparse_mips::build_islands`]); dimensions may be any multiple of four.
#[allow(clippy::too_many_arguments)]
pub(in crate::mesh_large) fn encode_islands(
    dimensions: [u32; 2],
    rgba: &[u8],
    support: &[u32],
    labels: Option<&[u32]>,
    isolated: u32,
    slot: TextureSlot,
    profile: Profile,
    memory_limit: u64,
    mask: Option<MaskDelivery<'_>>,
    ladder: Ladder<'_>,
) -> Result<Delivery, JobError> {
    encode_levels(
        dimensions,
        rgba,
        support,
        Levels::Complete { labels, isolated },
        slot,
        profile,
        memory_limit,
        mask,
        ladder,
    )
}

/// [`encode`] for a page whose sampler has no mipmap minification filter: level 0 alone,
/// since such a sampler never reads a reduced level (S3 review P3-2). Lossless pages are
/// the same exact PNG as [`encode_islands`] delivers.
#[allow(clippy::too_many_arguments)]
pub(in crate::mesh_large) fn encode_base(
    dimensions: [u32; 2],
    rgba: &[u8],
    support: &[u32],
    slot: TextureSlot,
    profile: Profile,
    memory_limit: u64,
    mask: Option<MaskDelivery<'_>>,
    ladder: Ladder<'_>,
) -> Result<Delivery, JobError> {
    encode_levels(
        dimensions,
        rgba,
        support,
        Levels::Base,
        slot,
        profile,
        memory_limit,
        mask,
        ladder,
    )
}

#[allow(clippy::too_many_arguments)]
fn encode_levels(
    dimensions: [u32; 2],
    rgba: &[u8],
    support: &[u32],
    levels: Levels<'_>,
    slot: TextureSlot,
    profile: Profile,
    memory_limit: u64,
    mask: Option<MaskDelivery<'_>>,
    ladder: Ladder<'_>,
) -> Result<Delivery, JobError> {
    let [width, height] = dimensions;
    let base = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(resource)?;
    if base == 0
        || u64::try_from(rgba.len()).map_err(|_| resource())?
            != base.checked_mul(4).ok_or_else(resource)?
        || u64::try_from(support.len()).map_err(|_| resource())? != base
    {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "delivery RGBA/support shape",
        ));
    }
    let label_bytes = if let Levels::Complete {
        labels: Some(_), ..
    } = levels
    {
        base.checked_mul(6).ok_or_else(resource)?
    } else {
        0
    };
    let charge = workspace_charge(dimensions, profile)?
        .checked_add(label_bytes)
        .ok_or_else(resource)?;
    if charge > memory_limit {
        return Err(resource());
    }
    if profile == Profile::Lossless {
        let (bytes, decoded) = png_round_trip(rgba, width, height)?;
        if decoded != rgba {
            return Err(error(
                JobErrorKind::InvalidState,
                "PNG changed literal RGBA8",
            ));
        }
        return Ok(Delivery {
            bytes,
            mime: "image/png",
            mip_count: 1,
            receipts: vec![
                json!({"profile":"lossless","level0_rgba_exact":true,"normal_normalized":false,"alpha_guard":false,"mips":"viewer generated outside guarantee","charged_bytes":charge}),
            ],
            escalation: None,
        });
    }
    let chain = timing::time(Stage::SparseMips, || match levels {
        Levels::Base => {
            sparse_mips::build_base(width, height, rgba, support, class(slot), memory_limit)
        }
        Levels::Complete { labels, isolated } => sparse_mips::build_islands(
            width,
            height,
            rgba,
            support,
            labels,
            isolated,
            class(slot),
            memory_limit,
        ),
    })?;
    encode_authored(&chain, slot, profile, memory_limit, mask, ladder, |_| {
        Ok(Value::Null)
    })
}

/// PNG-encodes one RGBA8 level and decodes it independently of the encoder.
fn png_round_trip(rgba: &[u8], width: u32, height: u32) -> Result<(Vec<u8>, Vec<u8>), JobError> {
    let base = u64::from(width) * u64::from(height);
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(
            usize::try_from(base.checked_mul(8).ok_or_else(resource)?).map_err(|_| resource())?,
        )
        .map_err(|_| resource())?;
    timing::time(Stage::PngEncode, || {
        image::codecs::png::PngEncoder::new(&mut bytes).write_image(
            rgba,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
    })
    .map_err(|e| error(JobErrorKind::InvalidState, format!("PNG encode {e}")))?;
    let decoded = timing::time(Stage::PngVerify, || {
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
    })
    .map_err(|e| error(JobErrorKind::InvalidState, format!("PNG verify {e}")))?
    .into_rgba8();
    timing::count(Counter::EncodedChainTexels, base);
    if decoded.dimensions() != (width, height) {
        return Err(error(JobErrorKind::InvalidState, "PNG changed dimensions"));
    }
    Ok((bytes, decoded.into_raw()))
}

/// The atlas owner supplies the complete chain and the independent source-sampler check.
pub(in crate::mesh_large) fn encode_authored(
    chain: &sparse_mips::Chain,
    slot: TextureSlot,
    profile: Profile,
    memory_limit: u64,
    mask: Option<MaskDelivery<'_>>,
    ladder: Ladder<'_>,
    validate: impl FnMut(&[mips::Level]) -> Result<Value, JobError>,
) -> Result<Delivery, JobError> {
    escalate(
        chain,
        slot,
        profile,
        memory_limit,
        mask,
        ladder,
        validate,
        &|_, _, _| {},
    )
}

/// Decoded-level hook between the encoder and the gates. Production passes a
/// no-op; tests corrupt one step's decoded level to prove the gates run there.
type Tamper<'a> = &'a dyn Fn(Step, usize, &mut [u8]);

/// Authored levels after MASK adjustment and the finite-sample guard (what is
/// encoded), and the pre-guard truth every gate compares against.
struct Prepared {
    delivered: Vec<Vec<u8>>,
    expected: Vec<Vec<u8>>,
    mask_receipts: Vec<Option<super::mask::MaskReceipt>>,
    guard_receipts: Vec<Value>,
}

/// Every gate of one decoded level beyond the channel floors: MASK coverage.
fn mask_gate(
    mask: Option<MaskDelivery<'_>>,
    reference: Option<&super::mask::MaskReceipt>,
    decoded: &[u8],
    level: &sparse_mips::Level,
    index: usize,
    profile: Profile,
) -> Result<Option<Result<Value, JobError>>, JobError> {
    let (Some(context), Some(reference)) = (mask, reference) else {
        return Ok(None);
    };
    let coverage = super::mask::coverage(
        decoded,
        level.width,
        level.height,
        context.samples,
        context.factor,
        context.cutoff,
        if index == 0 {
            context.linear
        } else {
            context.min_linear
        },
    )?;
    let floor = match profile {
        Profile::Fast => 0.02,
        Profile::Balanced => 0.01,
        _ => 0.005,
    };
    let residual = (coverage - reference.target).abs();
    let allowed = (reference.optimum_gap + floor).min(1.);
    let receipt = json!({"mip":index,"mask_target":reference.target,"fixed_semantic_gap":reference.optimum_gap,"scale":reference.scale,"scans":reference.scans,"decoded_coverage":coverage,"residual":residual,"allowed":allowed});
    Ok(Some(if residual > allowed {
        Err(fidelity(format!(
            "MASK mip{index} residual{residual} allowed{allowed}"
        )))
    } else {
        Ok(receipt)
    }))
}

#[allow(clippy::too_many_arguments)]
fn escalate(
    chain: &sparse_mips::Chain,
    slot: TextureSlot,
    profile: Profile,
    memory_limit: u64,
    mask: Option<MaskDelivery<'_>>,
    ladder: Ladder<'_>,
    mut validate: impl FnMut(&[mips::Level]) -> Result<Value, JobError>,
    tamper: Tamper<'_>,
) -> Result<Delivery, JobError> {
    if profile == Profile::Lossless {
        return Err(error(
            JobErrorKind::InvalidState,
            "PNG cannot carry an authored mip chain",
        ));
    }
    if chain.class == Class::Id {
        // Basis block codecs (ETC1S, UASTC) are lossy and blend IDs.
        return Err(error(
            JobErrorKind::InvalidRequest,
            "ID texture pages are lossless PNG only; Basis KTX2 refused",
        ));
    }
    let base = chain.levels.first().ok_or_else(resource)?;
    let charge = workspace_charge([base.width, base.height], profile)?;
    if charge > memory_limit {
        return Err(resource());
    }
    let mut prepared = prepare_levels(chain, mask)?;
    let scalar = scalar_occlusion(slot, profile);
    let mut misses = Vec::new();
    let mut witnesses = Vec::new();
    // Each step runs to completion and drops its encoder output before the
    // next starts, so the page's single lease and charge cover every step.
    for step in Step::LOSSY {
        if step != Step::Profile {
            (ladder.check)()?;
        }
        if scalar {
            // ETC1S stores R in every channel; UASTC steps restore the
            // authored G and B, which the guard and MASK never touch.
            let rrr = step == Step::Profile;
            for (pixels, truth) in prepared.delivered.iter_mut().zip(&prepared.expected) {
                for (pixel, t) in pixels
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(truth.as_chunks::<4>().0)
                {
                    pixel[1] = if rrr { pixel[0] } else { t[1] };
                    pixel[2] = if rrr { pixel[0] } else { t[2] };
                }
            }
        }
        let rrr = scalar && step == Step::Profile;
        let earlier = (&misses[..], &witnesses[..]);
        if let Some(miss) = probe(chain, &prepared, (slot, profile, step), earlier)? {
            misses.push(miss);
            continue;
        }
        match lossy_step(
            chain,
            &prepared,
            slot,
            profile,
            step,
            (mask, rrr, charge),
            &mut validate,
            tamper,
            &mut witnesses,
        )? {
            Ok(mut delivery) => {
                if step != Step::Profile {
                    delivery.escalation = Some(Escalation { step, misses });
                }
                return Ok(delivery);
            }
            Err(miss) => misses.push(miss),
        }
    }
    // Every page ends in its exact level 0, so one page that no codec step can deliver
    // within its gates never fails the run. A mip-sampled page keeps its sampler: its
    // islands sit whole on the 2^K grid with real-source gutters, so a viewer's box
    // filter matches the authored data-class levels through K, but those levels are the
    // viewer's, outside the decoded guarantee (as in the lossless profile).
    (ladder.check)()?;
    match png_step(
        chain,
        &prepared,
        slot,
        profile,
        mask,
        charge,
        &mut validate,
        tamper,
    )? {
        Ok(mut delivery) => {
            delivery.escalation = Some(Escalation {
                step: Step::LosslessPng,
                misses,
            });
            return Ok(delivery);
        }
        Err(miss) => misses.push(miss),
    }
    let tried: Vec<String> = misses
        .iter()
        .map(|m| match m.mip {
            Some(mip) => format!(
                "{} mip{mip} ({}): {}",
                m.step.name(),
                m.reason.name(),
                m.gate
            ),
            None => format!("{} ({}): {}", m.step.name(), m.reason.name(), m.gate),
        })
        .collect();
    Err(fidelity(format!(
        "texture {slot:?} {profile:?}: every encoder step missed its gate; {}",
        tried.join("; ")
    )))
}

fn prepare_levels(
    chain: &sparse_mips::Chain,
    mask: Option<MaskDelivery<'_>>,
) -> Result<Prepared, JobError> {
    let mut delivered = Vec::new();
    delivered
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    for level in &chain.levels {
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(level.rgba.len())
            .map_err(|_| resource())?;
        pixels.extend_from_slice(&level.rgba);
        delivered.push(pixels);
    }
    let mut mask_receipts = Vec::new();
    mask_receipts
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    for (index, (level, pixels)) in chain.levels.iter().zip(&mut delivered).enumerate() {
        let receipt = if let Some(context) = mask {
            Some(super::mask::adjust(
                pixels,
                level.width,
                level.height,
                context.samples,
                context.factor,
                context.cutoff,
                if index == 0 {
                    context.linear
                } else {
                    context.min_linear
                },
            )?)
        } else {
            None
        };
        mask_receipts.push(receipt);
    }
    // Fixed postscale, PRE-GUARD expected bytes. Guard never changes the
    // semantic gap or the alpha quality reference, nor feeds another mip.
    let mut expected = Vec::new();
    expected
        .try_reserve_exact(delivered.len())
        .map_err(|_| resource())?;
    let mut guard_receipts = Vec::new();
    guard_receipts
        .try_reserve_exact(delivered.len())
        .map_err(|_| resource())?;
    for (index, (level, pixels)) in chain.levels.iter().zip(&mut delivered).enumerate() {
        let mut truth = Vec::new();
        truth
            .try_reserve_exact(pixels.len())
            .map_err(|_| resource())?;
        truth.extend_from_slice(pixels);
        if let Some(context) = mask {
            let mut candidate = Vec::new();
            candidate
                .try_reserve_exact(pixels.len())
                .map_err(|_| resource())?;
            candidate.extend_from_slice(pixels);
            let mut guarded = 0usize;
            for pixel in candidate.as_chunks_mut::<4>().0 {
                if pixel[3] < 255 && f64::from(pixel[3]) / 255. * context.factor >= context.cutoff {
                    pixel[3] += 1;
                    guarded += 1;
                }
            }
            let safe = super::mask::preserves_classifications(
                &truth,
                &candidate,
                [level.width, level.height],
                context.samples,
                context.factor,
                context.cutoff,
                if index == 0 {
                    context.linear
                } else {
                    context.min_linear
                },
            )?;
            let enabled = safe && guarded > 0;
            if enabled {
                *pixels = candidate;
            }
            guard_receipts.push(json!({"mip":index,"finite_sample_guard_enabled":enabled,"classifications_preserved":safe,"candidate_texels":guarded,"max_alpha_delta":if enabled {1}else{0},"scope":"positive-weight declared geometry samples only"}));
        } else {
            guard_receipts.push(Value::Null);
        }
        expected.push(truth);
    }
    Ok(Prepared {
        delivered,
        expected,
        mask_receipts,
        guard_receipts,
    })
}

/// One KTX2 step: encode the prepared chain, then gate every decoded level.
/// `Ok(Err(miss))` is a gate miss the ladder may climb past.
#[allow(clippy::too_many_arguments)]
fn lossy_step(
    chain: &sparse_mips::Chain,
    prepared: &Prepared,
    slot: TextureSlot,
    profile: Profile,
    step: Step,
    (mask, scalar, charge): (Option<MaskDelivery<'_>>, bool, u64),
    validate: &mut impl FnMut(&[mips::Level]) -> Result<Value, JobError>,
    tamper: Tamper<'_>,
    witnesses: &mut Vec<(usize, Vec<u32>)>,
) -> Result<Result<Delivery, GateMiss>, JobError> {
    let mut levels = Vec::new();
    levels
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    for (level, pixels) in chain.levels.iter().zip(&prepared.delivered) {
        levels.push(codec::MipLevel {
            rgba: pixels,
            width: level.width,
            height: level.height,
        });
    }
    let (mode, params) = step_settings(slot, profile, step)?;
    let effort = params.effort;
    // The codec's verification runs in full at every step; a decoded-level
    // floor miss is a ladder miss like a gate miss, never accepted.
    let (bytes, mut decoded_levels) =
        match codec::encode_ktx2_mips_floor_checked(&levels, mode, params)? {
            Ok(encoded) => encoded,
            Err(miss) => {
                return Ok(Err(GateMiss {
                    step,
                    mip: Some(miss.level as usize),
                    reason: MissReason::CodecVerify,
                    gate: miss.error.message().to_owned(),
                }))
            }
        };
    let mut receipts = Vec::new();
    receipts
        .try_reserve_exact(chain.levels.len() * 3 + 1)
        .map_err(|_| resource())?;
    let mut decoded_chain = Vec::new();
    decoded_chain
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    for (index, level) in chain.levels.iter().enumerate() {
        timing::count(
            Counter::EncodedChainTexels,
            u64::from(level.width) * u64::from(level.height),
        );
        // The codec's mandatory verification decoded every level with the
        // vendored transcoder; reuse those bytes rather than decode again.
        let mut decoded = std::mem::take(
            decoded_levels
                .get_mut(index)
                .ok_or_else(|| error(JobErrorKind::InvalidState, "missing decoded level"))?,
        );
        if decoded.len() != level.rgba.len() {
            return Err(error(JobErrorKind::InvalidState, "decoded level shape"));
        }
        tamper(step, index, &mut decoded);
        let texels = u64::from(level.width) * u64::from(level.height);
        match timing::time(Stage::FidelityCheck, || {
            timing::level(timing::LEVEL_GATES, index, texels, || {
                check_level(
                    &prepared.expected[index],
                    &decoded,
                    &level.support,
                    level.width,
                    slot,
                    profile,
                    index,
                )
            })
        }) {
            Ok(receipt) => receipts.push(receipt),
            Err(e) => {
                let miss = missed(e, step, Some(index))?;
                let caps = BlockCaps::new(level, slot, profile);
                let blocks = caps.witnesses(level, &prepared.expected[index], &decoded);
                if !blocks.is_empty() {
                    witnesses.push((index, blocks));
                }
                return Ok(Err(miss));
            }
        }
        if mask.is_some() {
            receipts.push(prepared.guard_receipts[index].clone());
        }
        match mask_gate(
            mask,
            prepared.mask_receipts[index].as_ref(),
            &decoded,
            level,
            index,
            profile,
        )? {
            Some(Ok(receipt)) => receipts.push(receipt),
            Some(Err(e)) => return missed(e, step, Some(index)).map(Err),
            None => {}
        }
        decoded_chain.push(mips::Level {
            width: level.width,
            height: level.height,
            rgba: decoded,
        });
    }
    let sampler_receipt =
        match timing::time(Stage::SamplerCheckDecoded, || validate(&decoded_chain)) {
            Ok(receipt) => receipt,
            Err(e) => return missed(e, step, None).map(Err),
        };
    if !sampler_receipt.is_null() {
        receipts.push(sampler_receipt);
    }
    receipts.push(json!({"charged_bytes":charge,"native_workspace_provisional":true,"scalar_rrr":scalar,"effort":effort,"step":step.name(),"caller_levels":chain.levels.len()}));
    Ok(Ok(Delivery {
        bytes,
        mime: "image/ktx2",
        mip_count: chain.levels.len() as u32,
        receipts,
        escalation: None,
    }))
}

/// Texel `t` (row-major) of 4x4 block `b` of a `width`-texel level.
fn block_texel(width: usize, b: u32, t: usize) -> usize {
    let bw = width / 4;
    (b as usize / bw * 4 + t / 4) * width + b as usize % bw * 4 + t % 4
}
/// `blocks` of a `width`-texel level side by side in one row of blocks.
fn block_row(rgba: &[u8], width: usize, blocks: &[u32]) -> Vec<u8> {
    (0..blocks.len() * 16)
        .flat_map(|i| {
            let (t, j) = (
                i / (blocks.len() * 4) * 4 + i % 4,
                i % (blocks.len() * 4) / 4,
            );
            rgba[block_texel(width, blocks[j], t) * 4..][..4].to_vec()
        })
        .collect()
}

/// Decides a doomed UASTC step without encoding its chain. UASTC encodes every 4x4
/// block alone, so blocks or a level encoded on their own decode as inside the chain
/// (`a_level_alone_decodes_as_in_its_chain`): a miss here is the step's miss, exactly.
/// Blocks an earlier step missed a block-local cap on go first; before level 3 so do
/// the coarse levels, earlier misses first. Level 0 is never probed, so a step that
/// then passes costs at most a third more, on a page that already missed twice.
fn probe(
    chain: &sparse_mips::Chain,
    prepared: &Prepared,
    (slot, profile, step): (TextureSlot, Profile, Step),
    (misses, witnesses): (&[GateMiss], &[(usize, Vec<u32>)]),
) -> Result<Option<GateMiss>, JobError> {
    if step == Step::Profile {
        return Ok(None);
    }
    let (mode, params) = step_settings(slot, profile, step)?;
    for (index, blocks) in witnesses {
        let level = &chain.levels[*index];
        let w = level.width as usize;
        let row = block_row(&prepared.delivered[*index], w, blocks);
        let decoded =
            codec::encode_decoded_unverified(&row, row.len() as u32 / 16, 4, mode, params)?;
        let caps = BlockCaps::new(level, slot, profile);
        for (j, &b) in blocks.iter().enumerate() {
            if caps.over(|t| {
                let i = block_texel(w, b, t);
                let at = (t / 4 * blocks.len() * 4 + j * 4 + t % 4) * 4;
                (
                    level.support[i],
                    &prepared.expected[*index][i * 4..],
                    &decoded[at..],
                )
            }) {
                let gate = format!("block probe: mip{index} block {b} misses a block-local cap");
                return Ok(Some(GateMiss {
                    step,
                    mip: Some(*index),
                    reason: MissReason::Gate,
                    gate,
                }));
            }
        }
    }
    let mut seen = vec![step != Step::UastcLevel3; chain.levels.len()];
    seen[0] = true;
    let earlier = misses.iter().filter_map(|m| m.mip);
    for index in earlier.chain(1..chain.levels.len()) {
        if std::mem::replace(&mut seen[index], true) {
            continue;
        }
        let level = &chain.levels[index];
        let rgba = &prepared.delivered[index];
        let decoded =
            codec::encode_decoded_unverified(rgba, level.width, level.height, mode, params)?;
        let expected = &prepared.expected[index];
        if let Err(e) = check_level(
            expected,
            &decoded,
            &level.support,
            level.width,
            slot,
            profile,
            index,
        ) {
            let mut miss = missed(e, step, Some(index))?;
            miss.gate = format!("level probe: {}", miss.gate);
            return Ok(Some(miss));
        }
    }
    Ok(None)
}

/// The last step: exact PNG of the prepared level 0 (MASK-adjusted and
/// guarded like every KTX2 step), decoded independently and checked by the
/// same level-0 gates. A PNG that changes its input is a hard failure.
#[allow(clippy::too_many_arguments)]
fn png_step(
    chain: &sparse_mips::Chain,
    prepared: &Prepared,
    slot: TextureSlot,
    profile: Profile,
    mask: Option<MaskDelivery<'_>>,
    charge: u64,
    validate: &mut impl FnMut(&[mips::Level]) -> Result<Value, JobError>,
    tamper: Tamper<'_>,
) -> Result<Result<Delivery, GateMiss>, JobError> {
    let step = Step::LosslessPng;
    let level = chain.levels.first().ok_or_else(resource)?;
    let pixels = prepared.delivered.first().ok_or_else(resource)?;
    let (bytes, mut decoded) = png_round_trip(pixels, level.width, level.height)?;
    tamper(step, 0, &mut decoded);
    if decoded != *pixels {
        return Err(error(
            JobErrorKind::InvalidState,
            format!(
                "texture {slot:?} {profile:?} mip0: lossless PNG fallback changed literal RGBA8"
            ),
        ));
    }
    let mut receipts = Vec::new();
    match timing::time(Stage::FidelityCheck, || {
        check_level(
            &prepared.expected[0],
            &decoded,
            &level.support,
            level.width,
            slot,
            profile,
            0,
        )
    }) {
        Ok(receipt) => receipts.push(receipt),
        Err(e) => return missed(e, step, Some(0)).map(Err),
    }
    if mask.is_some() {
        receipts.push(prepared.guard_receipts[0].clone());
    }
    match mask_gate(
        mask,
        prepared.mask_receipts[0].as_ref(),
        &decoded,
        level,
        0,
        profile,
    )? {
        Some(Ok(receipt)) => receipts.push(receipt),
        Some(Err(e)) => return missed(e, step, Some(0)).map(Err),
        None => {}
    }
    let decoded = [mips::Level {
        width: level.width,
        height: level.height,
        rgba: decoded,
    }];
    match timing::time(Stage::SamplerCheckDecoded, || validate(&decoded)) {
        Ok(receipt) if receipt.is_null() => {}
        Ok(receipt) => receipts.push(receipt),
        Err(e) => return missed(e, step, None).map(Err),
    }
    receipts.push(json!({"charged_bytes":charge,"step":step.name(),"level0_rgba_exact":true,"mips":if chain.levels.len() > 1 {"viewer generated outside guarantee"} else {"page sampler reads level 0 only"},"caller_levels":chain.levels.len()}));
    Ok(Ok(Delivery {
        bytes,
        mime: "image/png",
        mip_count: 1,
        receipts,
        escalation: None,
    }))
}

/// One exact PNG per authored ID level. Level 0 is the delivered texture
/// (sampled NEAREST, no runtime mips); coarser levels are ID-safe pages for
/// coarse proxies. Every level is decoded independently and must equal the
/// authored bytes; `validate` then sees each decoded level.
#[derive(Debug)]
pub(in crate::mesh_large) struct IdDelivery {
    /// PNG bytes per level, level 0 first.
    pub levels: Vec<Vec<u8>>,
    pub receipts: Vec<Value>,
}
pub(in crate::mesh_large) fn encode_id_chain(
    chain: &sparse_mips::Chain,
    memory_limit: u64,
    mut validate: impl FnMut(usize, &mips::Level) -> Result<Value, JobError>,
) -> Result<IdDelivery, JobError> {
    if chain.class != Class::Id {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "ID delivery requires an ID-class chain",
        ));
    }
    let base = chain.levels.first().ok_or_else(resource)?;
    let charge = workspace_charge([base.width, base.height], Profile::Lossless)?;
    if charge > memory_limit {
        return Err(resource());
    }
    let mut levels = Vec::new();
    levels
        .try_reserve_exact(chain.levels.len())
        .map_err(|_| resource())?;
    let mut receipts = Vec::new();
    receipts
        .try_reserve_exact(chain.levels.len() + 1)
        .map_err(|_| resource())?;
    for (index, level) in chain.levels.iter().enumerate() {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(level.rgba.len().checked_mul(2).ok_or_else(resource)?)
            .map_err(|_| resource())?;
        timing::time(Stage::PngEncode, || {
            image::codecs::png::PngEncoder::new(&mut bytes).write_image(
                &level.rgba,
                level.width,
                level.height,
                image::ExtendedColorType::Rgba8,
            )
        })
        .map_err(|e| error(JobErrorKind::InvalidState, format!("ID PNG encode {e}")))?;
        let decoded = timing::time(Stage::PngVerify, || {
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        })
        .map_err(|e| error(JobErrorKind::InvalidState, format!("ID PNG verify {e}")))?
        .into_rgba8();
        timing::count(
            Counter::EncodedChainTexels,
            u64::from(level.width) * u64::from(level.height),
        );
        let decoded = mips::Level {
            width: decoded.width(),
            height: decoded.height(),
            rgba: decoded.into_raw(),
        };
        if decoded.width != level.width
            || decoded.height != level.height
            || decoded.rgba != level.rgba
        {
            return Err(error(
                JobErrorKind::InvalidState,
                format!("ID PNG level {index} changed authored IDs"),
            ));
        }
        receipts.push(validate(index, &decoded)?);
        levels.push(bytes);
    }
    receipts.push(json!({"profile":"id","encoding":"png","levels_exact":true,"charged_bytes":charge,"caller_levels":chain.levels.len()}));
    Ok(IdDelivery { levels, receipts })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_preserves_nonunit_normals_transparent_rgb_and_alpha() {
        let rgba = [
            255, 128, 128, 0, 128, 128, 200, 203, 170, 190, 220, 204, 1, 2, 3, 255,
        ];
        let out = encode(
            [4, 1],
            &rgba,
            &[1; 4],
            TextureSlot::Normal,
            Profile::Lossless,
            128 * 1024 * 1024,
            None,
            LADDER,
        )
        .unwrap();
        assert_eq!(out.mime, "image/png");
        assert_eq!(out.mip_count, 1);
        let decoded = image::load_from_memory_with_format(&out.bytes, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        assert_eq!(decoded.as_raw(), &rgba);
        for channel in 0..4 {
            let mut mutant = decoded.as_raw().clone();
            mutant[channel] ^= 1;
            assert_ne!(mutant.as_slice(), &rgba);
        }
        assert_eq!(
            encode(
                [4, 1],
                &rgba,
                &[1; 4],
                TextureSlot::Normal,
                Profile::Lossless,
                1,
                None,
                LADDER
            )
            .unwrap_err()
            .kind(),
            JobErrorKind::ResourceLimit
        );
    }
    #[test]
    fn all_profiles_and_slots_complete_independent_decode() {
        for profile in [
            Profile::Fast,
            Profile::Balanced,
            Profile::Quality,
            Profile::Lossless,
        ] {
            for slot in [
                TextureSlot::BaseColor,
                TextureSlot::Emissive,
                TextureSlot::MetallicRoughness,
                TextureSlot::Occlusion,
                TextureSlot::Normal,
            ] {
                let pixel = if slot == TextureSlot::Normal {
                    [128, 128, 255, 255]
                } else {
                    [90, 90, 90, 255]
                };
                let rgba = pixel.repeat(64);
                let support = vec![1; 64];
                let out = encode(
                    [8, 8],
                    &rgba,
                    &support,
                    slot,
                    profile,
                    128 * 1024 * 1024,
                    None,
                    LADDER,
                )
                .unwrap();
                if profile == Profile::Lossless {
                    assert_eq!(out.mip_count, 1);
                    continue;
                }
                // KTX2 DFD starts at the little-endian header offset 48.
                // The transfer byte is format metadata, independent of pixels.
                let dfd = u32::from_le_bytes(out.bytes[48..52].try_into().unwrap()) as usize;
                let transfer = out.bytes[dfd + 14];
                assert_eq!(
                    transfer,
                    if matches!(slot, TextureSlot::BaseColor | TextureSlot::Emissive) {
                        2
                    } else {
                        1
                    },
                    "wrong KTX transfer for {slot:?}/{profile:?}"
                );
                let independent = basisu::Transcoder::new(&out.bytes).unwrap();
                assert_eq!(independent.level_count(), 4);
                assert_eq!(out.mip_count, 4);
                let (mut w, mut h) = (8, 8);
                for mip in 0..4 {
                    let decoded = independent
                        .transcode(mip, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
                        .unwrap();
                    // Constant authored signal: independent expected values, no
                    // production mip/filter values used as expected truth.
                    let expected = pixel.repeat(w * h);
                    check_level(
                        &expected,
                        &decoded,
                        &vec![1; w * h],
                        w as u32,
                        slot,
                        profile,
                        mip as usize,
                    )
                    .unwrap();
                    w = (w / 2).max(1);
                    h = (h / 2).max(1);
                }
            }
        }
    }
    /// Colour and scalar occlusion use ETC1S in fast/balanced; quality,
    /// normal and metallic-roughness use UASTC. Read from the output KTX2 DFD
    /// colour model itself (basic block word 2, byte 0): 163 is ETC1S, 166 is UASTC.
    #[test]
    fn occlusion_is_scalar_etc1s_except_in_quality() {
        const ETC1S: u8 = 163;
        const UASTC: u8 = 166;
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            for slot in [
                TextureSlot::BaseColor,
                TextureSlot::Emissive,
                TextureSlot::Normal,
                TextureSlot::Occlusion,
                TextureSlot::MetallicRoughness,
            ] {
                let out = encode(
                    [8, 8],
                    &if slot == TextureSlot::Normal {
                        [128, 128, 255, 255]
                    } else {
                        [90, 90, 90, 255]
                    }
                    .repeat(64),
                    &[1; 64],
                    slot,
                    profile,
                    128 * 1024 * 1024,
                    None,
                    LADDER,
                )
                .unwrap();
                let dfd = u32::from_le_bytes(out.bytes[48..52].try_into().unwrap()) as usize;
                let scalar = matches!(
                    (slot, profile),
                    (TextureSlot::Occlusion, Profile::Fast | Profile::Balanced)
                );
                assert_eq!(
                    out.bytes[dfd + 12],
                    if scalar
                        || (matches!(slot, TextureSlot::BaseColor | TextureSlot::Emissive)
                            && profile != Profile::Quality)
                    {
                        ETC1S
                    } else {
                        UASTC
                    },
                    "{slot:?}/{profile:?} codec"
                );
                assert_eq!(
                    out.receipts.last().unwrap()["scalar_rrr"],
                    scalar,
                    "{slot:?}/{profile:?} receipt"
                );
            }
        }
    }
    // Independent centre LINEAR coverage for this constant-image fixture. The
    // source target/gap are literal mathematical fixture facts, not mask output.
    fn centre_coverage(
        pixels: &[u8],
        w: usize,
        h: usize,
        factor: f64,
        cutoff: f64,
        weights: [f64; 2],
    ) -> f64 {
        let x = w as f64 / 2. - 0.5;
        let y = h as f64 / 2. - 0.5;
        let fetch = |x: i64, y: i64| {
            f64::from(
                pixels[((y.clamp(0, h as i64 - 1) as usize) * w
                    + x.clamp(0, w as i64 - 1) as usize)
                    * 4
                    + 3],
            ) / 255.
        };
        let (ix, iy) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let alpha = (fetch(ix, iy) * (1. - fx) + fetch(ix + 1, iy) * fx) * (1. - fy)
            + (fetch(ix, iy + 1) * (1. - fx) + fetch(ix + 1, iy + 1) * fx) * fy;
        let total = weights.iter().sum::<f64>();
        if total == 0. {
            return 0.;
        }
        [1., 0.5]
            .iter()
            .zip(weights)
            .filter(|(c0, _)| alpha * factor * **c0 >= cutoff)
            .map(|(_, weight)| weight)
            .sum::<f64>()
            / total
    }
    #[test]
    fn mask_variable_colour_source_target_every_profile_and_mip() {
        use super::super::mask::SurfaceSample;
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            for (cutoff, weights) in [(0.5, [3., 1.]), (1.5, [3., 1.]), (0.5, [0., 0.])] {
                let factor = 0.75;
                let surface = [
                    SurfaceSample {
                        uv: [0.5, 0.5],
                        alpha: 1.,
                        weight: weights[0],
                        source_pass: cutoff < 1.,
                    },
                    SurfaceSample {
                        uv: [0.5, 0.5],
                        alpha: 0.5,
                        weight: weights[1],
                        source_pass: false,
                    },
                ];
                let target = if cutoff > 1. || weights == [0., 0.] {
                    0.
                } else {
                    0.75
                };
                // Alpha255: context1 passes .75>=.5; context.5 fails
                // .375<.5. This fixed semantic outcome is attainable at all
                // levels including1x1; independently established gap is zero.
                let rgba = [90, 90, 90, 255].repeat(64);
                let out = encode(
                    [8, 8],
                    &rgba,
                    &[1; 64],
                    TextureSlot::BaseColor,
                    profile,
                    128 * 1024 * 1024,
                    Some(MaskDelivery {
                        samples: &surface,
                        factor,
                        cutoff,
                        linear: true,
                        min_linear: true,
                    }),
                    LADDER,
                )
                .unwrap();
                let independent = basisu::Transcoder::new(&out.bytes).unwrap();
                assert_eq!(independent.level_count(), 4);
                let (mut w, mut h) = (8, 8);
                for mip in 0..4 {
                    let decoded = independent
                        .transcode(mip, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
                        .unwrap();
                    let actual = centre_coverage(&decoded, w, h, factor, cutoff, weights);
                    let floor = match profile {
                        Profile::Fast => 0.02,
                        Profile::Balanced => 0.01,
                        _ => 0.005,
                    };
                    println!("delivery-mask-independent profile={profile:?} mip={mip} target={target} fixed_gap=0 coverage={actual} floor={floor}");
                    assert!((actual - target).abs() <= floor);
                    check_level(
                        &[90, 90, 90, 255].repeat(w * h),
                        &decoded,
                        &vec![1; w * h],
                        w as u32,
                        TextureSlot::BaseColor,
                        profile,
                        mip as usize,
                    )
                    .unwrap();
                    w = (w / 2).max(1);
                    h = (h / 2).max(1);
                }
            }
        }
    }
    #[test]
    fn near_cutoff_safe_guard_positive_and_original_source_truth() {
        use super::super::mask::SurfaceSample;
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            for alpha in [203, 204] {
                let surface = [
                    SurfaceSample {
                        uv: [0.5, 0.5],
                        alpha: 1.,
                        weight: 1.,
                        source_pass: alpha >= 204,
                    },
                    SurfaceSample {
                        uv: [0.5, 0.5],
                        alpha: 0.5,
                        weight: 1.,
                        source_pass: false,
                    },
                ];
                let target = if alpha >= 204 { 0.5 } else { 0. };
                let rgba = [90, 90, 90, alpha].repeat(64);
                match encode(
                    [8, 8],
                    &rgba,
                    &[1; 64],
                    TextureSlot::BaseColor,
                    profile,
                    128 * 1024 * 1024,
                    Some(MaskDelivery {
                        samples: &surface,
                        factor: 1.,
                        cutoff: 0.8,
                        linear: true,
                        min_linear: true,
                    }),
                    LADDER,
                ) {
                    Err(error) => {
                        assert_ne!(
                            alpha, 204,
                            "safe 1/.5 guard must produce accepted fixture: {error:?}"
                        );
                        assert_eq!(error.kind(), JobErrorKind::Unsupported);
                        println!("delivery-mask-nearcutoff typed-refusal profile={profile:?} source_alpha={alpha} source_target={target} error={error:?}");
                    }
                    Ok(out) => {
                        if alpha == 204 {
                            assert_eq!(
                                out.receipts
                                    .iter()
                                    .filter(|r| r.get("finite_sample_guard_enabled")
                                        == Some(&Value::Bool(true)))
                                    .count(),
                                4
                            );
                        }
                        let independent = basisu::Transcoder::new(&out.bytes).unwrap();
                        let (mut w, mut h) = (8, 8);
                        for mip in 0..4 {
                            let decoded = independent
                                .transcode(
                                    mip,
                                    basisu::TargetFormat::Rgba32,
                                    basisu::DecodeFlags::NONE,
                                )
                                .unwrap();
                            let actual = centre_coverage(&decoded, w, h, 1., 0.8, [1., 1.]);
                            let floor = match profile {
                                Profile::Fast => 0.02,
                                Profile::Balanced => 0.01,
                                _ => 0.005,
                            };
                            assert!((actual - target).abs() <= floor);
                            println!("delivery-mask-nearcutoff decoded profile={profile:?} mip={mip} source_alpha={alpha} source_target={target} fixed_gap=0 coverage={actual}");
                            w = (w / 2).max(1);
                            h = (h / 2).max(1);
                        }
                    }
                }
            }
        }
    }
    /// Deterministic photograph-like page: smooth value noise plus texel
    /// detail and a few hard edges, opaque.
    fn photo_page(edge: usize) -> Vec<u8> {
        let hash = |x: usize, y: usize, s: usize| {
            let mut h = (x as u64).wrapping_mul(0x9E37_79B9)
                ^ (y as u64).wrapping_mul(0x85EB_CA6B)
                ^ s as u64;
            h ^= h >> 13;
            h = h.wrapping_mul(0xC2B2_AE35);
            (h >> 7) % 256
        };
        let smooth = |x: usize, y: usize, s: usize| {
            let (cx, cy) = (x / 16, y / 16);
            let (fx, fy) = ((x % 16) as f64 / 16., (y % 16) as f64 / 16.);
            let v = |i, j| hash(cx + i, cy + j, s) as f64;
            (v(0, 0) * (1. - fx) + v(1, 0) * fx) * (1. - fy)
                + (v(0, 1) * (1. - fx) + v(1, 1) * fx) * fy
        };
        let mut out = Vec::with_capacity(edge * edge * 4);
        for y in 0..edge {
            for x in 0..edge {
                let edge_shift = if (x / 37 + y / 53) % 3 == 0 { 40. } else { 0. };
                for c in 0..3 {
                    let v = smooth(x, y, c) * 0.7
                        + (hash(x, y, c + 7) as f64 - 128.) * 0.12
                        + edge_shift;
                    out.push(v.round().clamp(0., 255.) as u8);
                }
                out.push(255);
            }
        }
        out
    }
    fn decoded_levels(out: &Delivery, edge: usize) -> Vec<Vec<u8>> {
        let independent = basisu::Transcoder::new(&out.bytes).unwrap();
        (0..independent.level_count())
            .map(|mip| {
                let decoded = independent
                    .transcode(mip, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
                    .unwrap();
                assert_eq!(decoded.len(), (edge >> mip).max(1).pow(2) * 4);
                decoded
            })
            .collect()
    }
    #[test]
    fn robust_gate_accepts_codec_outliers_and_refuses_corruption_controls() {
        let edge = 128;
        let rgba = photo_page(edge);
        let support = vec![1; edge * edge];
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            let out = encode(
                [edge as u32, edge as u32],
                &rgba,
                &support,
                TextureSlot::BaseColor,
                profile,
                128 * 1024 * 1024,
                None,
                LADDER,
            )
            .unwrap();
            let decoded = decoded_levels(&out, edge);
            let check = |level: &[u8]| {
                check_level(
                    &rgba,
                    level,
                    &support,
                    edge as u32,
                    TextureSlot::BaseColor,
                    profile,
                    0,
                )
            };
            assert!(check(&decoded[0]).is_ok());
            // One hot texel at full-range error: the old maximum cap refused
            // this; the percentile, PSNR and block guard accept it.
            let mut outlier = decoded[0].clone();
            outlier[(40 * edge + 40) * 4] ^= 0xff;
            assert!(check(&outlier).is_ok(), "{profile:?} single outlier");
            // A corrupted 4x4 codec block is refused by the block guard alone.
            let mut corrupt = decoded[0].clone();
            for y in 64..68 {
                for x in 64..68 {
                    for c in 0..3 {
                        corrupt[(y * edge + x) * 4 + c] ^= 0x80;
                    }
                }
            }
            let stats = channel_stats(&rgba, &corrupt, &support, edge, 0);
            let gate = channel_gate(TextureSlot::BaseColor, profile, 0, stats.texels);
            assert!(stats.psnr >= gate.psnr && stats.p999 <= gate.p999.unwrap());
            assert!(check(&corrupt).is_err(), "{profile:?} corrupted block");
            // Swapped red/blue channels are refused.
            let mut swapped = decoded[0].clone();
            for px in swapped.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            assert!(check(&swapped).is_err(), "{profile:?} swapped channels");
            // Ten percent of texels uniformly damaged: the percentile refuses
            // even when the damage stays below the block guard.
            let mut damaged = decoded[0].clone();
            for (i, px) in damaged.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                if i % 10 == 0 {
                    px[1] = px[1].wrapping_add(96);
                }
            }
            assert!(check(&damaged).is_err(), "{profile:?} spread damage");
        }
    }
    #[test]
    fn shifted_levels_and_flipped_normal_green_are_refused_every_profile() {
        let edge = 64usize;
        let support = vec![1; edge * edge];
        // One-texel saturated checker: a level shifted by one (each texel
        // replaced by the next coarser level) loses all its contrast.
        let checker: Vec<u8> = (0..edge * edge)
            .flat_map(|i| {
                if (i % edge + i / edge).is_multiple_of(2) {
                    [0, 255, 0, 255]
                } else {
                    [255, 0, 255, 255]
                }
            })
            .collect();
        // Smooth tangent-space bumps, unit length.
        let normals: Vec<u8> = (0..edge * edge)
            .flat_map(|i| {
                let (x, y) = ((i % edge) as f64, (i / edge) as f64);
                let n = [0.25 * (x / 40.).sin(), 0.25 * (y / 48.).cos(), 1.];
                let l = n.iter().map(|v| v * v).sum::<f64>().sqrt();
                let q = |v: f64| ((v / l + 1.) * 127.5).round() as u8;
                [q(n[0]), q(n[1]), q(n[2]), 255]
            })
            .collect();
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            let out = encode(
                [edge as u32, edge as u32],
                &checker,
                &support,
                TextureSlot::BaseColor,
                profile,
                128 * 1024 * 1024,
                None,
                LADDER,
            )
            .unwrap();
            let decoded = decoded_levels(&out, edge);
            let half = edge / 2;
            let shifted: Vec<u8> = (0..edge * edge)
                .flat_map(|i| {
                    let (x, y) = (i % edge / 2, i / edge / 2);
                    decoded[1][(y * half + x) * 4..(y * half + x) * 4 + 4].to_vec()
                })
                .collect();
            let check = |slot, expected: &[u8], level: &[u8]| {
                check_level(expected, level, &support, edge as u32, slot, profile, 0)
            };
            assert!(check(TextureSlot::BaseColor, &checker, &decoded[0]).is_ok());
            assert!(check(TextureSlot::BaseColor, &checker, &shifted).is_err());
            let out = encode(
                [edge as u32, edge as u32],
                &normals,
                &support,
                TextureSlot::Normal,
                profile,
                128 * 1024 * 1024,
                None,
                LADDER,
            )
            .unwrap();
            let mut flipped = decoded_levels(&out, edge).swap_remove(0);
            for px in flipped.as_chunks_mut::<4>().0 {
                px[1] = 255 - px[1];
            }
            assert!(
                check(TextureSlot::Normal, &normals, &flipped).is_err(),
                "{profile:?} flipped green"
            );
        }
    }
    #[test]
    #[ignore = "measurement: class-correct smooth photo mip distributions"]
    fn measure_etc1s_smooth_photo_floors() {
        for edge in [32, 64, 128, 256] {
            let rgba = photo_page(edge);
            let authored = chain(&rgba, edge);
            let levels: Vec<_> = authored
                .levels
                .iter()
                .map(|l| codec::MipLevel {
                    rgba: &l.rgba,
                    width: l.width,
                    height: l.height,
                })
                .collect();
            for profile in [Profile::Fast, Profile::Balanced] {
                let (mode, params) = codec_settings(TextureSlot::BaseColor, profile);
                let (_, decoded) = codec::encode_ktx2_mips_decoded(&levels, mode, params).unwrap();
                for (mip, (level, decoded)) in authored.levels.iter().zip(&decoded).enumerate() {
                    for channel in 0..4 {
                        let stats = channel_stats(
                            &level.rgba,
                            decoded,
                            &level.support,
                            level.width as usize,
                            channel,
                        );
                        println!(
                            "etc1s-photo-measure {}",
                            json!({"fixture":format!("smooth-photo-{edge}"),"profile":format!("{profile:?}"),"mip":mip,"channel":channel,"texels":stats.texels,"psnr":stats.psnr,"p999":stats.p999,"max":stats.max,"blockRmse":stats.block_rmse})
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn etc1s_colour_policy_and_sensitive_controls() {
        for (profile, quality) in [(Profile::Fast, 50), (Profile::Balanced, 90)] {
            for slot in [TextureSlot::BaseColor, TextureSlot::Emissive] {
                let (mode, params) = codec_settings(slot, profile);
                assert_eq!(mode, codec::Mode::Etc1s { quality });
                assert_eq!(params.effort, 0);
                assert_eq!(params.transfer, codec::Transfer::Srgb);
                let edge = 64;
                let rgba = photo_page(edge);
                let out = encode(
                    [edge as u32; 2],
                    &rgba,
                    &vec![1; edge * edge],
                    slot,
                    profile,
                    MEMORY,
                    None,
                    LADDER,
                )
                .unwrap();
                assert!(out.escalation.is_none(), "{slot:?}/{profile:?}");
                let check = |expected: &[u8], actual: &[u8]| {
                    check_level(
                        expected,
                        actual,
                        &vec![1; edge * edge],
                        edge as u32,
                        slot,
                        profile,
                        0,
                    )
                };
                let decoded = decoded_levels(&out, edge).remove(0);
                let mut wrong_transfer = decoded.clone();
                for pixel in wrong_transfer.as_chunks_mut::<4>().0 {
                    for value in &mut pixel[..3] {
                        let s = f64::from(*value) / 255.;
                        *value = ((if s <= 0.04045 {
                            s / 12.92
                        } else {
                            ((s + 0.055) / 1.055).powf(2.4)
                        }) * 255.)
                            .round() as u8;
                    }
                }
                assert!(check(&rgba, &wrong_transfer).is_err(), "wrong colour space");
                let mut broken_alpha = decoded;
                for pixel in broken_alpha.as_chunks_mut::<4>().0 {
                    pixel[3] = 0;
                }
                assert!(check(&rgba, &broken_alpha).is_err(), "broken alpha");
                let flat = [32, 96, 208, 255].repeat(edge * edge);
                let mut block = flat.clone();
                for y in 16..20 {
                    for x in 16..20 {
                        block[(y * edge + x) * 4] += 100;
                    }
                }
                assert!(check(&flat, &block).is_err(), "damaged flat block");
                // The relaxed tiny and middle bands retain sensitive controls.
                for side in [4u32, 8, 32] {
                    let count = (side * side) as usize;
                    let flat = [32, 96, 208, 255].repeat(count);
                    let check_small = |actual: &[u8]| {
                        check_level(&flat, actual, &vec![1; count], side, slot, profile, 6)
                    };
                    assert!(
                        check_small(&[208, 96, 32, 255].repeat(count)).is_err(),
                        "small swapped channels"
                    );
                    assert!(
                        check_small(&[4, 30, 161, 255].repeat(count)).is_err(),
                        "small wrong transfer"
                    );
                    assert!(
                        check_small(&[32, 96, 208, 0].repeat(count)).is_err(),
                        "small broken alpha"
                    );
                    let contrast: Vec<u8> = (0..count)
                        .flat_map(|i| {
                            if (i % side as usize + i / side as usize).is_multiple_of(2) {
                                [0, 255, 0, 255]
                            } else {
                                [255, 0, 255, 255]
                            }
                        })
                        .collect();
                    assert!(
                        check_level(
                            &contrast,
                            &[128, 128, 128, 255].repeat(count),
                            &vec![1; count],
                            side,
                            slot,
                            profile,
                            6
                        )
                        .is_err(),
                        "small lost detail"
                    );
                }
                let checker: Vec<u8> = (0..edge * edge)
                    .flat_map(|i| {
                        if (i % edge + i / edge) % 2 == 0 {
                            [0, 255, 0, 255]
                        } else {
                            [255, 0, 255, 255]
                        }
                    })
                    .collect();
                assert!(
                    check(&checker, &[128, 128, 128, 255].repeat(edge * edge)).is_err(),
                    "lost high contrast detail"
                );
            }
        }
    }

    #[test]
    fn blank_inversion_wrong_mip_and_light_flat_transfer_controls() {
        for profile in [Profile::Fast, Profile::Balanced] {
            for side in [4u32, 8, 32, 64] {
                let count = (side * side) as usize;
                let support = vec![1; count];
                let flat = [32, 96, 208, 255].repeat(count);
                let gate = |expected: &[u8], actual: &[u8]| {
                    check_level(
                        expected,
                        actual,
                        &support,
                        side,
                        TextureSlot::BaseColor,
                        profile,
                        0,
                    )
                };
                assert!(
                    gate(&flat, &[0, 0, 0, 255].repeat(count)).is_err(),
                    "black page {side}"
                );
                assert!(
                    gate(&flat, &[223, 96, 208, 255].repeat(count)).is_err(),
                    "inverted red {side}"
                );
                let light = [200, 200, 200, 255].repeat(count);
                // Literal independent sRGB decoding of 200 rounds to147. The
                // former tiny 12dB floor admitted this wrong-transfer page.
                assert!(
                    gate(&light, &[147, 147, 147, 255].repeat(count)).is_err(),
                    "light flat wrong transfer {side}"
                );
                if side == 4 {
                    let error = gate(&light, &[147, 147, 147, 255].repeat(count)).unwrap_err();
                    assert!(error.message().contains("combined RGB"));
                    // Shared-palette one-channel tails remain acceptable:
                    // R13.65/G20.53/B21.69dB combine to17.07dB, unlike the
                    // wrong transfer's13.65dB in every channel.
                    let witness = gate(
                        &[180, 180, 180, 255].repeat(count),
                        &[127, 204, 201, 255].repeat(count),
                    )
                    .unwrap();
                    assert_eq!(witness["rgb"]["floor"], 16.);
                    assert_eq!(witness["channels"][0]["floor"], 13.);
                }
                let checker: Vec<u8> = (0..count)
                    .flat_map(|i| {
                        if (i % side as usize + i / side as usize).is_multiple_of(2) {
                            [0, 255, 0, 255]
                        } else {
                            [255, 0, 255, 255]
                        }
                    })
                    .collect();
                let mips = chain(&checker, side as usize);
                let coarse = &mips.levels[1];
                let wrong: Vec<u8> = (0..count)
                    .flat_map(|i| {
                        let x = i % side as usize / 2;
                        let y = i / side as usize / 2;
                        let offset = (y * coarse.width as usize + x) * 4;
                        coarse.rgba[offset..offset + 4].iter().copied()
                    })
                    .collect();
                assert_eq!(wrong.len(), checker.len());
                assert!(
                    gate(&checker, &wrong).is_err(),
                    "wrong mip at right size {side}"
                );
            }
        }
    }

    #[test]
    fn smooth_wrong_mip_is_a_documented_psnr_blind_spot() {
        let side = 64usize;
        let rgba: Vec<u8> = (0..side * side)
            .flat_map(|i| {
                let x = (i % side) as u8;
                [96 + x, 96 + x, 96 + x, 255]
            })
            .collect();
        let mips = chain(&rgba, side);
        let coarse = &mips.levels[1];
        let wrong: Vec<u8> = (0..side * side)
            .flat_map(|i| {
                let offset = (i / side / 2 * coarse.width as usize + i % side / 2) * 4;
                coarse.rgba[offset..offset + 4].iter().copied()
            })
            .collect();
        let stats = channel_stats(&rgba, &wrong, &vec![1; side * side], side, 0);
        assert!(stats.psnr > 40., "{stats:?}");
        assert_ne!(rgba, wrong);
        for profile in [Profile::Fast, Profile::Balanced] {
            assert!(check_level(
                &rgba,
                &wrong,
                &vec![1; side * side],
                side as u32,
                TextureSlot::BaseColor,
                profile,
                0
            )
            .is_ok());
        }
        // A detail-sensitive witness sees the changed spatial derivatives:
        // the source's adjacent x differences are all1, the shifted mip's
        // nearest-upsampled differences alternate0/2. A calibrated spatial
        // detail/gradient check is needed to enforce this additional contract.
        assert!((0..side - 1).all(|x| rgba[(x + 1) * 4] - rgba[x * 4] == 1));
        assert!((0..side - 1).any(|x| wrong[(x + 1) * 4] == wrong[x * 4]));
    }

    #[test]
    fn small_level_relaxation_is_colour_only_and_keeps_the_block_guard() {
        // 16x16 = 256 supported texels: a small level.
        let expected = [100u8, 120, 140, 255].repeat(256);
        let mut offset = expected.clone();
        for px in offset.as_chunks_mut::<4>().0 {
            px[2] += 30;
        }
        // Uniform 30-level error: PSNR 18.59 dB passes the middle ETC1S
        // band (16) but misses the large-level floor (20).
        assert!(check_level(
            &expected,
            &offset,
            &[1; 256],
            16,
            TextureSlot::BaseColor,
            Profile::Balanced,
            4
        )
        .is_ok());
        let big = [100u8, 120, 140, 255].repeat(1024);
        let mut big_offset = big.clone();
        for px in big_offset.as_chunks_mut::<4>().0 {
            px[2] += 30;
        }
        assert!(check_level(
            &big,
            &big_offset,
            &[1; 1024],
            32,
            TextureSlot::BaseColor,
            Profile::Balanced,
            3
        )
        .is_err());
        let normal = [128u8, 128, 255, 255].repeat(256);
        let mut normal_offset = normal.clone();
        for px in normal_offset.as_chunks_mut::<4>().0 {
            px[2] -= 17;
        }
        assert!(check_level(
            &normal,
            &normal_offset,
            &[1; 256],
            16,
            TextureSlot::Normal,
            Profile::Fast,
            4
        )
        .is_err());
        // Small data levels keep the full 30 dB floor: a uniform 17-level roughness
        // error (23.5 dB) is refused at any size.
        let data = [100u8, 120, 140, 255].repeat(256);
        let mut wrong = data.clone();
        for px in wrong.as_chunks_mut::<4>().0 {
            px[1] += 17;
        }
        assert!(check_level(
            &data,
            &wrong,
            &[1; 256],
            16,
            TextureSlot::MetallicRoughness,
            Profile::Balanced,
            4
        )
        .is_err());
        // Garbage on a small colour level is still refused.
        let garbage: Vec<u8> = expected
            .iter()
            .enumerate()
            .map(|(i, v)| if i % 4 == 3 { *v } else { v ^ 0x80 })
            .collect();
        assert!(check_level(
            &expected,
            &garbage,
            &[1; 256],
            16,
            TextureSlot::BaseColor,
            Profile::Balanced,
            4
        )
        .is_err());
    }
    #[test]
    fn normal_rgb_gate_detects_radial_error_below_fast_length_cap() {
        // Radial Z perturbation passes fast meanlength.08/max.20 and
        // angular3/12, but stored Z PSNR<30. RGB floor adds protection.
        let expected = [128, 128, 255, 255];
        let mutant = [128, 128, 246, 255];
        let error = check_level(
            &expected,
            &mutant,
            &[1],
            1,
            TextureSlot::Normal,
            Profile::Fast,
            0,
        )
        .unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
    }
    #[test]
    fn coupled_colour_guard_disabled_native_outcome_honest() {
        use super::super::mask::SurfaceSample;
        for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
            let samples = [
                SurfaceSample {
                    uv: [0.5, 0.5],
                    alpha: 1.,
                    weight: 1.,
                    source_pass: true,
                },
                SurfaceSample {
                    uv: [0.5, 0.5],
                    alpha: 0.999,
                    weight: 1.,
                    source_pass: false,
                },
            ];
            // Original204/255 passes1 but fails.999 at cutoff.8; candidate205
            // passes both. Independent pre-code classification counterexample.
            let rgba = [90, 90, 90, 204].repeat(64);
            match encode(
                [8, 8],
                &rgba,
                &[1; 64],
                TextureSlot::BaseColor,
                profile,
                128 * 1024 * 1024,
                Some(MaskDelivery {
                    samples: &samples,
                    factor: 1.,
                    cutoff: 0.8,
                    linear: true,
                    min_linear: true,
                }),
                LADDER,
            ) {
                Err(error) => {
                    assert_eq!(error.kind(), JobErrorKind::Unsupported);
                    println!("unsafe-coupled-guard typedrefusal {profile:?} {error:?}");
                }
                Ok(out) => {
                    let guards: Vec<_> = out
                        .receipts
                        .iter()
                        .filter(|r| r.get("finite_sample_guard_enabled").is_some())
                        .collect();
                    assert_eq!(guards.len(), 4);
                    for receipt in guards {
                        assert_eq!(receipt["finite_sample_guard_enabled"], false);
                        assert_eq!(receipt["classifications_preserved"], false);
                    }
                    let independent = basisu::Transcoder::new(&out.bytes).unwrap();
                    let (mut w, mut h) = (8, 8);
                    for mip in 0..4 {
                        let decoded = independent
                            .transcode(mip, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
                            .unwrap();
                        let first = centre_coverage(&decoded, w, h, 1., 0.8, [1., 0.]);
                        let second = centre_coverage(&decoded, w, h, 0.999, 0.8, [1., 0.]);
                        let actual = (first + second) / 2.;
                        let floor = match profile {
                            Profile::Fast => 0.02,
                            Profile::Balanced => 0.01,
                            _ => 0.005,
                        };
                        assert!((actual - 0.5).abs() <= floor);
                        println!("unsafe-coupled-guard honestnative {profile:?} mip{mip} target.5 gap0 actual{actual}");
                        w = (w / 2).max(1);
                        h = (h / 2).max(1);
                    }
                }
            }
        }
    }

    fn hash64(x: usize, y: usize, s: usize) -> u64 {
        let mut h = (x as u64).wrapping_mul(0x9E37_79B9)
            ^ (y as u64).wrapping_mul(0x85EB_CA6B)
            ^ (s as u64).wrapping_mul(0x27D4_EB2F);
        h ^= h >> 13;
        h = h.wrapping_mul(0xC2B2_AE35);
        h ^ (h >> 16)
    }
    /// [`photo_page`] with every `every`-th 4x4 block (by hash) replaced by
    /// four random colours scattered over its texels. UASTC's fastest pack
    /// level misses the partitions such blocks need; stronger levels find them.
    pub(super) fn hard_block_page(edge: usize, every: u64) -> Vec<u8> {
        let photo = photo_page(edge);
        let mut out = Vec::with_capacity(edge * edge * 4);
        for i in 0..edge * edge {
            let (x, y) = (i % edge, i / edge);
            if hash64(x / 4, y / 4, 77).is_multiple_of(every) {
                let c = hash64(x / 4, y / 4, (hash64(x, y, 9) % 4) as usize);
                out.extend([c as u8, (c >> 8) as u8, (c >> 16) as u8, 255]);
            } else {
                out.extend_from_slice(&photo[i * 4..i * 4 + 4]);
            }
        }
        out
    }
    /// Independent uniform noise in every colour channel: no lossy step meets
    /// the colour gate.
    fn noise_page(edge: usize) -> Vec<u8> {
        (0..edge * edge)
            .flat_map(|i| {
                let (x, y) = (i % edge, i / edge);
                [
                    hash64(x, y, 0) as u8,
                    hash64(x, y, 1) as u8,
                    hash64(x, y, 2) as u8,
                    255,
                ]
            })
            .collect()
    }
    const MEMORY: u64 = 128 * 1024 * 1024;
    fn chain(rgba: &[u8], edge: usize) -> sparse_mips::Chain {
        sparse_mips::build(
            edge as u32,
            edge as u32,
            rgba,
            &vec![1; edge * edge],
            Class::Colour,
            MEMORY,
        )
        .unwrap()
    }
    /// The delivered page against the authored chain, decoded by the
    /// independent transcoder (KTX2) or PNG decoder, under the run gates.
    fn independently_gated(out: &Delivery, chain: &sparse_mips::Chain, profile: Profile) {
        let edge = chain.levels[0].width as usize;
        if out.mime == "image/png" {
            let decoded = image::load_from_memory_with_format(&out.bytes, image::ImageFormat::Png)
                .unwrap()
                .into_rgba8();
            assert_eq!(decoded.as_raw(), &chain.levels[0].rgba, "PNG is exact");
            assert_eq!(out.mip_count, 1);
            return;
        }
        let decoded = decoded_levels(out, edge);
        assert_eq!(decoded.len(), chain.levels.len());
        for (mip, (level, decoded)) in chain.levels.iter().zip(&decoded).enumerate() {
            check_level(
                &level.rgba,
                decoded,
                &level.support,
                level.width,
                TextureSlot::BaseColor,
                profile,
                mip,
            )
            .unwrap();
        }
    }
    #[test]
    fn gate_misses_escalate_one_step_at_a_time_to_a_passing_step() {
        use Step::{LosslessPng, UastcLevel2, UastcLevel3};
        let profile = Profile::Balanced;
        for (edge, rgba, step, missed) in [
            (
                128,
                hard_block_page(128, 10),
                UastcLevel2,
                vec![Step::Profile],
            ),
            (
                128,
                hard_block_page(128, 3),
                UastcLevel2,
                vec![Step::Profile],
            ),
            (
                64,
                noise_page(64),
                LosslessPng,
                vec![Step::Profile, UastcLevel2, UastcLevel3],
            ),
        ] {
            let support = vec![1; edge * edge];
            let dimensions = [edge as u32; 2];
            let out = encode(
                dimensions,
                &rgba,
                &support,
                TextureSlot::BaseColor,
                profile,
                MEMORY,
                None,
                LADDER,
            )
            .unwrap();
            let escalation = out.escalation.as_ref().expect("escalated");
            assert_eq!(escalation.step, step);
            assert_eq!(
                escalation.misses.iter().map(|m| m.step).collect::<Vec<_>>(),
                missed
            );
            for miss in &escalation.misses {
                let mip = miss.mip.expect("per-level gate");
                assert!(miss.gate.contains(&format!("mip{mip} ")), "{miss:?}");
            }
            independently_gated(&out, &chain(&rgba, edge), profile);
            let last = out.receipts.last().unwrap();
            assert_eq!(last["step"], step.name());
        }
        // A page that meets its gate at the profile step is not escalated.
        let rgba = photo_page(64);
        let out = encode(
            [64; 2],
            &rgba,
            &[1; 64 * 64],
            TextureSlot::BaseColor,
            profile,
            MEMORY,
            None,
            LADDER,
        )
        .unwrap();
        assert!(out.escalation.is_none());
        assert_eq!(out.receipts.last().unwrap()["step"], "profile");
    }
    /// Corrupts one 4x4 block of decoded level 0 at the listed steps. A ladder
    /// that accepted any step without its gates would deliver that step.
    fn corrupt_at(steps: &[Step]) -> impl Fn(Step, usize, &mut [u8]) + '_ {
        move |step, mip, level| {
            if mip == 0 && steps.contains(&step) {
                let edge = (level.len() / 4).isqrt();
                for y in 8..12 {
                    for x in 8..12 {
                        for c in 0..3 {
                            level[(y * edge + x) * 4 + c] ^= 0x80;
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn every_ladder_step_is_rechecked_by_the_same_gates() {
        use Step::{LosslessPng, UastcLevel2, UastcLevel3};
        let profile = Profile::Balanced;
        let rgba = hard_block_page(128, 10);
        let chain = chain(&rgba, 128);
        let run = |steps: &[Step]| {
            escalate(
                &chain,
                TextureSlot::BaseColor,
                profile,
                MEMORY,
                None,
                LADDER,
                |_| Ok(Value::Null),
                &corrupt_at(steps),
            )
        };
        // Unmodified: delivered at level 2.
        assert_eq!(run(&[]).unwrap().escalation.unwrap().step, UastcLevel2);
        // A corrupted level-2 decode is refused and the ladder climbs on.
        for (steps, delivered) in [
            (&[UastcLevel2][..], UastcLevel3),
            (&[UastcLevel2, UastcLevel3][..], LosslessPng),
        ] {
            let out = run(steps).unwrap();
            let escalation = out.escalation.as_ref().unwrap();
            assert_eq!(escalation.step, delivered);
            for &step in steps {
                let miss = escalation.misses.iter().find(|m| m.step == step).unwrap();
                assert_eq!(miss.mip, Some(0));
                assert!(miss.gate.contains("blockRMSE"), "{miss:?}");
            }
            independently_gated(&out, &chain, profile);
        }
        // A PNG that does not decode to its input is a hard failure, never
        // delivered, and names the level.
        let error = run(&[UastcLevel2, UastcLevel3, LosslessPng]).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::InvalidState);
        assert!(error.message().contains("mip0"), "{error}");
    }
    #[test]
    fn escalation_polls_cancellation_before_every_re_encode() {
        let polls = std::cell::Cell::new(0);
        let cancelled = || {
            polls.set(polls.get() + 1);
            Err(JobError::new(JobErrorKind::Cancelled, "cancelled"))
        };
        let ladder = Ladder { check: &cancelled };
        let rgba = hard_block_page(128, 10);
        let error = encode(
            [128; 2],
            &rgba,
            &[1; 128 * 128],
            TextureSlot::BaseColor,
            Profile::Balanced,
            MEMORY,
            None,
            ladder,
        )
        .unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::Cancelled);
        assert_eq!(polls.get(), 1);
        // A page that passes at the profile step never re-encodes.
        let rgba = photo_page(64);
        encode(
            [64; 2],
            &rgba,
            &[1; 64 * 64],
            TextureSlot::BaseColor,
            Profile::Balanced,
            MEMORY,
            None,
            ladder,
        )
        .unwrap();
        assert_eq!(polls.get(), 1);
    }
    /// Every step runs under the page's one lease: the lossy charge covers
    /// the PNG step's workspace too.
    #[test]
    fn lossy_page_charge_covers_the_png_step() {
        for w in (2..=13).map(|e| 1u32 << e) {
            for h in (2..=13).map(|e| 1u32 << e) {
                for profile in [Profile::Fast, Profile::Balanced, Profile::Quality] {
                    assert!(
                        workspace_charge([w, h], profile).unwrap()
                            >= workspace_charge([w, h], Profile::Lossless).unwrap()
                    );
                }
            }
        }
    }

    /// The sharp modular pattern that the codec's own detail-correlation floor
    /// refuses at the profile step (found by a leaf-witness fixture).
    fn sharp_page(edge: usize) -> Vec<u8> {
        (0..edge * edge)
            .flat_map(|i| {
                let (x, y) = (i % edge, i / edge);
                let v = |k: usize| ((x * 7 + y * 13 + k * 31) % 97 + 80) as u8;
                [v(0), v(1), v(2), 255]
            })
            .collect()
    }
    #[test]
    fn codec_and_class_verification_misses_follow_the_selected_codec() {
        let profile = Profile::Balanced;
        let edge = 64;
        let rgba = sharp_page(edge);
        let chain = chain(&rgba, edge);
        // The codec check is not weakened: the profile setting is still refused by it.
        let levels: Vec<_> = chain
            .levels
            .iter()
            .map(|l| codec::MipLevel {
                rgba: &l.rgba,
                width: l.width,
                height: l.height,
            })
            .collect();
        // The former UASTC profile still refuses this exact detail witness.
        let mode = codec::Mode::Uastc { zstd: true };
        let params = codec::Params {
            effort: 1,
            transfer: codec::Transfer::Srgb,
            mipmaps: false,
        };
        let refused = codec::encode_ktx2_mips_decoded(&levels, mode, params).unwrap_err();
        assert_eq!(refused.kind(), JobErrorKind::InvalidState);
        assert!(refused
            .message()
            .contains("detail does not follow the input"));
        // ETC1S retains its own corruption floor; this witness instead misses
        // the colour-class gate and is delivered at UASTC level 2.
        let out = encode(
            [edge as u32; 2],
            &rgba,
            &vec![1; edge * edge],
            TextureSlot::BaseColor,
            profile,
            MEMORY,
            None,
            LADDER,
        )
        .unwrap();
        let escalation = out.escalation.as_ref().unwrap();
        assert_eq!(escalation.step, Step::UastcLevel2);
        assert_eq!(escalation.misses.len(), 1);
        let miss = &escalation.misses[0];
        assert_eq!(
            (miss.step, miss.mip, miss.reason),
            (Step::Profile, Some(0), MissReason::Gate)
        );
        assert!(miss.gate.contains("PSNR"), "{miss:?}");
        // The delivered step passes the codec's own verification and the run gates.
        let (mode, params) =
            step_settings(TextureSlot::BaseColor, profile, Step::UastcLevel2).unwrap();
        codec::verify_ktx2_mips(&out.bytes, &levels, mode, params).unwrap();
        independently_gated(&out, &chain, profile);
    }
    #[test]
    fn doomed_uastc_steps_are_decided_by_block_probes() {
        // Binary metallic noise: UASTC level 2 misses the data cap at mip1, so level 3 is
        // decided by re-encoding level 2's witness blocks, not the whole chain.
        let bit = |i: u32, k: u32| ((i.wrapping_mul(2_654_435_761) >> k) & 1) as u8 * 230;
        let rgba: Vec<u8> = (0..64 * 64u32)
            .flat_map(|i| [0, bit(i, 7), bit(i, 13), 255])
            .collect();
        let chain = sparse_mips::build(64, 64, &rgba, &[1; 64 * 64], Class::Data, MEMORY).unwrap();
        let out = escalate(
            &chain,
            TextureSlot::MetallicRoughness,
            Profile::Balanced,
            MEMORY,
            None,
            LADDER,
            |_| Ok(Value::Null),
            &|_, _, _| {},
        )
        .unwrap();
        let escalation = out.escalation.unwrap();
        assert_eq!(escalation.step, Step::LosslessPng);
        let probed: Vec<_> = escalation.misses[1..]
            .iter()
            .map(|m| (m.step, m.gate.starts_with("block probe: ")))
            .collect();
        assert_eq!(
            probed,
            [(Step::UastcLevel2, false), (Step::UastcLevel3, true)],
            "{escalation:?}"
        );
    }
    #[test]
    fn a_level_alone_decodes_as_in_its_chain() {
        // The level probe's premise: UASTC encodes each 4x4 block alone, so a level
        // encoded on its own decodes exactly as inside the complete chain.
        let rgba = hard_block_page(128, 10);
        let chain =
            sparse_mips::build(128, 128, &rgba, &[1; 128 * 128], Class::Data, MEMORY).unwrap();
        let levels: Vec<_> = chain
            .levels
            .iter()
            .map(|l| codec::MipLevel {
                rgba: &l.rgba,
                width: l.width,
                height: l.height,
            })
            .collect();
        for step in [Step::UastcLevel2, Step::UastcLevel3] {
            let (mode, params) =
                step_settings(TextureSlot::MetallicRoughness, Profile::Balanced, step).unwrap();
            let (_, whole) = codec::encode_ktx2_mips_decoded(&levels, mode, params).unwrap();
            for (index, level) in levels.iter().enumerate() {
                let (_, alone) = codec::encode_ktx2_mips_decoded(&[*level], mode, params).unwrap();
                assert_eq!(alone[0], whole[index], "{step:?} mip{index}");
            }
            // Blocks 5, 9 and 30 of level 0 side by side, as the block probe lays them.
            let blocks = [5, 9, 30];
            let row = block_row(levels[0].rgba, 128, &blocks);
            let alone = codec::encode_decoded_unverified(&row, 12, 4, mode, params).unwrap();
            assert_eq!(alone, block_row(&whole[0], 128, &blocks), "{step:?} blocks");
        }
    }
    #[test]
    fn exhausted_mip_sampled_ladder_ends_in_exact_level0_png() {
        use Step::{LosslessPng, UastcLevel2, UastcLevel3};
        let profile = Profile::Balanced;
        // A data page with an authored chain (texture-1e5's metallicRoughness): every
        // lossy step misses, so the page ships as its exact level 0 instead of failing.
        let rgba = sharp_page(64);
        let chain = sparse_mips::build(64, 64, &rgba, &[1; 64 * 64], Class::Data, MEMORY).unwrap();
        assert!(chain.levels.len() > 1);
        let out = escalate(
            &chain,
            TextureSlot::MetallicRoughness,
            profile,
            MEMORY,
            None,
            LADDER,
            |_| Ok(Value::Null),
            &corrupt_at(&[Step::Profile, UastcLevel2, UastcLevel3]),
        )
        .unwrap();
        assert_eq!((out.mime, out.mip_count), ("image/png", 1));
        let escalation = out.escalation.unwrap();
        assert_eq!(escalation.step, LosslessPng);
        assert_eq!(
            escalation.misses.iter().map(|m| m.step).collect::<Vec<_>>(),
            [Step::Profile, UastcLevel2, UastcLevel3]
        );
        let decoded = image::load_from_memory(&out.bytes).unwrap().into_rgba8();
        assert_eq!(decoded.into_raw(), chain.levels[0].rgba);
        assert!(out
            .receipts
            .iter()
            .any(|r| r["mips"] == "viewer generated outside guarantee"));
    }
}
