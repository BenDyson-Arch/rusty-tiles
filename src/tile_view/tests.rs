//! Decoded tile views of real tiler output. The textured sheet is tiled through
//! the public operation; references are the published bytes themselves, the
//! vendored transcoder used by codec verification and the independent
//! dev-only `basisu` transcoder.
use super::*;
use crate::{mesh_to_tileset, RunControl, TilerProfile, TilerRequest};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path};

/// `side`² quads on a gently varying surface with one smooth 128² image.
fn textured_sheet(dir: &Path, side: usize) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let (mut positions, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    for y in 0..=side {
        for x in 0..=side {
            let z = ((x * 7 + y * 3) % 5) as f32 * 0.01;
            positions.extend([x as f32, z, y as f32]);
            uvs.extend([x as f32 / side as f32, y as f32 / side as f32]);
        }
    }
    let at = |x: usize, y: usize| (y * (side + 1) + x) as u32;
    for y in 0..side {
        for x in 0..side {
            indices.extend([at(x, y), at(x, y + 1), at(x + 1, y)]);
            indices.extend([at(x + 1, y), at(x, y + 1), at(x + 1, y + 1)]);
        }
    }
    let mut bytes = Vec::new();
    for v in positions.iter().chain(&uvs) {
        bytes.extend(v.to_le_bytes());
    }
    let index_offset = bytes.len();
    for i in &indices {
        bytes.extend(i.to_le_bytes());
    }
    fs::write(dir.join("sheet.bin"), &bytes).unwrap();
    image::RgbaImage::from_fn(128, 128, |x, y| {
        image::Rgba([(x * 2) as u8, (y * 2) as u8, ((x + y) / 2) as u8, 255])
    })
    .save(dir.join("a.png"))
    .unwrap();
    let vertices = (side + 1) * (side + 1);
    let doc = json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":2,"material":0}]}],
        "buffers":[{"uri":"sheet.bin","byteLength":bytes.len()}],
        "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":vertices * 12},
            {"buffer":0,"byteOffset":vertices * 12,"byteLength":vertices * 8},
            {"buffer":0,"byteOffset":index_offset,"byteLength":indices.len() * 4}],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":vertices,"type":"VEC3",
             "min":[0,0,0],"max":[side as f32, 0.04, side as f32]},
            {"bufferView":1,"componentType":5126,"count":vertices,"type":"VEC2"},
            {"bufferView":2,"componentType":5125,"count":indices.len(),"type":"SCALAR"}],
        "images":[{"uri":"a.png"}],
        "textures":[{"source":0,"sampler":0}],
        "samplers":[{"minFilter":9987,"magFilter":9729}],
        "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}]
    });
    let input = dir.join("sheet.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    input
}

/// A balanced (KTX2) tileset with several leaves under coarse tiles.
fn tiled(work: &Path) -> PathBuf {
    let input = textured_sheet(&work.join("src"), 24);
    let output = work.join("sheet.3tz");
    mesh_to_tileset(
        TilerRequest::new(&input, &output)
            .with_profile(TilerProfile::Balanced)
            .with_test_leaf_triangles(256),
        &RunControl::default(),
    )
    .unwrap();
    output
}

fn decode(bytes: &[u8]) -> Result<Plain, JobError> {
    Plain::decode(bytes, &mut Budget::default())
}

fn parts(bytes: &[u8]) -> (Value, Vec<u8>) {
    let (doc, bin) = glb_parts(bytes).unwrap();
    (doc, bin.to_vec())
}

fn view_bytes<'a>(doc: &Value, bin: &'a [u8], view: usize) -> &'a [u8] {
    let view = &doc["bufferViews"][view];
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
    &bin[start..start + view["byteLength"].as_u64().unwrap() as usize]
}

fn members(archive: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut member = zip.by_index(i).unwrap();
            let mut bytes = Vec::new();
            member.read_to_end(&mut bytes).unwrap();
            (member.name().to_owned(), bytes)
        })
        .collect()
}

/// Every KTX2 page of `source` is a PNG in `doc` whose pixels equal level 0 from the
/// vendored transcoder and from the independent dev-only `basisu` transcoder.
fn check_pages(uri: &str, source: &Value, source_bin: &[u8], doc: &Value, bin: &[u8]) -> usize {
    let mut pages = 0;
    for (i, image) in source["images"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        assert_eq!(
            image["mimeType"], "image/ktx2",
            "{uri}: published pages are KTX2"
        );
        let view = image["bufferView"].as_u64().unwrap() as usize;
        let ktx2 = view_bytes(source, source_bin, view);
        assert_eq!(doc["images"][i]["mimeType"], "image/png");
        let png = view_bytes(
            doc,
            bin,
            doc["images"][i]["bufferView"].as_u64().unwrap() as usize,
        );
        let decoded = image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        let (width, height, vendored) = crate::basisu_ktx2::transcode_base_rgba(ktx2).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (width, height));
        assert_eq!(
            decoded.as_raw(),
            &vendored,
            "{uri} page {i}: vendored transcode"
        );
        let independent = basisu::Transcoder::new(ktx2)
            .unwrap()
            .transcode(0, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
            .unwrap();
        assert_eq!(
            decoded.as_raw(),
            &independent,
            "{uri} page {i}: independent transcode"
        );
        pages += 1;
    }
    pages
}

