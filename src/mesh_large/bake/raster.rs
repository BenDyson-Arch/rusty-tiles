//! Finite geometric coverage, including positive-area intersections containing
//! no texel centre. Support comes from clipped area, never copied gutters.
use super::{
    charts::Layout,
    frames::{self, Frame},
    invalid,
    math::{self, V3},
    resource,
    sampling::{ChildQuery, ProxySample},
    source::CapturedSource,
    types::{
        slot_index, BakeLimits, FrozenNode, NormalSample, SignalLevel, SignalPage, TerminalSample,
    },
    unsupported,
};
use crate::{gltf_source::source::TextureSlot, mesh_large::leaves::mask, JobError};
type P2 = [f64; 2];
const SLOTS: [TextureSlot; 5] = [
    TextureSlot::BaseColor,
    TextureSlot::MetallicRoughness,
    TextureSlot::Normal,
    TextureSlot::Occlusion,
    TextureSlot::Emissive,
];
pub(super) struct Raster {
    pub pages: Vec<SignalPage>,
    pub face_pages: Vec<[Option<usize>; 5]>,
    pub frames: Vec<[Frame; 3]>,
    pub normal_attributes: Vec<[[f32; 3]; 3]>,
    pub tangent_attributes: Vec<[[f32; 4]; 3]>,
    pub colors: Vec<[[f32; 4]; 3]>,
    pub masks: Vec<Vec<mask::SurfaceSample>>,
    pub owner: Vec<Vec<usize>>,
    pub receipt: serde_json::Value,
    /// Run diagnostics (never output bytes): samples, and faces with at least one
    /// sample, whose witness took the facing fallback; samples that took the fill one.
    pub facing_fallback_samples: u64,
    pub facing_fallback_faces: u64,
    pub fill_fallback_samples: u64,
    /// Density reported to the parent: area-weighted median decoded spacing, at least
    /// the planned texel.
    pub texel_size: f64,
}
fn identity() -> Frame {
    Frame {
        normal: [0., 0., 1.],
        tangent: [1., 0., 0.],
        bitangent: [0., 1., 0.],
    }
}
fn empty() -> TerminalSample {
    TerminalSample {
        source_rank: u64::MAX,
        barycentric: [1., 0., 0.],
        centre: [0.; 3],
        support_min: [0.; 3],
        support_max: [0.; 3],
        weight: 0,
        centre_is_centroid: false,
    }
}
fn orient(a: P2, b: P2, c: P2) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn bary(p: P2, t: [P2; 3]) -> V3 {
    let d = orient(t[0], t[1], t[2]);
    let a = orient(t[1], t[2], p) / d;
    let b = orient(t[2], t[0], p) / d;
    [a, b, 1. - a - b]
}
fn clip(poly: &[P2], axis: usize, bound: f64, greater: bool) -> Vec<P2> {
    let mut out = Vec::with_capacity(7);
    if poly.is_empty() {
        return out;
    }
    let inside = |p: P2| {
        if greater {
            p[axis] >= bound
        } else {
            p[axis] <= bound
        }
    };
    let mut previous = *poly.last().unwrap();
    for &p in poly {
        let a = inside(previous);
        let b = inside(p);
        if a != b {
            let f = (bound - previous[axis]) / (p[axis] - previous[axis]);
            let mut q = std::array::from_fn(|i| previous[i] + f * (p[i] - previous[i]));
            q[axis] = bound;
            out.push(q);
        }
        if b {
            out.push(p);
        }
        previous = p;
    }
    out
}
pub(super) fn coverage(t: [P2; 3], x: u32, y: u32) -> (f64, P2) {
    let mut p = t.to_vec();
    for (a, b, g) in [
        (0, x as f64, true),
        (0, x as f64 + 1., false),
        (1, y as f64, true),
        (1, y as f64 + 1., false),
    ] {
        p = clip(&p, a, b, g);
    }
    if p.len() < 3 {
        return (0., [0.; 2]);
    }
    let mut area = 0.;
    let mut centre = [0.; 2];
    for i in 1..p.len() - 1 {
        let a = orient(p[0], p[i], p[i + 1]).abs() * 0.5;
        area += a;
        for axis in 0..2 {
            centre[axis] += (p[0][axis] + p[i][axis] + p[i + 1][axis]) * a / 3.;
        }
    }
    if area <= 0. {
        (0., [0.; 2])
    } else {
        (area, centre.map(|v| v / area))
    }
}
fn linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(v: f64) -> f64 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }
}
fn byte(v: f64) -> u8 {
    (v.clamp(0., 1.) * 255.).round() as u8
}
/// One page [`build`] makes: its size, slot, and the raster workspace it charges.
pub(super) struct PlannedPage {
    pub size: [u32; 2],
    pub slot: TextureSlot,
    pub raster_charge: u64,
}
/// The pages [`build`] makes from `layout`, in its order: each layout page with a face
/// of nonzero area, once per texture slot its material binds.
pub(super) fn planned_pages(
    f: &FrozenNode,
    layout: &Layout,
    source: &CapturedSource<'_>,
) -> Result<Vec<PlannedPage>, JobError> {
    let mut out = Vec::new();
    for plan in &layout.pages {
        if !plan.charts.iter().any(|&ci| {
            layout.charts[ci].faces.iter().any(|&fi| {
                math::face_normal(f.faces[fi].positions.map(|p| p.map(f64::from))).is_some()
            })
        }) {
            continue;
        }
        let bindings: Vec<_> = if plan.material == u32::MAX {
            Vec::new()
        } else {
            source
                .captured
                .materials
                .get(plan.material as usize)
                .ok_or_else(|| invalid("immutable source material closure"))?
                .bindings()
                .copied()
                .collect()
        };
        let material = source.material(plan.material)?;
        for slot in SLOTS {
            if bindings.iter().any(|b| b.slot == slot) {
                let mask = slot == TextureSlot::BaseColor && material["alphaMode"] == "MASK";
                out.push(PlannedPage {
                    size: plan.size,
                    slot,
                    raster_charge: raster_charge(plan.size, slot, mask)?,
                });
            }
        }
    }
    Ok(out)
}
/// The raster workspace [`build`] charges for a page of `size` and `slot` (`mask`: a
/// MASK material's base colour).
fn raster_charge(size: [u32; 2], slot: TextureSlot, mask: bool) -> Result<u64, JobError> {
    (u64::from(size[0]) * u64::from(size[1]))
        .checked_mul(if slot == TextureSlot::Normal {
            4096
        } else if mask {
            2048
        } else {
            512
        })
        .ok_or_else(|| resource("raster workspace"))
}
/// The bytes [`build`]'s page of `size` and `slot` owns once rasterised, before its
/// mips: base RGBA, support and terminal samples twice (the page and its base level),
/// the base level's frames (and world normals on a normal page), and the texel owners.
/// Its renderer witnesses and MASK samples come on top.
pub(super) fn page_bytes(size: [u32; 2], slot: TextureSlot) -> Option<u64> {
    let per_texel = 2 * (4 + 4 + std::mem::size_of::<TerminalSample>())
        + std::mem::size_of::<Frame>()
        + std::mem::size_of::<usize>()
        + if slot == TextureSlot::Normal { 24 } else { 0 };
    (u64::from(size[0]) * u64::from(size[1])).checked_mul(per_texel as u64)
}
pub(super) fn build(
    f: &FrozenNode,
    layout: &Layout,
    source: &mut CapturedSource<'_>,
    children: &mut ChildQuery<'_>,
    limits: BakeLimits,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Raster, JobError> {
    let generated = frames::generate(f, layout, limits.memory_bytes, limits.query_work, check)?;
    let frames = generated.frames;
    let normal_attributes = generated.normals;
    let tangent_attributes = generated.tangents;
    let mut colors = Vec::with_capacity(f.faces.len());
    for face in &f.faces {
        check()?;
        let mut c = [[1.; 4]; 3];
        for (j, color) in c.iter_mut().enumerate() {
            *color = source.color(face.source, face.source_triangle, face.welds[j], check)?;
        }
        colors.push(c);
    }
    let mut pages = Vec::new();
    let mut masks = Vec::new();
    let mut owners = Vec::new();
    let mut face_pages = vec![[None; 5]; f.faces.len()];
    let mut direct = 0u64;
    let mut recursive = 0u64;
    let mut fallback = 0u64;
    let mut facing_fallback = 0u64;
    let mut facing_faces = vec![false; f.faces.len()];
    let mut displacement = 0f64;
    let mut drift = 0f64;
    let mut witness_distance = 0f64;
    let mut support_distance = 0f64;
    let mut centroid_samples = 0u64;
    let mut area_error = 0f64;
    let mut charged = (f.faces.len() as u64)
        .checked_mul(2048)
        .ok_or_else(|| resource("corner attributes lease overflow"))?;
    let mut maximum_texel = 0f64;
    let mut support_scales = Vec::new();
    for plan in &layout.pages {
        if !plan.charts.iter().any(|&ci| {
            layout.charts[ci].faces.iter().any(|&fi| {
                math::face_normal(f.faces[fi].positions.map(|p| p.map(f64::from))).is_some()
            })
        }) {
            continue;
        }
        let material = source.material(plan.material)?;
        let bindings: Vec<_> = if plan.material == u32::MAX {
            Vec::new()
        } else {
            source.captured.materials[plan.material as usize]
                .bindings()
                .copied()
                .collect()
        };
        for slot in SLOTS {
            if !bindings.iter().any(|b| b.slot == slot) {
                continue;
            }
            check()?;
            let [width, height] = plan.size;
            let count = (width as usize)
                .checked_mul(height as usize)
                .ok_or_else(|| resource("raster size"))?;
            let mask = slot == TextureSlot::BaseColor && material["alphaMode"] == "MASK";
            let charge = raster_charge([width, height], slot, mask)?;
            charged = charged
                .checked_add(charge)
                .ok_or_else(|| resource("raster owning sum"))?;
            if charged > limits.memory_bytes {
                return Err(resource("raster semantic pages exceed lease"));
            }
            let q = (u64::from(u32::MAX)
                .checked_sub(count as u64)
                .ok_or_else(|| resource("coverage support count"))?
                / (count as u64))
                .clamp(1, 65536) as f64;
            support_scales.push(q);
            let mut sums = vec![[0.; 4]; count];
            let mut weights = vec![0.; count];
            let mut samples = vec![empty(); count];
            let mut normal_frames = vec![identity(); count];
            let mut dominant = vec![0.; count];
            let mut dominant_signal = vec![[0., 0., 1.]; count];
            let mut owner = vec![usize::MAX; count];
            let mut surfaces = Vec::new();
            let mut normal_samples = Vec::new();
            // Faces in source order (material, source primitive, face), not packing
            // order: charts span a class's sources, and the shared decoded-image cache
            // keeps its hits only while one source's faces are rasterised together.
            let mut visit: Vec<(usize, usize)> = plan
                .charts
                .iter()
                .flat_map(|&ci| layout.charts[ci].faces.iter().map(move |&fi| (ci, fi)))
                .collect();
            visit.sort_unstable_by_key(|&(_, fi)| (f.faces[fi].material, f.faces[fi].source, fi));
            for (ci, fi) in visit {
                check()?;
                let face = &f.faces[fi];
                let points = face.positions.map(|p| p.map(f64::from));
                let Some(normal) = math::face_normal(points) else {
                    continue;
                };
                let t = layout.uv[fi].map(|p| {
                    [
                        f64::from(p[0]) * f64::from(width),
                        f64::from(p[1]) * f64::from(height),
                    ]
                });
                let actual_texel = math::pixel_size(points, t)
                    .ok_or_else(|| unsupported("decoded proxy UV has no finite pixel footprint"))?;
                maximum_texel = maximum_texel.max(actual_texel);
                let lo = std::array::from_fn::<_, 2, _>(|a| {
                    t.iter()
                        .map(|p| p[a])
                        .fold(f64::INFINITY, f64::min)
                        .floor()
                        .max(0.) as u32
                });
                let hi = std::array::from_fn::<_, 2, _>(|a| {
                    t.iter()
                        .map(|p| p[a])
                        .fold(f64::NEG_INFINITY, f64::max)
                        .ceil()
                        .max(0.) as u32
                });
                for y in lo[1]..hi[1].min(height) {
                    for x in lo[0]..hi[0].min(width) {
                        check()?;
                        let (area, centroid) = coverage(t, x, y);
                        if area <= 0. {
                            continue;
                        }
                        let mut points2 = Vec::with_capacity(16);
                        for sy in 0..4 {
                            for sx in 0..4 {
                                let p = [
                                    f64::from(x) + (sx as f64 + 0.5) / 4.,
                                    f64::from(y) + (sy as f64 + 0.5) / 4.,
                                ];
                                let b = bary(p, t);
                                if b.iter().all(|w| *w >= 0.) {
                                    points2.push((p, b));
                                }
                            }
                        }
                        if points2.is_empty() {
                            let b = bary(centroid, t);
                            points2.push((centroid, b));
                        }
                        let weight = area / points2.len() as f64;
                        let i = y as usize * width as usize + x as usize;
                        for (uv, bary) in points2 {
                            let bary = bary.map(|w| w.clamp(0., 1.));
                            let sum = bary.iter().sum::<f64>();
                            let bary = bary.map(|w| w / sum);
                            let p = math::bary(points, bary);
                            let corners = frames[fi];
                            let frame = Frame::interpolate(
                                corners.map(|f| f.normal),
                                corners.map(|f| f.tangent),
                                corners.map(|f| f.bitangent),
                                bary,
                            )?;
                            let signal = children
                                .signal(
                                    source,
                                    ProxySample {
                                        face,
                                        position: p,
                                        geometric_normal: normal,
                                        frame,
                                        texel_size: actual_texel,
                                        geometric_error: f.geometric_error,
                                        depth: f.node.depth(),
                                    },
                                    slot,
                                    check,
                                )?
                                .ok_or_else(|| invalid("proxy slot missing from source closure"))?;
                            if signal.direct {
                                direct += 1;
                            } else {
                                recursive += 1;
                            }
                            if signal.fill_fallback {
                                fallback += 1;
                            }
                            if signal.facing_fallback {
                                facing_fallback += 1;
                                facing_faces[fi] = true;
                            }
                            displacement = displacement.max(signal.displacement);
                            drift = drift.max(signal.drift);
                            witness_distance = witness_distance.max(signal.witness_distance);
                            support_distance = support_distance.max(signal.support_distance);
                            if signal.terminal.centre_is_centroid {
                                centroid_samples += 1;
                            }
                            let rgb = if slot == TextureSlot::Normal {
                                frame.transform([
                                    2. * signal.rgba[0] - 1.,
                                    2. * signal.rgba[1] - 1.,
                                    2. * signal.rgba[2] - 1.,
                                ])?
                            } else {
                                std::array::from_fn(|a| {
                                    if matches!(
                                        slot,
                                        TextureSlot::BaseColor | TextureSlot::Emissive
                                    ) {
                                        linear(signal.rgba[a])
                                    } else {
                                        signal.rgba[a]
                                    }
                                })
                            };
                            if slot == TextureSlot::Normal {
                                normal_samples.push(NormalSample {
                                    uv: [uv[0] / f64::from(width), uv[1] / f64::from(height)],
                                    frame,
                                    chart: ci,
                                    weight,
                                });
                            }
                            if weights[i] == 0. {
                                samples[i] = signal.terminal;
                                samples[i].centre = [0.; 3];
                                samples[i].support_min = [f64::INFINITY; 3];
                                samples[i].support_max = [f64::NEG_INFINITY; 3];
                            }
                            if weights[i] > 0. || signal.terminal.centre_is_centroid {
                                samples[i].centre_is_centroid = true;
                            }
                            for a in 0..3 {
                                sums[i][a] += rgb[a] * weight;
                                samples[i].centre[a] += signal.terminal.centre[a] * weight;
                                samples[i].support_min[a] =
                                    samples[i].support_min[a].min(signal.terminal.support_min[a]);
                                samples[i].support_max[a] =
                                    samples[i].support_max[a].max(signal.terminal.support_max[a]);
                            }
                            sums[i][3] += signal.rgba[3] * weight;
                            weights[i] += weight;
                            if weight > dominant[i] {
                                dominant[i] = weight;
                                dominant_signal[i] = rgb;
                                normal_frames[i] = frame;
                                owner[i] = fi;
                                samples[i].source_rank = signal.terminal.source_rank;
                                samples[i].barycentric = signal.terminal.barycentric;
                            }
                            if mask {
                                let w = source.record(
                                    signal.terminal.source_rank,
                                    signal.terminal.barycentric,
                                )?;
                                let original = source
                                    .signal(w, TextureSlot::BaseColor, 0., check)?
                                    .ok_or_else(|| invalid("MASK source base binding"))?
                                    .0;
                                let original_alpha = if w.triangle.flags
                                    & crate::mesh_large::attributes::COLOR
                                    == 0
                                {
                                    1.
                                } else {
                                    (0..3)
                                        .map(|a| {
                                            f64::from(w.triangle.corners[a].color[3]) * w.bary[a]
                                        })
                                        .sum()
                                };
                                let factor = material
                                    .pointer("/pbrMetallicRoughness/baseColorFactor/3")
                                    .and_then(serde_json::Value::as_f64)
                                    .unwrap_or(1.);
                                let cutoff = material["alphaCutoff"].as_f64().unwrap_or(0.5);
                                let alpha =
                                    (0..3).map(|a| f64::from(colors[fi][a][3]) * bary[a]).sum();
                                surfaces.push(mask::SurfaceSample {
                                    uv: [uv[0] / f64::from(width), uv[1] / f64::from(height)],
                                    alpha,
                                    weight,
                                    source_pass: original[3] * original_alpha * factor >= cutoff,
                                });
                            }
                        }
                    }
                }
                face_pages[fi][slot_index(slot)] = Some(pages.len());
            }
            let mut rgba = vec![0; count * 4];
            let mut support = vec![0; count];
            let mut world_normals = if slot == TextureSlot::Normal {
                vec![[0., 0., 1.]; count]
            } else {
                Vec::new()
            };
            for i in 0..count {
                if weights[i] == 0. {
                    continue;
                }
                if weights[i] > 1. + 1e-8 {
                    return Err(invalid("proxy chart raster interior double coverage"));
                }
                let w = (weights[i] * q).round().max(1.);
                if w > u32::MAX as f64 {
                    return Err(resource("coverage fixed point overflow"));
                }
                support[i] = w as u32;
                area_error = area_error.max((w / q - weights[i]).abs());
                samples[i].weight = support[i];
                samples[i].centre = samples[i].centre.map(|v| v / weights[i]);
                let rgb = if slot == TextureSlot::Normal {
                    let world = math::unit([sums[i][0], sums[i][1], sums[i][2]])
                        .unwrap_or(dominant_signal[i]);
                    world_normals[i] = world;
                    normal_frames[i]
                        .inverse_direction(world)?
                        .map(|v| v * 0.5 + 0.5)
                } else {
                    std::array::from_fn(|a| {
                        let v = sums[i][a] / weights[i];
                        if matches!(slot, TextureSlot::BaseColor | TextureSlot::Emissive) {
                            srgb(v)
                        } else {
                            v
                        }
                    })
                };
                for a in 0..3 {
                    rgba[i * 4 + a] = byte(rgb[a]);
                }
                rgba[i * 4 + 3] = byte(sums[i][3] / weights[i]);
                if slot == TextureSlot::Normal {
                    let raw: [f64; 3] =
                        std::array::from_fn(|a| f64::from(rgba[i * 4 + a]) / 127.5 - 1.);
                    if (math::dot(raw, raw).sqrt() - 1.).abs() > 0.01 {
                        return Err(unsupported("preencode normal violates unit length floor"));
                    }
                }
            }
            let binding = bindings.iter().find(|b| b.slot == slot).unwrap();
            let texture = &source.captured.textures[binding.texture];
            let logical = crate::mesh_large::leaves::crops::resolve(
                texture
                    .get("sampler")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|n| source.captured.samplers.get(n as usize)),
            )?;
            let sampler = serde_json::json!({"wrapS":33071,"wrapT":33071,"magFilter":logical.mag_filter,"minFilter":logical.min_filter});
            let base = SignalLevel {
                width,
                height,
                rgba: rgba.clone(),
                support: support.clone(),
                frames: normal_frames,
                samples: samples.clone(),
                world_normals,
            };
            pages.push(SignalPage {
                width,
                height,
                rgba,
                support,
                slot,
                samples,
                levels: vec![base],
                sampler,
                material: plan.material,
                receipt: serde_json::json!({"supportQ":q}),
                normal_samples,
                normal_scale: 1.,
            });
            masks.push(surfaces);
            owners.push(owner);
        }
    }
    // The density this proxy reports to its parent: the world-area-weighted median of
    // its rasterised faces' decoded worst-direction spacing, never finer than planned.
    // A face whose 16-bit UVs land just above the collapse threshold has an unbounded
    // spacing, and the maximum (kept in the receipt) would coarsen the whole parent.
    let rasterised: u64 = layout.charts.iter().map(|c| c.faces.len() as u64).sum();
    charged = rasterised
        .checked_mul(16)
        .and_then(|b| charged.checked_add(b))
        .ok_or_else(|| resource("proxy density sample lease overflow"))?;
    if charged > limits.memory_bytes {
        return Err(resource("proxy density samples exceed lease"));
    }
    let mut density = Vec::with_capacity(rasterised as usize);
    for plan in &layout.pages {
        for &ci in &plan.charts {
            for &fi in &layout.charts[ci].faces {
                check()?;
                let points = f.faces[fi].positions.map(|p| p.map(f64::from));
                let t = layout.uv[fi].map(|p| {
                    [
                        f64::from(p[0]) * f64::from(plan.size[0]),
                        f64::from(p[1]) * f64::from(plan.size[1]),
                    ]
                });
                if let Some(s) = math::pixel_size(points, t) {
                    density.push((s, math::triangle_area(points)));
                }
            }
        }
    }
    let median_texel = math::weighted_median(&mut density).unwrap_or(0.);
    drop(density);
    let facing_fallback_faces = facing_faces.iter().filter(|&&b| b).count() as u64;
    Ok(Raster {
        pages,
        face_pages,
        frames,
        normal_attributes,
        tangent_attributes,
        colors,
        masks,
        owner: owners,
        texel_size: median_texel.max(layout.texel_size),
        facing_fallback_samples: facing_fallback,
        facing_fallback_faces,
        fill_fallback_samples: fallback,
        receipt: serde_json::json!({"maximumTexel":maximum_texel,"medianTexel":median_texel,"directSamples":direct,"recursiveSamples":recursive,"fillUnfilteredFallbackSamples":fallback,
        "facingFallbackSamples":facing_fallback,"facingFallbackFaces":facing_fallback_faces,
        "surfaceWitnessDisplacement":witness_distance,"supportDisplacementUpper":support_distance,"centroidSamples":centroid_samples,"witnessMeaning":"actual deterministic source-surface witness; centroid and support are separate, not a closest-source certificate",
        "appearanceDisplacement":displacement,"deepDriftMaxByte":drift,"supportAreaQuantizationMax":area_error,"supportQByPage":support_scales,"finiteQuadrature":"4x4 and clipped centroid","owningProvisionalBytes":charged}),
    })
}
