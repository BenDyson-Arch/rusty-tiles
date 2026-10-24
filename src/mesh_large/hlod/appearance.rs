//! Coarse tile appearance (D5, slice S3): per material-class group of a final decoded
//! coarse tile,
//!
//! - **textured classes** get fresh charts packed into one content-sized page per bound
//!   slot (all slots share the layout), texel spacing `max(G/16, 1e-5 m)` capped at
//!   `max_edge` per side, baked from the nearest captured source surface of the class
//!   ([`bake`]): base colour × COLOR_0 in linear light, emissive, metallic-roughness and
//!   occlusion raw, and normal maps re-expressed in the coarse tangent frame (the source
//!   `normalTexture.scale` absorbed, the coarse scale 1). Literal factors, alpha mode and
//!   cutoff, double-sidedness and occlusion strength stay the class's;
//! - **untextured classes** keep S1's flat class factors, plus a per-vertex COLOR_0 from
//!   the nearest source surface when the run carries COLOR_0 (the small vertex-colour
//!   path; colourless input searches nothing).
//!
//! NORMAL is published only when the run's source carries normals (leaves without them are
//! flat shaded, and the coarse tiles must match). Pages are handed to the shared leaf
//! delivery encoder ([`deliver`], S4 owns codec and mip delivery); [`CoarsePage`] is the
//! pre-encode page. Geometry, triangle order and representative IDs are never changed.
use super::{
    bake::{self, Hit, ImageCache, Queries},
    charts,
};
use crate::{
    gltf_source::source::TextureSlot,
    mesh_large::{
        attributes::COLOR,
        config::Profile,
        full::FullCaptured,
        leaves::{delivery, mask as delivery_mask, mips::Class},
    },
    JobError, JobErrorKind,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// Balanced coarse page cap per side (D5); the page is otherwise sized to its content.
pub(crate) const MAX_PAGE_EDGE: u32 = 1024;
/// Finest requested spacing (D5's `1e-5 m` floor).
const MIN_SPACING: f64 = 1e-5;

/// The requested texel spacing of a tile with published error `g`.
pub(crate) fn spacing(g: f64) -> f64 {
    (g / 16.).max(MIN_SPACING)
}

/// Stage-wide appearance services.
pub(crate) struct Services<'a> {
    pub captured: &'a FullCaptured,
    pub cache: &'a ImageCache,
    pub profile: Profile,
    pub max_edge: u32,
    /// Per-page encoder workspace ceiling (the job's admitted page charge).
    pub page_memory: u64,
    /// The run carries COLOR_0 / NORMAL somewhere.
    pub colors: bool,
    pub normals: bool,
    pub check: &'a (dyn Fn() -> Result<(), JobError> + Sync),
}

/// One coarse page before encoding: the S3 → S4 handoff.
pub(crate) struct CoarsePage {
    pub slot: TextureSlot,
    pub size: [u32; 2],
    /// RGBA8, colour classes sRGB-encoded; gutters arbitrary.
    pub rgba: Vec<u8>,
    /// 1 on covered texels, 0 on gutters (filled by the encoder at every level).
    pub support: Vec<u32>,
}

pub(crate) struct DeliveredPage {
    pub slot: TextureSlot,
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub escalated: bool,
    /// A KTX2 chain whose every level passed its gates (mipmapped sampler); otherwise
    /// the ladder's exact level-0 PNG, sampled without mips.
    pub mipmapped: bool,
}

/// One material group of a tile: one glTF primitive.
pub(crate) struct Group {
    pub class: u32,
    /// Tile face indices, ascending.
    pub faces: Vec<usize>,
    /// The coarse material; texture indices are group-local (page order).
    pub material: Value,
    pub uvs: Option<Vec<[[f32; 2]; 3]>>,
    pub tangents: Option<Vec<[[f32; 4]; 3]>>,
    pub colors: Option<Vec<[[f32; 4]; 3]>>,
    pub pages: Vec<DeliveredPage>,
}

/// Per-tile appearance facts (deterministic).
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TileAppearance {
    pub textured_groups: u32,
    pub vertex_color_groups: u32,
    pub pages: u32,
    /// Page texels of one slot's pages (each slot shares the layout).
    pub layout_texels: u64,
    /// Texels over every slot page.
    pub page_texels: u64,
    pub covered_texels: u64,
    pub charts: u32,
    pub requested_spacing: f64,
    /// Coarsest achieved spacing among the tile's textured groups (0 without one).
    pub achieved_spacing: f64,
    pub capped: bool,
    pub page_size: [u32; 2],
    /// Exact nearest searches (texels whose previous donor was not kept).
    pub searches: u64,
    pub donor_reads: u64,
    /// Finest source texel spacing among the textured groups (0 without one): the
    /// page spacing floor.
    pub source_spacing: f64,
    /// The source's own texel spacing, not `G/16`, set some page's density.
    pub source_limited: bool,
    pub escalated_pages: u32,
    /// Pages delivered as exact level-0 PNG without mips (no codec step met every gate).
    pub unmipped_pages: u32,
}