/// The same page check over every tile of an external archive (`TILE_VIEW_ARCHIVE`).
#[test]
#[ignore = "external archive decoder; TILE_VIEW_ARCHIVE required"]
fn external_archive_pages_equal_both_transcodes() {
    let archive = PathBuf::from(std::env::var_os("TILE_VIEW_ARCHIVE").expect("TILE_VIEW_ARCHIVE"));
    let published = members(&archive);
    let mut set = TileSet::open(&archive).unwrap();
    let uris: Vec<_> = set.tiles().iter().map(|t| t.uri.clone()).collect();
    let mut pages = 0;
    for uri in &uris {
        let (source, source_bin) = parts(&published[uri]);
        let (doc, bin) = parts(&set.plain_glb(uri).unwrap());
        pages += check_pages(uri, &source, &source_bin, &doc, &bin);
    }
    println!("{}: {} tiles, {pages} pages", archive.display(), uris.len());
    assert!(pages > 0);
}

#[test]
fn listing_names_every_content_with_its_role_depth_and_placement() {
    let work = tempfile::tempdir().unwrap();
    let archive = tiled(work.path());
    let set = TileSet::open(&archive).unwrap();
    let tiles = set.tiles();
    let published: Vec<_> = members(&archive)
        .into_keys()
        .filter(|n| n.ends_with(".glb"))
        .collect();
    let mut listed: Vec<_> = tiles.iter().map(|t| t.uri.clone()).collect();
    listed.sort();
    assert_eq!(listed, published, "every payload is listed once");
    assert_eq!(tiles[0].depth, 0);
    assert_eq!(tiles[0].role, TileRole::Coarse);
    let leaves = tiles.iter().filter(|t| t.role == TileRole::Leaf).count();
    assert!(leaves >= 4, "{leaves} leaves");
    assert!(
        tiles
            .iter()
            .any(|t| t.role == TileRole::Coarse && t.depth > 0)
            || leaves > 0
    );
    for tile in tiles {
        assert_eq!(
            tile.transform,
            set.root_transform(),
            "only the root is placed"
        );
        assert!(tile.geometric_error.is_finite());
        if tile.role == TileRole::Leaf {
            assert!(tile.depth > 0);
        }
    }
}

#[test]
fn plain_glb_drops_only_the_codecs_and_pages_equal_the_basis_transcodes() {
    let work = tempfile::tempdir().unwrap();
    let archive = tiled(work.path());
    let published = members(&archive);
    let mut set = TileSet::open(&archive).unwrap();
    let uris: Vec<_> = set.tiles().iter().map(|t| t.uri.clone()).collect();
    let mut pages = 0;
    for uri in &uris {
        let (source, source_bin) = parts(&published[uri]);
        let plain = set.plain_glb(uri).unwrap();
        let (doc, bin) = parts(&plain);
        let text = doc.to_string();
        assert!(!text.contains(MESHOPT) && !text.contains(BASISU), "{uri}");
        assert!(!text.contains("image/ktx2"), "{uri}");
        assert_eq!(doc["buffers"].as_array().unwrap().len(), 1);
        let length = doc["buffers"][0]["byteLength"].as_u64().unwrap() as usize;
        assert_eq!(
            length.next_multiple_of(4),
            bin.len(),
            "BIN chunk pads the buffer"
        );
        // Everything but views, buffers, textures, images and the codec names is unchanged.
        for key in [
            "accessors",
            "meshes",
            "nodes",
            "scenes",
            "materials",
            "samplers",
            "extensions",
        ] {
            assert_eq!(doc.get(key), source.get(key), "{uri} {key}");
        }
        let used = |d: &Value| -> Vec<Value> {
            d["extensionsUsed"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|n| *n != MESHOPT && *n != BASISU)
                .cloned()
                .collect()
        };
        assert_eq!(used(&doc), used(&source));
        assert!(used(&doc).contains(&json!("KHR_mesh_quantization")));
        // Decoded views equal the linked decoder's output, byte for byte.
        for (i, view) in source["bufferViews"].as_array().unwrap().iter().enumerate() {
            let expected =
                decode_view(view, &source_bin, source["buffers"].as_array().unwrap()).unwrap();
            let is_image = doc["images"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|im| im["bufferView"] == i);
            if !is_image {
                assert_eq!(view_bytes(&doc, &bin, i), &expected[..], "{uri} view {i}");
            }
        }
        pages += check_pages(uri, &source, &source_bin, &doc, &bin);
        for texture in doc["textures"].as_array().into_iter().flatten() {
            assert!(texture.get("extensions").is_none());
            assert!(texture["source"].as_u64().is_some());
            assert!(texture["sampler"].as_u64().is_some(), "samplers kept");
        }
    }
    assert!(pages >= 2, "{pages} pages");
}

