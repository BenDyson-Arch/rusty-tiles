//! H17's immutable handoff adapter and H27's real complete-member protocol.
//! Missing owner leases/retained leaf frames are explicit integration errors;
//! no untextured or geometry-only substitute can satisfy this entry point.
use super::{assembler, bake_node, child_bridge, invalid, resource, types::*, CapturedSource};
use crate::{
    mesh_large::{
        bake_pass as root,
        lod::handoff,
        records::{Bounds, NodeKind, NodeRecord},
    },
    JobError,
};

/// Policy resolved by the caller from the actual stage-two allowance. Profile
/// and complete-member budget are owned by the live pass and set from it.
#[derive(Debug)]
pub(crate) struct ProxyTextureBaker {
    limits: BakeLimits,
}
impl ProxyTextureBaker {
    pub(crate) fn new(limits: BakeLimits) -> Result<Self, JobError> {
        if limits.memory_bytes == 0
            || limits.source_cache_bytes == 0
            || limits.source_cache_bytes >= limits.memory_bytes
            || limits.query_work == 0
            || limits.max_page_edge < 4
            || !limits.max_page_edge.is_power_of_two()
        {
            return Err(invalid("resolved proxy bake resource policy"));
        }
        Ok(Self { limits })
    }
    /// The production policy, resolved once from the actual run before the bake stage:
    /// - memory: half of what the host reports available after the captured state's
    ///   owned scratch (memory-backed), metadata and the stage reserve; a host that cannot
    ///   report its memory gets a fixed 1 GiB, never an unbounded allowance;
    /// - source cache: room for the largest decoded source image's mip chain (32 bytes
    ///   per source pixel, at least 1 MiB), at most a quarter of the memory;
    /// - query work: a finite runaway guard of 2^40 index visits per tile;
    /// - page edge: the profile's leaf page edge; eight complete-member density drops.
    pub(crate) fn for_stage(
        profile: crate::mesh_large::config::Profile,
        captured: &crate::mesh_large::full::FullCaptured,
    ) -> Result<Self, JobError> {
        use crate::mesh_large::config::{available_memory, Profile};
        const RESERVE: u64 = 256 << 20;
        let tmpfs = if captured.scratch.memory_backed() {
            captured.report.scratch.need_bytes
        } else {
            0
        };
        let owned = captured
            .report
            .memory
            .owned_metadata_bytes
            .saturating_add(captured.report.memory.geometry.document_bytes);
        let memory_bytes = match available_memory() {
            Some(a) => {
                a.saturating_sub(tmpfs)
                    .saturating_sub(owned)
                    .saturating_sub(RESERVE)
                    / 2
            }
            None => 1 << 30,
        };
        let largest = captured
            .image_info
            .iter()
            .map(|i| u64::from(i.width) * u64::from(i.height))
            .max()
            .unwrap_or(0);
        let source_cache_bytes = largest
            .saturating_mul(32)
            .max(1 << 20)
            .min(memory_bytes / 4);
        if memory_bytes < 64 << 20 {
            return Err(resource(
                "available memory cannot hold the proxy texture bake stage",
            ));
        }
        Self::new(BakeLimits {
            profile,
            memory_bytes,
            member_bytes: crate::mesh_large::config::TILE_BYTES,
            source_cache_bytes,
            query_work: 1 << 40,
            max_page_edge: match profile {
                Profile::Fast => 2048,
                Profile::Lossless => 8192,
                Profile::Balanced | Profile::Quality => 4096,
            },
            max_density_drops: 8,
        })
    }
    pub(crate) fn limits(&self) -> BakeLimits {
        self.limits
    }
    pub(super) fn limits_for(&self, pass: &root::BakePass<'_>) -> BakeLimits {
        BakeLimits {
            profile: pass.profile,
            member_bytes: pass.tile_bytes,
            ..self.limits
        }
    }
}
impl root::ProxyBaker for ProxyTextureBaker {
    /// Bake every coarse tile in postorder (children before parents): read the frozen
    /// packet and the certified member under exact leases, load the retained children,
    /// run the core with the owner's leased assembler, commit the verified bytes, retain
    /// this tile's exact packet for its parent, then release its children and packet.
    fn bake(&self, pass: &root::BakePass<'_>) -> Result<(), JobError> {
        (pass.check)()?;
        let limits = self.limits_for(pass);
        if limits.member_bytes == 0 || pass.jobs == 0 {
            return Err(invalid("live proxy bake pass policy"));
        }
        let mut check = || (pass.check)();
        let mut nodes = pass.nodes()?.peekable();
        if nodes.peek().is_none() {
            return Ok(());
        }
        let mut source = CapturedSource::new(pass.captured, limits.source_cache_bytes, &mut check)?;
        for next in nodes {
            check()?;
            let (first, depth) = next?;
            if first >= pass.captured.triangles.len() {
                return Err(invalid("proxy row outside captured hierarchy"));
            }
            // The decoded packet stays resident beside the core, so its exact lease comes
            // off the stage allowance first; the core owns the remainder.
            let lease = pass.frozen_lease_bytes(first, depth)?;
            let core = limits
                .memory_bytes
                .checked_sub(lease)
                .filter(|core| *core >= lease)
                .ok_or_else(|| resource("frozen tile packet exceeds the bake memory lease"))?;
            let frozen = pass.frozen(first, depth, lease)?;
            let parent = node_record(&frozen)?;
            let glb = pass.read_certified_proxy(first, depth, frozen.glb_bytes)?;
            let children =
                child_bridge::load_children(pass, &frozen, &parent, core / 2, &mut check)?;
            let accepted = bake_one(
                &frozen,
                glb,
                &mut source,
                &children,
                BakeLimits {
                    memory_bytes: core,
                    ..limits
                },
                &mut |prepared, memory| pass.assemble(&frozen, prepared, memory),
                &mut check,
            )?;
            drop(children);
            pass.commit(&frozen, accepted.assembly)?;
            if depth > 0 {
                child_bridge::persist(pass.preencode, &accepted.preencode, core, &mut check)?;
            }
            drop(accepted.preencode);
            child_bridge::release_children(pass.preencode, &frozen)?;
            pass.release(first, depth)?;
        }
        Ok(())
    }
}

