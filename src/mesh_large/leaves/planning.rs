//! Physical crop/page projection BEFORE the dataset lattice is frozen. This
//! preserves H1 leaf ranges/counts; it neither encodes images nor claims final
//! compressed GLB bytes. Lossy late encoded overruns remain a delivery concern.
use super::super::{
    attributes::FullTriangle,
    config::{Profile, TreeConfig},
    records::{NodeKind, NodeRecord},
};
use super::atlas::{self, AtlasResources};
use crate::{
    spill::{Record, Scratch, Sequence, SequenceWriter},
    JobError, JobErrorKind,
};
use std::{cmp::Ordering, mem::size_of};

#[derive(Clone, Copy, Debug)]
pub(in crate::mesh_large) struct LeafPlan {
    pub first: u64,
    pub end: u64,
    pub depth: u8,
    pub density_drop: u8,
    pub pages: u64,
    pub base_texels: u64,
    pub chain_texels: u64,
    pub geometry_bytes: u64,
    pub owned_bytes: u64,
}
impl PartialEq for LeafPlan {
    fn eq(&self, other: &Self) -> bool {
        (self.first, self.depth) == (other.first, other.depth)
    }
}
impl Eq for LeafPlan {}
impl PartialOrd for LeafPlan {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for LeafPlan {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.first, self.depth).cmp(&(other.first, other.depth))
    }
}
impl Record for LeafPlan {
    const SIZE: usize = 64;
    fn encode(&self, out: &mut [u8]) {
        out.fill(0);
        for (bytes, value) in out[..56].as_chunks_mut::<8>().0.iter_mut().zip([
            self.first,
            self.end,
            self.pages,
            self.base_texels,
            self.chain_texels,
            self.geometry_bytes,
            self.owned_bytes,
        ]) {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
        out[56] = self.depth;
        out[57] = self.density_drop;
    }
    fn decode(bytes: &[u8]) -> Self {
        let values: [u64; 7] = std::array::from_fn(|i| {
            u64::from_le_bytes(
                bytes[i * 8..i * 8 + 8]
                    .try_into()
                    .expect("fixed LeafPlan record"),
            )
        });
        Self {
            first: values[0],
            end: values[1],
            pages: values[2],
            base_texels: values[3],
            chain_texels: values[4],
            geometry_bytes: values[5],
            owned_bytes: values[6],
            depth: bytes[56],
            density_drop: bytes[57],
        }
    }
}
fn state(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}
fn resource(message: &str) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message)
}

