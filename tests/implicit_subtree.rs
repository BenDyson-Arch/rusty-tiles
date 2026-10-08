//! Independent byte-level checks of the implicit subtree writer. The decoder
//! below uses no writer internals or Morton library.
use rusty_tiles::implicit::{Coordinates, SubdivisionScheme, Subtree};
use serde_json::Value;

fn at(level: u32, x: u32, y: u32, z: u32) -> Coordinates {
    Coordinates { level, x, y, z }
}

fn chunks(bytes: &[u8]) -> (Value, &[u8]) {
    assert_eq!(&bytes[..4], b"subt");
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 1);
    let json_len = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
    let bin_len = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
    assert_eq!(json_len % 8, 0);
    assert_eq!(bin_len % 8, 0);
    assert_eq!(bytes.len(), 24 + json_len + bin_len);
    let json = &bytes[24..24 + json_len];
    let end = json.iter().rposition(|&b| b == b'}').unwrap() + 1;
    assert!(json[end..].iter().all(|&b| b == b' '));
    let doc: Value = serde_json::from_slice(json).unwrap();
    let binary = &bytes[24 + json_len..];
    if bin_len == 0 {
        assert!(doc.get("buffers").is_none());
        assert!(doc.get("bufferViews").is_none());
    } else {
        assert_eq!(doc["buffers"].as_array().unwrap().len(), 1);
        assert!(doc["buffers"][0].get("uri").is_none());
        let length = doc["buffers"][0]["byteLength"].as_u64().unwrap() as usize;
        assert!(length <= binary.len());
        assert!(binary[length..].iter().all(|&b| b == 0));
        let mut previous_end = 0;
        for view in doc["bufferViews"].as_array().unwrap() {
            assert_eq!(view["buffer"], 0);
            let offset = view["byteOffset"].as_u64().unwrap() as usize;
            let len = view["byteLength"].as_u64().unwrap() as usize;
            assert_eq!(offset % 8, 0);
            assert!(offset >= previous_end);
            assert!(offset + len <= length);
            assert!(binary[previous_end..offset].iter().all(|&b| b == 0));
            previous_end = offset + len;
        }
    }
    (doc, binary)
}

fn available(doc: &Value, binary: &[u8], availability: &Value, len: usize) -> Vec<usize> {
    if let Some(constant) = availability["constant"].as_u64() {
        assert!(availability.get("bitstream").is_none());
        return match constant {
            0 => Vec::new(),
            1 => (0..len).collect(),
            _ => panic!("invalid availability constant"),
        };
    }
    let view = &doc["bufferViews"][availability["bitstream"].as_u64().unwrap() as usize];
    let offset = view["byteOffset"].as_u64().unwrap() as usize;
    let byte_len = view["byteLength"].as_u64().unwrap() as usize;
    assert_eq!(byte_len, len.div_ceil(8));
    let bits = &binary[offset..offset + byte_len];
    for bit in len..byte_len * 8 {
        assert_eq!(bits[bit / 8] & (1 << (bit % 8)), 0, "nonzero trailing bit");
    }
    let indices: Vec<_> = (0..len)
        .filter(|&bit| bits[bit / 8] & (1 << (bit % 8)) != 0)
        .collect();
    assert_eq!(availability["availableCount"], indices.len());
    indices
}

#[test]
fn root_only_subtree_has_no_binary_chunk_or_content_templates() {
    for scheme in [SubdivisionScheme::Quadtree, SubdivisionScheme::Octree] {
        let mut subtree = Subtree::new(scheme, 1, 0).unwrap();
        subtree.set_tile(at(0, 0, 0, 0), &[]).unwrap();
        let bytes = subtree.to_bytes().unwrap();
        let (doc, binary) = chunks(&bytes);
        assert_eq!(doc["tileAvailability"]["constant"], 1);
        assert_eq!(doc["childSubtreeAvailability"]["constant"], 0);
        assert!(doc.get("contentAvailability").is_none());
        assert!(binary.is_empty());
    }
}

