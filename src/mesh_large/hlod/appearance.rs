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
        leaves::{delivery, mips::Class},
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
}

/// Thread CPU seconds of one tile's appearance work (wall seconds where the platform
/// has no thread clock). Page encoding runs on the tile's worker thread.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
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
    group: usize,
    /// Tile face of each group face.
    face_ids: Vec<usize>,
    layout: charts::Layout,
    slots: Vec<TextureSlot>,
    /// Per covered texel: (page texel, group face, barycentrics); query `i` is texel `i`.
    texels: Vec<(u32, u32, [f64; 3])>,
    /// Per group face corner: coarse tangent (normal maps only).
    tangents: Option<Vec<[[f32; 4]; 3]>>,
}

/// Build the appearance of one coarse tile.
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
    let source = source_spacing(services, query, tile, &by_class, target)?;
    timing.query += clock() - started;
    let mut groups = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();
    let mut queries: Vec<Queries> = Vec::new();
    // Untextured vertex-colour groups: (group, unique vertices in query order).
    let mut vertex_groups: Vec<(usize, Vec<u32>)> = Vec::new();
    let started = clock();
    for (class, faces) in by_class {
        check()?;
        let material = captured.materials.get(class as usize);
        let mut slots: Vec<TextureSlot> = material
            .map(|m| m.bindings().map(|b| b.slot).collect())
            .unwrap_or_default();
        slots.sort_by_key(|s| slot_order(*s));
        let index = groups.len();
        groups.push(Group {
            class,
            faces: faces.clone(),
            material: Value::Null,
            uvs: None,
            tangents: None,
            colors: None,
            pages: Vec::new(),
        });
        if !slots.is_empty() {
            let corners: Vec<[u32; 3]> = faces.iter().map(|&f| tile.triangles[f]).collect();
            // Never finer than the source itself carries there.
            let floor = source.get(&class).copied().unwrap_or(0.);
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
            let layout =
                charts::layout(tile.decoded, &corners, target.max(floor), services.max_edge)?;
            let mut owner = vec![false; (layout.size[0] * layout.size[1]) as usize];
            let mut texels = Vec::new();
            let mut points = Vec::new();
            let mut covered = Vec::new();
            for (gf, f) in corners.iter().enumerate() {
                charts::raster(&layout.pixels[gf], layout.size, &mut covered);
                let p = f.map(|v| tile.decoded[v as usize]);
                for &(texel, w) in &covered {
                    if std::mem::replace(&mut owner[texel as usize], true) {
                        continue;
                    }
                    texels.push((texel, gf as u32, w));
                    points.push(std::array::from_fn(|a| {
                        (f64::from(p[0][a]) * w[0] + f64::from(p[1][a]) * w[1] + f64::from(p[2][a]) * w[2])
                            as f32
                    }));
                }
            }
            let normal_map = slots.contains(&TextureSlot::Normal);
            let tangents = match (&normals, normal_map) {
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
            queries.push(Queries::new(class, points, 1. / layout.scale, normal_map));
            pending.push(Pending {
                group: index,
                face_ids: faces.clone(),
                layout,
                slots,
                texels,
                tangents,
            });
        } else if services.colors {
            let mut vertices: Vec<u32> = faces.iter().flat_map(|&f| tile.triangles[f]).collect();
            vertices.sort_unstable();
            vertices.dedup();
            let points = vertices.iter().map(|&v| tile.decoded[v as usize]).collect();
            queries.push(Queries::new(class, points, target, false));
            vertex_groups.push((index, vertices));
        }
    }
    timing.charts = clock() - started;
    let started = clock();
    (facts.searches, facts.donor_reads) = bake::resolve(query, &mut queries, check)?;
    timing.query += clock() - started;
    let mut queries = queries.into_iter();
    let mut vertex_groups = vertex_groups.into_iter().peekable();
    let mut pending = pending.into_iter().peekable();
    let mut sample_seconds = 0.;
    let mut encode_seconds = 0.;
    for (index, group) in groups.iter_mut().enumerate() {
        if pending.peek().is_some_and(|p| p.group == index) {
            let p = pending.next().unwrap();
            let q = queries.next().ok_or_else(|| state("coarse query order"))?;
            let started = clock();
            let pages = bake_pages(services, &p, &q, normals.as_deref(), tile)?;
            sample_seconds += clock() - started;
            let started = clock();
            let mut delivered = Vec::with_capacity(pages.len());
            for page in &pages {
                let d = deliver(services, page)?;
                facts.escalated_pages += u32::from(d.escalated);
                delivered.push(d);
            }
            encode_seconds += clock() - started;
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
        } else {
            group.material = flat(captured, group.class);
            if vertex_groups.peek().is_some_and(|(g, _)| *g == index) {
                let (_, vertices) = vertex_groups.next().unwrap();
                let q = queries.next().ok_or_else(|| state("coarse query order"))?;
                let colors: HashMap<u32, [f32; 4]> = vertices
                    .iter()
                    .zip(&q.hits)
                    .map(|(v, h)| (*v, h.color))
                    .collect();
                if q.hits.iter().any(|h| h.flags & COLOR != 0) {
                    facts.vertex_color_groups += 1;
                    group.colors = Some(
                        group
                            .faces
                            .iter()
                            .map(|&f| tile.triangles[f].map(|v| colors[&v]))
                            .collect(),
                    );
                }
            }
        }
    }
    timing.sample = sample_seconds;
    timing.encode = encode_seconds;
    Ok(Appearance {
        groups,
        normals,
        facts,
        timing,
    })
}