fn validate_transforms(frozen: &handoff::FrozenNode) -> Result<(), JobError> {
    for gltf in &frozen.gltf {
        let scale = f32::from_bits(gltf.scale[0]);
        if !scale.is_finite()
            || scale <= 0.
            || gltf.scale != [gltf.scale[0]; 3]
            || gltf
                .translation
                .iter()
                .any(|&word| !f32::from_bits(word).is_finite())
        {
            return Err(invalid(
                "immutable H17 transform must be finite positive uniform",
            ));
        }
    }
    Ok(())
}

pub(super) fn node_record(frozen: &handoff::FrozenNode) -> Result<NodeRecord, JobError> {
    let id = &frozen.identity;
    if id.depth != u32::from(id.cell_depth) + u32::from(id.path_len) {
        return Err(invalid("handoff node depth/identity disagreement"));
    }
    let mut bounds = Bounds::EMPTY;
    for face in &frozen.faces {
        for corner in &face.corners {
            bounds.include(corner.decoded.map(f32::from_bits));
        }
    }
    Ok(NodeRecord {
        cell_depth: id.cell_depth,
        cell: id.cell,
        path_len: id.path_len,
        path: id.path,
        last_octant: id.last_octant,
        kind: NodeKind::Internal,
        children: u8::try_from(frozen.children.len())
            .map_err(|_| invalid("handoff child count"))?,
        first: id.first,
        end: id.end,
        bytes: frozen.glb_bytes,
        bounds,
    })
}

fn words(raw: [u32; 3], quantized: bool) -> Result<PositionWord, JobError> {
    if quantized {
        Ok(PositionWord::U16([
            u16::try_from(raw[0]).map_err(|_| invalid("frozen u16 POSITION range"))?,
            u16::try_from(raw[1]).map_err(|_| invalid("frozen u16 POSITION range"))?,
            u16::try_from(raw[2]).map_err(|_| invalid("frozen u16 POSITION range"))?,
        ]))
    } else {
        Ok(PositionWord::Float(raw))
    }
}