#[test]
fn quadtree_sparse_levels_multiple_contents_and_child_links() {
    let mut subtree = Subtree::new(SubdivisionScheme::Quadtree, 3, 2).unwrap();
    subtree.set_tile(at(0, 0, 0, 0), &[true, false]).unwrap();
    subtree.set_tile(at(1, 1, 0, 0), &[false, true]).unwrap();
    subtree.set_tile(at(2, 2, 1, 0), &[true, true]).unwrap();
    subtree.set_child_subtree(at(3, 5, 2, 0)).unwrap();
    let bytes = subtree.to_bytes().unwrap();
    let (doc, binary) = chunks(&bytes);
    // Level offsets are 0, 1, 5. Morton codes are 0, 1, 6.
    assert_eq!(
        available(&doc, binary, &doc["tileAvailability"], 21),
        [0, 2, 11]
    );
    assert_eq!(
        available(&doc, binary, &doc["contentAvailability"][0], 21),
        [0, 11]
    );
    assert_eq!(
        available(&doc, binary, &doc["contentAvailability"][1], 21),
        [2, 11]
    );
    // Child-subtree indexing has no breadth-first offset.
    assert_eq!(
        available(&doc, binary, &doc["childSubtreeAvailability"], 64),
        [25]
    );
}

#[test]
fn octree_sparse_levels_use_x_y_z_bit_order() {
    let mut subtree = Subtree::new(SubdivisionScheme::Octree, 3, 1).unwrap();
    subtree.set_tile(at(0, 0, 0, 0), &[false]).unwrap();
    subtree.set_tile(at(1, 1, 0, 1), &[true]).unwrap();
    subtree.set_tile(at(2, 3, 0, 2), &[true]).unwrap();
    subtree.set_child_subtree(at(3, 6, 0, 5)).unwrap();
    let bytes = subtree.to_bytes().unwrap();
    let (doc, binary) = chunks(&bytes);
    // Level offsets are 0, 1, 9. Morton codes are 0, 5, 41.
    assert_eq!(
        available(&doc, binary, &doc["tileAvailability"], 73),
        [0, 6, 50]
    );
    assert_eq!(
        available(&doc, binary, &doc["contentAvailability"][0], 73),
        [6, 50]
    );
    assert_eq!(
        available(&doc, binary, &doc["childSubtreeAvailability"], 512),
        [332]
    );
}

#[test]
fn complete_levels_and_empty_content_slots_use_constants() {
    for (scheme, branches, depth) in [
        (SubdivisionScheme::Quadtree, 4, 1),
        (SubdivisionScheme::Octree, 8, 2),
    ] {
        let mut subtree = Subtree::new(scheme, 2, 2).unwrap();
        subtree.set_tile(at(0, 0, 0, 0), &[true, false]).unwrap();
        for z in 0..depth {
            for y in 0..2 {
                for x in 0..2 {
                    subtree.set_tile(at(1, x, y, z), &[true, false]).unwrap();
                }
            }
        }
        let bytes = subtree.to_bytes().unwrap();
        let (doc, binary) = chunks(&bytes);
        assert_eq!(
            available(&doc, binary, &doc["tileAvailability"], branches + 1).len(),
            branches + 1
        );
        assert_eq!(doc["tileAvailability"]["constant"], 1);
        assert_eq!(doc["contentAvailability"][0]["constant"], 1);
        assert_eq!(doc["contentAvailability"][1]["constant"], 0);
        assert_eq!(doc["childSubtreeAvailability"]["constant"], 0);
        assert!(binary.is_empty());
    }
}

#[test]
fn single_level_subtrees_link_all_immediate_children() {
    for (scheme, branches, depth) in [
        (SubdivisionScheme::Quadtree, 4, 1),
        (SubdivisionScheme::Octree, 8, 2),
    ] {
        let mut subtree = Subtree::new(scheme, 1, 1).unwrap();
        subtree.set_tile(at(0, 0, 0, 0), &[true]).unwrap();
        for z in 0..depth {
            for y in 0..2 {
                for x in 0..2 {
                    subtree.set_child_subtree(at(1, x, y, z)).unwrap();
                }
            }
        }
        let bytes = subtree.to_bytes().unwrap();
        let (doc, binary) = chunks(&bytes);
        assert_eq!(
            available(&doc, binary, &doc["childSubtreeAvailability"], branches).len(),
            branches
        );
        assert_eq!(doc["childSubtreeAvailability"]["constant"], 1);
        assert!(binary.is_empty());
    }
}