#[test]
fn arrays_match_the_plain_document_and_carry_leaf_identity() {
    let work = tempfile::tempdir().unwrap();
    let archive = tiled(work.path());
    let mut set = TileSet::open(&archive).unwrap();
    let tiles = set.tiles().to_vec();
    for tile in &tiles {
        let decoded = set.arrays(&tile.uri).unwrap();
        assert_eq!(&decoded.tile, tile);
        let plain = set.plain_glb(&tile.uri).unwrap();
        let (doc, bin) = parts(&plain);
        assert_eq!(
            decoded.pages.len(),
            doc["images"].as_array().map_or(0, Vec::len)
        );
        for (page, image) in decoded.pages.iter().zip(doc["images"].as_array().unwrap()) {
            let png = view_bytes(&doc, &bin, image["bufferView"].as_u64().unwrap() as usize);
            let png = image::load_from_memory(png).unwrap().into_rgba8();
            assert_eq!((page.width, page.height), png.dimensions());
            assert_eq!(page.rgba, png.into_raw());
        }
        let labels: Vec<_> = decoded
            .primitives
            .iter()
            .flat_map(|p| p.feature_ids.iter().map(|f| f.label.clone().unwrap()))
            .collect();
        let leaf = tile.role == TileRole::Leaf;
        assert_eq!(
            labels.contains(&"source_triangle".into()),
            leaf,
            "{}",
            tile.uri
        );
        assert!(labels.contains(&"source_primitive".into()));
        for p in &decoded.primitives {
            assert!(!p.indices.is_empty());
            assert!(p.positions.iter().flatten().all(|v| v.is_finite()));
            for set in &p.feature_ids {
                let table = &decoded.property_tables[set.property_table.unwrap()];
                assert_eq!(table.class, *set.label.as_ref().unwrap());
                assert_eq!(table.count, set.feature_count);
                for t in &p.indices {
                    let id = set.values[t[0] as usize];
                    assert!(u64::from(id) < set.feature_count);
                    assert!(
                        t.iter().all(|&c| set.values[c as usize] == id),
                        "per-face ID"
                    );
                }
            }
            if let Some(material) = p.material {
                assert_eq!(p.uvs.len(), 1);
                let texture = decoded.materials[material]["pbrMetallicRoughness"]
                    ["baseColorTexture"]["index"]
                    .as_u64()
                    .unwrap() as usize;
                let page = decoded.textures[texture]["source"].as_u64().unwrap() as usize;
                assert!(page < decoded.pages.len());
            }
        }
        // Source sheet is y-up with height in glTF y; the tile frame puts it in z.
        let heights: Vec<f64> = decoded
            .primitives
            .iter()
            .flat_map(|p| p.positions.iter().map(|q| q[2]))
            .collect();
        assert!(
            heights.iter().all(|h| (-0.01..=0.05).contains(h)),
            "{heights:?}"
        );
    }
}