/// Thread CPU seconds of one tile's appearance work (wall seconds where the platform
/// has no thread clock). Page encoding runs on the tile's worker thread.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    /// Building the tile's resident source surface (small ranges).
    pub surface: f64,
    pub charts: f64,
    pub query: f64,
    pub sample: f64,
    pub encode: f64,
}

pub(crate) struct Appearance {
    pub groups: Vec<Group>,
    /// Smooth per-vertex normals over the decoded positions, when published.
    pub normals: Option<Vec<[f32; 3]>>,
    pub facts: TileAppearance,
    pub timing: Timing,
}

fn clock() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    super::error::thread_cpu()
        .unwrap_or_else(|| START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64())
}

/// The tile as S1 publishes it.
pub(crate) struct TileView<'a> {
    pub decoded: &'a [[f32; 3]],
    pub triangles: &'a [[u32; 3]],
    /// Per triangle: the source material of its representative.
    pub materials: Vec<u32>,
    pub geometric_error: f64,
}

/// Area-weighted smooth normal per vertex over decoded positions.
pub(crate) fn smooth_normals(decoded: &[[f32; 3]], faces: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let mut sum = vec![[0f64; 3]; decoded.len()];
    for f in faces {
        let p = f.map(|i| decoded[i as usize].map(f64::from));
        let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        for &i in f {
            for a in 0..3 {
                sum[i as usize][a] += n[a];
            }
        }
    }
    sum.iter()
        .map(|n| {
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if length > 0. && length.is_finite() {
                n.map(|v| (v / length) as f32)
            } else {
                [0., 1., 0.]
            }
        })
        .collect()
}

fn slot_order(slot: TextureSlot) -> u8 {
    match slot {
        TextureSlot::BaseColor => 0,
        TextureSlot::MetallicRoughness => 1,
        TextureSlot::Normal => 2,
        TextureSlot::Occlusion => 3,
        TextureSlot::Emissive => 4,
    }
}
pub(crate) fn slot_class(slot: TextureSlot) -> Class {
    match slot {
        TextureSlot::BaseColor | TextureSlot::Emissive => Class::Colour,
        TextureSlot::Normal => Class::Normal,
        _ => Class::Data,
    }
}
/// Source images are sampled as colour (sRGB) or raw linear data; source normal images are
/// raw linear components, re-expressed after sampling.
fn source_class(slot: TextureSlot) -> Class {
    match slot_class(slot) {
        Class::Colour => Class::Colour,
        _ => Class::Data,
    }
}

fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (l > 1e-12 && l.is_finite()).then(|| v.map(|x| x / l))
}
fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// `t` made orthogonal to unit `n` (Gram-Schmidt), unit; `None` when parallel.
fn orthogonal(t: [f64; 3], n: [f64; 3]) -> Option<[f64; 3]> {
    let d = dot3(t, n);
    normalize([t[0] - n[0] * d, t[1] - n[1] * d, t[2] - n[2] * d])
}

/// A textured group's layout and texel → query mapping, before the source pass.
struct Pending {
    /// Tile face of each group face.
    face_ids: Vec<usize>,
    layout: charts::Layout,
    slots: Vec<TextureSlot>,
    /// Per covered texel: (page texel, group face, barycentrics).
    texels: Vec<(u32, u32, [f32; 3])>,
    /// Per group face corner: coarse tangent (normal maps only).
    tangents: Option<Vec<[[f32; 4]; 3]>>,
}