pub(in crate::mesh_large) fn preflight(
    scratch: &Scratch,
    triangles: &Sequence<FullTriangle>,
    nodes: &Sequence<NodeRecord>,
    resources: AtlasResources<'_>,
    config: &TreeConfig,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Sequence<LeafPlan>, JobError> {
    check()?;
    let mut records = triangles.reader()?;
    let mut plans = SequenceWriter::new(scratch, "leaf-plans", config.sort.buffer_bytes)?;
    let mut expected = 0u64;
    let mut prior_node = None;
    let mut leaf_count = 0u64;
    for node in nodes.reader()? {
        check()?;
        let node = node?;
        let key = (node.first, node.depth());
        if prior_node.is_some_and(|prior| prior >= key) {
            return Err(state("leaf projection tree node order/identity mismatch"));
        }
        prior_node = Some(key);
        if node.kind != NodeKind::Leaf {
            continue;
        }
        if node.first != expected || node.end <= node.first || node.end > triangles.len() {
            return Err(state(
                "leaf projection frozen ranges are not contiguous source coverage",
            ));
        }
        let count = node.end - node.first;
        if count > config.leaf_triangles() {
            return Err(state("leaf projection exceeds frozen triangle limit"));
        }
        let count = usize::try_from(count)
            .map_err(|_| resource("leaf projection count exceeds address domain"))?;
        count
            .checked_mul(size_of::<FullTriangle>())
            .ok_or_else(|| resource("leaf projection owned record bytes overflow"))?;
        let mut leaf = Vec::new();
        leaf.try_reserve_exact(count)
            .map_err(|_| resource("leaf projection record allocation"))?;
        for i in 0..count {
            if i.is_multiple_of(4096) {
                check()?;
            }
            leaf.push(
                records
                    .next()
                    .transpose()?
                    .ok_or_else(|| state("leaf projection source stream truncated"))?,
            );
        }
        let max_drop = if config.profile == Profile::Lossless {
            0
        } else {
            12
        };
        let mut selected = None;
        for density_drop in 0..=max_drop {
            check()?;
            match atlas::project(&leaf, resources, config.profile, density_drop, check) {
                Ok(projected) => {
                    if projected.chain_texels < projected.base_texels
                        || (projected.pages == 0) != (projected.base_texels == 0)
                    {
                        return Err(state("leaf physical projection counters inconsistent"));
                    }
                    selected = Some(LeafPlan {
                        first: node.first,
                        end: node.end,
                        depth: u8::try_from(node.depth())
                            .map_err(|_| resource("leaf projection depth overflow"))?,
                        density_drop: u8::try_from(density_drop)
                            .map_err(|_| resource("leaf projection density overflow"))?,
                        pages: projected.pages,
                        base_texels: projected.base_texels,
                        chain_texels: projected.chain_texels,
                        geometry_bytes: projected.geometry_bytes,
                        owned_bytes: projected.owned_bytes,
                    });
                    break;
                }
                Err(error)
                    if error.kind() == JobErrorKind::Unsupported && density_drop < max_drop => {}
                Err(error) => return Err(error),
            }
        }
        let plan = selected
            .ok_or_else(|| state("leaf projection finite density schedule produced no plan"))?;
        leaf_count = leaf_count
            .checked_add(1)
            .ok_or_else(|| resource("leaf plan count overflow"))?;
        leaf_count
            .checked_mul(LeafPlan::SIZE as u64)
            .ok_or_else(|| resource("leaf plan scratch byte count overflow"))?;
        plans.push(&plan)?;
        expected = node.end;
    }
    check()?;
    if expected != triangles.len() || records.next().transpose()?.is_some() {
        return Err(state(
            "leaf projection source/tree terminal coverage mismatch",
        ));
    }
    if plans.len() != leaf_count {
        return Err(state("leaf projection persisted record count mismatch"));
    }
    Ok(plans.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_record_roundtrip_and_key_order() {
        let original = LeafPlan {
            first: 7,
            end: 19,
            depth: 31,
            density_drop: 12,
            pages: 3,
            base_texels: 1024,
            chain_texels: 1365,
            geometry_bytes: 999,
            owned_bytes: 10000,
        };
        let mut bytes = [255; LeafPlan::SIZE];
        original.encode(&mut bytes);
        assert_eq!(&bytes[58..], &[0; 6]);
        let decoded = LeafPlan::decode(&bytes);
        let mut second = [0; LeafPlan::SIZE];
        decoded.encode(&mut second);
        assert_eq!(bytes, second);
        assert_eq!(&bytes[..8], &7u64.to_le_bytes());
        assert_eq!(bytes[56], 31);
        assert_eq!(bytes[57], 12);
        assert!(
            original
                < LeafPlan {
                    depth: 32,
                    ..original
                }
        );
        assert!(
            original
                < LeafPlan {
                    first: 8,
                    depth: 0,
                    ..original
                }
        );
    }
    fn fixture(scratch: &Scratch) -> (Sequence<FullTriangle>, Sequence<NodeRecord>) {
        use super::super::super::{
            attributes::CornerAttributes,
            records::{Bounds, Provenance, TriangleRecord},
        };
        let mut rows = SequenceWriter::new(scratch, "planning-test-triangles", 64).unwrap();
        for id in 0..2 {
            rows.push(&FullTriangle {
                triangle: TriangleRecord {
                    key: 0,
                    source: Provenance {
                        triangle: id,
                        ..Default::default()
                    },
                    vertices: [0, 1, 2],
                    positions: [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    bytes: 56,
                    degenerate: false,
                },
                expanded_ordinal: 0,
                material: u32::MAX,
                flags: 0,
                corners: [CornerAttributes::default(); 3],
            })
            .unwrap();
        }
        let mut nodes = SequenceWriter::new(scratch, "planning-test-nodes", 64).unwrap();
        for first in 0..2 {
            nodes
                .push(&NodeRecord {
                    cell_depth: 1,
                    cell: first,
                    path_len: 0,
                    path: 0,
                    last_octant: first as u8,
                    kind: NodeKind::Leaf,
                    children: 0,
                    first,
                    end: first + 1,
                    bytes: 56,
                    bounds: Bounds {
                        min: [0.0; 3],
                        max: [1.0, 1.0, 0.0],
                    },
                })
                .unwrap();
        }
        (rows.finish().unwrap(), nodes.finish().unwrap())
    }
    fn empty_resources() -> AtlasResources<'static> {
        static CLASSES: crate::mesh_large::batching::MaterialClasses =
            crate::mesh_large::batching::MaterialClasses::EMPTY;
        AtlasResources {
            materials: &[],
            classes: &CLASSES,
            textures: &[],
            samplers: &[],
            image_info: &[],
        }
    }
    #[test]
    fn owned_source_projection_matches_project_and_order() {
        let scratch = Scratch::create(None, 0).unwrap();
        let (triangles, nodes) = fixture(&scratch);
        let config = TreeConfig::new(Profile::Balanced);
        let plans = preflight(
            &scratch,
            &triangles,
            &nodes,
            empty_resources(),
            &config,
            &mut || Ok(()),
        )
        .unwrap();
        let plan_rows: Vec<_> = plans.reader().unwrap().map(Result::unwrap).collect();
        assert_eq!(plan_rows.len(), 2);
        for (i, plan) in plan_rows.iter().enumerate() {
            assert_eq!(plan.first, i as u64);
            assert_eq!(plan.end, i as u64 + 1);
            assert_eq!(plan.density_drop, 0);
            let row = triangles.reader().unwrap().nth(i).unwrap().unwrap();
            let reference =
                atlas::project(&[row], empty_resources(), Profile::Balanced, 0, &mut || {
                    Ok(())
                })
                .unwrap();
            assert_eq!(plan.pages, reference.pages);
            assert_eq!(plan.base_texels, reference.base_texels);
            assert_eq!(plan.chain_texels, reference.chain_texels);
            assert_eq!(plan.geometry_bytes, reference.geometry_bytes);
            assert_eq!(plan.owned_bytes, reference.owned_bytes);
        }
        drop(plans);
        drop(nodes);
        drop(triangles);
        scratch.close().unwrap();
    }
    #[test]
    fn early_cancellation_removes_only_unfinished_plan_stream() {
        let scratch = Scratch::create(None, 0).unwrap();
        let (triangles, nodes) = fixture(&scratch);
        let live = scratch.live_bytes();
        let config = TreeConfig::new(Profile::Balanced);
        let mut calls = 0;
        let error = preflight(
            &scratch,
            &triangles,
            &nodes,
            empty_resources(),
            &config,
            &mut || {
                calls += 1;
                if calls == 3 {
                    Err(JobError::new(JobErrorKind::Cancelled, "planning control"))
                } else {
                    Ok(())
                }
            },
        )
        .err()
        .expect("cancelled");
        assert_eq!(error.kind(), JobErrorKind::Cancelled);
        assert_eq!(scratch.live_bytes(), live);
        assert_eq!(triangles.len(), 2);
        assert_eq!(nodes.len(), 2);
        drop(nodes);
        drop(triangles);
        scratch.close().unwrap();
    }

    #[test]
    fn actual_crop_first_fit_drop_twelve_lossless_and_domain_endpoint() {
        use super::super::super::{
            attributes::{CornerAttributes, UV0},
            records::{Bounds, Provenance, TriangleRecord},
        };
        use crate::gltf_source::source::{Document, ImageInfo};
        let doc = serde_json::json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"material":0}]}],
            "buffers":[{"uri":"never-read.bin","byteLength":60}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":24}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
                {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
            "images":[{"uri":"never-read.png"}],"textures":[{"source":0,"sampler":0}],"samplers":[{"minFilter":9729}],"materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}]});
        let document = Document::parse(&serde_json::to_vec(&doc).unwrap(), || Ok(())).unwrap();
        let info = [ImageInfo {
            mime_type: "image/png",
            extension: "png",
            width: 4096,
            height: 1,
        }];
        let classes = crate::mesh_large::batching::MaterialClasses::default();
        let resources = AtlasResources {
            materials: document.materials(),
            classes: &classes,
            textures: document.textures(),
            samplers: document.samplers(),
            image_info: &info,
        };
        for (uv, profile, expected) in [
            (
                [[0.0, 0.0], [1.0, 0.0], [0.0, 0.0]],
                Profile::Fast,
                Some(2u8),
            ),
            (
                [[0.0, 0.0], [1022.0, 0.0], [0.0, 0.0]],
                Profile::Fast,
                Some(12),
            ),
            ([[1e6, 1e6]; 3], Profile::Fast, Some(0)),
            ([[0.0, 0.0], [1e6, 0.0], [0.0, 0.0]], Profile::Fast, None),
            (
                [[0.0, 0.0], [1022.0, 0.0], [0.0, 0.0]],
                Profile::Lossless,
                None,
            ),
        ] {
            let scratch = Scratch::create(None, 0).unwrap();
            let mut source = SequenceWriter::new(&scratch, "physical-test-source", 64).unwrap();
            let corners = std::array::from_fn(|i| CornerAttributes {
                texcoords: [uv[i], [0.0; 2]],
                ..Default::default()
            });
            source
                .push(&FullTriangle {
                    triangle: TriangleRecord {
                        key: 0,
                        source: Provenance::default(),
                        vertices: [0, 1, 2],
                        positions: [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                        bytes: 56,
                        degenerate: false,
                    },
                    expanded_ordinal: 0,
                    material: 0,
                    flags: UV0,
                    corners,
                })
                .unwrap();
            let source = source.finish().unwrap();
            let mut nodes = SequenceWriter::new(&scratch, "physical-test-nodes", 64).unwrap();
            nodes
                .push(&NodeRecord {
                    cell_depth: 1,
                    cell: 0,
                    path_len: 0,
                    path: 0,
                    last_octant: 0,
                    kind: NodeKind::Leaf,
                    children: 0,
                    first: 0,
                    end: 1,
                    bytes: 56,
                    bounds: Bounds {
                        min: [0.0; 3],
                        max: [1.0, 1.0, 0.0],
                    },
                })
                .unwrap();
            let nodes = nodes.finish().unwrap();
            let config = TreeConfig::new(profile);
            let before = scratch.live_bytes();
            let result = preflight(
                &scratch,
                &source,
                &nodes,
                resources,
                &config,
                &mut || Ok(()),
            );
            if let Some(expected_drop) = expected {
                let plan = result.unwrap();
                let row = plan.reader().unwrap().next().unwrap().unwrap();
                assert_eq!(row.density_drop, expected_drop);
                let original = source.reader().unwrap().next().unwrap().unwrap();
                let oracle = atlas::project(
                    &[original],
                    resources,
                    profile,
                    u32::from(expected_drop),
                    &mut || Ok(()),
                )
                .unwrap();
                assert_eq!(row.pages, oracle.pages);
                assert_eq!(row.base_texels, oracle.base_texels);
                assert_eq!(row.chain_texels, oracle.chain_texels);
                drop(plan);
            } else {
                assert_eq!(result.err().unwrap().kind(), JobErrorKind::Unsupported);
                assert_eq!(scratch.live_bytes(), before);
            }
            drop(nodes);
            drop(source);
            scratch.close().unwrap();
        }
    }
}