#[test]
fn an_unpacked_directory_reads_the_same_as_its_archive() {
    let work = tempfile::tempdir().unwrap();
    let archive = tiled(work.path());
    let dir = work.path().join("unpacked");
    for (name, bytes) in members(&archive) {
        let path = dir.join(&name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let mut a = TileSet::open(&archive).unwrap();
    let mut d = TileSet::open(&dir).unwrap();
    assert_eq!(a.tiles(), d.tiles());
    let uri = a.tiles().last().unwrap().uri.clone();
    assert_eq!(a.plain_glb(&uri).unwrap(), d.plain_glb(&uri).unwrap());
}

#[test]
fn refusals_are_typed() {
    let work = tempfile::tempdir().unwrap();
    let archive = tiled(work.path());
    let mut set = TileSet::open(&archive).unwrap();
    let kind = |r: Result<Vec<u8>, JobError>| r.unwrap_err().kind();
    assert_eq!(
        kind(set.plain_glb("tileset.json")),
        JobErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(set.plain_glb("content/none.glb")),
        JobErrorKind::InvalidRequest
    );
    let missing = TileSet::open(&work.path().join("missing.3tz"))
        .err()
        .unwrap();
    assert_eq!(missing.kind(), JobErrorKind::Io);
    let not_archive = work.path().join("src/sheet.bin");
    let refused = TileSet::open(&not_archive).err().unwrap();
    assert_eq!(refused.kind(), JobErrorKind::InvalidInput);
    for uri in ["../x.glb", "/x.glb", "a/./b.glb", "a%20b.glb", "c:x.glb"] {
        assert_eq!(
            segments(uri).unwrap_err().kind(),
            JobErrorKind::Unsupported,
            "{uri}"
        );
    }

    // One published coarse tile with targeted corruption.
    let uri = set.tiles()[0].uri.clone();
    let bytes = members(&archive)[&uri].clone();
    let (doc, bin) = parts(&bytes);
    let glb = |doc: &Value, bin: &[u8]| crate::glb::encode_glb(doc, bin).unwrap();
    assert!(decode(&glb(&doc, &bin))
        .unwrap()
        .into_glb(&mut Budget::default())
        .is_ok());
    let compressed = (0..doc["bufferViews"].as_array().unwrap().len())
        .find(|&i| doc["bufferViews"][i]["extensions"][MESHOPT]["mode"] == "ATTRIBUTES")
        .unwrap();
    let ext = &doc["bufferViews"][compressed]["extensions"][MESHOPT];
    let mut damaged = bin.clone();
    damaged[ext["byteOffset"].as_u64().unwrap_or(0) as usize] ^= 0xff;
    let failure = decode(&glb(&doc, &damaged)).err().unwrap();
    assert_eq!(failure.kind(), JobErrorKind::InvalidInput, "{failure}");
    let mut wrong = doc.clone();
    wrong["bufferViews"][compressed]["extensions"][MESHOPT]["filter"] = json!("EXPONENTIAL");
    let failure = decode(&glb(&wrong, &bin)).err().unwrap();
    assert_eq!(failure.kind(), JobErrorKind::Unsupported, "{failure}");
    let mut wrong = doc.clone();
    wrong["bufferViews"][compressed]["extensions"][MESHOPT]["count"] = json!(1u64 << 40);
    let failure = decode(&glb(&wrong, &bin)).err().unwrap();
    assert_eq!(failure.kind(), JobErrorKind::InvalidInput, "{failure}");
    let mut wrong = doc.clone();
    wrong["bufferViews"][compressed]["byteLength"] = json!(DECODED_LIMIT + 1);
    let failure = decode(&glb(&wrong, &bin)).err().unwrap();
    assert_eq!(failure.kind(), JobErrorKind::ResourceLimit, "{failure}");
    let mut wrong = doc.clone();
    wrong["buffers"][0]["uri"] = json!("data.bin");
    let failure = decode(&glb(&wrong, &bin)).err().unwrap();
    assert_eq!(failure.kind(), JobErrorKind::Unsupported, "{failure}");
    let mut b3dm = bytes.clone();
    b3dm[..4].copy_from_slice(b"b3dm");
    assert_eq!(
        decode(&b3dm).err().unwrap().kind(),
        JobErrorKind::Unsupported
    );
    let image = doc["images"][0]["bufferView"].as_u64().unwrap() as usize;
    let start = doc["bufferViews"][image]["byteOffset"].as_u64().unwrap() as usize;
    let mut damaged = bin.clone();
    damaged[start + 1] ^= 0xff;
    let failure = decode(&glb(&doc, &damaged))
        .unwrap()
        .into_glb(&mut Budget::default())
        .err()
        .unwrap();
    assert_eq!(failure.kind(), JobErrorKind::InvalidInput, "{failure}");
}

// ---- Hostile content: every path is refused before an unbounded allocation or walk ----

/// One float triangle at `bin` offset 0, as the whole document's only mesh.
fn triangle_doc() -> (Value, Vec<u8>) {
    let bin: Vec<u8> = [0f32, 0., 0., 1., 0., 0., 0., 1., 0.]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let doc = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteLength": bin.len()}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}]
    });
    (doc, bin)
}

fn arrays_of(doc: &Value, bin: &[u8]) -> Result<DecodedTile, JobError> {
    let bytes = crate::glb::encode_glb(doc, bin).unwrap();
    let entry = TileEntry {
        uri: "t.glb".into(),
        depth: 0,
        role: TileRole::Leaf,
        geometric_error: 0.,
        transform: IDENTITY,
    };
    let mut budget = Budget::default();
    Plain::decode(&bytes, &mut budget)?.into_arrays(entry, IDENTITY, &mut budget)
}

fn tile_limit(error: &JobError) -> TileLimit {
    assert_eq!(error.kind(), JobErrorKind::ResourceLimit, "{error}");
    *std::error::Error::source(error)
        .and_then(|cause| cause.downcast_ref::<TileLimit>())
        .unwrap_or_else(|| panic!("no TileLimit cause: {error}"))
}