/// Build the appearance of one coarse tile. Groups are processed one at a time: a
/// group's layout, texel inventory, pages and query state are released before the next
/// group starts, so the stage reserves one group's workspace (D8).
pub(crate) fn build(
    services: &Services<'_>,
    query: &mut super::source_index::SourceQuery<'_>,
    tile: &TileView<'_>,
) -> Result<Appearance, JobError> {
    let captured = services.captured;
    let check = services.check;
    let classes = &captured.material_classes;
    let mut by_class: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (t, m) in tile.materials.iter().enumerate() {
        by_class.entry(classes.of(*m)).or_default().push(t);
    }
    let target = spacing(tile.geometric_error);
    let mut facts = TileAppearance {
        requested_spacing: target,
        ..Default::default()
    };
    let mut timing = Timing::default();
    let any_normal_map = by_class.keys().any(|c| {
        captured
            .materials
            .get(*c as usize)
            .is_some_and(|m| m.has_normal_texture())
    });
    let normals = (services.normals || any_normal_map)
        .then(|| smooth_normals(tile.decoded, tile.triangles));
    let started = clock();
    let mut searcher = bake::Searcher::new(query)?;
    timing.surface = clock() - started;
    let started = clock();
    let source = source_spacing(services, &mut searcher, tile, &by_class)?;
    timing.query += clock() - started;
    let mut groups = Vec::with_capacity(by_class.len());
    for (class, faces) in by_class {
        check()?;
        let mut slots: Vec<TextureSlot> = captured
            .materials
            .get(class as usize)
            .map(|m| m.bindings().map(|b| b.slot).collect())
            .unwrap_or_default();
        slots.sort_by_key(|s| slot_order(*s));
        let mut group = Group {
            class,
            faces,
            material: Value::Null,
            uvs: None,
            tangents: None,
            colors: None,
            pages: Vec::new(),
        };
        if !slots.is_empty() {
            let floor = source.get(&class).copied().unwrap_or(0.);
            textured(
                services,
                &mut searcher,
                tile,
                &mut group,
                slots,
                (target, floor),
                normals.as_deref(),
                (&mut timing, &mut facts),
            )?;
        } else {
            group.material = flat(captured, class);
            if services.colors {
                let started = clock();
                vertex_colours(&mut searcher, tile, &mut group, check, &mut facts)?;
                timing.query += clock() - started;
            }
        }
        groups.push(group);
    }
    Ok(Appearance {
        groups,
        normals,
        facts,
        timing,
    })
}

/// One textured group: fresh charts, the chunked source bake, MASK coverage samples and
/// the labelled delivery of every slot page.
#[allow(clippy::too_many_arguments)]
fn textured(
    services: &Services<'_>,
    searcher: &mut bake::Searcher<'_, '_>,
    tile: &TileView<'_>,
    group: &mut Group,
    slots: Vec<TextureSlot>,
    (target, floor): (f64, f64),
    normals: Option<&[[f32; 3]]>,
    (timing, facts): (&mut Timing, &mut TileAppearance),
) -> Result<(), JobError> {
    let captured = services.captured;
    let started = clock();
    let corners: Vec<[u32; 3]> = group.faces.iter().map(|&f| tile.triangles[f]).collect();
    // Never finer than the source itself carries there.
    if floor > 0. {
        facts.source_spacing = if facts.source_spacing > 0. {
            facts.source_spacing.min(floor)
        } else {
            floor
        };
    }
    if floor > target {
        facts.source_limited = true;
    }
    let layout = charts::layout(tile.decoded, &corners, target.max(floor), services.max_edge)?;
    let mut owner = vec![false; (layout.size[0] * layout.size[1]) as usize];
    let mut texels = Vec::new();
    let mut covered = Vec::new();
    for gf in 0..corners.len() {
        charts::raster(&layout.pixels[gf], layout.size, &mut covered);
        for &(texel, w) in &covered {
            if std::mem::replace(&mut owner[texel as usize], true) {
                continue;
            }
            texels.push((texel, gf as u32, w.map(|v| v as f32)));
        }
    }
    drop(owner);
    let tangents = match (normals, slots.contains(&TextureSlot::Normal)) {
        (Some(n), true) => Some(
            corners
                .iter()
                .enumerate()
                .map(|(gf, f)| {
                    let x = layout.charts[layout.chart_of[gf] as usize].x;
                    f.map(|v| {
                        let nv = n[v as usize].map(f64::from);
                        let t = orthogonal(x, nv)
                            .or_else(|| orthogonal([1., 0., 0.], nv))
                            .or_else(|| orthogonal([0., 1., 0.], nv))
                            .unwrap_or([1., 0., 0.]);
                        [t[0] as f32, t[1] as f32, t[2] as f32, 1.]
                    })
                })
                .collect(),
        ),
        _ => None,
    };
    let labels = layout.labels();
    timing.charts += clock() - started;
    let p = Pending {
        face_ids: group.faces.clone(),
        layout,
        slots,
        texels,
        tangents,
    };
    let pages = bake_pages(services, searcher, &p, group.class, normals, tile, (timing, facts))?;
    let started = clock();
    let mask = captured
        .materials
        .get(group.class as usize)
        .and_then(|m| m.mask());
    let mut delivered = Vec::with_capacity(pages.len());
    for page in &pages {
        let samples = match mask {
            Some(m) if page.slot == TextureSlot::BaseColor => mask_samples(&p, page, tile, m),
            _ => Vec::new(),
        };
        let mask = mask
            .filter(|_| page.slot == TextureSlot::BaseColor)
            .map(|(factor, cutoff)| delivery::MaskDelivery {
                samples: &samples,
                factor,
                cutoff,
                linear: true,
                min_linear: true,
            });
        let d = deliver(
            (services.profile, services.page_memory, services.check),
            page,
            &labels,
            mask,
        )?;
        facts.escalated_pages += u32::from(d.escalated);
        facts.unmipped_pages += u32::from(!d.mipmapped);
        delivered.push(d);
    }
    timing.encode += clock() - started;
    let texels = u64::from(p.layout.size[0]) * u64::from(p.layout.size[1]);
    facts.textured_groups += 1;
    facts.pages += pages.len() as u32;
    facts.layout_texels += texels;
    facts.page_texels += texels * pages.len() as u64;
    facts.covered_texels += p.texels.len() as u64;
    facts.charts += p.layout.charts.len() as u32;
    facts.achieved_spacing = facts.achieved_spacing.max(1. / p.layout.scale);
    facts.capped |= p.layout.capped;
    facts.page_size = [
        facts.page_size[0].max(p.layout.size[0]),
        facts.page_size[1].max(p.layout.size[1]),
    ];
    let material = captured
        .materials
        .get(group.class as usize)
        .ok_or_else(|| state("coarse class material"))?;
    let patches: Vec<(TextureSlot, usize, usize)> =
        p.slots.iter().enumerate().map(|(i, s)| (*s, i, 0)).collect();
    let mut value = material.remap_slots(&patches)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("name");
        if let Some(normal) = object.get_mut("normalTexture") {
            // The source scale is absorbed into the re-expressed page.
            if let Some(n) = normal.as_object_mut() {
                n.remove("scale");
            }
        }
    }
    group.material = value;
    group.uvs = Some(
        (0..group.faces.len())
            .map(|gf| std::array::from_fn(|k| p.layout.uv(gf, k)))
            .collect(),
    );
    group.tangents = p.tangents;
    group.pages = delivered;
    Ok(())
}