/// Per textured class: the spacing at which a page carries as many texels as the source
/// images do over the same coarse surface, `sqrt(area / Σ area · density)`, from one
/// source query per coarse face centroid (density: squared source texels per metre of
/// the hit triangle's first bound slot). Requesting finer spacing adds no information.
fn source_spacing(
    services: &Services<'_>,
    query: &mut super::source_index::SourceQuery<'_>,
    tile: &TileView<'_>,
    by_class: &BTreeMap<u32, Vec<usize>>,
    target: f64,
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
        let mut points = Vec::with_capacity(faces.len());
        let mut area = Vec::with_capacity(faces.len());
        for &f in faces {
            let p = tile.triangles[f].map(|v| tile.decoded[v as usize].map(f64::from));
            points.push(std::array::from_fn(|a| ((p[0][a] + p[1][a] + p[2][a]) / 3.) as f32));
            let e1: [f64; 3] = std::array::from_fn(|a| p[1][a] - p[0][a]);
            let e2: [f64; 3] = std::array::from_fn(|a| p[2][a] - p[0][a]);
            area.push(0.5 * dot3(cross3(e1, e2), cross3(e1, e2)).sqrt());
        }
        queries.push(Queries::new(class, points, target, false));
        areas.push(area);
    }
    bake::resolve(query, &mut queries, services.check)?;
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
            let Some(&slot) = slots.first() else { continue };
            if let Some((image, _, set)) = bake::binding(captured, hit.material, slot)? {
                let info = captured.image_info.get(image).ok_or_else(|| state("image info"))?;
                let per_metre = f64::from(hit.gradient[set])
                    * (f64::from(info.width) * f64::from(info.height)).sqrt();
                texels += a * per_metre * per_metre;
            }
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

/// Sample every slot page of one textured group from its resolved hits.
fn bake_pages(
    services: &Services<'_>,
    p: &Pending,
    q: &Queries,
    normals: Option<&[[f32; 3]]>,
    tile: &TileView<'_>,
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
    let mut bindings: HashMap<(u32, u8), Binding> = HashMap::new();
    let mut pages = Vec::with_capacity(p.slots.len());
    for &slot in &p.slots {
        check()?;
        let mut rgba = vec![0u8; texels * 4];
        // (image, query) requests, grouped by image.
        let mut requests: Vec<(usize, u32)> = Vec::with_capacity(p.texels.len());
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
                    write(&mut rgba, p.texels[i].0, slot, [1., 1., 1., 1.], hit, None);
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
            let chain = services
                .cache
                .get(captured, image, source_class(slot), check)?;
            let size = {
                let l = &chain.levels[0];
                (f64::from(l.width) * f64::from(l.height)).sqrt()
            };
            for (k, &(_, i)) in requests[start..end].iter().enumerate() {
                if k % 65536 == 0 {
                    check()?;
                }
                let i = i as usize;
                let hit = &q.hits[i];
                let (_, sampler, set) = bindings[&(hit.material, slot_order(slot))].unwrap();
                let rho = spacing * f64::from(hit.gradient[set]) * size;
                let lod = if rho > 1. { rho.log2() } else { 0. };
                let value = bake::sample(&chain, source_class(slot), sampler, hit.uv[set], lod);
                let frame = (slot == TextureSlot::Normal).then(|| {
                    let (texel, gf, w) = p.texels[i];
                    let _ = texel;
                    let face = tile.triangles[p.layout_face(gf)];
                    let n = normals.map(|n| face.map(|v| n[v as usize])).unwrap_or([[0., 1., 0.]; 3]);
                    let t = p.tangents.as_ref().map(|t| t[gf as usize]).unwrap_or([[1., 0., 0., 1.]; 3]);
                    let lerp = |c: [[f32; 3]; 3]| -> [f64; 3] {
                        std::array::from_fn(|a| {
                            f64::from(c[0][a]) * w[0] + f64::from(c[1][a]) * w[1] + f64::from(c[2][a]) * w[2]
                        })
                    };
                    let source = q.frames[i];
                    (
                        lerp(n),
                        lerp(t.map(|t| [t[0], t[1], t[2]])),
                        source,
                        source_scale(captured, hit.material),
                    )
                });
                write(&mut rgba, p.texels[i].0, slot, value, hit, frame);
            }
            start = end;
        }
        pages.push(CoarsePage {
            slot,
            size: p.layout.size,
            rgba,
            support: support.clone(),
        });
    }
    Ok(pages)
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
/// codec, sparse colour-correct mips with gutters filled per level, decoded gates).
pub(crate) fn deliver(services: &Services<'_>, page: &CoarsePage) -> Result<DeliveredPage, JobError> {
    let check: &dyn Fn() -> Result<(), JobError> = services.check;
    let delivered = delivery::encode(
        page.size,
        &page.rgba,
        &page.support,
        page.slot,
        services.profile,
        services.page_memory,
        None,
        delivery::Ladder {
            level0_sampled: false,
            check,
        },
    )?;
    Ok(DeliveredPage {
        slot: page.slot,
        bytes: delivered.bytes,
        mime: delivered.mime,
        escalated: delivered.escalation.is_some(),
    })
}

/// Bytes one page encode admits: the page's RGBA and support per slot, the encoder
/// workspace and output, at `size`.
pub(crate) fn page_charge(size: [u32; 2], slots: u64, profile: Profile) -> Result<u64, JobError> {
    let texels = u64::from(size[0]) * u64::from(size[1]);
    let codec = delivery::workspace_charge(size, profile)?;
    Ok(texels
        .saturating_mul(8)
        .saturating_mul(slots)
        .saturating_add(codec)
        .saturating_add(texels.saturating_mul(8)))
}
