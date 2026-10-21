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
/// min/mag filter) is equal. Wrap modes are not part of a class: both tile writers
/// resolve wrapping into page content and publish `CLAMP_TO_EDGE`.
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
            let (amin, amax) = a.iter().map(project).fold((i128::MAX, i128::MIN), |m, v| {
                (m.0.min(v), m.1.max(v))
            });
            let (bmin, bmax) = b.iter().map(project).fold((i128::MAX, i128::MIN), |m, v| {
                (m.0.min(v), m.1.max(v))
            });
            if amax <= bmin || bmax <= amin {
                return false;
            }
        }
    }
    true
}

/// Per footprint, whether its decoded-UV interior shares area with another footprint on
/// the same page. Exact for coordinates on the 2^-56 grid below 8 in magnitude; a
/// footprint outside that domain, and any footprint whose box overlaps its box, counts as
/// shared. A footprint of zero exact area owns no interior and is never shared.
pub(in crate::mesh_large) fn shared_footprints(
    footprints: &[[[f32; 2]; 3]],
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Vec<bool>, JobError> {
    let n = footprints.len();
    let mut shared = vec![false; n];
    // Exact coordinates, or None (outside the exact domain).
    let exact: Vec<Option<Fixed>> = footprints
        .iter()
        .map(|t| {
            let mut out = [[0i128; 2]; 3];
            for (c, p) in t.iter().enumerate() {
                for a in 0..2 {
                    out[c][a] = i128::from(fixed(p[a])?);
                }
            }
            Some(out)
        })
        .collect();
    // f64 boxes (f32 values are exact in f64) for the sweep; degenerate exact footprints
    // own no interior and leave the sweep.
    let mut order: Vec<(usize, [f64; 2], [f64; 2])> = Vec::with_capacity(n);
    for (i, t) in footprints.iter().enumerate() {
        match &exact[i] {
            Some(f) if twice_area(f) == 0 => continue,
            Some(_) => {}
            None => shared[i] = true,
        }
        let lo = [0, 1].map(|a| t.iter().map(|p| f64::from(p[a])).fold(f64::INFINITY, f64::min));
        let hi =
            [0, 1].map(|a| t.iter().map(|p| f64::from(p[a])).fold(f64::NEG_INFINITY, f64::max));
        // A non-finite box overlaps everything near it; it is already shared.
        let lo = lo.map(|v| if v.is_nan() { f64::NEG_INFINITY } else { v });
        let hi = hi.map(|v| if v.is_nan() { f64::INFINITY } else { v });
        order.push((i, lo, hi));
    }
    order.sort_by(|x, y| x.1[0].total_cmp(&y.1[0]).then(x.0.cmp(&y.0)));
    let mut active: Vec<usize> = Vec::new();
    for (k, &(i, lo, hi)) in order.iter().enumerate() {
        if k % 4096 == 0 {
            check()?;
        }
        // Boxes that end at or before this start can only touch: a vertical line
        // separates them, so their interiors are disjoint.
        active.retain(|&j| order[j].2[0] > lo[0]);
        for &j in &active {
            let (o, olo, ohi) = order[j];
            if ohi[1] <= lo[1] || hi[1] <= olo[1] {
                continue;
            }
            let overlap = match (&exact[i], &exact[o]) {
                (Some(a), Some(b)) => interiors_overlap(a, b),
                _ => true,
            };
            if overlap {
                shared[i] = true;
                shared[o] = true;
            }
        }
        active.push(k);
    }
    Ok(shared)
}

fn page_of(atlas: &PreparedAtlas, texture: &Value) -> Option<usize> {
    let texture = atlas.textures.get(usize::try_from(texture.as_u64()?).ok()?)?;
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
    for users in pages.values() {
        check()?;
        let mut footprints = Vec::new();
        let mut owners = Vec::new();
        for &(p, set) in users {
            let uv = &atlas.primitives[p].uvs[set];
            for (i, corners) in uv.chunks_exact(3).enumerate() {
                footprints.push([corners[0], corners[1], corners[2]]);
                owners.push((p, i));
            }
        }
        for (k, flag) in shared_footprints(&footprints, check)?.into_iter().enumerate() {
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
        assert_ne!(material_key(&mask(0.5), false), material_key(&mask(0.25), false));
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
        assert_eq!(fixed(1. / 3.).map(|v| v as f64 / (1u64 << 56) as f64), Some(f64::from(1f32 / 3.)));
        // A power of two is on the grid however small; its next float is not.
        assert_eq!(fixed(2f32.powi(-40)), Some(1 << 16));
        assert_eq!(fixed(f32::from_bits(2f32.powi(-40).to_bits() + 1)), None);
        assert_eq!(fixed(8.), None);
        assert_eq!(fixed(f32::NAN), None);
    }
}