#[test]
fn huge_accessor_counts_are_refused_before_allocation() {
    let (doc, bin) = triangle_doc();
    assert_eq!(
        arrays_of(&doc, &bin).unwrap().primitives[0].indices,
        [[0, 1, 2]]
    );
    for count in [1u64 << 40, u64::MAX / 2, (DECODED_LIMIT / 24) + 1] {
        for viewless in [true, false] {
            let mut doc = doc.clone();
            doc["accessors"][0]["count"] = json!(count);
            if viewless {
                doc["accessors"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("bufferView");
            }
            let refused = arrays_of(&doc, &bin).err().unwrap();
            assert_eq!(
                tile_limit(&refused).limit,
                "decodedBytes",
                "{count} {viewless}"
            );
        }
    }
    // A viewless accessor within the budget is zeros, not a refusal.
    let mut zeros = doc.clone();
    zeros["accessors"][0]
        .as_object_mut()
        .unwrap()
        .remove("bufferView");
    let decoded = arrays_of(&zeros, &bin).unwrap();
    assert!(decoded.primitives[0]
        .positions
        .iter()
        .flatten()
        .all(|&v| v == 0.));
}

#[test]
fn shared_children_and_mass_instancing_are_bounded() {
    let (mut doc, bin) = triangle_doc();
    // 2 x 40 nodes; each pair shares the next pair as children: 2^40 paths.
    let mut nodes = Vec::new();
    for level in 0..40 {
        let children = if level < 39 {
            json!([2 * level + 2, 2 * level + 3])
        } else {
            json!([])
        };
        nodes.push(json!({"children": children, "mesh": 0}));
        nodes.push(json!({"children": children}));
    }
    doc["nodes"] = Value::Array(nodes);
    doc["scenes"] = json!([{"nodes": [0, 1]}]);
    let start = std::time::Instant::now();
    let refused = arrays_of(&doc, &bin).err().unwrap();
    assert_eq!(tile_limit(&refused).limit, "sceneWork");
    assert!(start.elapsed().as_secs() < 10, "{:?}", start.elapsed());
    let (mut doc, bin) = triangle_doc();
    let many = SCENE_WORK_LIMIT as usize;
    doc["nodes"] = Value::Array(vec![json!({"mesh": 0}); many]);
    doc["scenes"] = json!([{"nodes": (0..many).collect::<Vec<_>>()}]);
    assert_eq!(
        tile_limit(&arrays_of(&doc, &bin).err().unwrap()).limit,
        "sceneWork"
    );
    doc["nodes"] = Value::Array(vec![json!({"mesh": 0}); 100]);
    doc["scenes"] = json!([{"nodes": (0..100).collect::<Vec<_>>()}]);
    assert_eq!(arrays_of(&doc, &bin).unwrap().primitives.len(), 100);
}

/// A PNG signature and IHDR claiming `width` x `height` RGBA8, then a bogus IDAT.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let chunk = |kind: &[u8], data: &[u8]| {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend(kind);
        out.extend(data);
        let crc = crc32fast::hash(&[kind, data].concat());
        out.extend(crc.to_be_bytes());
        out
    };
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    [
        b"\x89PNG\r\n\x1a\n".to_vec(),
        chunk(b"IHDR", &ihdr),
        chunk(b"IDAT", b"not deflate"),
        chunk(b"IEND", b""),
    ]
    .concat()
}

#[test]
fn page_sizes_are_refused_or_charged_from_the_header() {
    let (doc, bin) = triangle_doc();
    let with_image = |png: Vec<u8>, mime: &str| {
        let mut doc = doc.clone();
        let mut bin = bin.clone();
        crate::glb::pad_to(&mut bin, 4);
        let offset = bin.len();
        bin.extend(&png);
        doc["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer": 0, "byteOffset": offset, "byteLength": png.len()}));
        doc["buffers"][0]["byteLength"] = json!(bin.len());
        doc["images"] = json!([{"bufferView": 1, "mimeType": mime}]);
        arrays_of(&doc, &bin)
    };
    let refused = with_image(png_header(16_384, 8_193), "image/png")
        .err()
        .unwrap();
    let limit = tile_limit(&refused);
    assert_eq!(
        (limit.limit, limit.observed),
        ("pageTexels", 16_384 * 8_193)
    );
    // 8192² passes the size limits and is charged; its bogus pixel data then fails.
    let refused = with_image(png_header(8192, 8192), "image/png")
        .err()
        .unwrap();
    assert_eq!(
        refused.kind(),
        JobErrorKind::InvalidInput,
        "bogus pixel data: {refused}"
    );
    let small = image::RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8, y as u8, 7, 255]));
    let mut png = Vec::new();
    small
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let page = &with_image(png, "image/png").unwrap().pages[0];
    assert_eq!((page.width, page.height, page.rgba.len()), (3, 2, 24));
    let refused = with_image(b"\x89PNG".to_vec(), "image/png").err().unwrap();
    assert_eq!(refused.kind(), JobErrorKind::InvalidInput);
}