/// Barycentric MASK coverage sample points per face (a 10-point triangular lattice).
const MASK_POINTS: [[f64; 3]; 10] = [
    [1., 0., 0.],
    [0., 1., 0.],
    [0., 0., 1.],
    [2. / 3., 1. / 3., 0.],
    [1. / 3., 2. / 3., 0.],
    [0., 2. / 3., 1. / 3.],
    [0., 1. / 3., 2. / 3.],
    [1. / 3., 0., 2. / 3.],
    [2. / 3., 0., 1. / 3.],
    [1. / 3., 1. / 3., 1. / 3.],
];

/// MASK coverage targets of a coarse base-colour page: area-weighted surface samples of
/// every group face, classified by the baked level-0 page itself (the coarse appearance's
/// source of truth; `COLOR_0` alpha is already baked in) under the class's factor and
/// cutoff, bilinear over covered texels as the page sampler reads it. The shared
/// delivery then keeps coverage through every mip level, as for leaves.
fn mask_samples(
    p: &Pending,
    page: &CoarsePage,
    tile: &TileView<'_>,
    (factor, cutoff): (f64, f64),
) -> Vec<delivery_mask::SurfaceSample> {
    let [w, h] = page.size;
    let alpha = |x: f64, y: f64| -> Option<f64> {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (mut sum, mut weight) = (0., 0.);
        for (dx, wx) in [(0., 1. - tx), (1., tx)] {
            for (dy, wy) in [(0., 1. - ty), (1., ty)] {
                let (cx, cy) = ((x0 + dx).clamp(0., f64::from(w - 1)), (y0 + dy).clamp(0., f64::from(h - 1)));
                let i = cy as usize * w as usize + cx as usize;
                if page.support[i] > 0 && wx * wy > 0. {
                    sum += wx * wy * f64::from(page.rgba[i * 4 + 3]) / 255.;
                    weight += wx * wy;
                }
            }
        }
        (weight > 0.).then(|| sum / weight)
    };
    let mut out = Vec::with_capacity(p.face_ids.len() * MASK_POINTS.len());
    for (gf, &face) in p.face_ids.iter().enumerate() {
        let q = tile.triangles[face].map(|v| tile.decoded[v as usize].map(f64::from));
        let e1: [f64; 3] = std::array::from_fn(|a| q[1][a] - q[0][a]);
        let e2: [f64; 3] = std::array::from_fn(|a| q[2][a] - q[0][a]);
        let area = 0.5 * dot3(cross3(e1, e2), cross3(e1, e2)).sqrt();
        if area.is_nan() || area <= 0. {
            continue;
        }
        let px = p.layout.pixels[gf];
        for b in MASK_POINTS {
            let x = b[0] * px[0][0] + b[1] * px[1][0] + b[2] * px[2][0];
            let y = b[0] * px[0][1] + b[1] * px[1][1] + b[2] * px[2][1];
            let Some(a) = alpha(x, y) else { continue };
            out.push(delivery_mask::SurfaceSample {
                uv: [x / f64::from(w), y / f64::from(h)],
                alpha: 1.,
                weight: area / MASK_POINTS.len() as f64,
                source_pass: a * factor >= cutoff,
            });
        }
    }
    out
}