#[test]
fn exhaustive_morton_indices_match_independent_bit_interleaving() {
    for (scheme, dimensions) in [
        (SubdivisionScheme::Quadtree, 2),
        (SubdivisionScheme::Octree, 3),
    ] {
        let branches = 1usize << dimensions;
        let levels = 4;
        let len = (branches.pow(levels) - 1) / (branches - 1);
        let mut subtree = Subtree::new(scheme, levels, 1).unwrap();
        let mut expected = Vec::new();
        for level in 0..levels {
            let width = 1 << level;
            let z_width = if dimensions == 3 { width } else { 1 };
            for z in 0..z_width {
                for y in 0..width {
                    for x in 0..width {
                        // An asymmetric subset distinguishes all coordinate lanes.
                        let has_content = (x + 2 * y + 3 * z) % 5 == 0;
                        subtree
                            .set_tile(at(level, x, y, z), &[has_content])
                            .unwrap();
                        if has_content {
                            let mut morton = 0usize;
                            for bit in 0..level {
                                for (lane, axis) in [x, y, z].iter().enumerate().take(dimensions) {
                                    morton |= (((axis >> bit) & 1) as usize)
                                        << (bit as usize * dimensions + lane);
                                }
                            }
                            expected.push((branches.pow(level) - 1) / (branches - 1) + morton);
                        }
                    }
                }
            }
        }
        expected.sort_unstable();
        let bytes = subtree.to_bytes().unwrap();
        let (doc, binary) = chunks(&bytes);
        assert_eq!(
            available(&doc, binary, &doc["contentAvailability"][0], len),
            expected
        );
    }
}

#[test]
fn missing_roots_tile_parents_and_child_subtree_parents_are_rejected() {
    for scheme in [SubdivisionScheme::Quadtree, SubdivisionScheme::Octree] {
        let mut subtree = Subtree::new(scheme, 3, 0).unwrap();
        assert!(subtree
            .to_bytes()
            .unwrap_err()
            .to_string()
            .contains("root tile"));
        subtree.set_tile(at(0, 0, 0, 0), &[]).unwrap();
        subtree.set_tile(at(2, 3, 0, 0), &[]).unwrap();
        assert!(subtree
            .to_bytes()
            .unwrap_err()
            .to_string()
            .contains("unavailable parent"));
        // Adding the missing ancestor repairs the tree.
        subtree.set_tile(at(1, 1, 0, 0), &[]).unwrap();
        subtree.to_bytes().unwrap();
        subtree.set_child_subtree(at(3, 0, 0, 0)).unwrap();
        assert!(subtree
            .to_bytes()
            .unwrap_err()
            .to_string()
            .contains("unavailable parent tile"));
        subtree.set_tile(at(1, 0, 0, 0), &[]).unwrap();
        subtree.set_tile(at(2, 0, 0, 0), &[]).unwrap();
        subtree.to_bytes().unwrap();
    }
}

