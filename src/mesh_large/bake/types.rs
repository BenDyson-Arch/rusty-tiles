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
    /// Maximum over the retained children of their texel spacing in local metres (a
    /// leaf child's is its area-weighted median face spacing; see `child_bridge`).
    pub child_texel_size: f64,
    pub faces: Vec<FrozenFace>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct BakeLimits {
    pub profile: Profile,
    pub memory_bytes: u64,
    pub member_bytes: u64,
    pub source_cache_bytes: u64,
    /// Bytes of the lease the bake's shared decoded-image cache may hold beyond the
    /// workers' decode reservations: each of `n` running workers reserves
    /// `source_cache_bytes + shared_image_bytes / n` (see `integration::run_depth`).
    pub shared_image_bytes: u64,
    /// The most bytes a parent's cache of child texel tiles may own; it also gets at
    /// most an eighth of the parent's lease (see `retained::TexelCache`).
    pub child_texel_bytes: u64,
    pub query_work: u64,
    pub max_page_edge: u32,
    pub max_density_drops: u32,
    /// Bake categorical feature-ID texture pages (`baked_feature_id_*`). Off in
    /// production (H28): every proxy feature-ID set is already an exact per-face vertex
    /// attribute against the same table. Kept reachable for a future annotate pass.
    pub id_textures: bool,
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
    /// The material class representative of every face on the page (H28).
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
/// A real leaf may crop/remap each material slot independently; the page and
/// its actual decoded per-face coordinates are one inseparable binding.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChildBinding {
    pub page: usize,
    pub uv: [[f32; 2]; 3],
}
pub(crate) struct ChildFace {
    pub source: SourceKey,
    pub source_triangle: u64,
    pub material: u32,
    pub positions: [[f32; 3]; 3],
    pub frames: [Frame; 3],
    /// Concrete page+UV for each original slot, in TextureSlot order.
    pub bindings: [Option<ChildBinding>; 5],
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
    /// The material class representative of every face on the page (H28); a leaf page
    /// witness names the material of the binding it serves.
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
    /// Per face, the vertex attributes its source primitive's leaves carry
    /// (`attributes::NORMAL`, `attributes::COLOR`): a proxy publishes NORMAL and COLOR_0
    /// only where its leaves do, so both render with the same shading model.
    pub face_attributes: Vec<u8>,
    pub corners: Vec<RenderCorner>,
    pub face_indices: Vec<[u32; 3]>,
    pub face_identities: Vec<FaceIdentity>,
    pub pages: Vec<EncodedPage>,
    /// Complete literal material closure keyed by material CLASS representative (H28):
    /// every source material of a class has equal literal factors and slots.
    pub source_materials: std::collections::BTreeMap<u32, serde_json::Value>,
    /// The class representative of each source material present (H28).
    pub material_classes: std::collections::BTreeMap<u32, u32>,
    /// Page indexes by source face and TextureSlot order; a page serves one material class.
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
    pub stats: BakeStats,
}
/// Run diagnostics of one tile's bake, never part of any output bytes: wall seconds
/// of its steps over every density attempt, and the accepted attempt's appearance
/// fallbacks.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BakeStats {
    pub charts_seconds: f64,
    pub raster_seconds: f64,
    pub encode_seconds: f64,
    pub assemble_seconds: f64,
    pub facing_fallback_samples: u64,
    pub facing_fallback_faces: u64,
    pub fill_fallback_samples: u64,
    /// Source images this tile decoded into its sampling cache, and their seconds.
    pub source_decodes: u64,
    pub source_decode_seconds: f64,
    /// Child texel tiles read from retained packets.
    pub child_tile_reads: u64,
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