/// Per-vertex `COLOR_0` of an untextured group from the nearest source surface (the
/// small vertex-colour path); kept only when some donor carries `COLOR_0`.
fn vertex_colours(
    searcher: &mut bake::Searcher<'_, '_>,
    tile: &TileView<'_>,
    group: &mut Group,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
    facts: &mut TileAppearance,
) -> Result<(), JobError> {
    let mut vertices: Vec<u32> = group.faces.iter().flat_map(|&f| tile.triangles[f]).collect();
    vertices.sort_unstable();
    vertices.dedup();
    let points = vertices.iter().map(|&v| tile.decoded[v as usize]).collect();
    let mut q = [Queries::new(group.class, points, false)];
    let (searches, reads) = bake::resolve(searcher, &mut q, check)?;
    facts.searches += searches;
    facts.donor_reads += reads;
    let [q] = q;
    if q.hits.iter().any(|h| h.flags & COLOR != 0) {
        let colors: HashMap<u32, [f32; 4]> =
            vertices.iter().zip(&q.hits).map(|(v, h)| (*v, h.color)).collect();
        facts.vertex_color_groups += 1;
        group.colors = Some(
            group
                .faces
                .iter()
                .map(|&f| tile.triangles[f].map(|v| colors[&v]))
                .collect(),
        );
    }
    Ok(())
}

/// Faces per class whose centroids estimate the source texel density (evenly strided).
const DENSITY_FACES: usize = 2048;

/// Per textured class: the spacing at which a page carries as many texels as the source
/// images do over the same coarse surface, `sqrt(area / Σ area · density)`, from source
/// queries at up to [`DENSITY_FACES`] evenly strided coarse face centroids (density:
/// squared source texels per metre of the hit triangle's densest bound slot).
/// Requesting finer spacing adds no information.
fn source_spacing(
    services: &Services<'_>,
    searcher: &mut bake::Searcher<'_, '_>,
    tile: &TileView<'_>,
    by_class: &BTreeMap<u32, Vec<usize>>,
) -> Result<BTreeMap<u32, f64>, JobError> {
    let captured = services.captured;
    let mut queries = Vec::new();
    let mut areas = Vec::new();
    for (&class, faces) in by_class {
        let textured = captured
            .materials
            .get(class as usize)
            .is_some_and(|m| m.bindings().next().is_some());
        if !textured {
            continue;
        }
        let stride = faces.len().div_ceil(DENSITY_FACES);
        let mut points = Vec::with_capacity(DENSITY_FACES);
        let mut area = Vec::with_capacity(DENSITY_FACES);
        for &f in faces.iter().step_by(stride) {
            let p = tile.triangles[f].map(|v| tile.decoded[v as usize].map(f64::from));
            points.push(std::array::from_fn(|a| ((p[0][a] + p[1][a] + p[2][a]) / 3.) as f32));
            let e1: [f64; 3] = std::array::from_fn(|a| p[1][a] - p[0][a]);
            let e2: [f64; 3] = std::array::from_fn(|a| p[2][a] - p[0][a]);
            area.push(0.5 * dot3(cross3(e1, e2), cross3(e1, e2)).sqrt());
        }
        queries.push(Queries::new(class, points, false));
        areas.push(area);
    }
    bake::resolve(searcher, &mut queries, services.check)?;
    let mut out = BTreeMap::new();
    for (q, area) in queries.iter().zip(&areas) {
        let (mut total, mut texels) = (0f64, 0f64);
        for (hit, a) in q.hits.iter().zip(area) {
            total += a;
            let slots = captured
                .materials
                .get(hit.material as usize)
                .map(|m| {
                    let mut s: Vec<TextureSlot> = m.bindings().map(|b| b.slot).collect();
                    s.sort_by_key(|s| slot_order(*s));
                    s
                })
                .unwrap_or_default();
            // The densest bound slot: a low-resolution base colour never hides a finer
            // normal, data or emissive image.
            let mut density = 0f64;
            for &slot in &slots {
                if let Some((image, _, set)) = bake::binding(captured, hit.material, slot)? {
                    let info = captured.image_info.get(image).ok_or_else(|| state("image info"))?;
                    let per_metre = f64::from(hit.gradient[set])
                        * (f64::from(info.width) * f64::from(info.height)).sqrt();
                    density = density.max(per_metre * per_metre);
                }
            }
            texels += a * density;
        }
        if texels > 0. && total > 0. && (total / texels).is_finite() {
            out.insert(q.class, (total / texels).sqrt());
        }
    }
    Ok(out)
}