fn features(
    frozen: &handoff::FrozenNode,
    face: usize,
    values: &[(u32, Option<u32>)],
) -> Result<Vec<FeatureIdentity>, JobError> {
    let face = frozen
        .faces
        .get(face)
        .ok_or_else(|| invalid("decoded face index"))?;
    let gltf = frozen
        .gltf
        .get(face.gltf as usize)
        .ok_or_else(|| invalid("decoded glTF primitive index"))?;
    let declarations = gltf
        .feature_ids
        .as_array()
        .ok_or_else(|| invalid("frozen feature declaration array"))?;
    let mut result = Vec::with_capacity(values.len());
    let mut values = values.iter();
    for declaration in declarations {
        let Some(attribute) = declaration
            .get("attribute")
            .and_then(serde_json::Value::as_u64)
        else {
            continue;
        };
        let &(value, table) = values
            .next()
            .ok_or_else(|| invalid("decoded feature declaration/value closure"))?;
        result.push(FeatureIdentity {
            set: u32::try_from(attribute)
                .map_err(|_| invalid("frozen feature attribute number"))?,
            table,
            value: u64::from(value),
        });
    }
    if values.next().is_some() {
        return Err(invalid("undeclared frozen feature value"));
    }
    Ok(result)
}

/// The caller has leased the immutable owner packet, certified GLB and children
/// before invoking this conversion. Core packet_charge checks their combined
/// resident ownership again, including cloned dynamic features/ranges.
pub(super) fn frozen_node(
    frozen: &handoff::FrozenNode,
    original_glb: Vec<u8>,
    children: &[PreencodeChild],
    memory_bytes: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<FrozenNode, JobError> {
    frozen.validate()?;
    validate_transforms(frozen)?;
    if original_glb.len() as u64 != frozen.glb_bytes {
        return Err(invalid("certified proxy byte count"));
    }
    let mut charge = original_glb.capacity() as u64;
    for face in &frozen.faces {
        charge = charge
            .checked_add(std::mem::size_of::<FrozenFace>() as u64)
            .and_then(|n| {
                n.checked_add(
                    face.features.len() as u64 * std::mem::size_of::<FeatureIdentity>() as u64,
                )
            })
            .and_then(|n| {
                n.checked_add(
                    frozen.donor_ranges(face).len() as u64
                        * std::mem::size_of::<std::ops::Range<u64>>() as u64,
                )
            })
            .ok_or_else(|| resource("frozen bake adapter ownership"))?;
    }
    if charge > memory_bytes {
        return Err(resource("frozen bake adapter lease"));
    }
    let mut faces = Vec::with_capacity(frozen.faces.len());
    for (ordinal, face) in frozen.faces.iter().enumerate() {
        check()?;
        let gltf = &frozen.gltf[face.gltf as usize];
        let source = &frozen.primitives[face.source as usize];
        let fill_bound = face.fill.map(|fill| frozen.fills[fill as usize].bound);
        let mut position_words = [PositionWord::Float([0; 3]); 3];
        for (word, corner) in position_words.iter_mut().zip(&face.corners) {
            *word = words(corner.raw, gltf.quantized)?;
        }
        faces.push(FrozenFace {
            ordinal: ordinal as u64,
            source: SourceKey {
                node: source.node,
                mesh: source.mesh,
                primitive: source.primitive,
                instance: source.expanded_ordinal,
            },
            source_triangle: face.origin.triangle,
            material: source.material,
            positions: face.corners.map(|c| c.decoded.map(f32::from_bits)),
            words: position_words,
            transform: PositionTransform {
                translation: gltf.translation,
                scale: gltf.scale,
            },
            welds: face.corners.map(|c| c.weld),
            features: features(frozen, ordinal, &face.features)?,
            fill_bound,
            donor_ranges: frozen.donor_ranges(face).to_vec(),
        });
    }
    Ok(FrozenNode {
        node: node_record(frozen)?,
        original_glb,
        original_sha256: frozen.glb_sha256,
        geometric_error: frozen.geometric_error,
        child_texel_size: children
            .iter()
            .map(|child| child.texel_size)
            .fold(0., f64::max),
        faces,
    })
}

fn decoded_identity(
    frozen: &handoff::FrozenNode,
    face: &root::FaceIdentity,
) -> Result<FaceIdentity, JobError> {
    let original = frozen
        .faces
        .get(face.face as usize)
        .ok_or_else(|| invalid("assembled face identity index"))?;
    let gltf = &frozen.gltf[original.gltf as usize];
    let mut raw = [PositionWord::Float([0; 3]); 3];
    for (word, values) in raw.iter_mut().zip(face.raw) {
        *word = words(values, gltf.quantized)?;
    }
    Ok(FaceIdentity {
        ordinal: u64::from(face.face),
        source: SourceKey {
            node: face.source.0,
            mesh: face.source.1,
            primitive: face.source.2,
            instance: face.source.3,
        },
        source_triangle: face.origin.triangle,
        material: face.material,
        position_bits: face.decoded,
        words: raw,
        // The root independently verified actual assembled TRS against these
        // exact frozen bits before producing the private Verified token.
        transform: PositionTransform {
            translation: gltf.translation,
            scale: gltf.scale,
        },
        features: features(frozen, face.face as usize, &face.features)?,
    })
}

pub(super) struct AcceptedNode {
    pub assembly: root::Assembly,
    pub preencode: PreencodeChild,
    pub receipts: serde_json::Value,
}
/// A real, owner-leased complete assembler. Its allowance includes translated
/// packet, native meshopt/codec, document, decoded readback and output ownership.
pub(super) type LeasedAssembler<'a> =
    dyn FnMut(&root::PreparedNode, u64) -> Result<root::Assembly, JobError> + 'a;

/// Exercise the unchanged core with the actual verified root Assembly. Only
/// the GLB Vec is moved through core validation, then reunited with the exact
/// original private Verified token. The caller can pass this to pass.commit.
pub(super) fn bake_one(
    frozen: &handoff::FrozenNode,
    original_glb: Vec<u8>,
    source: &mut CapturedSource<'_>,
    children: &[PreencodeChild],
    limits: BakeLimits,
    assemble: &mut LeasedAssembler<'_>,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<AcceptedNode, JobError> {
    let core = frozen_node(frozen, original_glb, children, limits.memory_bytes, check)?;
    let checks = std::cell::RefCell::new(check);
    let mut accepted = None;
    let mut callback = |prepared: &PreparedNode| {
        let packet = assembler::prepare(frozen, prepared, &mut || (checks.borrow_mut())())?;
        (checks.borrow_mut())()?;
        match assemble(&packet.node, packet.writer_memory_bytes)? {
            root::Assembly::PhysicalBudget { actual_bytes } => {
                Ok(Assembly::PhysicalBudget { actual_bytes })
            }
            root::Assembly::Fit {
                glb,
                decoded,
                verified,
            } => {
                let identities = decoded
                    .iter()
                    .map(|face| decoded_identity(frozen, face))
                    .collect::<Result<Vec<_>, _>>()?;
                accepted = Some((decoded, verified));
                Ok(Assembly::Fit {
                    glb,
                    decoded: identities,
                })
            }
        }
    };
    let baked = bake_node(&core, source, children, limits, &mut callback, &mut || {
        (checks.borrow_mut())()
    })?;
    let (decoded, verified) =
        accepted.ok_or_else(|| invalid("bake core accepted without real verified assembly"))?;
    Ok(AcceptedNode {
        assembly: root::Assembly::Fit {
            glb: baked.glb,
            decoded,
            verified,
        },
        preencode: baked.preencode,
        receipts: baked.receipts,
    })
}

#[cfg(test)]
pub(super) fn handoff_fixture() -> handoff::FrozenNode {
    use crate::mesh_large::lod::mesh::Origin;
    use serde_json::json;
    let primitives = (0..2)
        .map(|row| handoff::FrozenPrimitive {
            node: 10 + row,
            mesh: 20 + row,
            primitive: 30 + row,
            expanded_ordinal: 40 + u64::from(row),
            material: 5,
            row,
            error: 0.25,
            donor_ranges: std::iter::once(u64::from(row)..u64::from(row) + 1).collect(),
        })
        .collect();
    let faces = (0..2)
        .map(|source| {
            let positions: [[f32; 3]; 3] = if source == 0 {
                [[-0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
            } else {
                [[0.; 3]; 3]
            };
            handoff::FrozenFace {
                gltf: 0,
                source,
                corners: positions.map(|position| handoff::FrozenCorner {
                    raw: position.map(f32::to_bits),
                    decoded: position.map(|v| if v == 0. { 0 } else { v.to_bits() }),
                    weld: position.map(|v| if v == 0. { 0 } else { v.to_bits() }),
                    ell: 0,
                }),
                origin: Origin {
                    node: 10 + source,
                    mesh: 20 + source,
                    primitive: 30 + source,
                    triangle: u64::from(source) + 100,
                },
                fill: None,
                features: vec![(source, Some(0)), (99 + source, Some(3))],
            }
        })
        .collect();
    handoff::FrozenNode {
        identity: handoff::NodeIdentity {
            cell_depth: 0,
            cell: 0,
            path_len: 0,
            path: 0,
            last_octant: 0,
            depth: 0,
            first: 0,
            end: 2,
        },
        profile: "balanced".into(),
        geometric_error: 0.25,
        proxy_to_source: 0.25,
        source_to_proxy: 0.25,
        lattice_sha256: String::new(),
        glb_sha256: [0; 32],
        glb_bytes: 0,
        primitives,
        gltf: vec![handoff::FrozenGltfPrimitive {
            node: 0,
            mesh: 0,
            index: 0,
            sources: 0..2,
            material: Some(0),
            translation: [0; 3],
            scale: [1.0f32.to_bits(); 3],
            quantized: false,
            feature_ids: json!([
                {"attribute":7,"featureCount":2,"propertyTable":0,"label":"source_primitive"},
                {"attribute":3,"featureCount":101,"propertyTable":3,"label":"custom"}
            ]),
            faces: 0..2,
        }],
        faces,
        fills: Vec::new(),
        vanished: Vec::new(),
        children: Vec::new(),
        tables: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handoff_maps_each_source_with_shared_material_gltf_and_original_attribute_numbers() {
        let frozen = handoff_fixture();
        let mapped = frozen_node(&frozen, Vec::new(), &[], 1 << 20, &mut || Ok(())).unwrap();
        assert_eq!(mapped.faces[0].source.instance, 40);
        assert_eq!(mapped.faces[1].source.instance, 41);
        assert_eq!(mapped.faces[1].source.node, 11);
        assert_eq!(mapped.faces[1].source_triangle, 101);
        assert_eq!(mapped.faces[1].donor_ranges, vec![1..2]);
        assert_eq!(mapped.faces[1].features[0].set, 7);
        assert_eq!(mapped.faces[1].features[1].set, 3);
        assert_eq!(mapped.faces[1].features[1].table, Some(3));
        assert_eq!(
            mapped.faces[0].words[0],
            PositionWord::Float([(-0.0f32).to_bits(), 0, 0])
        );
        assert_eq!(mapped.faces[0].positions[0][0].to_bits(), 0);
    }
    #[test]
    fn handoff_transform_rejects_nonuniform_reflected_and_nonfinite_before_bake() {
        for scale in [[1., 2., 1.], [-1.; 3], [f32::INFINITY; 3]] {
            let mut frozen = handoff_fixture();
            frozen.gltf[0].scale = scale.map(f32::to_bits);
            assert_eq!(
                frozen_node(&frozen, Vec::new(), &[], 1 << 20, &mut || Ok(()))
                    .err()
                    .unwrap()
                    .kind(),
                crate::JobErrorKind::InvalidState
            );
        }
        let frozen = handoff_fixture();
        assert_eq!(
            frozen_node(&frozen, Vec::new(), &[], 1, &mut || Ok(()))
                .err()
                .unwrap()
                .kind(),
            crate::JobErrorKind::ResourceLimit
        );
    }
}
