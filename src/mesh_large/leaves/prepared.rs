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
}
