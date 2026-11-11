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
    assert!(Plain::decode(&glb(&doc, &bin)).unwrap().into_glb().is_ok());
    let compressed = (0..doc["bufferViews"].as_array().unwrap().len())
        .find(|&i| doc["bufferViews"][i]["extensions"][MESHOPT]["mode"] == "ATTRIBUTES")
        .unwrap();
    let ext = &doc["bufferViews"][compressed]["extensions"][MESHOPT];
    let mut damaged = bin.clone();
    damaged[ext["byteOffset"].as_u64().unwrap_or(0) as usize] ^= 0xff;
    let failure = Plain::decode(&glb(&doc, &damaged)).err().unwrap();
    assert_eq!(failure.kind(), JobErrorKind::InvalidInput, "{failure}");
    let mut wrong = doc.clone();
    wrong["bufferViews"][compressed]["extensions"][MESHOPT]["filter"] = json!("EXPONENTIAL");
    let failure = Plain::decode(&glb(&wrong, &bin)).err().unwrap();
    assert_eq!(failure.kind(), JobErrorKind::Unsupported, "{failure}");
    let mut wrong = doc.clone();
    wrong["bufferViews"][compressed]["extensions"][MESHOPT]["count"] = json!(1u64 << 40);
    let failure = Plain::decode(&glb(&wrong, &bin)).err().unwrap();
    assert_eq!(failure.kind(), JobErrorKind::InvalidInput, "{failure}");
    let mut wrong = doc.clone();
    wrong["bufferViews"][compressed]["byteLength"] = json!(DECODED_LIMIT + 1);
    let failure = Plain::decode(&glb(&wrong, &bin)).err().unwrap();
    assert_eq!(failure.kind(), JobErrorKind::ResourceLimit, "{failure}");
    let mut wrong = doc.clone();
    wrong["buffers"][0]["uri"] = json!("data.bin");
    let failure = Plain::decode(&glb(&wrong, &bin)).err().unwrap();
    assert_eq!(failure.kind(), JobErrorKind::Unsupported, "{failure}");
    let mut b3dm = bytes.clone();
    b3dm[..4].copy_from_slice(b"b3dm");
    assert_eq!(
        Plain::decode(&b3dm).err().unwrap().kind(),
        JobErrorKind::Unsupported
    );
    let image = doc["images"][0]["bufferView"].as_u64().unwrap() as usize;
    let start = doc["bufferViews"][image]["byteOffset"].as_u64().unwrap() as usize;
    let mut damaged = bin.clone();
    damaged[start + 1] ^= 0xff;
    let failure = Plain::decode(&glb(&doc, &damaged))
        .unwrap()
        .into_glb()
        .err()
        .unwrap();
    assert_eq!(failure.kind(), JobErrorKind::InvalidInput, "{failure}");
}
