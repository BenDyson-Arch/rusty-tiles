//! H28 per-tile primitive batching: material classes and texture-space ownership.
//!
//! A material class is the admitted material without texture identity: what a merged
//! glTF primitive may share. Texel ownership is the exact "one triangle per texel"
//! test on published decoded UVs (see `docs/architecture/h28-tile-batching-contract.md`).
use super::leaves::{crops, prepared::PreparedAtlas};
use crate::{gltf_source::source::Material, JobError, JobErrorKind};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// JSON pointers of the five material texture slots, in `TextureSlot` order, with the
/// glTF name and default of the slot's extra scalar.
const SLOTS: [(&str, Option<&str>); 5] = [
    ("/pbrMetallicRoughness/baseColorTexture", None),
    ("/pbrMetallicRoughness/metallicRoughnessTexture", None),
    ("/normalTexture", Some("scale")),
    ("/occlusionTexture", Some("strength")),
    ("/emissiveTexture", None),
];

/// Numbers as f64 (so `1` and `1.0` agree), recursively through arrays.
fn normalized(value: Value) -> Value {
    match value {
        Value::Number(n) => n.as_f64().map_or(Value::Null, |f| json!(f)),
        Value::Array(a) => Value::Array(a.into_iter().map(normalized).collect()),
        other => other,
    }
}

/// The literal (non-texture) factors of a material with glTF defaults filled in, numbers
/// as f64: base colour, metallic, roughness, emissive, `doubleSided`, `alphaMode` and
/// `alphaCutoff` (MASK only; glTF ignores it otherwise).
pub(in crate::mesh_large) fn literal_factors(material: &Value) -> [Value; 7] {
    let pbr = |k: &str, default: Value| {
        material
            .get("pbrMetallicRoughness")
            .and_then(|p| p.get(k))
            .cloned()
            .unwrap_or(default)
    };
    let top = |k: &str, default: Value| material.get(k).cloned().unwrap_or(default);
    let alpha = top("alphaMode", json!("OPAQUE"));
    let mask = alpha == json!("MASK");
    [
        pbr("baseColorFactor", json!([1.0, 1.0, 1.0, 1.0])),
        pbr("metallicFactor", json!(1.0)),
        pbr("roughnessFactor", json!(1.0)),
        top("emissiveFactor", json!([0.0, 0.0, 0.0])),
        top("doubleSided", json!(false)),
        alpha,
        if mask {
            top("alphaCutoff", json!(0.5))
        } else {
            Value::Null
        },
    ]
    .map(normalized)
}

/// Canonical form of a material for batching: its literal factors and, per slot, `null`
/// or the slot's extra scalar (`normalTexture.scale`, `occlusionTexture.strength`, glTF
/// default 1). With `texture`, each bound slot also carries its published texture index
/// and `texCoord`, so equal keys bind the same textures through the same UV sets.
pub(in crate::mesh_large) fn material_key(material: &Value, texture: bool) -> Value {
    let slots: Vec<Value> = SLOTS
        .iter()
        .map(|(pointer, extra)| {
            let Some(info) = material.pointer(pointer) else {
                return Value::Null;
            };
            let mut slot = json!({});
            if let Some(name) = extra {
                slot["extra"] = normalized(info.get(*name).cloned().unwrap_or(json!(1.0)));
            }
            if texture {
                slot["index"] = normalized(info.get("index").cloned().unwrap_or(Value::Null));
                slot["texCoord"] = normalized(info.get("texCoord").cloned().unwrap_or(json!(0)));
            }
            slot
        })
        .collect();
    json!({"factors": literal_factors(material).to_vec(), "slots": slots})
}

/// Source material ID to the representative of its class: the lowest ID whose class
/// (literal factors, slot presence and extras, and each bound slot's source sampler
/// min/mag filter) is equal. Wrap modes are not part of a class: tile writers resolve
/// wrapping into page content and publish `CLAMP_TO_EDGE`, except a leaf's source period
/// page, which is a texture of its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::mesh_large) struct MaterialClasses(Vec<u32>);