#[test]
fn invalid_levels_coordinates_and_content_counts_are_refused_without_mutation() {
    for scheme in [SubdivisionScheme::Quadtree, SubdivisionScheme::Octree] {
        assert!(Subtree::new(scheme, 0, 1).is_err());
        assert!(Subtree::new(scheme, u32::MAX, 1).is_err());
        assert!(Subtree::new(scheme, 32, 1).is_err());
        assert!(Subtree::new(scheme, 1, usize::MAX).is_err());
        let mut subtree = Subtree::new(scheme, 3, 1).unwrap();
        subtree.set_tile(at(0, 0, 0, 0), &[true]).unwrap();
        let before = subtree.to_bytes().unwrap();
        for coordinates in [
            at(3, 0, 0, 0),
            at(u32::MAX, 0, 0, 0),
            at(1, 2, 0, 0),
            at(1, 0, 2, 0),
            at(1, 0, 0, 2),
        ] {
            assert!(subtree.set_tile(coordinates, &[true]).is_err());
        }
        assert!(subtree.set_tile(at(0, 0, 0, 0), &[]).is_err());
        assert!(subtree.set_tile(at(0, 0, 0, 0), &[true, false]).is_err());
        assert!(subtree.set_child_subtree(at(2, 0, 0, 0)).is_err());
        assert!(subtree.set_child_subtree(at(3, 8, 0, 0)).is_err());
        if scheme == SubdivisionScheme::Quadtree {
            assert!(subtree.set_tile(at(1, 0, 0, 1), &[true]).is_err());
            assert!(subtree.set_child_subtree(at(3, 0, 0, 1)).is_err());
        }
        assert_eq!(before, subtree.to_bytes().unwrap());
    }
}

#[test]
fn serialization_is_independent_of_insertion_order_and_content_can_be_replaced() {
    let tiles = [at(0, 0, 0, 0), at(1, 1, 0, 1), at(2, 3, 0, 2)];
    let mut forward = Subtree::new(SubdivisionScheme::Octree, 3, 1).unwrap();
    let mut reverse = Subtree::new(SubdivisionScheme::Octree, 3, 1).unwrap();
    for coordinates in tiles {
        forward.set_tile(coordinates, &[true]).unwrap();
    }
    for coordinates in tiles.into_iter().rev() {
        reverse.set_tile(coordinates, &[false]).unwrap();
        reverse.set_tile(coordinates, &[true]).unwrap();
    }
    forward.set_child_subtree(at(3, 6, 0, 5)).unwrap();
    reverse.set_child_subtree(at(3, 6, 0, 5)).unwrap();
    assert_eq!(forward.to_bytes().unwrap(), reverse.to_bytes().unwrap());
}