#[test]
fn mirrored_nodes_keep_counter_clockwise_front_faces() {
    let (mut doc, bin) = triangle_doc();
    doc["nodes"] = json!([{"mesh": 0}, {"mesh": 0, "scale": [-1, 1, 1]},
                          {"mesh": 0, "scale": [-1, -1, 1]}]);
    doc["scenes"] = json!([{"nodes": [0, 1, 2]}]);
    let decoded = arrays_of(&doc, &bin).unwrap();
    let windings: Vec<_> = decoded.primitives.iter().map(|p| p.indices[0]).collect();
    assert_eq!(windings, [[0, 1, 2], [0, 2, 1], [0, 1, 2]]);
    // Every face then faces the same way in the tile frame.
    let z: Vec<f64> = decoded
        .primitives
        .iter()
        .map(|p| {
            let [a, b, c] = p.indices[0].map(|i| p.positions[i as usize]);
            let (u, v) = (
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            // Face normal's tile-frame y component (glTF z mirrored by the swizzle).
            u[2] * v[0] - u[0] * v[2]
        })
        .collect();
    assert!(
        z.iter().all(|&y| y == z[0]),
        "same facing for every winding: {z:?}"
    );
}

#[test]
fn property_value_transforms_are_returned_and_other_types_skipped() {
    let (mut doc, mut bin) = triangle_doc();
    let offset = bin.len();
    bin.extend([1u8, 0, 2, 0]);
    doc["bufferViews"].as_array_mut().unwrap().push(json!({
        "buffer": 0, "byteOffset": offset, "byteLength": 4}));
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    doc["extensions"] = json!({"EXT_structural_metadata": {
        "schema": {"classes": {"c": {"properties": {
            "a": {"type": "SCALAR", "componentType": "UINT16", "normalized": true,
                  "offset": 1.0, "scale": 2.0},
            "v": {"type": "VEC3", "componentType": "FLOAT32"},
            "s": {"type": "STRING"},
            "n": {"type": "SCALAR", "componentType": "UINT8", "array": true}}}}},
        "propertyTables": [{"class": "c", "count": 2, "properties": {
            "a": {"values": 1, "scale": 3.0}, "v": {"values": 1},
            "s": {"values": 1, "stringOffsets": 1}, "n": {"values": 1, "arrayOffsets": 1},
            "x": {"values": 1}}}]}});
    let tables = arrays_of(&doc, &bin).unwrap().property_tables;
    let table = &tables[0];
    assert_eq!(table.columns.len(), 1);
    let a = &table.columns[0];
    assert_eq!(
        (a.name.as_str(), a.normalized, a.offset, a.scale),
        ("a", true, Some(1.0), Some(3.0))
    );
    assert_eq!(a.bytes, [1, 0, 2, 0]);
    let skipped: BTreeMap<_, _> = table.skipped.iter().cloned().collect();
    assert_eq!(skipped.keys().collect::<Vec<_>>(), ["n", "s", "v", "x"]);
    assert_eq!(skipped["x"], "not declared by the class");
    assert!(skipped["v"].contains("VEC3"), "{}", skipped["v"]);
    let mut bad = doc.clone();
    bad["extensions"]["EXT_structural_metadata"]["propertyTables"][0]["class"] = json!("missing");
    assert_eq!(
        arrays_of(&bad, &bin).err().unwrap().kind(),
        JobErrorKind::InvalidInput
    );
}

fn basis_pages() -> Vec<(&'static str, Vec<u8>)> {
    use crate::basisu_ktx2::{encode_ktx2, Mode, Params, Transfer};
    let rgba: Vec<u8> = (0..32u32 * 24)
        .flat_map(|i| [(i * 7) as u8, (i * 3) as u8, (i / 5) as u8, 255])
        .collect();
    let params = Params {
        effort: 0,
        transfer: Transfer::Srgb,
        mipmaps: false,
        etc1_hints: crate::basisu_ktx2::Etc1Hints::Full,
    };
    [
        ("etc1s", Mode::Etc1s { quality: 50 }),
        ("uastc", Mode::Uastc { zstd: false }),
        ("uastc-zstd", Mode::Uastc { zstd: true }),
    ]
    .into_iter()
    .map(|(name, mode)| (name, encode_ktx2(&rgba, 32, 24, mode, params).unwrap()))
    .collect()
}

#[test]
fn malformed_ktx2_pages_are_refused_before_the_transcoder() {
    use crate::basisu_ktx2::transcode_base_rgba;
    let put = |bytes: &mut [u8], at: usize, value: u32| {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes())
    };
    for (name, page) in basis_pages() {
        let (w, h, rgba) = transcode_base_rgba(&page).unwrap();
        assert_eq!((w, h, rgba.len()), (32, 24, 32 * 24 * 4), "{name}");
        let kind = |mutate: &dyn Fn(&mut Vec<u8>)| {
            let mut bytes = page.clone();
            mutate(&mut bytes);
            transcode_base_rgba(&bytes).err().map(|e| e.kind())
        };
        use JobErrorKind::*;
        assert_eq!(
            kind(&|b| b.truncate(60)),
            Some(InvalidInput),
            "{name} truncated"
        );
        assert_eq!(
            kind(&|b| b[1] = b'X'),
            Some(InvalidInput),
            "{name} identifier"
        );
        assert_eq!(
            kind(&|b| put(b, 12, 37)),
            Some(Unsupported),
            "{name} vkFormat"
        );
        assert_eq!(
            kind(&|b| put(b, 16, 4)),
            Some(InvalidInput),
            "{name} typeSize"
        );
        assert_eq!(
            kind(&|b| put(b, 20, 20_000)),
            Some(ResourceLimit),
            "{name} width"
        );
        assert_eq!(kind(&|b| put(b, 36, 6)), Some(Unsupported), "{name} faces");
        assert_eq!(kind(&|b| put(b, 44, 3)), Some(Unsupported), "{name} zlib");
        assert_eq!(
            kind(&|b| put(b, 40, 40)),
            Some(InvalidInput),
            "{name} level count"
        );
        // Level 0 entry: byteOffset at 80, byteLength at 88, uncompressedByteLength at 96.
        assert_eq!(
            kind(&|b| put(b, 80, u32::MAX)),
            Some(InvalidInput),
            "{name} offset"
        );
        assert_eq!(
            kind(&|b| put(b, 96, 5)),
            Some(InvalidInput),
            "{name} level length"
        );
        if name == "etc1s" {
            // Supercompression global data is required for BasisLZ.
            assert_eq!(
                kind(&|b| put(b, 72, 0)),
                Some(InvalidInput),
                "{name} global data"
            );
        }
    }
}