impl MaterialClasses {
    /// No materials.
    pub(in crate::mesh_large) const EMPTY: Self = Self(Vec::new());
    pub(in crate::mesh_large) fn new(
        materials: &[Material],
        textures: &[Value],
        samplers: &[Value],
    ) -> Result<Self, JobError> {
        let invalid = || {
            JobError::new(
                JobErrorKind::InvalidState,
                "material class closure: texture or sampler reference",
            )
        };
        let mut first = BTreeMap::<String, u32>::new();
        let mut classes = Vec::with_capacity(materials.len());
        for (id, material) in materials.iter().enumerate() {
            let id = u32::try_from(id).map_err(|_| invalid())?;
            let mut key = material_key(material.literal(), false);
            for binding in material.bindings() {
                let texture = textures.get(binding.texture).ok_or_else(invalid)?;
                let sampler = texture
                    .get("sampler")
                    .and_then(Value::as_u64)
                    .map(|s| samplers.get(s as usize).ok_or_else(invalid))
                    .transpose()?;
                let sampler = crops::resolve(sampler)?;
                let slot = &mut key["slots"][slot_index(binding.slot)];
                slot["minFilter"] = json!(sampler.min_filter);
                slot["magFilter"] = json!(sampler.mag_filter);
            }
            classes.push(*first.entry(key.to_string()).or_insert(id));
        }
        Ok(Self(classes))
    }
    /// Each listed material is its own class (fixtures without a capture).
    #[cfg(test)]
    pub(in crate::mesh_large) fn identity(count: u32) -> Self {
        Self((0..count).collect())
    }
    /// The class representative of `material`; `u32::MAX` (no material) and IDs outside
    /// the table are their own class.
    pub(in crate::mesh_large) fn of(&self, material: u32) -> u32 {
        self.0.get(material as usize).copied().unwrap_or(material)
    }
}

fn slot_index(slot: crate::gltf_source::source::TextureSlot) -> usize {
    use crate::gltf_source::source::TextureSlot;
    match slot {
        TextureSlot::BaseColor => 0,
        TextureSlot::MetallicRoughness => 1,
        TextureSlot::Normal => 2,
        TextureSlot::Occlusion => 3,
        TextureSlot::Emissive => 4,
    }
}

/// A coordinate as an exact integer multiple of 2^-56, for |v| < 8 (`None` otherwise).
/// Scaling an f32 by a power of two is exact in f64; the product is an integer exactly
/// when `v` lies on that grid, which holds for 0 and every f32 with |v| >= 2^-33.
fn fixed(v: f32) -> Option<i64> {
    if !v.is_finite() || v.abs() >= 8. {
        return None;
    }
    let scaled = f64::from(v) * (1u64 << 56) as f64;
    (scaled.fract() == 0.).then_some(scaled as i64)
}

type Fixed = [[i128; 2]; 3];

fn twice_area(t: &Fixed) -> i128 {
    (t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[1][1] - t[0][1]) * (t[2][0] - t[0][0])
}

/// Exact: the interiors of two non-degenerate triangles share area unless a line parallel
/// to one of their six edges weakly separates them (convex sets with disjoint interiors
/// always have such a line). Coordinates are below 2^59, so every product fits i128.
fn interiors_overlap(a: &Fixed, b: &Fixed) -> bool {
    for t in [a, b] {
        for i in 0..3 {
            let e = [t[(i + 1) % 3][0] - t[i][0], t[(i + 1) % 3][1] - t[i][1]];
            let axis = [-e[1], e[0]];
            let project = |p: &[i128; 2]| axis[0] * p[0] + axis[1] * p[1];
            let (amin, amax) = a
                .iter()
                .map(project)
                .fold((i128::MAX, i128::MIN), |m, v| (m.0.min(v), m.1.max(v)));
            let (bmin, bmax) = b
                .iter()
                .map(project)
                .fold((i128::MAX, i128::MIN), |m, v| (m.0.min(v), m.1.max(v)));
            if amax <= bmin || bmax <= amin {
                return false;
            }
        }
    }
    true
}

/// Pair examinations between cancellation checks in the texel-ownership sweep.
const SHARED_SWEEP_CHECK_PAIRS: u32 = 1 << 16;

