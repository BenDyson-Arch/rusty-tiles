//! Immutable geometry and the actual complete-member assembly handoff.
use super::frames::Frame;
use crate::{
    gltf_source::source::TextureSlot,
    mesh_large::{config::Profile, records::NodeRecord},
    JobError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceKey {
    pub node: u32,
    pub mesh: u32,
    pub primitive: u32,
    pub instance: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PositionWord {
    Float([u32; 3]),
    U16([u16; 3]),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PositionTransform {
    pub translation: [u32; 3],
    pub scale: [u32; 3],
}
impl Default for PositionTransform {
    fn default() -> Self {
        Self {
            translation: [0; 3],
            scale: [1.0f32.to_bits(); 3],
        }
    }
}
impl PositionTransform {
    pub(crate) fn decode(self, word: PositionWord) -> [f32; 3] {
        let p = match word {
            PositionWord::Float(bits) => bits.map(f32::from_bits),
            PositionWord::U16(c) => c.map(f32::from),
        };
        std::array::from_fn(|i| {
            p[i] * f32::from_bits(self.scale[i]) + f32::from_bits(self.translation[i])
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FeatureIdentity {
    pub set: u32,
    pub table: Option<u32>,
    pub value: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct FrozenFace {
    pub ordinal: u64,
    pub source: SourceKey,
    pub source_triangle: u64,
    pub material: u32,
    pub positions: [[f32; 3]; 3],
    pub words: [PositionWord; 3],
    pub transform: PositionTransform,
    /// Original welded source positions BEFORE proxy lattice encoding.
    pub welds: [[u32; 3]; 3],
    pub features: Vec<FeatureIdentity>,
    /// Certified same-primitive donor distance, fixed BEFORE the fill was accepted.
    pub fill_bound: Option<f64>,
    /// Exact immutable eligibility/correspondence restriction. Sorted disjoint
    /// source-rank intervals, never inferred from a fill triangle sentinel.
    pub donor_ranges: Vec<std::ops::Range<u64>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FaceIdentity {
    pub ordinal: u64,
    pub source: SourceKey,
    pub source_triangle: u64,
    pub material: u32,
    pub position_bits: [[u32; 3]; 3],
    pub words: [PositionWord; 3],
    pub transform: PositionTransform,
    pub features: Vec<FeatureIdentity>,
}
impl FrozenFace {
    pub(crate) fn identity(&self) -> FaceIdentity {
        FaceIdentity {
            ordinal: self.ordinal,
            source: self.source,
            source_triangle: self.source_triangle,
            material: self.material,
            position_bits: self.positions.map(|p| p.map(f32::to_bits)),
            words: self.words,
            transform: self.transform,
            features: self.features.clone(),
        }
    }
}
pub(crate) struct FrozenNode {
    pub node: NodeRecord,
    pub original_glb: Vec<u8>,
    pub original_sha256: [u8; 32],
    pub geometric_error: f64,
    /// Maximum actual post-drop child texel spacing in local metres.
    pub child_texel_size: f64,
    pub faces: Vec<FrozenFace>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct BakeLimits {
    pub profile: Profile,
    pub memory_bytes: u64,
    pub member_bytes: u64,
    pub source_cache_bytes: u64,
    pub query_work: u64,
    pub max_page_edge: u32,
    pub max_density_drops: u32,
}
#[derive(Clone, Debug)]
pub(crate) struct RenderCorner {
    pub face: usize,
    pub corner: u8,
    pub position: PositionWord,
    pub transform: PositionTransform,
    pub uv: [f32; 2],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub color: [f32; 4],
}
pub(crate) struct EncodedPage {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub size: [u32; 2],
    pub slot: TextureSlot,
    pub material: u32,
    pub sampler: serde_json::Value,
    pub receipt: serde_json::Value,
}
/// Child appearance BEFORE codec encoding; never reconstructed from KTX2/PNG.
pub(crate) struct PreencodeChild {
    pub node: NodeRecord,
    pub texel_size: f64,
    pub faces: Vec<ChildFace>,
    pub pages: Vec<SignalPage>,
}
pub(crate) struct ChildFace {
    pub source: SourceKey,
    pub source_triangle: u64,
    pub material: u32,
    pub positions: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
    pub frames: [Frame; 3],
    /// Page for each original material binding, in TextureSlot order.
    pub pages: [Option<usize>; 5],
}
pub(crate) struct SignalPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub support: Vec<u32>,
    pub slot: TextureSlot,
    pub samples: Vec<TerminalSample>,
    /// Complete unadjusted semantic chain, retained before codec and MASK guard.
    pub levels: Vec<SignalLevel>,
    pub sampler: serde_json::Value,
    pub material: u32,
    pub receipt: serde_json::Value,
    /// Finite renderer-frame witnesses owned once, never replicated per mip.
    pub normal_samples: Vec<NormalSample>,
    /// H8 leaf donors retain literal scale; proxy donors have absorbed it once.
    pub normal_scale: f64,
}
pub(crate) struct SignalLevel {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub support: Vec<u32>,
    /// Representative shader frame of each normal texel (including mixed charts).
    pub frames: Vec<Frame>,
    pub samples: Vec<TerminalSample>,
    pub world_normals: Vec<[f64; 3]>,
}
#[derive(Clone, Copy)]
pub(crate) struct NormalSample {
    pub uv: [f64; 2],
    pub frame: Frame,
    pub chart: usize,
    pub weight: f64,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct TerminalSample {
    pub source_rank: u64,
    pub barycentric: [f64; 3],
    pub centre: [f64; 3],
    pub support_min: [f64; 3],
    pub support_max: [f64; 3],
    pub weight: u32,
    /// A weighted correspondence centre is not necessarily on a source face.
    /// source_rank/barycentric remains a separate actual surface witness.
    pub centre_is_centroid: bool,
}
pub(crate) struct PreparedNode {
    /// Residual lease for complete writer/meshopt/readback/output ownership.
    /// The adapter must account it before allocations, including Budget paths.
    pub assembly_memory_bytes: u64,
    /// These exact faces have zero geometric area: retain every corner/feature/
    /// source/COLOR, with no invented texture support or appearance claim.
    pub no_raster_faces: Vec<usize>,
    pub corners: Vec<RenderCorner>,
    pub face_indices: Vec<[u32; 3]>,
    pub face_identities: Vec<FaceIdentity>,
    pub pages: Vec<EncodedPage>,
    /// Complete literal material closure keyed by ORIGINAL source material ID.
    pub source_materials: std::collections::BTreeMap<u32, serde_json::Value>,
    /// Page indexes by source face and TextureSlot order; multiple materials never merge.
    pub face_pages: Vec<[Option<usize>; 5]>,
    /// Arbitrary feature sets keep their original integer attributes. These
    /// additional exact PNG texture sets are appended by the complete writer.
    pub ids: Vec<IdPacket>,
    pub receipts: serde_json::Value,
}
pub(crate) struct IdPacket {
    pub page: usize,
    pub faces: Vec<usize>,
    pub sets: Vec<FeatureIdentity>,
    pub texture: crate::mesh_large::leaves::id_texture::IdTexture,
}
pub(crate) enum Assembly {
    Fit {
        glb: Vec<u8>,
        decoded: Vec<FaceIdentity>,
    },
    PhysicalBudget {
        actual_bytes: u64,
    },
}
pub(crate) type Assemble<'a> = dyn FnMut(&PreparedNode) -> Result<Assembly, JobError> + 'a;
pub(crate) struct BakedNode {
    pub glb: Vec<u8>,
    pub preencode: PreencodeChild,
    pub receipts: serde_json::Value,
}
pub(super) fn slot_index(slot: TextureSlot) -> usize {
    match slot {
        TextureSlot::BaseColor => 0,
        TextureSlot::MetallicRoughness => 1,
        TextureSlot::Normal => 2,
        TextureSlot::Occlusion => 3,
        TextureSlot::Emissive => 4,
    }
}