/// The class's flat material (S1): literal factors, no textures.
pub(crate) fn flat(captured: &FullCaptured, class: u32) -> Value {
    match captured.materials.get(class as usize) {
        Some(m) => {
            let mut value = m.flat_proxy();
            if let Some(object) = value.as_object_mut() {
                object.remove("name");
            }
            value
        }
        None => json!({"pbrMetallicRoughness":{"metallicFactor":0.0,"roughnessFactor":1.0}}),
    }
}

type Binding = Option<(usize, crate::mesh_large::leaves::crops::SourceSampler, usize)>;

/// Bake every slot page of one textured group: per chunk of covered texels, resolve the
/// texel points against the source (bounded state), then sample each slot's source images
/// grouped by image at the texel's source level of detail.
fn bake_pages(
    services: &Services<'_>,
    searcher: &mut bake::Searcher<'_, '_>,
    p: &Pending,
    class: u32,
    normals: Option<&[[f32; 3]]>,
    tile: &TileView<'_>,
    (timing, facts): (&mut Timing, &mut TileAppearance),
) -> Result<Vec<CoarsePage>, JobError> {
    let captured = services.captured;
    let check = services.check;
    let [w, h] = p.layout.size;
    let texels = (w * h) as usize;
    let mut support = vec![0u32; texels];
    for (t, _, _) in &p.texels {
        support[*t as usize] = 1;
    }
    let spacing = 1. / p.layout.scale;
    let normal_map = p.slots.contains(&TextureSlot::Normal);
    let mut pages: Vec<Vec<u8>> = p.slots.iter().map(|_| vec![0u8; texels * 4]).collect();
    let mut bindings: HashMap<(u32, u8), Binding> = HashMap::new();
    // (image, chunk query) requests, grouped by image.
    let mut requests: Vec<(usize, u32)> = Vec::new();
    let mut lods: Vec<f64> = Vec::new();
    for chunk in p.texels.chunks(bake::CHUNK) {
        check()?;
        let started = clock();
        let points = chunk
            .iter()
            .map(|&(_, gf, w)| {
                let f = tile.triangles[p.layout_face(gf)].map(|v| tile.decoded[v as usize]);
                std::array::from_fn(|a| {
                    (f64::from(f[0][a]) * f64::from(w[0])
                        + f64::from(f[1][a]) * f64::from(w[1])
                        + f64::from(f[2][a]) * f64::from(w[2])) as f32
                })
            })
            .collect();
        let mut chunk_queries = [Queries::new(class, points, normal_map)];
        let (searches, reads) = bake::resolve(searcher, &mut chunk_queries, check)?;
        facts.searches += searches;
        facts.donor_reads += reads;
        let [q] = chunk_queries;
        timing.query += clock() - started;
        let started = clock();
        for (&slot, rgba) in p.slots.iter().zip(pages.iter_mut()) {
            requests.clear();
            for (i, hit) in q.hits.iter().enumerate() {
                let b = *match bindings.entry((hit.material, slot_order(slot))) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(bake::binding(captured, hit.material, slot)?)
                    }
                };
                match b {
                    Some((image, _, _)) => requests.push((image, i as u32)),
                    None => {
                        // A class member always binds the slot; defensively neutral.
                        write(rgba, chunk[i].0, slot, [1., 1., 1., 1.], hit, None);
                    }
                }
            }
            requests.sort_unstable();
            let mut start = 0;
            while start < requests.len() {
                let image = requests[start].0;
                let mut end = start;
                while end < requests.len() && requests[end].0 == image {
                    end += 1;
                }
                let size = {
                    let info = captured.image_info.get(image).ok_or_else(|| state("image info"))?;
                    (f64::from(info.width) * f64::from(info.height)).sqrt()
                };
                // Source level of detail per texel: its footprint in source texels.
                lods.clear();
                lods.extend(requests[start..end].iter().map(|&(_, i)| {
                    let hit = &q.hits[i as usize];
                    let (_, _, set) = bindings[&(hit.material, slot_order(slot))].unwrap();
                    let rho = spacing * f64::from(hit.gradient[set]) * size;
                    if rho > 1. {
                        rho.log2()
                    } else {
                        0.
                    }
                }));
                let need = lods.iter().copied().fold(f64::INFINITY, f64::min).floor().max(0.) as u32;
                let chain = services
                    .cache
                    .get(captured, image, source_class(slot), need, check)?;
                for (&(_, i), &lod) in requests[start..end].iter().zip(&lods) {
                    let i = i as usize;
                    let hit = &q.hits[i];
                    let (_, sampler, set) = bindings[&(hit.material, slot_order(slot))].unwrap();
                    let value = bake::sample(&chain, source_class(slot), sampler, hit.uv[set], lod);
                    let frame = (slot == TextureSlot::Normal).then(|| {
                        let (_, gf, w) = chunk[i];
                        let face = tile.triangles[p.layout_face(gf)];
                        let n = normals.map(|n| face.map(|v| n[v as usize])).unwrap_or([[0., 1., 0.]; 3]);
                        let t = p.tangents.as_ref().map(|t| t[gf as usize]).unwrap_or([[1., 0., 0., 1.]; 3]);
                        let lerp = |c: [[f32; 3]; 3]| -> [f64; 3] {
                            std::array::from_fn(|a| {
                                (0..3).map(|k| f64::from(c[k][a]) * f64::from(w[k])).sum()
                            })
                        };
                        (
                            lerp(n),
                            lerp(t.map(|t| [t[0], t[1], t[2]])),
                            q.frames[i],
                            source_scale(captured, hit.material),
                        )
                    });
                    write(rgba, chunk[i].0, slot, value, hit, frame);
                }
                start = end;
            }
        }
        timing.sample += clock() - started;
    }
    Ok(p
        .slots
        .iter()
        .zip(pages)
        .map(|(&slot, rgba)| CoarsePage {
            slot,
            size: p.layout.size,
            rgba,
            support: support.clone(),
        })
        .collect())
}

