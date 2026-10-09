//! Validated core PBR material bindings; no image ownership, I/O or encoding.
use super::{array, invalid, list, number, object, uint, unsupported, vector, Result};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mesh_archive) enum TextureSlot {
    BaseColor,
    MetallicRoughness,
    Normal,
    Occlusion,
    Emissive,
}

#[derive(Clone, Copy)]
enum Extra {
    Scale,
    Strength,
}

struct SlotSpec {
    slot: TextureSlot,
    path: &'static [&'static str],
    extra: Option<Extra>,
}

// This finite table is the sole mapping from semantic roles to JSON locations.
const SLOTS: [SlotSpec; 5] = [
    SlotSpec {
        slot: TextureSlot::BaseColor,
        path: &["pbrMetallicRoughness", "baseColorTexture"],
        extra: None,
    },
    SlotSpec {
        slot: TextureSlot::MetallicRoughness,
        path: &["pbrMetallicRoughness", "metallicRoughnessTexture"],
        extra: None,
    },
    SlotSpec {
        slot: TextureSlot::Normal,
        path: &["normalTexture"],
        extra: Some(Extra::Scale),
    },
    SlotSpec {
        slot: TextureSlot::Occlusion,
        path: &["occlusionTexture"],
        extra: Some(Extra::Strength),
    },
    SlotSpec {
        slot: TextureSlot::Emissive,
        path: &["emissiveTexture"],
        extra: None,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mesh_archive) struct TextureBinding {
    pub texture: usize,
    pub texcoord: usize,
    pub slot: TextureSlot,
}

#[derive(Clone, Debug)]
pub(in crate::mesh_archive) struct Material {
    value: Value,
    bindings: [Option<TextureBinding>; 5],
}

impl Material {
    pub(in crate::mesh_archive) fn bindings(&self) -> impl Iterator<Item = &TextureBinding> {
        self.bindings.iter().flatten()
    }

    pub(in crate::mesh_archive) fn has_normal_texture(&self) -> bool {
        self.bindings()
            .any(|binding| binding.slot == TextureSlot::Normal)
    }

    /// Preserve every admitted field, including omissions; replace only indices.
    pub(in crate::mesh_archive) fn remap(
        &self,
        texture_ids: &BTreeMap<usize, usize>,
    ) -> Result<Value> {
        let mut value = self.value.clone();
        for (spec, binding) in SLOTS.iter().zip(&self.bindings) {
            let Some(binding) = binding else {
                continue;
            };
            let local = texture_ids
                .get(&binding.texture)
                .ok_or_else(|| invalid("prepared material texture missing from local closure"))?;
            let mut info = &mut value;
            for key in spec.path {
                info = info
                    .get_mut(*key)
                    .ok_or_else(|| invalid("prepared material binding path is absent"))?;
            }
            let index = info
                .get_mut("index")
                .ok_or_else(|| invalid("prepared material binding index is absent"))?;
            *index = Value::from(*local);
        }
        Ok(value)
    }
}

fn unit(value: &Value) -> Result<()> {
    if !(0.0..=1.0).contains(&number(value)?) {
        return Err(invalid("material factor outside [0,1]"));
    }
    Ok(())
}

fn binding(
    material: &Value,
    spec: &SlotSpec,
    texture_count: usize,
) -> Result<Option<TextureBinding>> {
    let mut info = material;
    for key in spec.path {
        let Some(next) = info.get(*key) else {
            return Ok(None);
        };
        info = next;
    }
    let allowed: &[&str] = match spec.extra {
        Some(Extra::Scale) => &["index", "texCoord", "scale"],
        Some(Extra::Strength) => &["index", "texCoord", "strength"],
        None => &["index", "texCoord"],
    };
    object(info, allowed)?;
    let texture = uint(&info["index"])?;
    if texture >= texture_count {
        return Err(invalid("material texture reference out of range"));
    }
    let texcoord = info.get("texCoord").map(uint).transpose()?.unwrap_or(0);
    if texcoord > 1 {
        return Err(unsupported(
            "core material textures support UV sets 0 and 1",
        ));
    }
    match spec.extra {
        Some(Extra::Scale) => {
            if let Some(scale) = info.get("scale") {
                // The core schema allows any finite number, including negative.
                number(scale)?;
            }
        }
        Some(Extra::Strength) => {
            if let Some(strength) = info.get("strength") {
                unit(strength)?;
            }
        }
        None => {}
    }
    Ok(Some(TextureBinding {
        texture,
        texcoord,
        slot: spec.slot,
    }))
}

pub(super) fn parse(doc: &Value) -> Result<Vec<Material>> {
    let input = list(doc, "materials")?;
    if input.len() > 256 {
        return Err(unsupported("core material count exceeds 256"));
    }
    let texture_count = list(doc, "textures")?.len();
    let mut materials = Vec::with_capacity(input.len());
    for material in input {
        object(
            material,
            &[
                "name",
                "pbrMetallicRoughness",
                "normalTexture",
                "occlusionTexture",
                "emissiveTexture",
                "emissiveFactor",
                "alphaMode",
                "alphaCutoff",
                "doubleSided",
            ],
        )?;
        if let Some(pbr) = material.get("pbrMetallicRoughness") {
            object(
                pbr,
                &[
                    "baseColorFactor",
                    "metallicFactor",
                    "roughnessFactor",
                    "baseColorTexture",
                    "metallicRoughnessTexture",
                ],
            )?;
            if let Some(factor) = pbr.get("baseColorFactor") {
                vector::<4>(factor)?;
                for component in array(factor)? {
                    unit(component)?;
                }
            }
            for key in ["metallicFactor", "roughnessFactor"] {
                if let Some(factor) = pbr.get(key) {
                    unit(factor)?;
                }
            }
        }
        if let Some(factor) = material.get("emissiveFactor") {
            vector::<3>(factor)?;
            for component in array(factor)? {
                unit(component)?;
            }
        }
        if let Some(mode) = material.get("alphaMode") {
            match mode.as_str() {
                Some("OPAQUE" | "MASK") => {}
                Some("BLEND") => {
                    return Err(unsupported(
                        "partitioning does not preserve blended draw order",
                    ));
                }
                _ => return Err(invalid("invalid alphaMode")),
            }
        }
        if material
            .get("alphaCutoff")
            .map(number)
            .transpose()?
            .is_some_and(|cutoff| cutoff < 0.)
        {
            return Err(invalid("negative alphaCutoff"));
        }
        if material.get("doubleSided").is_some_and(|v| !v.is_boolean()) {
            return Err(invalid("doubleSided must be boolean"));
        }
        let mut bindings = [None; 5];
        for (index, spec) in SLOTS.iter().enumerate() {
            bindings[index] = binding(material, spec, texture_count)?;
        }
        let mut value = material.clone();
        value
            .as_object_mut()
            .ok_or_else(|| invalid("material must be an object"))?
            .remove("name");
        materials.push(Material { value, bindings });
    }
    Ok(materials)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JobErrorKind;
    use serde_json::json;

    fn document(material: Value) -> Value {
        json!({"textures":[{"source":0},{"source":0},{"source":1}],"materials":[material]})
    }

    #[test]
    fn all_roles_remap_without_inventing_defaults_or_merging_bindings() {
        let original = json!({
            "name":"descriptive label",
            "pbrMetallicRoughness":{
                "baseColorTexture":{"index":2},
                "metallicRoughnessTexture":{"index":0,"texCoord":1},
                "metallicFactor":0.4
            },
            "normalTexture":{"index":1,"texCoord":0,"scale":-2.0},
            "occlusionTexture":{"index":0,"strength":0},
            "emissiveTexture":{"index":2,"texCoord":1},
            "emissiveFactor":[0.1,0.2,0.3],"alphaMode":"MASK","doubleSided":false
        });
        let material = parse(&document(original)).unwrap().remove(0);
        assert!(material.has_normal_texture());
        assert_eq!(
            material
                .bindings()
                .map(|b| (b.texture, b.texcoord))
                .collect::<Vec<_>>(),
            vec![(2, 0), (0, 1), (1, 0), (0, 0), (2, 1)]
        );
        assert_eq!(
            material
                .remap(&BTreeMap::from([(0, 7), (1, 4), (2, 9)]))
                .unwrap(),
            json!({
                "pbrMetallicRoughness":{
                    "baseColorTexture":{"index":9},
                    "metallicRoughnessTexture":{"index":7,"texCoord":1},
                    "metallicFactor":0.4
                },
                "normalTexture":{"index":4,"texCoord":0,"scale":-2.0},
                "occlusionTexture":{"index":7,"strength":0},
                "emissiveTexture":{"index":9,"texCoord":1},
                "emissiveFactor":[0.1,0.2,0.3],"alphaMode":"MASK","doubleSided":false
            })
        );
        assert_eq!(
            material
                .remap(&BTreeMap::from([(0, 0), (2, 1)]))
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidInput
        );
    }

    #[test]
    fn malformed_bindings_and_valid_exclusions_have_distinct_kinds() {
        for (material, kind) in [
            (json!({"normalTexture":{}}), JobErrorKind::InvalidInput),
            (
                json!({"normalTexture":{"index":true}}),
                JobErrorKind::InvalidInput,
            ),
            (
                json!({"normalTexture":{"index":3}}),
                JobErrorKind::InvalidInput,
            ),
            (
                json!({"normalTexture":{"index":0,"texCoord":-1}}),
                JobErrorKind::InvalidInput,
            ),
            (
                json!({"normalTexture":{"index":0,"texCoord":0.5}}),
                JobErrorKind::InvalidInput,
            ),
            (
                json!({"emissiveTexture":{"index":0,"texCoord":2}}),
                JobErrorKind::Unsupported,
            ),
            (
                json!({"normalTexture":{"index":0,"scale":"one"}}),
                JobErrorKind::InvalidInput,
            ),
            (
                json!({"occlusionTexture":{"index":0,"strength":1.1}}),
                JobErrorKind::InvalidInput,
            ),
            (
                json!({"emissiveTexture":{"index":0,"scale":1}}),
                JobErrorKind::Unsupported,
            ),
            (json!({"alphaMode":"BLEND"}), JobErrorKind::Unsupported),
        ] {
            assert_eq!(parse(&document(material)).unwrap_err().kind(), kind);
        }
    }

    #[test]
    fn no_textures_and_boundary_scalars_preserve_presence() {
        let plain = parse(&json!({"materials":[{}]})).unwrap().remove(0);
        assert_eq!(plain.bindings().count(), 0);
        assert!(!plain.has_normal_texture());
        assert_eq!(plain.remap(&BTreeMap::new()).unwrap(), json!({}));
        for scale in [-2.0, 0.0, 1.0, 3.0] {
            for strength in [0.0, 1.0] {
                let value = json!({"normalTexture":{"index":0,"scale":scale},
                    "occlusionTexture":{"index":0,"strength":strength}});
                let material = parse(&document(value.clone())).unwrap().remove(0);
                assert_eq!(material.remap(&BTreeMap::from([(0, 0)])).unwrap(), value);
            }
        }
    }
}