#[test]
fn linked_subtrees_package_with_spatially_placed_multiple_contents() {
    // Set this only to retain the invented cases for an external validator or
    // Cesium probe. Normal cargo tests keep all output in a temporary directory.
    let temporary = tempfile::tempdir().unwrap();
    let output = std::env::var_os("RUSTY_TILES_IMPLICIT_FIXTURES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| temporary.path().to_owned());
    for (scheme, content_count) in [
        (SubdivisionScheme::Quadtree, 1),
        (SubdivisionScheme::Quadtree, 2),
        (SubdivisionScheme::Octree, 1),
        (SubdivisionScheme::Octree, 2),
    ] {
        let name = if content_count == 1 {
            format!("single-{}", scheme.as_str().to_ascii_lowercase())
        } else {
            scheme.as_str().to_ascii_lowercase()
        };
        let case = output.join(&name);
        std::fs::create_dir_all(case.join("subtrees")).unwrap();
        std::fs::create_dir_all(case.join("t")).unwrap();
        let octree = scheme == SubdivisionScheme::Octree;
        let root = at(0, 0, 0, 0);
        let child = at(1, 1, 0, u32::from(octree));
        let next = at(2, 3, 0, if octree { 2 } else { 0 });
        let leaf = at(3, 7, 1, if octree { 4 } else { 0 });
        let key = |c: Coordinates| {
            if octree {
                format!("{}-{}-{}-{}", c.level, c.x, c.y, c.z)
            } else {
                format!("{}-{}-{}", c.level, c.x, c.y)
            }
        };
        let mut first = Subtree::new(scheme, 2, content_count).unwrap();
        first
            .set_tile(root, &[true, false][..content_count])
            .unwrap();
        first
            .set_tile(child, &[false, true][..content_count])
            .unwrap();
        first.set_child_subtree(next).unwrap();
        let first_bytes = first.to_bytes().unwrap();
        let mut second = Subtree::new(scheme, 2, content_count).unwrap();
        second
            .set_tile(root, &[true, true][..content_count])
            .unwrap();
        second
            .set_tile(at(1, 1, 1, 0), &[true, false][..content_count])
            .unwrap();
        let second_bytes = second.to_bytes().unwrap();
        std::fs::write(
            case.join(format!("subtrees/{}.subtree", key(root))),
            &first_bytes,
        )
        .unwrap();
        std::fs::write(
            case.join(format!("subtrees/{}.subtree", key(next))),
            &second_bytes,
        )
        .unwrap();
        for (coordinates, slots) in [
            (root, [true, false]),
            (child, [false, true]),
            (next, [true, true]),
            (leaf, [true, false]),
        ] {
            let cell_size = 32. / f32::powi(2., coordinates.level as i32);
            let x = -16. + (coordinates.x as f32 + 0.5) * cell_size;
            let y = -16. + (coordinates.y as f32 + 0.5) * cell_size;
            let z = if octree {
                -16. + (coordinates.z as f32 + 0.5) * cell_size
            } else {
                0.
            };
            // glTF is Y-up; the implicit box is Z-up. The small triangle is
            // inside the computed cell at every level, including after a link.
            let glb = rusty_tiles::glb_write::write_glb(&[rusty_tiles::glb_write::TilePrimitive {
                positions: vec![
                    [x - 0.2, z, -y - 0.2],
                    [x + 0.2, z, -y - 0.2],
                    [x, z, -y + 0.2],
                ],
                normals: vec![[0., 1., 0.]; 3],
                indices: vec![0, 1, 2],
                ..Default::default()
            }])
            .unwrap();
            for (slot, exists) in slots.into_iter().take(content_count).enumerate() {
                if exists {
                    std::fs::write(
                        case.join(format!("t/{}-{slot}.glb", key(coordinates))),
                        &glb,
                    )
                    .unwrap();
                }
            }
        }
        let template = if octree {
            "{level}-{x}-{y}-{z}"
        } else {
            "{level}-{x}-{y}"
        };
        let mut doc = serde_json::json!({
            "asset": {"version": "1.1"}, "geometricError": 32,
            "root": {
                "boundingVolume": {"box": [0,0,0,16,0,0,0,16,0,0,0,16]},
                "geometricError": 16, "refine": "REPLACE",
                "implicitTiling": {"subdivisionScheme": scheme.as_str(), "subtreeLevels": 2, "availableLevels": 4,
                    "subtrees": {"uri": format!("subtrees/{template}.subtree")}}
            }
        });
        if content_count == 1 {
            doc["root"]["content"] = serde_json::json!({"uri": format!("t/{template}-0.glb")});
        } else {
            doc["root"]["contents"] = serde_json::json!([
                {"uri": format!("t/{template}-0.glb")}, {"uri": format!("t/{template}-1.glb")}
            ]);
        }
        std::fs::write(
            case.join("tileset.json"),
            serde_json::to_vec_pretty(&doc).unwrap(),
        )
        .unwrap();
        // Check both sides of the link with the independent decoder.
        let (doc, binary) = chunks(&first_bytes);
        let branches = if octree { 8 } else { 4 };
        assert_eq!(
            available(&doc, binary, &doc["tileAvailability"], branches + 1),
            [0, if octree { 6 } else { 2 }]
        );
        assert_eq!(
            available(
                &doc,
                binary,
                &doc["childSubtreeAvailability"],
                branches * branches
            ),
            [if octree { 41 } else { 5 }]
        );
        let (doc, binary) = chunks(&second_bytes);
        assert_eq!(
            available(&doc, binary, &doc["tileAvailability"], branches + 1),
            [0, 4]
        );
        assert!(available(
            &doc,
            binary,
            &doc["childSubtreeAvailability"],
            branches * branches
        )
        .is_empty());
        let archive = output.join(format!("{name}.3tz"));
        rusty_tiles::convert_to_3tz(
            &case,
            &archive,
            &rusty_tiles::pack::PackOptions { force: true },
        )
        .unwrap();
        rusty_tiles::validate_3tz(&archive).unwrap();
    }
}
