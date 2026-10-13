//! Immutable local closure handed to the leaf wire writer after atlas preparation.
use super::super::attributes::FullTriangle;
use serde_json::Value;

pub(super) struct PreparedPrimitive {
    pub triangles: Vec<FullTriangle>,
    pub corner_depths: Vec<[u8; 3]>,
    /// Dense output TEXCOORD streams, each in exact triangle/corner order.
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub material: Option<Value>,
}
pub(super) struct PreparedPage {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}
pub(super) struct PreparedAtlas {
    pub primitives: Vec<PreparedPrimitive>,
    pub pages: Vec<PreparedPage>,
    pub textures: Vec<Value>,
    pub samplers: Vec<Value>,
}
