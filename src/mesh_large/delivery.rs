//! H16 preparation only: bounded H1 addressing and exact generated-member budget.
//! Test-only registered preparation pending M1/H8 integration. No publisher,
//! serializer copy, legacy fallback or whole-tree representation is provided.
use super::records::{Bounds, NodeKind, NodeRecord, KEY_DEPTH};
use crate::{implicit::Coordinates, JobError, JobErrorKind};

fn state(message: &'static str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}

/// Logical transport address, not a spatial containment box. Synthetic run
/// slots are appended after the spatial prefix; packed leaves use their FIRST
/// slot. Last-octant coverage remains metadata on the original H1 record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Address {
    depth: u32,
    axes: [u32; 3],
}

impl Address {
    pub(super) fn from_node(node: &NodeRecord) -> Result<Self, JobError> {
        if node.cell_depth > KEY_DEPTH
            || node.depth() > 31
            || (node.path_len > 0 && node.cell_depth != KEY_DEPTH)
            || node.cell >= (1_u64 << (3 * u32::from(node.cell_depth)))
            || node.path >= (1_u64 << (3 * u32::from(node.path_len)))
            || node.last_octant > 7
            || node.last_octant < node.slot()
            || node.first >= node.end
            || (node.kind == NodeKind::Leaf && node.children != 0)
            || (node.kind == NodeKind::Internal && !(1..=8).contains(&node.children))
        {
            return Err(state("H16 received an invalid captured node identity"));
        }
        let mut axes = [0_u32; 3];
        for (length, bits) in [(node.cell_depth, node.cell), (node.path_len, node.path)] {
            for level in (0..length).rev() {
                let slot = (bits >> (3 * u32::from(level))) & 7;
                for (axis, value) in axes.iter_mut().enumerate() {
                    *value = (*value << 1) | ((slot >> axis) & 1) as u32;
                }
            }
        }
        Ok(Self {
            depth: node.depth(),
            axes,
        })
    }

    pub(super) fn relative_to(self, root: Self) -> Result<Coordinates, JobError> {
        let level = self
            .depth
            .checked_sub(root.depth)
            .ok_or_else(|| state("subtree root is deeper than its tile"))?;
        if self
            .axes
            .iter()
            .zip(root.axes)
            .any(|(value, ancestor)| value >> level != ancestor)
        {
            return Err(state("tile is outside its declared logical subtree root"));
        }
        let mask = (1_u32 << level) - 1;
        Ok(Coordinates {
            level,
            x: self.axes[0] & mask,
            y: self.axes[1] & mask,
            z: self.axes[2] & mask,
        })
    }
}

/// Exact basis conversion of final decoded GLB bounds, not H1 centroid cells.
/// glTF default Y-up -> Tiles-local Z-up: (x, y, z) -> (x, -z, y).
pub(super) fn tile_bounds(decoded: Bounds) -> Result<Bounds, JobError> {
    if (0..3).any(|a| {
        !decoded.min[a].is_finite()
            || !decoded.max[a].is_finite()
            || decoded.min[a] > decoded.max[a]
    }) {
        return Err(state("decoded leaf bounds are not a finite ordered box"));
    }
    Ok(Bounds {
        min: [decoded.min[0], -decoded.max[2], decoded.min[1]],
        max: [decoded.max[0], -decoded.min[2], decoded.max[1]],
    })
}