/// A deterministic mutation sweep over valid pages. Every outcome must be a decode or
/// a typed refusal; a crash or abort in the transcoder fails the test binary.
#[test]
fn mutated_ktx2_pages_never_crash_the_transcoder() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut outcomes = BTreeMap::<String, usize>::new();
    for (name, page) in basis_pages() {
        for round in 0..1500 {
            let mut bytes = page.clone();
            for _ in 0..1 + next() % 4 {
                // Half the flips land in the header, index and DFD.
                let span = if round % 2 == 0 {
                    bytes.len().min(256)
                } else {
                    bytes.len()
                };
                let at = (next() % span as u64) as usize;
                bytes[at] ^= 1 << (next() % 8);
            }
            if next() % 16 == 0 {
                let keep = (next() % bytes.len() as u64) as usize;
                bytes.truncate(keep);
            }
            let outcome = match crate::basisu_ktx2::transcode_base_rgba(&bytes) {
                Ok((w, h, rgba)) => {
                    assert_eq!(rgba.len(), w as usize * h as usize * 4);
                    "decoded".to_owned()
                }
                Err(e) => {
                    assert!(
                        matches!(
                            e.kind(),
                            JobErrorKind::InvalidInput
                                | JobErrorKind::Unsupported
                                | JobErrorKind::ResourceLimit
                        ),
                        "{name}: {e}"
                    );
                    format!("{:?}", e.kind())
                }
            };
            *outcomes.entry(format!("{name} {outcome}")).or_default() += 1;
        }
    }
    println!("{outcomes:?}");
}