/// One footprint for the sweep: exact coordinates (`None` outside the exact domain: it is
/// shared, and it overlaps whatever its box overlaps) and its box on the 2^-56 grid.
struct Copy {
    exact: Option<Fixed>,
    lo: [i128; 2],
    hi: [i128; 2],
}
impl Copy {
    fn exact(t: Fixed) -> Self {
        let lo = [0, 1].map(|a| t.iter().map(|p| p[a]).min().unwrap());
        let hi = [0, 1].map(|a| t.iter().map(|p| p[a]).max().unwrap());
        Self {
            exact: Some(t),
            lo,
            hi,
        }
    }
    fn of(t: &[[f32; 2]; 3]) -> Self {
        let exact = (|| {
            let mut out = [[0i128; 2]; 3];
            for (c, p) in t.iter().enumerate() {
                for a in 0..2 {
                    out[c][a] = i128::from(fixed(p[a])?);
                }
            }
            Some(out)
        })();
        if let Some(t) = exact {
            return Self::exact(t);
        }
        // Scaling a finite f32 by 2^56 is exact in f64; a NaN box overlaps everything.
        let scale = |v: f32, up: bool| {
            let x = f64::from(v) * (1u64 << 56) as f64;
            match (x.is_nan(), up) {
                (true, false) => i128::MIN,
                (true, true) => i128::MAX,
                (false, false) => x.floor() as i128,
                (false, true) => x.ceil() as i128,
            }
        };
        let lo = [0, 1].map(|a| t.iter().map(|p| scale(p[a], false)).min().unwrap());
        let hi = [0, 1].map(|a| t.iter().map(|p| scale(p[a], true)).max().unwrap());
        Self {
            exact: None,
            lo,
            hi,
        }
    }
}