/// Counts from the frozen chosen layout, including all JSON documents/debug
/// members. The index indexes these members, excluding itself. The archive
/// member ceiling includes its one final index member, as the H1 gate does.
#[derive(Clone, Copy, Debug)]
pub(super) struct GeneratedMembers {
    pub payloads: u64,
    pub subtrees: u64,
    pub documents: u64,
    pub debug: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MemberBudget {
    pub indexed: u64,
    pub total: u64,
    pub index_bytes: u64,
}
impl GeneratedMembers {
    pub(super) fn check(self) -> Result<MemberBudget, JobError> {
        if self.documents == 0 {
            return Err(state("delivery has no root tileset document"));
        }
        let overflow = || {
            JobError::new(
                JobErrorKind::ResourceLimit,
                "delivery member arithmetic exceeds u64",
            )
        };
        let indexed = self
            .payloads
            .checked_add(self.subtrees)
            .and_then(|n| n.checked_add(self.documents))
            .and_then(|n| n.checked_add(self.debug))
            .ok_or_else(overflow)?;
        let total = indexed.checked_add(1).ok_or_else(overflow)?;
        if total > super::config::MEMBER_CEILING_3TZ {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                "3TZ delivery exceeds 178,956,970 total members",
            ));
        }
        let index_bytes = indexed.checked_mul(24).ok_or_else(overflow)?;
        if index_bytes > u64::from(u32::MAX) {
            return Err(state("accepted 3TZ count exceeded its index wire bound"));
        }
        Ok(MemberBudget {
            indexed,
            total,
            index_bytes,
        })
    }
}

/// Honest preparation gate. Replace with a bridge returning M1 TileRecord,
/// ContentRef and TilesetDocument once their owning branch merges. Do not
/// introduce local facsimiles of those types or serialize a placeholder.
pub(super) fn publication_pending() -> JobError {
    JobError::new(JobErrorKind::Unsupported, "H16 hierarchy publication is not wired; merged M1 typed serializer and H8 leaves are required")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_is_explicitly_pending() {
        assert_eq!(publication_pending().kind(), JobErrorKind::Unsupported);
    }
    fn node(cell_depth: u8, cell: u64, path_len: u8, path: u64, last_octant: u8) -> NodeRecord {
        NodeRecord {
            cell_depth,
            cell,
            path_len,
            path,
            last_octant,
            kind: NodeKind::Leaf,
            children: 0,
            first: 0,
            end: 1,
            bytes: 93,
            bounds: Bounds {
                min: [0.; 3],
                max: [1.; 3],
            },
        }
    }
    #[test]
    fn packed_first_slot_and_synthetic_depth31_are_addressed() {
        let packed = node(1, 2, 0, 0, 6);
        assert!(packed.is_packed());
        assert_eq!(
            Address::from_node(&packed).unwrap(),
            Address {
                depth: 1,
                axes: [0, 1, 0]
            }
        );
        let synthetic = node(21, 0, 10, (1 << 30) - 1, 7);
        let address = Address::from_node(&synthetic).unwrap();
        assert_eq!(address.depth, 31);
        assert_eq!(address.axes, [1023; 3]);
        let root = Address::from_node(&node(21, 0, 0, 0, 0)).unwrap();
        assert_eq!(
            address.relative_to(root).unwrap(),
            Coordinates {
                level: 10,
                x: 1023,
                y: 1023,
                z: 1023
            }
        );
        assert!(address
            .relative_to(Address {
                depth: 21,
                axes: [1, 0, 0]
            })
            .is_err());
    }
    #[test]
    fn default_gltf_axis_conversion_uses_final_bounds() {
        assert_eq!(
            tile_bounds(Bounds {
                min: [1., 2., 3.],
                max: [4., 5., 6.]
            })
            .unwrap(),
            Bounds {
                min: [1., -6., 2.],
                max: [4., -3., 5.]
            }
        );
    }
    #[test]
    fn exact_archive_boundary_includes_index_and_overflow_is_resource() {
        let count = super::super::config::MEMBER_CEILING_3TZ;
        let good = GeneratedMembers {
            payloads: count - 2,
            subtrees: 0,
            documents: 1,
            debug: 0,
        };
        assert_eq!(good.check().unwrap().total, count);
        assert_eq!(
            GeneratedMembers {
                payloads: count - 1,
                ..good
            }
            .check()
            .unwrap_err()
            .kind(),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            GeneratedMembers {
                payloads: u64::MAX,
                ..good
            }
            .check()
            .unwrap_err()
            .kind(),
            JobErrorKind::ResourceLimit
        );
    }
}