#[cfg(unix)]
#[test]
fn directory_members_are_regular_files_below_the_root() {
    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    let (doc, bin) = triangle_doc();
    fs::write(
        dir.join("a.glb"),
        crate::glb::encode_glb(&doc, &bin).unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(dir.join("a.glb"), dir.join("b.glb")).unwrap();
    let fifo = std::ffi::CString::new(dir.join("c.glb").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    fs::create_dir(dir.join("real")).unwrap();
    fs::copy(dir.join("a.glb"), dir.join("real/d.glb")).unwrap();
    std::os::unix::fs::symlink(dir.join("real"), dir.join("linked")).unwrap();
    let tile = |uri: &str| {
        json!({"boundingVolume": {"box": [0,0,0,1,0,0,0,1,0,0,0,1]},
                                  "geometricError": 0, "content": {"uri": uri}})
    };
    let mut root = tile("a.glb");
    root["geometricError"] = json!(1);
    root["children"] = json!([
        tile("b.glb"),
        tile("c.glb"),
        tile("real/d.glb"),
        tile("linked/d.glb")
    ]);
    fs::write(
        dir.join("tileset.json"),
        serde_json::to_vec(
            &json!({"asset": {"version": "1.1"}, "geometricError": 1, "root": root}),
        )
        .unwrap(),
    )
    .unwrap();
    let mut set = TileSet::open(dir).unwrap();
    assert!(set.plain_glb("a.glb").is_ok());
    assert!(set.plain_glb("real/d.glb").is_ok());
    for uri in ["b.glb", "c.glb", "linked/d.glb"] {
        let refused = set.plain_glb(uri).err().unwrap();
        assert_eq!(
            refused.kind(),
            JobErrorKind::Unsupported,
            "{uri}: {refused}"
        );
    }
}

/// The triangle document with `images` appended as views, one image each.
fn with_images(images: &[(&[u8], &str)]) -> (Value, Vec<u8>) {
    let (mut doc, mut bin) = triangle_doc();
    let mut list = Vec::new();
    for (bytes, mime) in images {
        crate::glb::pad_to(&mut bin, 4);
        let view = doc["bufferViews"].as_array().unwrap().len();
        doc["bufferViews"].as_array_mut().unwrap().push(json!({
            "buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len()}));
        bin.extend(*bytes);
        list.push(json!({"bufferView": view, "mimeType": mime}));
    }
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    doc["images"] = Value::Array(list);
    (doc, bin)
}

#[test]
fn json_text_is_capped_and_charged_as_parsed() {
    // A JSON chunk claimed over the cap is refused before the file is examined.
    let mut claim = b"glTF".to_vec();
    for word in [2u32, 0, JSON_LIMIT as u32 + 1, 0x4E4F_534A] {
        claim.extend(word.to_le_bytes());
    }
    let refused = decode(&claim).err().unwrap();
    assert_eq!(tile_limit(&refused).limit, "jsonBytes");
    // `[0,0,...]` expands about 16x when parsed; the charge covers it.
    let (mut doc, bin) = triangle_doc();
    doc["extras"] = json!(vec![0; 1 << 18]);
    let bytes = crate::glb::encode_glb(&doc, &bin).unwrap();
    let json = word(&bytes, 12).unwrap() as u64;
    assert!(json > 1 << 19);
    let mut budget = Budget::default();
    Plain::decode(&bytes, &mut budget).unwrap();
    assert!(
        budget.held >= JSON_EXPANSION * json,
        "{} < 16 x {json}",
        budget.held
    );
    // Under a budget below 16x, the same tile is refused before parsing.
    let mut small = Budget::with_limit(JSON_EXPANSION * json - 1);
    let refused = Plain::decode(&bytes, &mut small).err().unwrap();
    assert_eq!(tile_limit(&refused).limit, "decodedBytes");
    // A tileset document over the cap is refused when the tileset is opened.
    let work = tempfile::tempdir().unwrap();
    let mut text = br#"{"asset":{"version":"1.1"}}"#.to_vec();
    text.resize(JSON_LIMIT as usize + 1, b' ');
    fs::write(work.path().join("tileset.json"), text).unwrap();
    let refused = TileSet::open(work.path()).err().unwrap();
    assert_eq!(tile_limit(&refused).limit, "jsonBytes");
}

/// A real 8192 bake: two 8192² pages (256 MiB of RGBA each) decode as arrays within
/// the 1 GiB limit, because transient scratch is credited and the caller copies one
/// page at a time. A third such page is refused.
#[test]
fn two_full_size_pages_decode_within_the_limit() {
    let mut png = Vec::new();
    {
        let page = image::RgbaImage::from_pixel(8192, 8192, image::Rgba([90, 120, 30, 255]));
        image::codecs::png::PngEncoder::new_with_quality(
            &mut png,
            image::codecs::png::CompressionType::Default,
            image::codecs::png::FilterType::Sub,
        )
        .write_image(page.as_raw(), 8192, 8192, image::ExtendedColorType::Rgba8)
        .unwrap();
    }
    assert!(png.len() < 8 << 20, "fixture PNG is {} bytes", png.len());
    let (doc, bin) = with_images(&[(&png, "image/png"), (&png, "image/png")]);
    let decoded = arrays_of(&doc, &bin).unwrap();
    assert_eq!(decoded.pages.len(), 2);
    assert!(decoded
        .pages
        .iter()
        .all(|p| (p.width, p.height) == (8192, 8192)));
    assert_eq!(&decoded.pages[1].rgba[..4], &[90, 120, 30, 255]);
    drop(decoded);
    let (doc, bin) = with_images(&[
        (&png, "image/png"),
        (&png, "image/png"),
        (&png, "image/png"),
    ]);
    assert_eq!(
        tile_limit(&arrays_of(&doc, &bin).err().unwrap()).limit,
        "decodedBytes"
    );
}

#[test]
fn transient_pages_are_credited_and_shared_views_decode_once() {
    let pages = basis_pages();
    let (ktx2, uastc) = (&pages[0].1, &pages[1].1);
    let rgba = 32 * 24 * 4u64;
    // GLB path: each RGBA page is freed once its PNG replaces the view.
    let (doc, bin) = with_images(&[(ktx2, "image/ktx2"), (uastc, "image/ktx2")]);
    let bytes = crate::glb::encode_glb(&doc, &bin).unwrap();
    let mut budget = Budget::default();
    let plain = Plain::decode(&bytes, &mut budget).unwrap();
    let before = budget.held;
    let glb = plain.into_glb(&mut budget).unwrap();
    // Two pages are never held together, and only the GLB and its copy stay held.
    assert!(
        budget.peak < before + 2 * rgba + 4 * glb.len() as u64,
        "{}",
        budget.peak
    );
    assert!(budget.held <= before + 2 * glb.len() as u64);
    // Arrays: images sharing one view decode once and are copied.
    let (mut doc, bin) = with_images(&[(ktx2, "image/ktx2")]);
    doc["images"] = json!([{"bufferView": 1, "mimeType": "image/ktx2"},
                           {"bufferView": 1, "mimeType": "image/ktx2"}]);
    let decoded = arrays_of(&doc, &bin).unwrap();
    assert_eq!(decoded.pages.len(), 2);
    assert_eq!(decoded.pages[0], decoded.pages[1]);
}

#[test]
fn oversized_ktx2_edges_name_their_limit() {
    let mut page = basis_pages().remove(0).1;
    page[20..24].copy_from_slice(&20_000u32.to_le_bytes());
    let (doc, bin) = with_images(&[(&page, "image/ktx2")]);
    let limit = tile_limit(&arrays_of(&doc, &bin).err().unwrap());
    assert_eq!(
        (limit.limit, limit.value, limit.observed),
        ("pageEdge", 16_384, 20_000)
    );
    let bytes = crate::glb::encode_glb(&doc, &bin).unwrap();
    let refused = decode(&bytes)
        .unwrap()
        .into_glb(&mut Budget::default())
        .err()
        .unwrap();
    assert_eq!(tile_limit(&refused).limit, "pageEdge");
    let refused = with_images(&[(&png_header(16_385, 2), "image/png")]);
    let limit = tile_limit(&arrays_of(&refused.0, &refused.1).err().unwrap());
    assert_eq!(limit.limit, "pageEdge");
}