/// Per footprint, whether its decoded-UV interior shares area with another footprint on
/// the same page. Exact for coordinates on the 2^-56 grid below 8 in magnitude; a
/// footprint outside that domain, and any footprint whose box overlaps its box, counts as
/// shared. A footprint of zero exact area owns no interior and is never shared.
pub(in crate::mesh_large) fn shared_footprints(
    footprints: &[[[f32; 2]; 3]],
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Vec<bool>, JobError> {
    sweep(footprints.iter().map(Copy::of).collect(), check)
}

fn sweep(
    copies: Vec<Copy>,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Vec<bool>, JobError> {
    let mut shared = vec![false; copies.len()];
    // Degenerate exact footprints own no interior and leave the sweep.
    let mut order: Vec<usize> = Vec::with_capacity(copies.len());
    for (i, c) in copies.iter().enumerate() {
        match &c.exact {
            Some(f) if twice_area(f) == 0 => continue,
            Some(_) => {}
            None => shared[i] = true,
        }
        order.push(i);
    }
    order.sort_by_key(|&i| (copies[i].lo[0], i));
    let mut active: Vec<usize> = Vec::new();
    // Candidate pairs examined since the last cancellation check: a dense page makes the
    // sweep quadratic, so the cost is bounded per pair, not per outer iteration.
    let mut pairs = 0u32;
    for (k, &i) in order.iter().enumerate() {
        if k % 4096 == 0 {
            check()?;
        }
        let (lo, hi) = (copies[i].lo, copies[i].hi);
        // Boxes that end at or before this start can only touch: a vertical line
        // separates them, so their interiors are disjoint.
        active.retain(|&o| copies[o].hi[0] > lo[0]);
        for &o in &active {
            pairs += 1;
            if pairs >= SHARED_SWEEP_CHECK_PAIRS {
                pairs = 0;
                check()?;
            }
            if copies[o].hi[1] <= lo[1] || hi[1] <= copies[o].lo[1] {
                continue;
            }
            // Both already shared: the exact test cannot change either answer.
            if shared[i] && shared[o] {
                continue;
            }
            let overlap = match (&copies[i].exact, &copies[o].exact) {
                (Some(a), Some(b)) => interiors_overlap(a, b),
                _ => true,
            };
            if overlap {
                shared[i] = true;
                shared[o] = true;
            }
        }
        active.push(i);
    }
    Ok(shared)
}

/// The copies of a footprint on a page with a period axis (`wrap`): shifted by whole
/// periods so it starts in its first period (in f64, where `c - k` of an f32 `c` and an
/// integer `k` is exact unless `c` is a tiny negative; such a result off the 2^-56 grid
/// makes the footprint shared, conservatively), then every image under the axis'
/// symmetry (REPEAT: x - 1; MIRRORED: 2 - x and x - 2) that reaches into [0, 1], where
/// texels live. Two copies overlapping anywhere read the same texels. `None` when a period
/// axis spans more than one period: the footprint covers some texel twice and its images
/// are unbounded.
fn period_copies(t: &[[f32; 2]; 3], wrap: [u32; 2]) -> Option<Vec<Copy>> {
    let one = 1i128 << 56;
    let mut base = t.map(|p| p.map(f64::from));
    for (a, &w) in wrap.iter().enumerate() {
        let period = match w {
            10497 => 1.,
            33648 => 2.,
            _ => continue,
        };
        let lo = base.iter().map(|p| p[a]).fold(f64::INFINITY, f64::min);
        let hi = base.iter().map(|p| p[a]).fold(f64::NEG_INFINITY, f64::max);
        if hi - lo > 1. || (hi - lo).is_nan() {
            return None;
        }
        let shift = (lo / period).floor() * period;
        for p in &mut base {
            p[a] -= shift;
        }
    }
    let grid = |v: f64| {
        let scaled = v * (1u64 << 56) as f64;
        (v.abs() < 8. && scaled.fract() == 0.).then_some(i128::from(scaled as i64))
    };
    let mut t = [[0i128; 2]; 3];
    for (c, p) in base.iter().enumerate() {
        for a in 0..2 {
            t[c][a] = grid(p[a])?;
        }
    }
    let mut copies = vec![t];
    for (a, &w) in wrap.iter().enumerate() {
        let images: &[(i128, i128)] = match w {
            10497 => &[(1, -1)],
            33648 => &[(-1, 2), (1, -2)],
            _ => continue,
        };
        let mut next = Vec::new();
        for c in &copies {
            next.push(*c);
            for &(sign, offset) in images {
                let mut image = *c;
                for p in &mut image {
                    p[a] = sign * p[a] + offset * one;
                }
                let (lo, hi) = (
                    image.iter().map(|p| p[a]).min().unwrap(),
                    image.iter().map(|p| p[a]).max().unwrap(),
                );
                if lo < one && hi > 0 {
                    next.push(image);
                }
            }
        }
        copies = next;
    }
    Some(copies.into_iter().map(Copy::exact).collect())
}

/// The published wrap of a texture's sampler.
fn texture_wrap(atlas: &PreparedAtlas, texture: &Value) -> [u32; 2] {
    let sampler = (texture.as_u64())
        .and_then(|t| atlas.textures.get(t as usize)?.get("sampler")?.as_u64())
        .and_then(|s| atlas.samplers.get(s as usize));
    ["wrapS", "wrapT"].map(|k| {
        sampler
            .and_then(|s| s.get(k))
            .and_then(Value::as_u64)
            .map_or(crops::CLAMP, |w| w as u32)
    })
}

fn page_of(atlas: &PreparedAtlas, texture: &Value) -> Option<usize> {
    let texture = atlas
        .textures
        .get(usize::try_from(texture.as_u64()?).ok()?)?;
    texture
        .get("source")
        .or_else(|| texture.pointer("/extensions/KHR_texture_basisu/source"))
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
}

/// Leaf texel ownership after atlasing: the number of triangles whose published
/// decoded-UV footprint shares interior area with another's on some page. Every slot
/// binding and every ID-texture set of every prepared primitive is a footprint on its
/// page; a page read through the same (primitive, UV set) twice counts once.
pub(in crate::mesh_large) fn leaf_shared_triangles(
    atlas: &PreparedAtlas,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<u64, JobError> {
    let invalid = || {
        JobError::new(
            JobErrorKind::InvalidState,
            "texel ownership: prepared binding closure",
        )
    };
    let mut pages = BTreeMap::<usize, Vec<(usize, usize)>>::new();
    let mut wraps = BTreeMap::<usize, [u32; 2]>::new();
    for (p, primitive) in atlas.primitives.iter().enumerate() {
        let mut infos = Vec::new();
        if let Some(material) = &primitive.material {
            for (pointer, _) in SLOTS {
                if let Some(info) = material.pointer(pointer) {
                    infos.push(info);
                }
            }
        }
        for entry in &primitive.feature_id_textures {
            if let Some(info) = entry.get("texture") {
                infos.push(info);
            }
        }
        for info in infos {
            let page = page_of(atlas, &info["index"]).ok_or_else(invalid)?;
            let set = info.get("texCoord").and_then(Value::as_u64).unwrap_or(0) as usize;
            if primitive.uvs.get(set).is_none() {
                return Err(invalid());
            }
            wraps.insert(page, texture_wrap(atlas, &info["index"]));
            let users = pages.entry(page).or_default();
            if !users.contains(&(p, set)) {
                users.push((p, set));
            }
        }
    }
    let mut shared: Vec<Vec<bool>> = atlas
        .primitives
        .iter()
        .map(|p| vec![false; p.triangles.len()])
        .collect();
    for (&page, users) in &pages {
        check()?;
        let mut footprints = Vec::new();
        let mut owners = Vec::new();
        for &(p, set) in users {
            let uv = &atlas.primitives[p].uvs[set];
            for (i, corners) in uv.as_chunks::<3>().0.iter().enumerate() {
                footprints.push([corners[0], corners[1], corners[2]]);
                owners.push((p, i));
            }
        }
        let wrap = wraps[&page];
        let flags = if wrap == [crops::CLAMP; 2] {
            shared_footprints(&footprints, check)?
        } else {
            // A period page: copies of each footprint on the texels it reads.
            let (mut copies, mut copy_owner) = (Vec::new(), Vec::new());
            let mut wide = false;
            for (k, t) in footprints.iter().enumerate() {
                match period_copies(t, wrap) {
                    Some(c) => {
                        copy_owner.extend(std::iter::repeat_n(k, c.len()));
                        copies.extend(c);
                    }
                    None => wide = true,
                }
            }
            let mut flags = vec![wide; footprints.len()];
            if !wide {
                for (c, flag) in sweep(copies, check)?.into_iter().enumerate() {
                    flags[copy_owner[c]] |= flag;
                }
            }
            flags
        };
        for (k, flag) in flags.into_iter().enumerate() {
            if flag {
                let (p, i) = owners[k];
                *shared[p].get_mut(i).ok_or_else(invalid)? = true;
            }
        }
    }
    Ok(shared.iter().flatten().filter(|s| **s).count() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> impl FnMut() -> Result<(), JobError> {
        || Ok(())
    }

    #[test]
    fn period_copies_shift_negative_uvs_exactly() {
        // Edges at u = -0.1 and u = 0.9 (f32) a period apart: 1 + f32(-0.1) is just above
        // f32(0.9), a sliver an f32 shift rounds away (#306 review r2 N3).
        let a = [[-0.2f32, 0.], [-0.1, 0.], [-0.1, 1.]];
        let b = [[0.9f32, 0.], [1.0, 0.], [0.9, 1.]];
        let wrap = [10497, crops::CLAMP];
        let (ca, cb) = (
            period_copies(&a, wrap).unwrap(),
            period_copies(&b, wrap).unwrap(),
        );
        let owner: Vec<usize> = std::iter::repeat_n(0, ca.len())
            .chain(std::iter::repeat_n(1, cb.len()))
            .collect();
        let flags = sweep(ca.into_iter().chain(cb).collect(), &mut none()).unwrap();
        assert!([0, 1]
            .iter()
            .all(|&o| (0..owner.len()).any(|i| owner[i] == o && flags[i])));
    }
    #[test]
    fn material_key_ignores_texture_identity_and_fills_defaults() {
        let a = json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":3,"texCoord":1},
            "metallicFactor":0}, "normalTexture":{"index":4}});
        let b = json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":9},
            "metallicFactor":0.0,"roughnessFactor":1}, "normalTexture":{"index":2,"scale":1.0},
            "alphaCutoff":0.25});
        assert_eq!(material_key(&a, false), material_key(&b, false));
        assert_ne!(material_key(&a, true), material_key(&b, true));
        for different in [
            json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":3},"metallicFactor":0.5},
                "normalTexture":{"index":4}}),
            json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":3},"metallicFactor":0},
                "normalTexture":{"index":4,"scale":-1}}),
            json!({"pbrMetallicRoughness":{"metallicFactor":0}, "normalTexture":{"index":4}}),
            json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":3},"metallicFactor":0},
                "normalTexture":{"index":4},"doubleSided":true}),
            json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":3},"metallicFactor":0},
                "normalTexture":{"index":4},"alphaMode":"MASK"}),
        ] {
            assert_ne!(material_key(&a, false), material_key(&different, false));
        }
        // MASK cutoffs are part of the class; an OPAQUE cutoff is ignored by glTF.
        let mask = |c: f64| json!({"alphaMode":"MASK","alphaCutoff":c});
        assert_ne!(
            material_key(&mask(0.5), false),
            material_key(&mask(0.25), false)
        );
        assert_eq!(
            material_key(&mask(0.5), false),
            material_key(&json!({"alphaMode":"MASK"}), false)
        );
    }

    #[test]
    fn classes_split_on_sampler_filters_but_not_wrap_or_texture() {
        use crate::gltf_source::source::parse_materials_for_tests;
        let doc = json!({
            "materials":[
                {"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}},
                {"pbrMetallicRoughness":{"baseColorTexture":{"index":1}}},
                {"pbrMetallicRoughness":{"baseColorTexture":{"index":2}}},
                {"pbrMetallicRoughness":{"baseColorTexture":{"index":3}}},
                {}
            ],
            "textures":[{"source":0,"sampler":0},{"source":1,"sampler":1},
                {"source":2,"sampler":2},{"source":3}]
        });
        let samplers = vec![
            json!({"minFilter":9729,"magFilter":9729}),
            json!({"minFilter":9729,"magFilter":9729,"wrapS":33071}),
            json!({"minFilter":9728,"magFilter":9729}),
        ];
        let materials = parse_materials_for_tests(&doc);
        let textures = doc["textures"].as_array().unwrap().clone();
        let classes = MaterialClasses::new(&materials, &textures, &samplers).unwrap();
        assert_eq!(
            (0..6).map(|m| classes.of(m)).collect::<Vec<_>>(),
            [0, 0, 2, 3, 4, 5]
        );
        assert_eq!(classes.of(u32::MAX), u32::MAX);
    }

    #[test]
    fn exact_texel_ownership_accepts_shared_edges_and_corners_and_finds_overlaps() {
        let grid = |x: f32, y: f32| [x / 64., y / 64.];
        // A strip of edge-sharing triangles and a fan meeting at one corner.
        let mut clean = Vec::new();
        for i in 0..8 {
            let x = i as f32;
            clean.push([grid(x, 0.), grid(x + 1., 0.), grid(x, 1.)]);
            clean.push([grid(x + 1., 0.), grid(x + 1., 1.), grid(x, 1.)]);
        }
        for k in 0..6 {
            let a = std::f32::consts::TAU * k as f32 / 6.;
            let b = std::f32::consts::TAU * (k + 1) as f32 / 6.;
            clean.push([
                [0.5, 0.5],
                [0.5 + 0.1 * a.cos(), 0.5 + 0.1 * a.sin()],
                [0.5 + 0.1 * b.cos(), 0.5 + 0.1 * b.sin()],
            ]);
        }
        // A zero-area footprint inside another owns no interior.
        clean.push([grid(0.25, 0.25), grid(0.5, 0.5), grid(0.75, 0.75)]);
        let flags = shared_footprints(&clean, &mut none()).unwrap();
        assert!(flags.iter().all(|s| !s), "{flags:?}");
        // Overlap controls: a copy (mirrored source UVs), a sliver crossing an edge by
        // one 2^-24 step, and a nested triangle.
        let first = clean[0];
        let mut overlapping = clean.clone();
        overlapping.push(first);
        let flags = shared_footprints(&overlapping, &mut none()).unwrap();
        assert_eq!(flags.iter().filter(|s| **s).count(), 2);
        assert!(flags[0] && flags[flags.len() - 1]);
        let step = 2f32.powi(-24);
        // A sliver just right of the strip's first cell lies inside the next cell.
        let mut touching = clean.clone();
        touching.push([grid(1., 0.), [1. / 64. + step, 1. / 128.], grid(1., 1.)]);
        assert!(shared_footprints(&touching, &mut none()).unwrap()[clean.len()]);
        // The first cell's upper triangle, its free corner moved one step across the
        // shared diagonal; unmoved, the pair only shares the edge.
        let crossing = [grid(1., 0.), grid(1., 1.), [0., 1. / 64. - step]];
        assert_eq!(
            shared_footprints(&[clean[0], crossing], &mut none()).unwrap(),
            [true, true]
        );
        assert_eq!(
            shared_footprints(&[clean[0], clean[1]], &mut none()).unwrap(),
            [false, false]
        );
        let nested = [grid(0.2, 0.2), grid(0.4, 0.2), grid(0.2, 0.4)];
        assert_eq!(
            shared_footprints(&[clean[0], nested], &mut none()).unwrap(),
            [true, true]
        );
        // Outside the exact domain the verdict is conservative.
        let far = [[9., 9.], [10., 9.], [9., 10.]];
        assert_eq!(
            shared_footprints(&[far, clean[0]], &mut none()).unwrap(),
            [true, false]
        );
    }

    #[test]
    fn fixed_grid_is_exact_or_refused() {
        assert_eq!(fixed(0.), Some(0));
        assert_eq!(fixed(-0.), Some(0));
        assert_eq!(fixed(0.5), Some(1 << 55));
        assert_eq!(
            fixed(1. / 3.).map(|v| v as f64 / (1u64 << 56) as f64),
            Some(f64::from(1f32 / 3.))
        );
        // A power of two is on the grid however small; its next float is not.
        assert_eq!(fixed(2f32.powi(-40)), Some(1 << 16));
        assert_eq!(fixed(f32::from_bits(2f32.powi(-40).to_bits() + 1)), None);
        assert_eq!(fixed(8.), None);
        assert_eq!(fixed(f32::NAN), None);
    }
}