impl Pending {
    /// The tile face of group face `gf`.
    fn layout_face(&self, gf: u32) -> usize {
        self.face_ids[gf as usize]
    }
}

fn source_scale(captured: &FullCaptured, material: u32) -> f64 {
    captured
        .materials
        .get(material as usize)
        .and_then(|m| m.literal().pointer("/normalTexture/scale"))
        .and_then(Value::as_f64)
        .unwrap_or(1.)
}

type NormalFrame = ([f64; 3], [f64; 3], bake::Frame, f64);

/// Write one baked texel. Colour slots receive linear values and are sRGB-encoded here;
/// base colour carries the source COLOR_0 (linear) multiplied in.
fn write(rgba: &mut [u8], texel: u32, slot: TextureSlot, v: [f64; 4], hit: &Hit, frame: Option<NormalFrame>) {
    let q = |x: f64| (x.clamp(0., 1.) * 255.).round() as u8;
    let i = texel as usize * 4;
    let out: [u8; 4] = match slot {
        TextureSlot::BaseColor => {
            let c = hit.color.map(f64::from);
            [
                bake::srgb_byte(v[0] * c[0]),
                bake::srgb_byte(v[1] * c[1]),
                bake::srgb_byte(v[2] * c[2]),
                q(v[3] * c[3]),
            ]
        }
        TextureSlot::Emissive => [
            bake::srgb_byte(v[0]),
            bake::srgb_byte(v[1]),
            bake::srgb_byte(v[2]),
            q(v[3]),
        ],
        TextureSlot::Normal => {
            let (n_c, t_c, source, scale) = frame.expect("normal slot frame");
            let x = (2. * v[0] - 1.) * scale;
            let y = (2. * v[1] - 1.) * scale;
            let z = 2. * v[2] - 1.;
            let flat = [0.5, 0.5, 1., 1.].map(q);
            let world = (|| {
                let n_s = normalize(source.normal.map(f64::from))?;
                let t_s = orthogonal([0, 1, 2].map(|a| f64::from(source.tangent[a])), n_s)?;
                let b_s = cross3(n_s, t_s).map(|c| c * f64::from(source.tangent[3]));
                let w: [f64; 3] = std::array::from_fn(|a| t_s[a] * x + b_s[a] * y + n_s[a] * z);
                let w = normalize(w)?;
                let n_c = normalize(n_c)?;
                let t_c = orthogonal(t_c, n_c)?;
                let b_c = cross3(n_c, t_c);
                normalize([dot3(w, t_c), dot3(w, b_c), dot3(w, n_c)])
            })();
            match world {
                Some(n) => [q((n[0] + 1.) * 0.5), q((n[1] + 1.) * 0.5), q((n[2] + 1.) * 0.5), 255],
                None => flat,
            }
        }
        _ => [q(v[0]), q(v[1]), q(v[2]), q(v[3])],
    };
    rgba[i..i + 4].copy_from_slice(&out);
}

