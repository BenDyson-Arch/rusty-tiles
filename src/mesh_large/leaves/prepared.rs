//! Immutable local closure handed to the leaf wire writer after atlas preparation.
use super::super::attributes::FullTriangle;
use serde_json::Value;

pub(in crate::mesh_large) struct PreparedPrimitive {
    pub triangles: Vec<FullTriangle>,
    pub corner_depths: Vec<[u8; 3]>,
    /// Dense output TEXCOORD streams, each in exact triangle/corner order.
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub material: Option<Value>,
    /// Further `EXT_mesh_features.featureIds` entries after the two source
    /// attribute sets, e.g. ID textures from `id_texture`; any count.
    pub feature_id_textures: Vec<Value>,
}
pub(in crate::mesh_large) struct PreparedPage {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}
pub(in crate::mesh_large) struct PreparedAtlas {
    pub primitives: Vec<PreparedPrimitive>,
    pub pages: Vec<PreparedPage>,
    pub textures: Vec<Value>,
    pub samplers: Vec<Value>,
    /// The accepted pages' PRE-ENCODE values, page order, when a bake pass will consume
    /// them (empty otherwise).
    pub preencode: Vec<PreencodePage>,
}

/// One pre-encode level: RGBA8 texels before any codec.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(in crate::mesh_large) struct PreencodeLevel {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A leaf page exactly as authored before encoding, with its sampling context. Pages built
/// from original-source mips carry every authored level; packed pages carry the base level
/// and its support mask (`1` where a crop interior texel is real source support), from
/// which `delivery::encode` derived its sparse chain.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(in crate::mesh_large) struct PreencodePage {
    pub page: u32,
    /// glTF material slot role (`baseColor`, `normal`, ...).
    pub slot: String,
    /// `colour`, `normal`, `data` or `id`.
    pub class: String,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<PreencodeLevel>,
    pub support: Option<Vec<u32>>,
    pub min_filter: u32,
    pub mag_filter: u32,
    pub density_drop: u32,
    pub source_mips: bool,
}