/// Encode one pre-encode page through the shared leaf delivery path (the profile's
/// codec, sparse colour-correct mips kept apart per chart (`labels`) with gutters filled
/// per level, MASK coverage for MASK base colour, decoded gates).
pub(crate) fn deliver(
    (profile, page_memory, check): (Profile, u64, &dyn Fn() -> Result<(), JobError>),
    page: &CoarsePage,
    labels: &[u32],
    mask: Option<delivery::MaskDelivery<'_>>,
) -> Result<DeliveredPage, JobError> {
    let delivered = delivery::encode_islands(
        page.size,
        &page.rgba,
        &page.support,
        Some(labels),
        page.slot,
        profile,
        page_memory,
        mask,
        // A chain that misses its gates on every codec step (small levels of a page
        // mixing charts inside codec blocks) is delivered as the exact level-0 PNG and
        // sampled without mips, rather than refusing the run or loosening a gate.
        delivery::Ladder {
            level0_sampled: true,
            check,
        },
    )?;
    Ok(DeliveredPage {
        slot: page.slot,
        mipmapped: delivered.mime == "image/ktx2",
        bytes: delivered.bytes,
        mime: delivered.mime,
        escalated: delivered.escalation.is_some(),
    })
}

/// Bytes one page encode admits: the page's RGBA and support per slot, the chart labels
/// and their reduced chain, MASK samples, the encoder workspace and output, at `size`.
pub(crate) fn page_charge(size: [u32; 2], slots: u64, profile: Profile) -> Result<u64, JobError> {
    let texels = u64::from(size[0]) * u64::from(size[1]);
    let codec = delivery::workspace_charge(size, profile)?;
    Ok(texels
        .saturating_mul(8)
        .saturating_mul(slots)
        .saturating_add(codec)
        .saturating_add(texels.saturating_mul(8 + 10))
        .saturating_add(super::TRIANGLE_CAP as u64 * MASK_POINTS.len() as u64 * 40))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review probe: a MASK base-colour coarse page (cutoff 0.75) whose level-1 alphas
    /// box-filter to 64 and 191. Unadjusted, 191 < 191.25 drops every texel of level 1
    /// (coverage 25% -> 0%); the shared coverage-preserving delivery scales alpha so
    /// level 1 keeps passing texels.
    #[test]
    fn coarse_mask_pages_keep_coverage_through_their_mips() {
        let (w, h) = (8u32, 8u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let a = if x < 4 { if y % 2 == 0 { 0 } else { 128 } } else if y % 2 == 0 { 255 } else { 127 };
                rgba[i..i + 4].copy_from_slice(&[180, 120, 60, a]);
            }
        }
        let page = CoarsePage {
            slot: TextureSlot::BaseColor,
            size: [w, h],
            rgba,
            support: vec![1; (w * h) as usize],
        };
        let labels = vec![1u32; (w * h) as usize];
        let (factor, cutoff) = (1., 0.75);
        let samples: Vec<delivery_mask::SurfaceSample> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                delivery_mask::SurfaceSample {
                    uv: [(f64::from(x) + 0.5) / f64::from(w), (f64::from(y) + 0.5) / f64::from(h)],
                    alpha: 1.,
                    weight: 1.,
                    source_pass: f64::from(page.rgba[(i * 4 + 3) as usize]) / 255. * factor >= cutoff,
                }
            })
            .collect();
        let check = || Ok(());
        let level1_passing = |mask: Option<delivery::MaskDelivery<'_>>| {
            let d = deliver((Profile::Balanced, 1 << 28, &check), &page, &labels, mask).unwrap();
            let decoder = basisu::Transcoder::new(&d.bytes).unwrap();
            let level = decoder
                .transcode(1, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
                .unwrap();
            level.chunks(4).filter(|p| f64::from(p[3]) / 255. * factor >= cutoff).count()
        };
        assert_eq!(level1_passing(None), 0, "control: the unadjusted level 1 loses MASK coverage");
        let kept = level1_passing(Some(delivery::MaskDelivery {
            samples: &samples,
            factor,
            cutoff,
            linear: true,
            min_linear: true,
        }));
        assert!(kept > 0, "coverage-preserving delivery keeps level-1 MASK texels");
    }
}
