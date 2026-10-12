//! A-1 and A-8 (PW): the bounded implicit forest writer, its documents, and the reversible
//! payload rebase, asserted on the written bytes. P1–P12 refer to design v3 §4.2.

use base64::Engine;
use rusty_tiles::implicit::{
    expand_tileset, rebase_payload, restore_payload, write_forest, ForestOutput, RebaseRecord,
    SubdivisionScheme, TileRole,
};
use rusty_tiles::Error;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

// ---- Hierarchy fixtures ----

/// A producer tile. Children are listed in ascending slot order, so H's preorder equals the
/// expanded forest's preorder.
#[derive(Clone, Default)]
struct Spec {
    /// Written as `extras.implicitChildIndex` (R-8 strips it).
    slot: Option<usize>,
    translation: Option<[f64; 3]>,
    contents: Vec<&'static str>,
    refine: Option<&'static str>,
    extras: Option<Value>,
    children: Vec<Spec>,
}

fn tile(contents: &[&'static str], children: Vec<Spec>) -> Spec {
    Spec {
        contents: contents.to_vec(),
        children,
        ..Default::default()
    }
}

fn leaf() -> Spec {
    tile(&["glb"], Vec::new())
}

/// A single-child chain of `levels` tiles, each with one content.
fn chain(levels: usize) -> Spec {
    let mut spec = leaf();
    for _ in 1..levels {
        spec = tile(&["glb"], vec![spec]);
    }
    spec
}

impl Spec {
    fn at(mut self, slot: usize) -> Self {
        self.slot = Some(slot);
        self
    }
    fn moved(mut self, translation: [f64; 3]) -> Self {
        self.translation = Some(translation);
        self
    }
    fn refine(mut self, refine: &'static str) -> Self {
        self.refine = Some(refine);
        self
    }
    fn extras(mut self, extras: Value) -> Self {
        self.extras = Some(extras);
        self
    }
}

fn glb(document: &Value, bin: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut out = b"glTF".to_vec();
    out.extend(2u32.to_le_bytes());
    out.extend(((28 + json.len() + bin.len()) as u32).to_le_bytes());
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(&json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(&bin);
    out
}

fn b3dm(glb: &[u8]) -> Vec<u8> {
    let mut table = br#"{"BATCH_LENGTH":0}"#.to_vec();
    while !(28 + table.len()).is_multiple_of(8) {
        table.push(b' ');
    }
    let mut out = b"b3dm".to_vec();
    out.extend(1u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend((table.len() as u32).to_le_bytes());
    out.extend([0u8; 12]);
    out.extend(&table);
    out.extend(glb);
    while !out.len().is_multiple_of(8) {
        out.push(0);
    }
    let length = out.len() as u32;
    out[8..12].copy_from_slice(&length.to_le_bytes());
    out
}

fn payload(id: usize, slot: usize, suffix: &str) -> Vec<u8> {
    let mut node = json!({"name": format!("n{id}-{slot}")});
    if id % 2 == 1 {
        node["translation"] = json!([0.5, 1.5, -2.0]);
    }
    let document = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[node],"buffers":[{"byteLength":8}]});
    let bytes = glb(&document, &(id as u64).to_le_bytes());
    match suffix {
        "glb" => bytes,
        "b3dm" => b3dm(&bytes),
        "gltf" => serde_json::to_vec(&json!({"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],
            "nodes":[{"name":format!("n{id}")}]}))
        .unwrap(),
        other => other.as_bytes().to_vec(),
    }
}

struct Built {
    manifest: Value,
    /// Raw payload bytes by producer URI.
    raw: BTreeMap<String, Vec<u8>>,
}

fn build(dir: &Path, spec: &Spec) -> Built {
    fn tile(
        spec: &Spec,
        dir: &Path,
        next: &mut usize,
        depth: i32,
        raw: &mut BTreeMap<String, Vec<u8>>,
    ) -> Value {
        let id = *next;
        *next += 1;
        let mut value = json!({
            "boundingVolume":{"box":[id as f64 + 0.5, -(id as f64), 0.25, 2.,0.,0., 0.,3.,0., 0.,0.,1.]},
            "geometricError": 64. / 2f64.powi(depth),
        });
        if depth == 0 {
            value["refine"] = json!("REPLACE");
        }
        if let Some(refine) = spec.refine {
            value["refine"] = json!(refine);
        }
        if let Some([x, y, z]) = spec.translation {
            value["transform"] =
                json!([1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., x, y, z, 1.]);
        }
        let mut extras = spec.extras.clone().unwrap_or_else(|| json!({}));
        extras["id"] = json!(id);
        if let Some(slot) = spec.slot {
            extras["implicitChildIndex"] = json!(slot);
        }
        value["extras"] = extras;
        let contents: Vec<Value> = spec
            .contents
            .iter()
            .enumerate()
            .map(|(slot, suffix)| {
                let uri = format!("src/{id}-{slot}.{suffix}");
                let bytes = payload(id, slot, suffix);
                std::fs::write(dir.join(&uri), &bytes).unwrap();
                raw.insert(uri.clone(), bytes);
                let mut content = json!({ "uri": uri });
                if *suffix == "glb" {
                    content["extensions"] = json!({"3DTILES_content_gltf_vector":{"vector":true}});
                }
                content
            })
            .collect();
        if contents.len() == 1 {
            value["content"] = contents[0].clone();
        } else if !contents.is_empty() {
            value["contents"] = json!(contents);
        }
        if !spec.children.is_empty() {
            value["children"] = spec
                .children
                .iter()
                .map(|child| tile(child, dir, next, depth + 1, raw))
                .collect();
        }
        value
    }
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let mut raw = BTreeMap::new();
    let root = tile(spec, dir, &mut 0, 0, &mut raw);
    Built {
        manifest: json!({"asset":{"version":"1.1","generator":"implicit_forest"},
            "extensionsUsed":["3DTILES_content_gltf_vector"],"geometricError":128.,"root":root}),
        raw,
    }
}

fn write(
    dir: &Path,
    manifest: &mut Value,
    scheme: SubdivisionScheme,
) -> Result<ForestOutput, Error> {
    write_forest(manifest, dir, scheme, &mut || Ok(()))
}

fn files(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let name = path.strip_prefix(dir).unwrap().to_string_lossy();
                out.insert(name.replace('\\', "/"));
            }
        }
    }
    out
}

fn reader(dir: &Path) -> impl FnMut(&str) -> Result<Vec<u8>, Error> + '_ {
    move |name| Ok(std::fs::read(dir.join(name))?)
}

// ---- Independent checks on the written bytes ----

/// Value nodes (root, containers and scalars) and depth (document object 0), as the validator
/// counts them.
fn measure(value: &Value, depth: usize) -> (usize, usize) {
    let children: Vec<&Value> = match value {
        Value::Array(values) => values.iter().collect(),
        Value::Object(values) => values.values().collect(),
        _ => return (1, depth),
    };
    children
        .into_iter()
        .map(|child| measure(child, depth + 1))
        .fold((1, depth), |(n, d), (cn, cd)| (n + cn, d.max(cd)))
}

fn subtree_json(bytes: &[u8]) -> Value {
    assert_eq!(&bytes[..4], b"subt");
    let length = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&bytes[24..24 + length]).unwrap()
}

#[derive(Debug, Default)]
struct Audit {
    documents: Vec<String>,
    /// Document name → (value nodes, bytes, depth, holds an implicit root, has schema).
    shape: BTreeMap<String, (usize, usize, usize, bool, bool)>,
    links: usize,
    roots: Vec<Value>,
    /// The deepest chain of links below `tileset.json`.
    link_depth: usize,
}

/// P1, P2, P4 (names), P6, P8, P11 and P12, read from the documents and subtrees alone.
fn audit_forest(dir: &Path, manifest: &Value, scheme: SubdivisionScheme) -> Audit {
    let mut audit = Audit::default();
    let mut prefixes = BTreeSet::new();
    let mut referenced_documents = BTreeSet::new();
    let mut pending = vec![("tileset.json".to_owned(), manifest.clone(), 0usize)];
    while let Some((name, document, link_depth)) = pending.pop() {
        audit.link_depth = audit.link_depth.max(link_depth);
        let text = serde_json::to_vec(&document).unwrap();
        assert!(
            !String::from_utf8_lossy(&text).contains("implicitChildIndex"),
            "P11 in {name}"
        );
        let (nodes, depth) = measure(&document, 0);
        assert!(
            nodes <= 16_384 && text.len() <= 2 * 1024 * 1024 && depth <= 32,
            "P8 {name}"
        );
        let mut holds_root = false;
        let mut stack = vec![&document["root"]];
        while let Some(tile) = stack.pop() {
            if let Some(tiling) = tile.get("implicitTiling") {
                holds_root = true;
                audit.roots.push(tile.clone());
                let levels = tiling["subtreeLevels"].as_u64().unwrap();
                assert_eq!(tiling["availableLevels"].as_u64(), Some(levels), "P2");
                assert!((1..=4).contains(&levels), "P2: at most S levels");
                assert!(tile.get("children").is_none());
                let uri = tiling["subtrees"]["uri"].as_str().unwrap();
                let digits = uri
                    .strip_prefix("subtrees/r")
                    .and_then(|rest| rest.split_once('-'))
                    .map(|(digits, _)| digits)
                    .unwrap();
                assert!(digits.bytes().all(|b| b.is_ascii_digit()), "R-7 {uri}");
                assert!(prefixes.insert(digits.to_owned()), "P4: distinct prefixes");
                let coordinates = if scheme == SubdivisionScheme::Octree {
                    "{level}-{x}-{y}-{z}"
                } else {
                    "{level}-{x}-{y}"
                };
                assert_eq!(uri, format!("subtrees/r{digits}-{coordinates}.subtree"));
                let templates: Vec<&Value> = match (tile.get("content"), tile.get("contents")) {
                    (Some(content), None) => vec![content],
                    (None, Some(contents)) => contents.as_array().unwrap().iter().collect(),
                    (None, None) => Vec::new(),
                    _ => panic!("content and contents"),
                };
                for (slot, template) in templates.iter().enumerate() {
                    let uri = template["uri"].as_str().unwrap();
                    assert!(!uri.to_ascii_lowercase().ends_with(".json"), "P1 {uri}");
                    assert!(uri
                        .starts_with(&format!("implicit-content/r{digits}-{coordinates}-{slot}.")));
                }
                let origin = if scheme == SubdivisionScheme::Octree {
                    "0-0-0-0"
                } else {
                    "0-0-0"
                };
                let bytes = std::fs::read(dir.join(format!("subtrees/r{digits}-{origin}.subtree")))
                    .unwrap();
                assert!(!String::from_utf8_lossy(&bytes).contains("implicitChildIndex"));
                let subtree = subtree_json(&bytes);
                assert_eq!(
                    subtree["childSubtreeAvailability"],
                    json!({"constant":0}),
                    "P2"
                );
                assert_eq!(
                    subtree["contentAvailability"]
                        .as_array()
                        .map_or(0, Vec::len),
                    templates.len(),
                    "P10"
                );
                assert!(subtree.get("contentAvailability").is_some() || templates.is_empty());
                assert_eq!(subtree["tileMetadata"], 0, "P3: rows everywhere");
                continue;
            }
            if let Some(uri) = tile["content"]["uri"]
                .as_str()
                .filter(|u| u.ends_with(".json"))
            {
                audit.links += 1;
                assert!(tile.get("children").is_none());
                assert!(
                    referenced_documents.insert(uri.to_owned()),
                    "P1: one link each"
                );
                let linked: Value =
                    serde_json::from_slice(&std::fs::read(dir.join(uri)).unwrap()).unwrap();
                assert_eq!(
                    linked["root"]["boundingVolume"], tile["boundingVolume"],
                    "P6"
                );
                assert_eq!(
                    linked["root"]["geometricError"], tile["geometricError"],
                    "P6"
                );
                assert_eq!(linked["geometricError"], tile["geometricError"]);
                assert_eq!(linked["root"]["refine"], tile["refine"]);
                assert!(linked["root"].get("transform").is_none());
                assert_eq!(linked["extensionsUsed"], manifest["extensionsUsed"]);
                pending.push((uri.to_owned(), linked, link_depth + 1));
                continue;
            }
            for child in tile["children"].as_array().into_iter().flatten() {
                stack.push(child);
            }
        }
        let has_schema = document.get("schema").is_some();
        assert!(!holds_root || has_schema, "P12: {name} holds a root");
        audit.shape.insert(
            name.clone(),
            (nodes, text.len(), depth, holds_root, has_schema),
        );
        audit.documents.push(name);
    }
    assert!(
        audit.shape["tileset.json"].4,
        "P12: the root document has the schema"
    );
    let on_disk: BTreeSet<String> = files(dir)
        .into_iter()
        .filter(|f| f.starts_with("tileset"))
        .collect();
    assert_eq!(
        on_disk, referenced_documents,
        "P1: documents are exactly the links"
    );
    audit
}

/// A tile in the root frame: centre, half axes, error, effective refine, extras without
/// `implicitChildIndex` and `encodedBytes`, and the parent's preorder index.
type Flat = ([f64; 3], [f64; 9], f64, String, Value, Option<usize>);

fn flatten(root: &Value) -> Vec<(Flat, Vec<String>)> {
    fn walk(
        tile: &Value,
        parent: Option<usize>,
        offset: [f64; 3],
        refine: &str,
        out: &mut Vec<(Flat, Vec<String>)>,
    ) {
        let refine = tile["refine"].as_str().unwrap_or(refine).to_owned();
        let mut offset = offset;
        if parent.is_some() {
            if let Some(transform) = tile["transform"].as_array() {
                for (lane, value) in offset.iter_mut().zip(&transform[12..15]) {
                    *lane += value.as_f64().unwrap();
                }
            }
        }
        let b: Vec<f64> = tile["boundingVolume"]["box"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let mut extras = tile.get("extras").cloned().unwrap_or_else(|| json!({}));
        if let Some(map) = extras.as_object_mut() {
            map.remove("implicitChildIndex");
            map.remove("encodedBytes");
        }
        let uris = match (tile.get("content"), tile.get("contents")) {
            (Some(c), _) => vec![c["uri"].as_str().unwrap().to_owned()],
            (_, Some(cs)) => cs
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["uri"].as_str().unwrap().to_owned())
                .collect(),
            _ => Vec::new(),
        };
        let id = out.len();
        out.push((
            (
                [b[0] + offset[0], b[1] + offset[1], b[2] + offset[2]],
                std::array::from_fn(|i| b[3 + i]),
                tile["geometricError"].as_f64().unwrap(),
                refine.clone(),
                extras,
                parent,
            ),
            uris,
        ));
        for child in tile["children"].as_array().into_iter().flatten() {
            walk(child, Some(id), offset, &refine, out);
        }
    }
    let mut out = Vec::new();
    walk(root, None, [0.; 3], "REPLACE", &mut out);
    out
}

/// P5: expanding every document and root yields exactly H, and every payload restores to the
/// producer's bytes. Also checks `ForestOutput.tiles` (R-6) and closure (P4).
fn effective_tree_equals_h(dir: &Path, h: &Built, manifest: &Value, output: &ForestOutput) {
    let expanded = expand_tileset(manifest, reader(dir)).unwrap();
    assert!(!serde_json::to_string(&expanded)
        .unwrap()
        .contains("implicitChildIndex"));
    let ours = flatten(&expanded["root"]);
    let theirs = flatten(&h.manifest["root"]);
    assert_eq!(ours.len(), theirs.len());
    assert_eq!(output.tiles.len(), theirs.len(), "R-6: one record per tile");
    let mut referenced: BTreeSet<String> = files(dir)
        .into_iter()
        .filter(|f| f.starts_with("tileset-d") || f.starts_with("subtrees/"))
        .collect();
    for (index, ((flat, uris), (expected, raw_uris))) in ours.iter().zip(&theirs).enumerate() {
        assert_eq!(flat, expected, "tile {index}");
        assert_eq!(uris.len(), raw_uris.len(), "tile {index}");
        let record = &output.tiles[index];
        assert_eq!(record.contents.len(), uris.len());
        for ((uri, raw_uri), written) in uris.iter().zip(raw_uris).zip(&record.contents) {
            assert_eq!(&written.member, uri);
            referenced.insert(uri.clone());
            let published = std::fs::read(dir.join(uri)).unwrap();
            assert_eq!(written.bytes, published.len() as u64);
            let raw = &h.raw[raw_uri];
            match &written.rebase {
                None => {
                    assert!(matches!(
                        record.role,
                        TileRole::Skeleton | TileRole::ExplicitLeaf
                    ));
                    assert_eq!(uri, raw_uri);
                    assert_eq!(&published, raw);
                }
                Some(rebase) => {
                    assert!(uri.starts_with("implicit-content/"));
                    assert_eq!(rebase.raw_sha256, <[u8; 32]>::from(Sha256::digest(raw)));
                    assert_eq!(&restore_payload(&published, rebase).unwrap(), raw);
                }
            }
        }
    }
    assert_eq!(
        files(dir),
        referenced,
        "P4: every written member is referenced"
    );
    let members: BTreeSet<String> = output.members.iter().cloned().collect();
    assert_eq!(members.len(), output.members.len(), "members are distinct");
    assert_eq!(members, referenced, "members list every written member");
}

fn roles(output: &ForestOutput) -> Vec<TileRole> {
    output.tiles.iter().map(|t| t.role).collect()
}

fn find_root(audit: &Audit, n: usize) -> &Value {
    audit
        .roots
        .iter()
        .find(|r| {
            r["implicitTiling"]["subtrees"]["uri"]
                .as_str()
                .unwrap()
                .starts_with(&format!("subtrees/r{n}-"))
        })
        .unwrap()
}

fn data_error(result: Result<ForestOutput, Error>) -> String {
    match result {
        Err(Error::Data(message)) => message,
        other => panic!(
            "expected a typed data error, got {:?}",
            other.map(|o| o.documents)
        ),
    }
}

// ---- A-1: layout ----

#[test]
fn skeleton_leaves_and_roots_with_slot_lists_rows_and_rebased_payloads() {
    let dir = tempfile::tempdir().unwrap();
    // Preorder ids: 0 root; 1 A (h=4); 2 leaf; 3 R1 (h=3, translated chain); 7 R2 (h=2, a glb
    // slot and a b3dm slot); 11 leaf; 12 R3 (h=1, no content).
    let r1 = tile(
        &["glb"],
        vec![tile(&["glb"], vec![chain(2).at(2).moved([0.25, -4., 0.])]).moved([8., 2., 1.])],
    );
    let r2 = tile(
        &["glb"],
        vec![
            tile(&["glb", "b3dm"], vec![leaf().at(1)])
                .at(0)
                .moved([-16., 0.5, 0.]),
            leaf().at(3),
        ],
    );
    let r3 = tile(&[], vec![tile(&[], Vec::new())]).extras(json!({"note":"empty"}));
    let a = tile(&["glb"], vec![leaf(), r1.at(3).moved([2., 4., 0.])]);
    let spec = tile(
        &["glb"],
        vec![a, r2.at(1).moved([1., 1., 1.]), leaf().at(2), r3.at(3)],
    );
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
    use TileRole::*;
    assert_eq!(
        roles(&output),
        [
            Skeleton,
            Skeleton,
            ExplicitLeaf,
            ImplicitRoot,
            Implicit,
            Implicit,
            Implicit,
            ImplicitRoot,
            Implicit,
            Implicit,
            Implicit,
            ExplicitLeaf,
            ImplicitRoot,
            Implicit
        ]
    );
    assert_eq!((output.documents, output.implicit_roots), (1, 3));
    let audit = audit_forest(dir.path(), &manifest, SubdivisionScheme::Quadtree);
    assert_eq!(audit.documents, ["tileset.json"]);
    assert!(manifest["root"].get("implicitTiling").is_none());
    assert_eq!(manifest["schema"]["id"], "rustyTilesImplicit");
    // R-4/P10: three slot lists.
    let r1 = find_root(&audit, 3);
    assert_eq!(r1["implicitTiling"]["subtreeLevels"], 4);
    assert_eq!(
        r1["content"]["uri"],
        "implicit-content/r3-{level}-{x}-{y}-0.glb"
    );
    assert_eq!(
        r1["content"]["extensions"],
        json!({"3DTILES_content_gltf_vector":{"vector":true}}),
        "content headers are copied per slot"
    );
    assert_eq!(
        r1["transform"][12], 2.,
        "the producer's transform stays on the root"
    );
    let r2 = find_root(&audit, 7);
    let r2_uris: Vec<&str> = r2["contents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["uri"].as_str().unwrap())
        .collect();
    assert_eq!(
        r2_uris,
        [
            "implicit-content/r7-{level}-{x}-{y}-0.glb",
            "implicit-content/r7-{level}-{x}-{y}-1.b3dm"
        ]
    );
    let r3 = find_root(&audit, 12);
    assert!(r3.get("content").is_none() && r3.get("contents").is_none());
    assert_eq!(r3["implicitTiling"]["subtreeLevels"], 2);
    // P3: the level-0 row is the root tile's box, bitwise.
    let expanded = expand_tileset(&manifest, reader(dir.path())).unwrap();
    let expanded_r1 = &expanded["root"]["children"][0]["children"][1];
    assert_eq!(expanded_r1["boundingVolume"], r1["boundingVolume"]);
    assert_eq!(expanded_r1["extras"], json!({"id":3}));
    // Raw sources of rewritten payloads are deleted; explicit payloads stay in place.
    assert!(!dir.path().join("src/4-0.glb").exists());
    assert!(dir.path().join("src/2-0.glb").exists());
    // A translated payload carries a rebase record; an implicit root's own payload is a copy.
    let record = output.tiles[4].contents[0].rebase.as_ref().unwrap();
    assert_eq!(record.delta, [8., 2., 1.]);
    assert!(record.raw_root_node.starts_with('{'));
    let root_copy = output.tiles[3].contents[0].rebase.as_ref().unwrap();
    assert_eq!(
        (
            root_copy.delta,
            root_copy.raw_root_node.as_str(),
            root_copy.raw_json_length
        ),
        ([0.; 3], "", 0)
    );
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);
}

#[test]
fn short_octree_is_one_implicit_root_with_its_transform() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = tile(
        &["glb"],
        vec![
            tile(
                &["glb"],
                vec![leaf().at(2), leaf().at(6).moved([0.5, 0.5, 0.5])],
            )
            .at(0),
            leaf().at(5).moved([-1., 2., -3.]),
            tile(&["glb"], vec![leaf().at(7)]).at(7).moved([4., 4., 4.]),
        ],
    );
    spec.translation = Some([1000., 2000., 3000.]);
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Octree).unwrap();
    assert_eq!(output.implicit_roots, 1);
    assert_eq!(output.tiles[0].role, TileRole::ImplicitRoot);
    assert!(output.tiles[1..]
        .iter()
        .all(|t| t.role == TileRole::Implicit));
    let root = &manifest["root"];
    assert_eq!(root["implicitTiling"]["subdivisionScheme"], "OCTREE");
    assert_eq!(root["implicitTiling"]["subtreeLevels"], 3);
    assert_eq!(
        root["implicitTiling"]["subtrees"]["uri"],
        "subtrees/r0-{level}-{x}-{y}-{z}.subtree"
    );
    assert_eq!(root["transform"][12], 1000.);
    assert_eq!(root["refine"], "REPLACE");
    audit_forest(dir.path(), &manifest, SubdivisionScheme::Octree);
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);
    assert_eq!(
        files(dir.path())
            .iter()
            .filter(|f| f.starts_with("src/"))
            .count(),
        0
    );
}

/// Implicit tiles are published at δ accumulated in f64 and their payloads are rebased in
/// f64, so content drifts from H's exact placement by a few ulps where a sum is inexact
/// (0.1 + 0.2 here, and 0.5 + 0.1 in tile 3's rebased root). The implicit root and its
/// explicit ancestors grow outward by that drift; exact sums leave every box unchanged.
#[test]
fn inexact_translation_sums_widen_rows_and_ancestors_outward() {
    let dir = tempfile::tempdir().unwrap();
    let spec = tile(
        &["glb"],
        vec![tile(
            &["glb"],
            vec![tile(
                &["glb"],
                vec![tile(&["glb"], vec![leaf().moved([0.2, 0., 0.])]).moved([0.1, 0., 0.])],
            )],
        )],
    );
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Octree).unwrap();
    assert_eq!(
        roles(&output)[..2],
        [TileRole::Skeleton, TileRole::ImplicitRoot]
    );
    let skeleton = &manifest["root"];
    let root = &skeleton["children"][0];
    assert!(root.get("implicitTiling").is_some());
    for tile in [skeleton, root] {
        let b = tile["boundingVolume"]["box"].as_array().unwrap();
        assert_eq!(b[3].as_f64(), Some(2f64.next_up()));
        assert_eq!((b[7].as_f64(), b[11].as_f64()), (Some(3.), Some(1.)));
    }
    // Every row of the implicit root grows on x only, and only outward.
    let expanded = expand_tileset(&manifest, reader(dir.path())).unwrap();
    let ours = flatten(&expanded["root"]);
    let theirs = flatten(&h.manifest["root"]);
    for (index, ((ours, _), (theirs, _))) in ours.iter().zip(&theirs).enumerate() {
        // Half axes, row major: x half at 0, y half at 4, z half at 8.
        let (half, expected) = (ours.1, theirs.1);
        assert!(half[0] > expected[0], "tile {index}");
        assert_eq!(half[1..], expected[1..], "tile {index}");
    }
    // Only the payload rebase is inexact: δ = 0 + 0.1 is exact, but tile 1's raw scene
    // root translation 0.5 + 0.1 rounds, so the implicit root still grows on x.
    let dir = tempfile::tempdir().unwrap();
    let spec = tile(&["glb"], vec![leaf().moved([0.1, 0., 0.])]);
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Octree).unwrap();
    assert_eq!(roles(&output)[0], TileRole::ImplicitRoot);
    let b = manifest["root"]["boundingVolume"]["box"]
        .as_array()
        .unwrap();
    assert_eq!(b[3].as_f64(), Some(2f64.next_up()));
    assert_eq!((b[7].as_f64(), b[11].as_f64()), (Some(3.), Some(1.)));
}

#[test]
fn thirty_two_levels_link_two_documents_deep_and_thirty_three_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    // A comb: a 32-level chain whose tiles each also carry a two-tile implicit root with a glb
    // (extension header) and a b3dm slot, so the deepest member is
    // `contents[i].extensions.3DTILES_content_gltf_vector.vector` at skeleton level 11.
    let mut spec = leaf();
    for level in (0..31).rev() {
        let side = if level == 30 {
            tile(&["glb", "b3dm"], Vec::new())
        } else {
            tile(&["glb", "b3dm"], vec![tile(&["glb", "b3dm"], Vec::new())])
        };
        spec = tile(&["glb"], vec![spec.at(0), side.at(1)]);
    }
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
    // Skeleton levels 0..=27 (height ≥ 4); a document holds 11 skeleton levels.
    assert_eq!(output.documents, 3);
    let audit = audit_forest(dir.path(), &manifest, SubdivisionScheme::Quadtree);
    assert_eq!(audit.links, 2);
    assert_eq!(audit.link_depth, 2, "a link two levels deep");
    let depth = audit.shape.values().map(|s| s.2).max().unwrap();
    assert_eq!(
        depth, 28,
        "1 + 2·11 + 5: the D-3 cap keeps 4 levels below 32"
    );
    // The tileset.json link is the skeleton tile at level 11.
    let mut tile = &manifest["root"];
    for _ in 0..11 {
        tile = &tile["children"][0];
    }
    assert_eq!(tile["content"]["uri"], "tileset-d1.json");
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);

    let dir = tempfile::tempdir().unwrap();
    let h = build(dir.path(), &chain(33));
    let before = files(dir.path());
    let mut manifest = h.manifest.clone();
    assert_eq!(
        data_error(write(
            dir.path(),
            &mut manifest,
            SubdivisionScheme::Quadtree
        )),
        "hierarchy has 33 levels; implicit output admits at most 32"
    );
    assert_eq!(files(dir.path()), before, "refused before writing");
    assert_eq!(manifest, h.manifest, "the manifest is unchanged on failure");
}

#[test]
fn schema_is_in_the_root_document_and_every_document_holding_a_root() {
    let dir = tempfile::tempdir().unwrap();
    let h = build(dir.path(), &chain(32));
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
    let audit = audit_forest(dir.path(), &manifest, SubdivisionScheme::Quadtree);
    let schema = |name: &str| (audit.shape[name].3, audit.shape[name].4);
    // The only implicit root is at level 28, in the second linked document.
    assert_eq!(
        schema("tileset.json"),
        (false, true),
        "Cesium reads the root schema"
    );
    assert_eq!(schema("tileset-d1.json"), (false, false));
    assert_eq!(schema("tileset-d2.json"), (true, true));
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);
}

/// A skeleton tile of height 4 whose extras carry `pad`.
fn padded(pad: Value) -> Spec {
    chain(5).extras(json!({ "pad": pad }))
}

#[test]
fn documents_pack_at_the_node_and_byte_budgets() {
    // Nodes: three 5,000-node units fit in 16,384 with the root; the fourth links.
    let dir = tempfile::tempdir().unwrap();
    let pad = json!(vec![0; 5000]);
    let spec = tile(
        &["glb"],
        (0..4).map(|s| padded(pad.clone()).at(s)).collect(),
    );
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
    assert_eq!(output.documents, 2);
    let audit = audit_forest(dir.path(), &manifest, SubdivisionScheme::Quadtree);
    assert!(audit.shape["tileset.json"].0 > 15_000);
    assert_eq!(
        manifest["root"]["children"][3]["content"]["uri"],
        "tileset-d1.json"
    );
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);

    // Bytes: two 700 kB units fit in 2 MiB; the third and fourth link.
    let dir = tempfile::tempdir().unwrap();
    let blob = json!("x".repeat(700_000));
    let spec = tile(
        &["glb"],
        (0..4).map(|s| padded(blob.clone()).at(s)).collect(),
    );
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
    assert_eq!(output.documents, 3);
    let audit = audit_forest(dir.path(), &manifest, SubdivisionScheme::Quadtree);
    assert!(audit.shape["tileset.json"].1 > 1_400_000);
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);
}

#[test]
fn a_single_unit_near_the_budget_fits_and_one_over_it_is_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let h = build(dir.path(), &padded(json!(vec![0; 16_000])));
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
    assert_eq!(output.documents, 1);
    let audit = audit_forest(dir.path(), &manifest, SubdivisionScheme::Quadtree);
    assert!(audit.shape["tileset.json"].0 > 16_000);
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);

    let dir = tempfile::tempdir().unwrap();
    let h = build(dir.path(), &padded(json!(vec![0; 16_400])));
    let mut manifest = h.manifest.clone();
    let message = data_error(write(
        dir.path(),
        &mut manifest,
        SubdivisionScheme::Quadtree,
    ));
    assert!(
        message.contains("single skeleton unit exceeds the document budget"),
        "{message}"
    );
}

#[test]
fn json_depth_is_a_packing_criterion() {
    fn nested(levels: usize) -> Value {
        (0..levels).fold(json!(0), |inner, _| json!([inner]))
    }
    // At skeleton level 0 the extras reach depth 1 + 1 + 29 = 31; at level 1 they would reach
    // 33, so the child is linked instead.
    let dir = tempfile::tempdir().unwrap();
    let spec = tile(&["glb"], vec![padded(nested(28)).at(0), chain(5).at(1)]);
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
    assert_eq!(output.documents, 2);
    assert_eq!(
        manifest["root"]["children"][0]["content"]["uri"],
        "tileset-d1.json"
    );
    let audit = audit_forest(dir.path(), &manifest, SubdivisionScheme::Quadtree);
    assert_eq!(audit.shape["tileset-d1.json"].2, 31);
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);

    let dir = tempfile::tempdir().unwrap();
    let h = build(dir.path(), &padded(nested(31)));
    let mut manifest = h.manifest.clone();
    let message = data_error(write(
        dir.path(),
        &mut manifest,
        SubdivisionScheme::Quadtree,
    ));
    assert!(message.contains("JSON depth 32"), "{message}");
}

#[test]
fn refine_is_the_producers_and_a_change_below_a_root_is_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    // An ADD implicit root under a REPLACE skeleton, and a root inheriting ADD from its
    // skeleton parent.
    let add_root = tile(&["glb"], vec![leaf(), leaf().at(1)]).refine("ADD");
    let add_parent = tile(
        &["glb"],
        vec![chain(4).at(0), tile(&["glb"], vec![leaf()]).at(1)],
    )
    .refine("ADD");
    let spec = tile(
        &["glb"],
        vec![chain(5).at(0), add_root.at(1), add_parent.at(2)],
    );
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
    let audit = audit_forest(dir.path(), &manifest, SubdivisionScheme::Quadtree);
    let refines: BTreeMap<String, String> = audit
        .roots
        .iter()
        .map(|r| {
            (
                r["implicitTiling"]["subtrees"]["uri"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                r["refine"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(refines["subtrees/r2-{level}-{x}-{y}.subtree"], "REPLACE");
    assert_eq!(refines["subtrees/r6-{level}-{x}-{y}.subtree"], "ADD");
    assert_eq!(
        refines["subtrees/r14-{level}-{x}-{y}.subtree"], "ADD",
        "inherited"
    );
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);

    let dir = tempfile::tempdir().unwrap();
    let spec = tile(&["glb"], vec![leaf(), leaf().refine("ADD").at(1)]);
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let message = data_error(write(
        dir.path(),
        &mut manifest,
        SubdivisionScheme::Quadtree,
    ));
    assert!(
        message.contains("refine ADD differs from its implicit root's REPLACE"),
        "{message}"
    );
}

#[test]
fn gltf_is_a_byte_copy_at_an_implicit_root_and_refused_below_it() {
    let dir = tempfile::tempdir().unwrap();
    let h = build(dir.path(), &tile(&["gltf"], Vec::new()));
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Octree).unwrap();
    assert_eq!(
        manifest["root"]["content"]["uri"],
        "implicit-content/r0-{level}-{x}-{y}-{z}-0.gltf"
    );
    assert_eq!(
        std::fs::read(dir.path().join("implicit-content/r0-0-0-0-0-0.gltf")).unwrap(),
        h.raw["src/0-0.gltf"]
    );
    assert_eq!(
        output.tiles[0].contents[0].rebase.as_ref().unwrap().delta,
        [0.; 3]
    );
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);

    // An explicit leaf may hold .gltf; an implicit tile may not.
    let dir = tempfile::tempdir().unwrap();
    let spec = tile(
        &["glb"],
        vec![chain(4).at(0), tile(&["gltf"], Vec::new()).at(1)],
    );
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
    assert_eq!(output.tiles[5].role, TileRole::ExplicitLeaf);
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);

    let dir = tempfile::tempdir().unwrap();
    let spec = tile(&["glb"], vec![tile(&["gltf"], Vec::new())]);
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let message = data_error(write(
        dir.path(),
        &mut manifest,
        SubdivisionScheme::Quadtree,
    ));
    assert!(
        message.contains(".gltf content is admitted only"),
        "{message}"
    );
}

#[test]
fn encoded_bytes_follow_the_published_payload_for_implicit_tiles_only() {
    let dir = tempfile::tempdir().unwrap();
    let sized = |spec: Spec| spec.extras(json!({"encodedBytes": 1}));
    // 0 root (skeleton); 1 R (h=3); 5 R' (h=1); 6 its translated child.
    let spec = sized(tile(
        &["glb"],
        vec![
            sized(chain(4)).at(0),
            sized(tile(
                &["glb"],
                vec![sized(leaf()).at(0).moved([1., 1., 1.])],
            ))
            .at(1),
        ],
    ));
    let h = build(dir.path(), &spec);
    let mut manifest = h.manifest.clone();
    let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
    let published = |id: usize| output.tiles[id].contents[0].bytes;
    assert_eq!(output.tiles[0].role, TileRole::Skeleton);
    assert_eq!(
        output.tiles[0].extras["encodedBytes"], 1,
        "explicit: the producer's value"
    );
    for id in [1, 5, 6] {
        assert_eq!(
            output.tiles[id].extras["encodedBytes"],
            published(id),
            "tile {id}"
        );
    }
    assert!(
        published(6) > h.raw["src/6-0.glb"].len() as u64,
        "the rebase adds a translation"
    );
    effective_tree_equals_h(dir.path(), &h, &manifest, &output);
}

#[test]
fn malformed_hierarchies_are_typed_errors() {
    type Edit = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Edit)> = vec![
        (
            "duplicate child slot",
            Box::new(|m| m["root"]["children"][1]["extras"]["implicitChildIndex"] = json!(0)),
        ),
        (
            "outside the QUADTREE scheme",
            Box::new(|m| m["root"]["children"][0]["extras"]["implicitChildIndex"] = json!(4)),
        ),
        (
            "must be a translation",
            Box::new(|m| {
                m["root"]["children"][0]["transform"] =
                    json!([2., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.])
            }),
        ),
        (
            "needs a box",
            Box::new(|m| m["root"]["children"][0]["boundingVolume"] = json!({"sphere":[0,0,0,1]})),
        ),
        (
            "finite nonnegative geometricError",
            Box::new(|m| m["root"]["children"][0]["geometricError"] = json!(-1)),
        ),
        (
            "must be glb, gltf or b3dm",
            Box::new(|m| m["root"]["children"][0]["content"]["uri"] = json!("src/1-0.pnts")),
        ),
        (
            "header differs",
            Box::new(|m| m["root"]["children"][1]["content"]["extensions"] = json!({})),
        ),
        (
            "cannot be expressed",
            Box::new(|m| {
                m["root"]["children"][0]["viewerRequestVolume"] =
                    json!({"box":[0,0,0,1,0,0,0,1,0,0,0,1]})
            }),
        ),
        (
            "relative member name",
            Box::new(|m| m["root"]["children"][0]["content"]["uri"] = json!("../1-0.glb")),
        ),
    ];
    for (expected, edit) in cases {
        let dir = tempfile::tempdir().unwrap();
        let h = build(dir.path(), &tile(&["glb"], vec![leaf(), leaf()]));
        let mut manifest = h.manifest.clone();
        edit(&mut manifest);
        let original = manifest.clone();
        let message = data_error(write(
            dir.path(),
            &mut manifest,
            SubdivisionScheme::Quadtree,
        ));
        assert!(message.contains(expected), "{expected}: {message}");
        assert_eq!(manifest, original);
    }
    // Cancellation through the checkpoint stops the writer with the caller's error.
    let dir = tempfile::tempdir().unwrap();
    let h = build(dir.path(), &chain(6));
    let mut manifest = h.manifest.clone();
    let mut calls = 0;
    let result = write_forest(
        &mut manifest,
        dir.path(),
        SubdivisionScheme::Quadtree,
        &mut || {
            calls += 1;
            if calls > 3 {
                Err(Error::Message("cancelled".into()))
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(result, Err(Error::Message(m)) if m == "cancelled"));
}

#[test]
fn writing_is_a_function_of_h() {
    let spec = tile(
        &["glb"],
        vec![
            chain(6).at(1),
            tile(&["glb", "b3dm"], vec![leaf().moved([3., 0., 0.])]).at(2),
        ],
    );
    let mut outputs = Vec::new();
    for _ in 0..2 {
        let dir = tempfile::tempdir().unwrap();
        let h = build(dir.path(), &spec);
        let mut manifest = h.manifest.clone();
        let output = write(dir.path(), &mut manifest, SubdivisionScheme::Quadtree).unwrap();
        let bytes: BTreeMap<String, Vec<u8>> = files(dir.path())
            .into_iter()
            .map(|f| (f.clone(), std::fs::read(dir.path().join(f)).unwrap()))
            .collect();
        outputs.push((serde_json::to_vec(&manifest).unwrap(), bytes, output));
    }
    assert_eq!(outputs[0], outputs[1], "P7");
}

// ---- A-8: the reversible rebase ----

struct Payload {
    name: &'static str,
    delta: [f64; 3],
    base64: &'static str,
}

fn decode(payload: &Payload) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(payload.base64)
        .unwrap()
}

/// (JSON chunk text without padding, JSON chunk length, bytes after the JSON chunk).
fn glb_parts(bytes: &[u8]) -> (Vec<u8>, usize, Vec<u8>) {
    let offset = if bytes.starts_with(b"b3dm") {
        28 + (3..7)
            .map(|i| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()) as usize)
            .sum::<usize>()
    } else {
        0
    };
    let glb = &bytes[offset..];
    let length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json = &glb[20..20 + length];
    let end = json.iter().rposition(|&b| b != b' ').unwrap() + 1;
    (json[..end].to_vec(), length, glb[20 + length..].to_vec())
}

fn root_node(json: &Value) -> &Value {
    let root = json["scenes"][0]["nodes"][0].as_u64().unwrap() as usize;
    &json["nodes"][root]
}

/// Round trip, digest, untouched BIN, equal parsed JSON outside the root translation, and
/// alignment kept.
fn assert_exact(raw: &[u8], delta: [f64; 3]) -> (Vec<u8>, RebaseRecord) {
    let (published, record) = rebase_payload(raw, delta).unwrap();
    assert_ne!(published, raw);
    assert_eq!(record.raw_sha256, <[u8; 32]>::from(Sha256::digest(raw)));
    assert_eq!(record.delta, delta);
    assert_eq!(
        restore_payload(&published, &record).unwrap(),
        raw,
        "byte-exact restore"
    );
    let (raw_json, raw_length, raw_rest) = glb_parts(raw);
    let (json, length, rest) = glb_parts(&published);
    assert_eq!(rest, raw_rest, "BIN untouched");
    assert_eq!(length % 8, raw_length % 8, "JSON chunk alignment kept");
    assert_eq!(record.raw_json_length as usize, raw_length);
    assert_eq!(published.len() % 8, raw.len() % 8);
    let mut raw_doc: Value = serde_json::from_slice(&raw_json).unwrap();
    let mut doc: Value = serde_json::from_slice(&json).unwrap();
    let raw_node = root_node(&raw_doc).clone();
    assert_eq!(serde_json::to_string(&raw_node).unwrap(), {
        let parsed: Value = serde_json::from_str(&record.raw_root_node).unwrap();
        serde_json::to_string(&parsed).unwrap()
    });
    let step = [delta[0], delta[2], -delta[1]];
    let index = raw_doc["scenes"][0]["nodes"][0].as_u64().unwrap() as usize;
    if let Some(matrix) = raw_node.get("matrix") {
        let expected: Vec<f64> = (0..16)
            .map(|i| {
                matrix[i].as_f64().unwrap()
                    + if (12..15).contains(&i) {
                        step[i - 12]
                    } else {
                        0.
                    }
            })
            .collect();
        assert_eq!(doc["nodes"][index]["matrix"], json!(expected));
        raw_doc["nodes"][index]["matrix"] = Value::Null;
        doc["nodes"][index]["matrix"] = Value::Null;
    } else {
        let t: Vec<f64> = (0..3)
            .map(|i| raw_node["translation"][i].as_f64().unwrap_or(0.) + step[i])
            .collect();
        assert_eq!(doc["nodes"][index]["translation"], json!(t));
        raw_doc["nodes"][index]["translation"] = Value::Null;
        doc["nodes"][index]["translation"] = Value::Null;
    }
    assert_eq!(doc, raw_doc, "every other JSON value is unchanged");
    (published, record)
}

#[test]
fn frozen_production_payloads_restore_byte_exactly() {
    for payload in [&DEFAULT, &QUANTIZE, &MESHOPT, &QUANTIZE_MESHOPT] {
        let raw = decode(payload);
        let stem = payload
            .name
            .trim_start_matches("t/")
            .trim_end_matches(".glb");
        let digest: String = Sha256::digest(&raw)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(digest, stem, "the frozen copy is the immutable raw member");
        assert_exact(&raw, payload.delta);
    }
    // Meshopt raw JSON keeps unsorted keys: a whole-document re-serialization is not
    // invertible, the splice is.
    for payload in [&MESHOPT, &QUANTIZE_MESHOPT] {
        let (json, _, _) = glb_parts(&decode(payload));
        let resorted =
            serde_json::to_vec(&serde_json::from_slice::<Value>(&json).unwrap()).unwrap();
        assert_ne!(resorted, json, "{}", payload.name);
    }
    // Quantized roots carry a translation, and naive un-baking in f64 misses for these
    // payloads: the recorded text is what makes the restore exact.
    for payload in [&QUANTIZE, &QUANTIZE_MESHOPT] {
        let (json, _, _) = glb_parts(&decode(payload));
        let doc: Value = serde_json::from_slice(&json).unwrap();
        let t: Vec<f64> = root_node(&doc)["translation"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let d = payload.delta;
        let step = [d[0], d[2], -d[1]];
        assert!(
            (0..3).any(|i| (t[i] + step[i]) - step[i] != t[i]),
            "{}",
            payload.name
        );
    }
}

#[test]
fn rebase_covers_b3dm_alignment_matrix_roots_and_zero_translation() {
    let raw = decode(&DEFAULT);
    // b3dm-wrapped GLB: the b3dm byteLength follows, 8-byte alignment is kept.
    let wrapped = b3dm(&raw);
    let (published, _) = assert_exact(&wrapped, DEFAULT.delta);
    assert_eq!(
        u32::from_le_bytes(published[8..12].try_into().unwrap()) as usize,
        published.len()
    );
    assert_eq!(published.len() % 8, 0);
    // A JSON chunk aligned to 8 (vector's raw chunks are aligned to 4).
    let doc = json!({"asset":{"version":"2.0"},"scenes":[{"nodes":[1]}],"nodes":[{"name":"child"},{"children":[0],"translation":[1.,2.,3.]}]});
    let mut aligned = glb(&doc, &[1, 2, 3, 4]);
    let json_length = u32::from_le_bytes(aligned[12..16].try_into().unwrap()) as usize;
    if !json_length.is_multiple_of(8) {
        aligned.splice(20 + json_length..20 + json_length, *b"    ");
        aligned[12..16].copy_from_slice(&((json_length + 4) as u32).to_le_bytes());
        let total = aligned.len() as u32;
        aligned[8..12].copy_from_slice(&total.to_le_bytes());
    }
    assert_eq!(glb_parts(&aligned).1 % 8, 0);
    assert_exact(&aligned, [0.1, 0.2, 0.3]);
    // An affine matrix root: δ goes into its translation column.
    let doc = json!({"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],
        "nodes":[{"matrix":[0.,0.,-1.,0., 0.,1.,0.,0., 1.,0.,0.,0., 5.,6.,7.,1.]}]});
    assert_exact(&glb(&doc, &[]), [1., 2., 3.]);
    // δ = 0 is a byte copy with an empty record, for any payload.
    for bytes in [raw.clone(), b"{\"asset\":{}}".to_vec()] {
        let (copy, record) = rebase_payload(&bytes, [0.; 3]).unwrap();
        assert_eq!(copy, bytes);
        assert_eq!(
            (record.raw_root_node.as_str(), record.raw_json_length),
            ("", 0)
        );
        assert_eq!(restore_payload(&copy, &record).unwrap(), bytes);
    }
}

fn rebase_error(result: Result<(Vec<u8>, RebaseRecord), Error>) -> String {
    match result {
        Err(Error::Data(message)) => message,
        other => panic!("expected a typed error, got {:?}", other.map(|o| o.1)),
    }
}

fn restore_error(result: Result<Vec<u8>, Error>) -> String {
    match result {
        Err(Error::Data(message)) => {
            assert!(
                message.starts_with("invalid previous payload record: "),
                "{message}"
            );
            message
        }
        other => panic!("expected a typed error, got {:?}", other.map(|o| o.len())),
    }
}

#[test]
fn unsupported_payloads_and_malformed_records_are_typed_errors() {
    let raw = decode(&QUANTIZE);
    let delta = QUANTIZE.delta;
    // Rebase refuses what it cannot invert exactly.
    let two_roots = glb(
        &json!({"asset":{"version":"2.0"},"scenes":[{"nodes":[0]},{"nodes":[1]}],"nodes":[{},{}]}),
        &[],
    );
    assert!(rebase_error(rebase_payload(&two_roots, delta)).contains("2 scene root nodes"));
    let both = glb(
        &json!({"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],"nodes":[{"matrix":[1.,0.,0.,0., 0.,1.,0.,0., 0.,0.,1.,0., 0.,0.,0.,1.],"scale":[1.,1.,1.]}]}),
        &[],
    );
    assert!(rebase_error(rebase_payload(&both, delta)).contains("both matrix and TRS"));
    assert!(rebase_error(rebase_payload(b"{\"asset\":{}}", delta)).contains("neither GLB nor b3dm"));
    assert!(rebase_error(rebase_payload(&raw, [f64::NAN, 0., 0.])).contains("not finite"));
    let mut truncated = raw.clone();
    truncated.truncate(100);
    assert!(rebase_error(rebase_payload(&truncated, delta)).contains("length"));

    let (published, record) = rebase_payload(&raw, delta).unwrap();
    let digest = |bytes: &[u8]| <[u8; 32]>::from(Sha256::digest(bytes));
    // An altered published member or a stale record restores to bytes whose digest differs
    // ("previous content checksum failed" in the caller).
    let mut altered = published.clone();
    *altered.last_mut().unwrap() ^= 1;
    assert_ne!(
        digest(&restore_payload(&altered, &record).unwrap()),
        record.raw_sha256
    );
    let stale = RebaseRecord {
        raw_root_node: record
            .raw_root_node
            .replace("641.006591796875", "641.006591796874"),
        ..record.clone()
    };
    assert_ne!(stale.raw_root_node, record.raw_root_node);
    assert_ne!(
        digest(&restore_payload(&published, &stale).unwrap()),
        record.raw_sha256
    );
    // Records that cannot describe the published payload are typed errors (D-4).
    let short = RebaseRecord {
        raw_json_length: record.raw_json_length - 64,
        ..record.clone()
    };
    assert!(restore_error(restore_payload(&published, &short)).contains("shorter than"));
    // A recorded length beyond alignment padding is refused before it is allocated; the
    // largest u32 would otherwise ask for 4 GiB.
    for extra in [8, u32::MAX - record.raw_json_length] {
        let long = RebaseRecord {
            raw_json_length: record.raw_json_length + extra,
            ..record.clone()
        };
        assert!(
            restore_error(restore_payload(&published, &long)).contains("more than alignment"),
            "{extra}"
        );
    }
    let copy_with_text = RebaseRecord {
        delta: [0.; 3],
        ..record.clone()
    };
    restore_error(restore_payload(&published, &copy_with_text));
    let no_text = RebaseRecord {
        raw_root_node: String::new(),
        ..record.clone()
    };
    restore_error(restore_payload(&published, &no_text));
    let mut unparsable = published.clone();
    unparsable[20] = b'[';
    assert!(restore_error(restore_payload(&unparsable, &record)).contains("cannot be read"));
    assert!(restore_error(restore_payload(&two_roots, &record)).contains("scene root nodes"));
    restore_error(restore_payload(b"not a payload", &record));
}

// Frozen copies of `/tmp/rusty-tiles-pimpl-design-20261011/evidence/v3` production payloads
// (develop-equivalent binary `369c3c81…`, perf-lane vector input; `conversions.json`). Each is
// the smallest raw payload of its archive with a nonzero δ, or, for the quantized archives, the
// smallest whose naive f64 un-bake misses.

/// `prev-perf-default.3tz` `t/d05846b93cfd4afb1e6c5f8b772780dc6442f79747c483becdfcaf3afbfacebc.glb` (evidence/v3 conversions), δ from `vector-build.json` `explicitRoot`.
const DEFAULT: Payload = Payload {
    name: "t/d05846b93cfd4afb1e6c5f8b772780dc6442f79747c483becdfcaf3afbfacebc.glb",
    delta: [4662.092502250245, 5363.886543299424, -8.107066483069048],
    base64: concat!(
        "Z2xURgIAAACwIQAAjA0AAEpTT057ImFjY2Vzc29ycyI6W3siYnVmZmVyVmlldyI6MTEsImNvbXBvbmVudFR5cGUiOjUxMjYs",
        "ImNvdW50IjoyOSwibWF4IjpbMzAwLjc0MjMwOTU3MDMxMjUsMS4wMjI5MDgwOTE1NDUxMDUsNDAyLjQzOTcyNzc4MzIwMzFd",
        "LCJtaW4iOlstMzAwLjc0MjMwOTU3MDMxMjUsLTEuMDIyOTA4MDkxNTQ1MTA1LC00NTUuOTUwNTYxNTIzNDM3NV0sInR5cGUi",
        "OiJWRUMzIn0seyJidWZmZXJWaWV3IjoxMiwiY29tcG9uZW50VHlwZSI6NTEyMywiY291bnQiOjI5LCJ0eXBlIjoiU0NBTEFS",
        "In0seyJidWZmZXJWaWV3IjoxMywiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjI5LCJ0eXBlIjoiU0NBTEFSIn0seyJi",
        "dWZmZXJWaWV3IjoxNCwiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjEwLCJ0eXBlIjoiU0NBTEFSIn0seyJidWZmZXJW",
        "aWV3IjoxNSwiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjY0LCJ0eXBlIjoiU0NBTEFSIn0seyJidWZmZXJWaWV3Ijox",
        "NiwiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjEwLCJ0eXBlIjoiU0NBTEFSIn0seyJidWZmZXJWaWV3IjoxNywiY29t",
        "cG9uZW50VHlwZSI6NTEyNiwiY291bnQiOjU0LCJtYXgiOlsyMTkuODA2MTM3MDg0OTYwOTQsMC45MTIyMDU0NTc2ODczNzc5",
        "LDQ1NS45NTA1NjE1MjM0Mzc1XSwibWluIjpbLTE5Mi4xODcyODYzNzY5NTMxMiwtMC43MjMxMjM4NDg0MzgyNjI5LC0zNDIu",
        "NTIxMzMxNzg3MTA5NF0sInR5cGUiOiJWRUMzIn0seyJidWZmZXJWaWV3IjoxOCwiY29tcG9uZW50VHlwZSI6NTEyMywiY291",
        "bnQiOjU0LCJ0eXBlIjoiU0NBTEFSIn0seyJidWZmZXJWaWV3IjoxOSwiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjEw",
        "OCwidHlwZSI6IlNDQUxBUiJ9XSwiYXNzZXQiOnsiZ2VuZXJhdG9yIjoicnVzdHktdGlsZXMgbmF0aXZlIHZlY3RvciIsInZl",
        "cnNpb24iOiIyLjAifSwiYnVmZmVyVmlld3MiOlt7ImJ1ZmZlciI6MCwiYnl0ZUxlbmd0aCI6MTU2LCJieXRlT2Zmc2V0Ijow",
        "fSx7ImJ1ZmZlciI6MCwiYnl0ZUxlbmd0aCI6MTYwLCJieXRlT2Zmc2V0IjoxNjB9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3Ro",
        "IjoyMzQsImJ5dGVPZmZzZXQiOjMyMH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjE2MCwiYnl0ZU9mZnNldCI6NTYwfSx7",
        "ImJ1ZmZlciI6MCwiYnl0ZUxlbmd0aCI6MzEyLCJieXRlT2Zmc2V0Ijo3MjB9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjoz",
        "MTIsImJ5dGVPZmZzZXQiOjEwMzJ9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjozMTIsImJ5dGVPZmZzZXQiOjEzNDR9LHsi",
        "YnVmZmVyIjowLCJieXRlTGVuZ3RoIjoxOTYsImJ5dGVPZmZzZXQiOjE2NTZ9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjox",
        "NjAsImJ5dGVPZmZzZXQiOjE4NTZ9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjo3MjcsImJ5dGVPZmZzZXQiOjIwMTZ9LHsi",
        "YnVmZmVyIjowLCJieXRlTGVuZ3RoIjoxNjAsImJ5dGVPZmZzZXQiOjI3NDR9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjoz",
        "NDgsImJ5dGVPZmZzZXQiOjI5MDR9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjoxMTYsImJ5dGVPZmZzZXQiOjMyNTYsImJ5",
        "dGVTdHJpZGUiOjR9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjoxMTYsImJ5dGVPZmZzZXQiOjMzNzZ9LHsiYnVmZmVyIjow",
        "LCJieXRlTGVuZ3RoIjo0MCwiYnl0ZU9mZnNldCI6MzQ5Nn0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjI1NiwiYnl0ZU9m",
        "ZnNldCI6MzUzNn0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjQwLCJieXRlT2Zmc2V0IjozNzkyfSx7ImJ1ZmZlciI6MCwi",
        "Ynl0ZUxlbmd0aCI6NjQ4LCJieXRlT2Zmc2V0IjozODMyfSx7ImJ1ZmZlciI6MCwiYnl0ZUxlbmd0aCI6MjE2LCJieXRlT2Zm",
        "c2V0Ijo0NDgwLCJieXRlU3RyaWRlIjo0fSx7ImJ1ZmZlciI6MCwiYnl0ZUxlbmd0aCI6NDMyLCJieXRlT2Zmc2V0Ijo0Njk2",
        "fV0sImJ1ZmZlcnMiOlt7ImJ5dGVMZW5ndGgiOjUxMjh9XSwiZXh0ZW5zaW9ucyI6eyJFWFRfc3RydWN0dXJhbF9tZXRhZGF0",
        "YSI6eyJwcm9wZXJ0eVRhYmxlcyI6W3siY2xhc3MiOiJmZWF0dXJlIiwiY291bnQiOjM5LCJuYW1lIjoiZmVhdHVyZXMiLCJw",
        "cm9wZXJ0aWVzIjp7Il9zb3VyY2VfaWQiOnsic3RyaW5nT2Zmc2V0VHlwZSI6IlVJTlQzMiIsInN0cmluZ09mZnNldHMiOjEs",
        "InZhbHVlcyI6MH0sIl9zb3VyY2VfbGF5ZXIiOnsic3RyaW5nT2Zmc2V0VHlwZSI6IlVJTlQzMiIsInN0cmluZ09mZnNldHMi",
        "OjMsInZhbHVlcyI6Mn0sImdpcnRoIjp7InZhbHVlcyI6NH0sImhlaWdodCI6eyJ2YWx1ZXMiOjV9LCJpZCI6eyJ2YWx1ZXMi",
        "OjZ9LCJraW5kIjp7InN0cmluZ09mZnNldFR5cGUiOiJVSU5UMzIiLCJzdHJpbmdPZmZzZXRzIjo4LCJ2YWx1ZXMiOjd9LCJu",
        "YW1lIjp7InN0cmluZ09mZnNldFR5cGUiOiJVSU5UMzIiLCJzdHJpbmdPZmZzZXRzIjoxMCwidmFsdWVzIjo5fX19XSwic2No",
        "ZW1hIjp7ImNsYXNzZXMiOnsiZmVhdHVyZSI6eyJwcm9wZXJ0aWVzIjp7Il9zb3VyY2VfaWQiOnsidHlwZSI6IlNUUklORyJ9",
        "LCJfc291cmNlX2xheWVyIjp7InR5cGUiOiJTVFJJTkcifSwiZ2lydGgiOnsiY29tcG9uZW50VHlwZSI6IkZMT0FUNjQiLCJu",
        "b0RhdGEiOi0xLjc5NzY5MzEzNDg2MjMxNTdlKzMwOCwidHlwZSI6IlNDQUxBUiJ9LCJoZWlnaHQiOnsiY29tcG9uZW50VHlw",
        "ZSI6IklOVDY0Iiwibm9EYXRhIjotOTAwNzE5OTI1NDc0MDk5MiwidHlwZSI6IlNDQUxBUiJ9LCJpZCI6eyJjb21wb25lbnRU",
        "eXBlIjoiSU5UNjQiLCJ0eXBlIjoiU0NBTEFSIn0sImtpbmQiOnsidHlwZSI6IlNUUklORyJ9LCJuYW1lIjp7Im5vRGF0YSI6",
        "Il9fUlVTVFlfVElMRVNfTUlTU0lOR19fIiwidHlwZSI6IlNUUklORyJ9fX19LCJpZCI6InJ1c3R5X3RpbGVzX3ZlY3RvciJ9",
        "fX0sImV4dGVuc2lvbnNVc2VkIjpbIkVYVF9tZXNoX2ZlYXR1cmVzIiwiRVhUX3N0cnVjdHVyYWxfbWV0YWRhdGEiLCJFWFRf",
        "bWVzaF9wb2x5Z29uIl0sIm1lc2hlcyI6W3sicHJpbWl0aXZlcyI6W3siYXR0cmlidXRlcyI6eyJQT1NJVElPTiI6MCwiX0ZF",
        "QVRVUkVfSURfMCI6MX0sImV4dGVuc2lvbnMiOnsiRVhUX21lc2hfZmVhdHVyZXMiOnsiZmVhdHVyZUlkcyI6W3siYXR0cmli",
        "dXRlIjowLCJmZWF0dXJlQ291bnQiOjM5LCJwcm9wZXJ0eVRhYmxlIjowfV19fSwiaW5kaWNlcyI6MiwibW9kZSI6MH0seyJh",
        "dHRyaWJ1dGVzIjp7IlBPU0lUSU9OIjo2LCJfRkVBVFVSRV9JRF8wIjo3fSwiZXh0ZW5zaW9ucyI6eyJFWFRfbWVzaF9mZWF0",
        "dXJlcyI6eyJmZWF0dXJlSWRzIjpbeyJhdHRyaWJ1dGUiOjAsImZlYXR1cmVDb3VudCI6MzksInByb3BlcnR5VGFibGUiOjB9",
        "XX0sIkVYVF9tZXNoX3BvbHlnb24iOnsiY291bnQiOjEwLCJpbmRpY2VzT2Zmc2V0cyI6MywibG9vcEluZGljZXMiOjQsImxv",
        "b3BJbmRpY2VzT2Zmc2V0cyI6NX19LCJpbmRpY2VzIjo4LCJtb2RlIjo0fV19XSwibm9kZXMiOlt7Im1lc2giOjB9XSwic2Nl",
        "bmUiOjAsInNjZW5lcyI6W3sibm9kZXMiOlswXX1dfSAIFAAAQklOADMxNzkzMjM3MzIzODMyMzkzMjk3MzI5ODMyOTkzMzU3",
        "MzM1ODMzNTk0MDk1NDI1NTQ1MTA0NzU2NDk5OTUwNTE1MDU3NTMzMjU4NzA2MDEyNjQzMDY2MTk2NjgwNjY5MTY4OTM3MTk5",
        "NzI3MTc3Mzg3ODk3Nzk2Njc5ODU4MjA3ODM1Njg3NjE5MzkyOTU2NTk1OTc5NzYyOTg4NgAAAAAAAAAABAAAAAgAAAAMAAAA",
        "EAAAABQAAAAYAAAAHAAAACAAAAAkAAAAKAAAACwAAAAwAAAANAAAADgAAAA8AAAAQAAAAEQAAABIAAAATAAAAFAAAABUAAAA",
        "WAAAAFwAAABgAAAAZAAAAGgAAABsAAAAcAAAAHQAAAB4AAAAfAAAAIAAAACEAAAAiAAAAIwAAACQAAAAlAAAAJgAAACcAAAA",
        "dmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9y",
        "dmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9y",
        "dmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9y",
        "dmVjdG9ydmVjdG9ydmVjdG9yAAAAAAAAAAAAAAYAAAAMAAAAEgAAABgAAAAeAAAAJAAAACoAAAAwAAAANgAAADwAAABCAAAA",
        "SAAAAE4AAABUAAAAWgAAAGAAAABmAAAAbAAAAHIAAAB4AAAAfgAAAIQAAACKAAAAkAAAAJYAAACcAAAAogAAAKgAAACuAAAA",
        "tAAAALoAAADAAAAAxgAAAMwAAADSAAAA2AAAAN4AAADkAAAA6gAAAP///////+//////////7//////////v/////////+//",
        "////////7//////////v/////////+//////////7//////////v/////////+//PQrXo3A94j8K16NwPQq3P65H4XoUrvc/",
        "KVyPwvUo/D/hehSuR+EGQK5H4XoUrtc/hetRuB6F+z9cj8L1KFwHQOF6FK5H4fY/H4XrUbge8T+PwvUoXI8GQOxRuB6F6wdA",
        "j8L1KFyP/j/Xo3A9Ctf3PwrXo3A9Cvc/hetRuB6FB0DhehSuR+HqP5qZmZmZmQFAH4XrUbgeA0Bcj8L1KFwBQFyPwvUoXAVA",
        "zczMzMzM9D8zMzMzMzPzP2ZmZmZmZu4/w/UoXI/C8T9xPQrXo3AHQAAAAAAAAOg/exSuR+F69D8K16NwPQoBQBYAAAAAAAAA",
        "KAAAAAAAAAAQAAAAAAAAABMAAAAAAAAAGQAAAAAAAAAgAAAAAAAAACYAAAAAAAAAEgAAAAAAAAAoAAAAAAAAAAsAAAAAAAAA",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/2sMAAAAAAAApQwAAAAAAACmDAAAAAAAAKcMAAAAAAAA4QwAAAAAAADiDAAAAAAAAOMMAAAAAAAA",
        "HQ0AAAAAAAAeDQAAAAAAAB8NAAAAAAAA/w8AAAAAAACfEAAAAAAAAJ4RAAAAAAAAlBIAAAAAAACHEwAAAAAAALsTAAAAAAAA",
        "wRMAAAAAAADUFAAAAAAAAO4WAAAAAAAAfBcAAAAAAAAeGQAAAAAAANsZAAAAAAAAGBoAAAAAAAAjGgAAAAAAAO0aAAAAAAAA",
        "HxwAAAAAAABnHAAAAAAAADoeAAAAAAAA2R4AAAAAAAAeHwAAAAAAADEfAAAAAAAADyAAAAAAAACkIAAAAAAAADkiAAAAAAAA",
        "sCQAAAAAAABdJQAAAAAAAH0lAAAAAAAAIiYAAAAAAACeJgAAAAAAAGJ1aWxkaW5nYnVpbGRpbmdidWlsZGluZ2J1aWxkaW5n",
        "YnVpbGRpbmdidWlsZGluZ2J1aWxkaW5nYnVpbGRpbmdidWlsZGluZ2J1aWxkaW5ndHJlZXRyZWV0cmVldHJlZXRyZWV0cmVl",
        "dHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVl",
        "dHJlZXRyZWV0cmVldHJlZXRyZWUAAAAAAAAAAAgAAAAQAAAAGAAAACAAAAAoAAAAMAAAADgAAABAAAAASAAAAFAAAABUAAAA",
        "WAAAAFwAAABgAAAAZAAAAGgAAABsAAAAcAAAAHQAAAB4AAAAfAAAAIAAAACEAAAAiAAAAIwAAACQAAAAlAAAAJgAAACcAAAA",
        "oAAAAKQAAACoAAAArAAAALAAAAC0AAAAuAAAALwAAADAAAAAxAAAAGI1Mi01OWI1My01N2I1My01OGI1My01OWI1NC01N2I1",
        "NC01OGI1NC01OWI1NS01N2I1NS01OGI1NS01OV9fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5H",
        "X19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19f",
        "X1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JV",
        "U1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZ",
        "X1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJ",
        "TEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVT",
        "X01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01J",
        "U1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJ",
        "TkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdf",
        "X19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX18AAAAAAAYAAAAMAAAAEgAAABgAAAAeAAAA",
        "JAAAACoAAAAwAAAANgAAADwAAABTAAAAagAAAIEAAACYAAAArwAAAMYAAADdAAAA9AAAAAsBAAAiAQAAOQEAAFABAABnAQAA",
        "fgEAAJUBAACsAQAAwwEAANoBAADxAQAACAIAAB8CAAA2AgAATQIAAGQCAAB7AgAAkgIAAKkCAADAAgAA1wIAANhEjcMxTUA/",
        "q7+XQoKgdMM3uYE+5Ykdwyu4WkOn7oK/rPnjwyv3KcMpZoi+mOS4w7CWAsNCmgA/O5l7Qin8FENEMQG+PFI5wioghsOEdiY/",
        "ZUIFQi/agMPdGEc/QP7aQgTAtsE5V5O+F62Fw+osaUMdcWK/PdW6w9H60kJb4hi/PV2jwzBXJ8L4hn8/STjJQwHKZcM5Y4K+",
        "kpHMw10WwUJyo+o+R8JeQ1xTdcG+5787Dp3YwtKtGMPky2q+Ub6owwRflsPTFhc/hN69wbmlqsKEoUw+iD92wgbzJENoHGW/",
        "MHHYw6Opj8On7oI/w9RZQ8v5yEIidcq+eDNiw6APlMPwVZ8+C80mwwRflkOrRH8+VfOLQ7Dwh8P3ASS+3zXEwyQJHcO6fGM/",
        "mRd5Q+QihsKpekU/oiaBQ2pNxkKsYlk9yp0CQT99gUPOboG/vBDRw+3xhUMi5MM+iUyiQwAAAAAKAAAACwAAAAwAAAANAAAA",
        "DgAAAA8AAAAQAAAAEQAAABIAAAATAAAAFAAAABUAAAAWAAAAFwAAABgAAAAZAAAAGgAAABsAAAAcAAAAHQAAAB4AAAAfAAAA",
        "IAAAACEAAAAiAAAAIwAAACQAAAAlAAAAJgAAAAAAAAAAAAAAAQAAAAIAAAADAAAABAAAAAUAAAAGAAAABwAAAAgAAAAJAAAA",
        "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",
    ),
};

/// `perf-quant.3tz` `t/b5e01963cdeb225f2695d450ef351fe22fbae647095ca0b19e65f5638e3f1539.glb` (evidence/v3 conversions), δ from `vector-build.json` `explicitRoot`.
const QUANTIZE: Payload = Payload {
    name: "t/b5e01963cdeb225f2695d450ef351fe22fbae647095ca0b19e65f5638e3f1539.glb",
    delta: [-3997.0360376601798, 4574.8828832519575, 0.6623462713405388],
    base64: concat!(
        "Z2xURgIAAAAYIgAA9A0AAEpTT057ImFjY2Vzc29ycyI6W3siYnVmZmVyVmlldyI6MTEsImNvbXBvbmVudFR5cGUiOjUxMjMs",
        "ImNvdW50IjoyNywibWF4IjpbNjU1MzUsNjU1MzUsNjU1MzVdLCJtaW4iOls2NTI5LDkzNDksNzM0OF0sIm5vcm1hbGl6ZWQi",
        "OnRydWUsInR5cGUiOiJWRUMzIn0seyJidWZmZXJWaWV3IjoxMiwiY29tcG9uZW50VHlwZSI6NTEyMywiY291bnQiOjI3LCJ0",
        "eXBlIjoiU0NBTEFSIn0seyJidWZmZXJWaWV3IjoxMywiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjI3LCJ0eXBlIjoi",
        "U0NBTEFSIn0seyJidWZmZXJWaWV3IjoxNCwiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjEyLCJ0eXBlIjoiU0NBTEFS",
        "In0seyJidWZmZXJWaWV3IjoxNSwiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjgxLCJ0eXBlIjoiU0NBTEFSIn0seyJi",
        "dWZmZXJWaWV3IjoxNiwiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjEyLCJ0eXBlIjoiU0NBTEFSIn0seyJidWZmZXJW",
        "aWV3IjoxNywiY29tcG9uZW50VHlwZSI6NTEyMywiY291bnQiOjY4LCJtYXgiOls1OTMwMyw2MDk4NCw1NzUwOF0sIm1pbiI6",
        "WzAsMCwwXSwibm9ybWFsaXplZCI6dHJ1ZSwidHlwZSI6IlZFQzMifSx7ImJ1ZmZlclZpZXciOjE4LCJjb21wb25lbnRUeXBl",
        "Ijo1MTIzLCJjb3VudCI6NjgsInR5cGUiOiJTQ0FMQVIifSx7ImJ1ZmZlclZpZXciOjE5LCJjb21wb25lbnRUeXBlIjo1MTI1",
        "LCJjb3VudCI6MTQ0LCJ0eXBlIjoiU0NBTEFSIn1dLCJhc3NldCI6eyJnZW5lcmF0b3IiOiJydXN0eS10aWxlcyBuYXRpdmUg",
        "dmVjdG9yIiwidmVyc2lvbiI6IjIuMCJ9LCJidWZmZXJWaWV3cyI6W3siYnVmZmVyIjowLCJieXRlTGVuZ3RoIjoxNTYsImJ5",
        "dGVPZmZzZXQiOjB9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjoxNjAsImJ5dGVPZmZzZXQiOjE2MH0seyJidWZmZXIiOjAs",
        "ImJ5dGVMZW5ndGgiOjIzNCwiYnl0ZU9mZnNldCI6MzIwfSx7ImJ1ZmZlciI6MCwiYnl0ZUxlbmd0aCI6MTYwLCJieXRlT2Zm",
        "c2V0Ijo1NjB9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjozMTIsImJ5dGVPZmZzZXQiOjcyMH0seyJidWZmZXIiOjAsImJ5",
        "dGVMZW5ndGgiOjMxMiwiYnl0ZU9mZnNldCI6MTAzMn0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjMxMiwiYnl0ZU9mZnNl",
        "dCI6MTM0NH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjIwNCwiYnl0ZU9mZnNldCI6MTY1Nn0seyJidWZmZXIiOjAsImJ5",
        "dGVMZW5ndGgiOjE2MCwiYnl0ZU9mZnNldCI6MTg2NH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjY4MSwiYnl0ZU9mZnNl",
        "dCI6MjAyNH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjE2MCwiYnl0ZU9mZnNldCI6MjcxMn0seyJidWZmZXIiOjAsImJ5",
        "dGVMZW5ndGgiOjIxNiwiYnl0ZU9mZnNldCI6Mjg3MiwiYnl0ZVN0cmlkZSI6OH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgi",
        "OjEwOCwiYnl0ZU9mZnNldCI6MzA4OCwiYnl0ZVN0cmlkZSI6NH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjEwOCwiYnl0",
        "ZU9mZnNldCI6MzIwMH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjQ4LCJieXRlT2Zmc2V0IjozMzEyfSx7ImJ1ZmZlciI6",
        "MCwiYnl0ZUxlbmd0aCI6MzI0LCJieXRlT2Zmc2V0IjozMzYwfSx7ImJ1ZmZlciI6MCwiYnl0ZUxlbmd0aCI6NDgsImJ5dGVP",
        "ZmZzZXQiOjM2ODh9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjo1NDQsImJ5dGVPZmZzZXQiOjM3MzYsImJ5dGVTdHJpZGUi",
        "Ojh9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjoyNzIsImJ5dGVPZmZzZXQiOjQyODAsImJ5dGVTdHJpZGUiOjR9LHsiYnVm",
        "ZmVyIjowLCJieXRlTGVuZ3RoIjo1NzYsImJ5dGVPZmZzZXQiOjQ1NTJ9XSwiYnVmZmVycyI6W3siYnl0ZUxlbmd0aCI6NTEy",
        "OH1dLCJleHRlbnNpb25zIjp7IkVYVF9zdHJ1Y3R1cmFsX21ldGFkYXRhIjp7InByb3BlcnR5VGFibGVzIjpbeyJjbGFzcyI6",
        "ImZlYXR1cmUiLCJjb3VudCI6MzksIm5hbWUiOiJmZWF0dXJlcyIsInByb3BlcnRpZXMiOnsiX3NvdXJjZV9pZCI6eyJzdHJp",
        "bmdPZmZzZXRUeXBlIjoiVUlOVDMyIiwic3RyaW5nT2Zmc2V0cyI6MSwidmFsdWVzIjowfSwiX3NvdXJjZV9sYXllciI6eyJz",
        "dHJpbmdPZmZzZXRUeXBlIjoiVUlOVDMyIiwic3RyaW5nT2Zmc2V0cyI6MywidmFsdWVzIjoyfSwiZ2lydGgiOnsidmFsdWVz",
        "Ijo0fSwiaGVpZ2h0Ijp7InZhbHVlcyI6NX0sImlkIjp7InZhbHVlcyI6Nn0sImtpbmQiOnsic3RyaW5nT2Zmc2V0VHlwZSI6",
        "IlVJTlQzMiIsInN0cmluZ09mZnNldHMiOjgsInZhbHVlcyI6N30sIm5hbWUiOnsic3RyaW5nT2Zmc2V0VHlwZSI6IlVJTlQz",
        "MiIsInN0cmluZ09mZnNldHMiOjEwLCJ2YWx1ZXMiOjl9fX1dLCJzY2hlbWEiOnsiY2xhc3NlcyI6eyJmZWF0dXJlIjp7InBy",
        "b3BlcnRpZXMiOnsiX3NvdXJjZV9pZCI6eyJ0eXBlIjoiU1RSSU5HIn0sIl9zb3VyY2VfbGF5ZXIiOnsidHlwZSI6IlNUUklO",
        "RyJ9LCJnaXJ0aCI6eyJjb21wb25lbnRUeXBlIjoiRkxPQVQ2NCIsIm5vRGF0YSI6LTEuNzk3NjkzMTM0ODYyMzE1N2UrMzA4",
        "LCJ0eXBlIjoiU0NBTEFSIn0sImhlaWdodCI6eyJjb21wb25lbnRUeXBlIjoiSU5UNjQiLCJub0RhdGEiOi05MDA3MTk5MjU0",
        "NzQwOTkyLCJ0eXBlIjoiU0NBTEFSIn0sImlkIjp7ImNvbXBvbmVudFR5cGUiOiJJTlQ2NCIsInR5cGUiOiJTQ0FMQVIifSwi",
        "a2luZCI6eyJ0eXBlIjoiU1RSSU5HIn0sIm5hbWUiOnsibm9EYXRhIjoiX19SVVNUWV9USUxFU19NSVNTSU5HX18iLCJ0eXBl",
        "IjoiU1RSSU5HIn19fX0sImlkIjoicnVzdHlfdGlsZXNfdmVjdG9yIn19fSwiZXh0ZW5zaW9uc1JlcXVpcmVkIjpbIktIUl9t",
        "ZXNoX3F1YW50aXphdGlvbiJdLCJleHRlbnNpb25zVXNlZCI6WyJFWFRfbWVzaF9mZWF0dXJlcyIsIkVYVF9zdHJ1Y3R1cmFs",
        "X21ldGFkYXRhIiwiRVhUX21lc2hfcG9seWdvbiIsIktIUl9tZXNoX3F1YW50aXphdGlvbiJdLCJtZXNoZXMiOlt7InByaW1p",
        "dGl2ZXMiOlt7ImF0dHJpYnV0ZXMiOnsiUE9TSVRJT04iOjAsIl9GRUFUVVJFX0lEXzAiOjF9LCJleHRlbnNpb25zIjp7IkVY",
        "VF9tZXNoX2ZlYXR1cmVzIjp7ImZlYXR1cmVJZHMiOlt7ImF0dHJpYnV0ZSI6MCwiZmVhdHVyZUNvdW50IjozOSwicHJvcGVy",
        "dHlUYWJsZSI6MH1dfX0sImluZGljZXMiOjIsIm1vZGUiOjB9LHsiYXR0cmlidXRlcyI6eyJQT1NJVElPTiI6NiwiX0ZFQVRV",
        "UkVfSURfMCI6N30sImV4dGVuc2lvbnMiOnsiRVhUX21lc2hfZmVhdHVyZXMiOnsiZmVhdHVyZUlkcyI6W3siYXR0cmlidXRl",
        "IjowLCJmZWF0dXJlQ291bnQiOjM5LCJwcm9wZXJ0eVRhYmxlIjowfV19LCJFWFRfbWVzaF9wb2x5Z29uIjp7ImNvdW50Ijox",
        "MiwiaW5kaWNlc09mZnNldHMiOjMsImxvb3BJbmRpY2VzIjo0LCJsb29wSW5kaWNlc09mZnNldHMiOjV9fSwiaW5kaWNlcyI6",
        "OCwibW9kZSI6NH1dfV0sIm5vZGVzIjpbeyJtZXNoIjowLCJzY2FsZSI6WzY0MS4wMDY1OTE3OTY4NzUsMS41NjcyNDIyNjQ3",
        "NDc2MTk2LDkwMC43NzU4Nzg5MDYyNV0sInRyYW5zbGF0aW9uIjpbLTMyMC41MDMyOTU4OTg0Mzc1LC0wLjc4MzYyMTEzMjM3",
        "MzgwOTgsLTQ1MC4zODc5Mzk0NTMxMjVdfV0sInNjZW5lIjowLCJzY2VuZXMiOlt7Im5vZGVzIjpbMF19XX0gIAgUAABCSU4A",
        "Mjk0NTI5NDYyOTQ3MzAwNDMwMDUzMDA2MzAwNzMwNjUzMDY2MzA2NzMxMjUzMTI3NDgxODQ4ODQ1MTIzNTIyNDUzODA1NDE0",
        "NTQ4ODYyODA2Mzc1NjYyNTY3MzQ3MDA5NzAzMTcyMjg3MjY2NzUxODc2NDI4MDIxODM1MDg0MjU4NzA4OTEyODkxNDA5NjEx",
        "OTc4ODk4MDM5ODI5AAAAAAAAAAAEAAAACAAAAAwAAAAQAAAAFAAAABgAAAAcAAAAIAAAACQAAAAoAAAALAAAADAAAAA0AAAA",
        "OAAAADwAAABAAAAARAAAAEgAAABMAAAAUAAAAFQAAABYAAAAXAAAAGAAAABkAAAAaAAAAGwAAABwAAAAdAAAAHgAAAB8AAAA",
        "gAAAAIQAAACIAAAAjAAAAJAAAACUAAAAmAAAAJwAAAB2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0",
        "b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0",
        "b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0",
        "b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3IAAAAAAAAAAAAABgAAAAwAAAASAAAA",
        "GAAAAB4AAAAkAAAAKgAAADAAAAA2AAAAPAAAAEIAAABIAAAATgAAAFQAAABaAAAAYAAAAGYAAABsAAAAcgAAAHgAAAB+AAAA",
        "hAAAAIoAAACQAAAAlgAAAJwAAACiAAAAqAAAAK4AAAC0AAAAugAAAMAAAADGAAAAzAAAANIAAADYAAAA3gAAAOQAAADqAAAA",
        "////////7//////////v/////////+//////////7//////////v/////////+//////////7//////////v/////////+//",
        "////////7//////////v/////////+//uB6F61G43j9xPQrXo3ADQNejcD0K19M/16NwPQrX9z9mZmZmZmbuP5qZmZmZmek/",
        "XI/C9Shc5z8AAAAAAAAGQLgehetRuPI/16NwPQrX4z+4HoXrUbjuPwrXo3A9Cvs/SOF6FK5H8T9SuB6F61EGQLgehetRuPI/",
        "7FG4HoXrAUCF61G4HoUDQFK4HoXrUeA/exSuR+F6xD/2KFyPwvX0P1yPwvUoXPs/4XoUrkfh9j+uR+F6FK7/P6RwPQrXowZA",
        "cT0K16Nw8T+PwvUoXI/+P1yPwvUoXPc/FQAAAAAAAAAFAAAAAAAAABwAAAAAAAAAHgAAAAAAAAALAAAAAAAAAAUAAAAAAAAA",
        "JwAAAAAAAAAbAAAAAAAAAAsAAAAAAAAACwAAAAAAAAAWAAAAAAAAAAQAAAAAAAAAAAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/gQsAAAAAAACCCwAAAAAAAIMLAAAAAAAA",
        "vAsAAAAAAAC9CwAAAAAAAL4LAAAAAAAAvwsAAAAAAAD5CwAAAAAAAPoLAAAAAAAA+wsAAAAAAAA1DAAAAAAAADcMAAAAAAAA",
        "0hIAAAAAAAAUEwAAAAAAAAMUAAAAAAAAaBQAAAAAAAAEFQAAAAAAACYVAAAAAAAAcBUAAAAAAACIGAAAAAAAAOcYAAAAAAAA",
        "4RkAAAAAAABOGgAAAAAAAGEbAAAAAAAAdxsAAAAAAAA8HAAAAAAAAGIcAAAAAAAAXh0AAAAAAADaHQAAAAAAAFUfAAAAAAAA",
        "niAAAAAAAADpIAAAAAAAAAQiAAAAAAAAqCMAAAAAAAC0IwAAAAAAAIslAAAAAAAAPCYAAAAAAABLJgAAAAAAAGUmAAAAAAAA",
        "YnVpbGRpbmdidWlsZGluZ2J1aWxkaW5nYnVpbGRpbmdidWlsZGluZ2J1aWxkaW5nYnVpbGRpbmdidWlsZGluZ2J1aWxkaW5n",
        "YnVpbGRpbmdidWlsZGluZ2J1aWxkaW5ndHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVl",
        "dHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVlAAAAAAAAAAAIAAAA",
        "EAAAABgAAAAgAAAAKAAAADAAAAA4AAAAQAAAAEgAAABQAAAAWAAAAGAAAABkAAAAaAAAAGwAAABwAAAAdAAAAHgAAAB8AAAA",
        "gAAAAIQAAACIAAAAjAAAAJAAAACUAAAAmAAAAJwAAACgAAAApAAAAKgAAACsAAAAsAAAALQAAAC4AAAAvAAAAMAAAADEAAAA",
        "yAAAAMwAAABiNDktNWI0OS02YjQ5LTdiNTAtNGI1MC01YjUwLTZiNTAtN2I1MS01YjUxLTZiNTEtN2I1Mi01YjUyLTdfX1JV",
        "U1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZ",
        "X1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJ",
        "TEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVT",
        "X01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01J",
        "U1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJ",
        "TkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdf",
        "X19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19f",
        "UlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVT",
        "VFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX18AAAAAAAAAAAAAAAUAAAAKAAAADwAAABQAAAAZAAAA",
        "HgAAACMAAAAoAAAALQAAADIAAAA3AAAAPAAAAFMAAABqAAAAgQAAAJgAAACvAAAAxgAAAN0AAAD0AAAACwEAACIBAAA5AQAA",
        "UAEAAGcBAAB+AQAAlQEAAKwBAADDAQAA2gEAAPEBAAAIAgAAHwIAADYCAABNAgAAZAIAAHsCAACSAgAAqQIAADAor0VtOAAA",
        "QEQjo0qVAADUKnrrotwAACv//////wAAZEcaj6CBAADNmF9BkDoAANTVsEQBQgAApCqlYKhSAAAQZIdoWl0AAJjDXXEubAAA",
        "S3aFJLQcAAAn6wrFWsIAAJsmy2pZXAAAfvGq/nn9AACUzKmWnpEAAABwp8ZDuwAA3EmZ9I3nAAAZra7IUcEAAEnVXZX8kAAA",
        "JPEFKWkpAAC9k1RUlkwAAIEZPbkuqQAAJrWVRYZAAAD//4h9z3wAADb+tY6ijQAAX72Gfat3AAAQ83kq9SoAAAwAAAANAAAA",
        "DgAAAA8AAAAQAAAAEQAAABIAAAATAAAAFAAAABUAAAAWAAAAFwAAABgAAAAZAAAAGgAAABsAAAAcAAAAHQAAAB4AAAAfAAAA",
        "IAAAACEAAAAiAAAAIwAAACQAAAAlAAAAJgAAAAAAAAAAAAAAAQAAAAIAAAADAAAABAAAAAUAAAAGAAAABwAAAAgAAAAJAAAA",
        "CgAAAAsAAAAMAAAADQAAAA4AAAAPAAAAEAAAABEAAAASAAAAEwAAABQAAAAVAAAAFgAAABcAAAAYAAAAGQAAABoAAAAAAAAA",
        "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",
        "PwAAAEMAAAA/AAAAQQAAAA==",
    ),
};

/// `perf-meshopt.3tz` `t/3d0acc4baf8b514a770f6197429113b405d91c48e5fce44d6182b0aa670c2bdf.glb` (evidence/v3 conversions), δ from `vector-build.json` `explicitRoot`.
const MESHOPT: Payload = Payload {
    name: "t/3d0acc4baf8b514a770f6197429113b405d91c48e5fce44d6182b0aa670c2bdf.glb",
    delta: [-2182.101928462704, -2057.5955182584257, 8.418375440384352],
    base64: concat!(
        "Z2xURgIAAADIJAAAdBMAAEpTT057ImFjY2Vzc29ycyI6W3siYnVmZmVyVmlldyI6MTEsImNvbXBvbmVudFR5cGUiOjUxMjYs",
        "ImNvdW50IjoyOSwibWF4IjpbMjAzLjY5OTkzNTkxMzA4NTk0LDAuMzExNTQ1NzU5NDM5NDY4NCw0MjAuMTAwMDk3NjU2MjVd",
        "LCJtaW4iOlstMjUzLjMyNjA2NTA2MzQ3NjU2LC0wLjIxMjYxMDAyMTIzMzU1ODY1LC0yNTUuNDM1NDQwMDYzNDc2NTZdLCJ0",
        "eXBlIjoiVkVDMyJ9LHsiYnVmZmVyVmlldyI6MTIsImNvbXBvbmVudFR5cGUiOjUxMjMsImNvdW50IjoyOSwidHlwZSI6IlND",
        "QUxBUiJ9LHsiYnVmZmVyVmlldyI6MTMsImNvbXBvbmVudFR5cGUiOjUxMjUsImNvdW50IjoyOSwidHlwZSI6IlNDQUxBUiJ9",
        "LHsiYnVmZmVyVmlldyI6MTQsImNvbXBvbmVudFR5cGUiOjUxMjUsImNvdW50IjoxMCwidHlwZSI6IlNDQUxBUiJ9LHsiYnVm",
        "ZmVyVmlldyI6MTUsImNvbXBvbmVudFR5cGUiOjUxMjUsImNvdW50Ijo2NCwidHlwZSI6IlNDQUxBUiJ9LHsiYnVmZmVyVmll",
        "dyI6MTYsImNvbXBvbmVudFR5cGUiOjUxMjUsImNvdW50IjoxMCwidHlwZSI6IlNDQUxBUiJ9LHsiYnVmZmVyVmlldyI6MTcs",
        "ImNvbXBvbmVudFR5cGUiOjUxMjYsImNvdW50Ijo1NCwibWF4IjpbMjUzLjMyNjA2NTA2MzQ3NjU2LDAuMzI5NTU0MDIxMzU4",
        "NDksMzcxLjQyNDU5MTA2NDQ1MzFdLCJtaW4iOlstMTQzLjE2MTg2NTIzNDM3NSwtMC4zMjk1NTQwMjEzNTg0OSwtNDIwLjEw",
        "MDA5NzY1NjI1XSwidHlwZSI6IlZFQzMifSx7ImJ1ZmZlclZpZXciOjE4LCJjb21wb25lbnRUeXBlIjo1MTIzLCJjb3VudCI6",
        "NTQsInR5cGUiOiJTQ0FMQVIifSx7ImJ1ZmZlclZpZXciOjE5LCJjb21wb25lbnRUeXBlIjo1MTI1LCJjb3VudCI6MTA4LCJ0",
        "eXBlIjoiU0NBTEFSIn1dLCJhc3NldCI6eyJnZW5lcmF0b3IiOiJydXN0eS10aWxlcyBuYXRpdmUgdmVjdG9yIiwidmVyc2lv",
        "biI6IjIuMCJ9LCJidWZmZXJWaWV3cyI6W3siYnVmZmVyIjowLCJieXRlTGVuZ3RoIjoxNTYsImJ5dGVPZmZzZXQiOjB9LHsi",
        "YnVmZmVyIjowLCJieXRlTGVuZ3RoIjoxNjAsImJ5dGVPZmZzZXQiOjE2MH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjIz",
        "NCwiYnl0ZU9mZnNldCI6MzIwfSx7ImJ1ZmZlciI6MCwiYnl0ZUxlbmd0aCI6MTYwLCJieXRlT2Zmc2V0Ijo1NjB9LHsiYnVm",
        "ZmVyIjowLCJieXRlTGVuZ3RoIjozMTIsImJ5dGVPZmZzZXQiOjcyMH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjMxMiwi",
        "Ynl0ZU9mZnNldCI6MTAzMn0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjMxMiwiYnl0ZU9mZnNldCI6MTM0NH0seyJidWZm",
        "ZXIiOjAsImJ5dGVMZW5ndGgiOjE5NiwiYnl0ZU9mZnNldCI6MTY1Nn0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjE2MCwi",
        "Ynl0ZU9mZnNldCI6MTg1Nn0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjcyNywiYnl0ZU9mZnNldCI6MjAxNn0seyJidWZm",
        "ZXIiOjAsImJ5dGVMZW5ndGgiOjE2MCwiYnl0ZU9mZnNldCI6Mjc0NH0seyJidWZmZXIiOjEsImJ5dGVMZW5ndGgiOjM0OCwi",
        "Ynl0ZU9mZnNldCI6MjkwNCwiZXh0ZW5zaW9ucyI6eyJFWFRfbWVzaG9wdF9jb21wcmVzc2lvbiI6eyJidWZmZXIiOjAsImJ5",
        "dGVPZmZzZXQiOjI5MDQsImJ5dGVMZW5ndGgiOjQwOCwiYnl0ZVN0cmlkZSI6MTIsImNvdW50IjoyOSwibW9kZSI6IkFUVFJJ",
        "QlVURVMiLCJmaWx0ZXIiOiJOT05FIn19fSx7ImJ1ZmZlciI6MSwiYnl0ZUxlbmd0aCI6MTE2LCJieXRlT2Zmc2V0IjozMjU2",
        "LCJieXRlU3RyaWRlIjo0LCJleHRlbnNpb25zIjp7IkVYVF9tZXNob3B0X2NvbXByZXNzaW9uIjp7ImJ1ZmZlciI6MCwiYnl0",
        "ZU9mZnNldCI6MzMxMiwiYnl0ZUxlbmd0aCI6NDUsImJ5dGVTdHJpZGUiOjQsImNvdW50IjoyOSwibW9kZSI6IkFUVFJJQlVU",
        "RVMiLCJmaWx0ZXIiOiJOT05FIn19fSx7ImJ1ZmZlciI6MSwiYnl0ZUxlbmd0aCI6MTE2LCJieXRlT2Zmc2V0IjozMzc2LCJl",
        "eHRlbnNpb25zIjp7IkVYVF9tZXNob3B0X2NvbXByZXNzaW9uIjp7ImJ1ZmZlciI6MCwiYnl0ZU9mZnNldCI6MzM2MCwiYnl0",
        "ZUxlbmd0aCI6NDUsImJ5dGVTdHJpZGUiOjQsImNvdW50IjoyOSwibW9kZSI6IkFUVFJJQlVURVMiLCJmaWx0ZXIiOiJOT05F",
        "In19fSx7ImJ1ZmZlciI6MSwiYnl0ZUxlbmd0aCI6NDAsImJ5dGVPZmZzZXQiOjM0OTYsImV4dGVuc2lvbnMiOnsiRVhUX21l",
        "c2hvcHRfY29tcHJlc3Npb24iOnsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjozNDA4LCJieXRlTGVuZ3RoIjo1MCwiYnl0ZVN0",
        "cmlkZSI6NCwiY291bnQiOjEwLCJtb2RlIjoiQVRUUklCVVRFUyIsImZpbHRlciI6Ik5PTkUifX19LHsiYnVmZmVyIjoxLCJi",
        "eXRlTGVuZ3RoIjoyNTYsImJ5dGVPZmZzZXQiOjM1MzYsImV4dGVuc2lvbnMiOnsiRVhUX21lc2hvcHRfY29tcHJlc3Npb24i",
        "OnsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjozNDY0LCJieXRlTGVuZ3RoIjoxMjEsImJ5dGVTdHJpZGUiOjQsImNvdW50Ijo2",
        "NCwibW9kZSI6IkFUVFJJQlVURVMiLCJmaWx0ZXIiOiJOT05FIn19fSx7ImJ1ZmZlciI6MSwiYnl0ZUxlbmd0aCI6NDAsImJ5",
        "dGVPZmZzZXQiOjM3OTIsImV4dGVuc2lvbnMiOnsiRVhUX21lc2hvcHRfY29tcHJlc3Npb24iOnsiYnVmZmVyIjowLCJieXRl",
        "T2Zmc2V0IjozNTkyLCJieXRlTGVuZ3RoIjo0NiwiYnl0ZVN0cmlkZSI6NCwiY291bnQiOjEwLCJtb2RlIjoiQVRUUklCVVRF",
        "UyIsImZpbHRlciI6Ik5PTkUifX19LHsiYnVmZmVyIjoxLCJieXRlTGVuZ3RoIjo2NDgsImJ5dGVPZmZzZXQiOjM4MzIsImV4",
        "dGVuc2lvbnMiOnsiRVhUX21lc2hvcHRfY29tcHJlc3Npb24iOnsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjozNjQwLCJieXRl",
        "TGVuZ3RoIjo2MDcsImJ5dGVTdHJpZGUiOjEyLCJjb3VudCI6NTQsIm1vZGUiOiJBVFRSSUJVVEVTIiwiZmlsdGVyIjoiTk9O",
        "RSJ9fX0seyJidWZmZXIiOjEsImJ5dGVMZW5ndGgiOjIxNiwiYnl0ZU9mZnNldCI6NDQ4MCwiYnl0ZVN0cmlkZSI6NCwiZXh0",
        "ZW5zaW9ucyI6eyJFWFRfbWVzaG9wdF9jb21wcmVzc2lvbiI6eyJidWZmZXIiOjAsImJ5dGVPZmZzZXQiOjQyNDgsImJ5dGVM",
        "ZW5ndGgiOjUzLCJieXRlU3RyaWRlIjo0LCJjb3VudCI6NTQsIm1vZGUiOiJBVFRSSUJVVEVTIiwiZmlsdGVyIjoiTk9ORSJ9",
        "fX0seyJidWZmZXIiOjEsImJ5dGVMZW5ndGgiOjQzMiwiYnl0ZU9mZnNldCI6NDY5NiwiZXh0ZW5zaW9ucyI6eyJFWFRfbWVz",
        "aG9wdF9jb21wcmVzc2lvbiI6eyJidWZmZXIiOjAsImJ5dGVPZmZzZXQiOjQzMDQsImJ5dGVMZW5ndGgiOjEwMCwiYnl0ZVN0",
        "cmlkZSI6NCwiY291bnQiOjEwOCwibW9kZSI6IkFUVFJJQlVURVMiLCJmaWx0ZXIiOiJOT05FIn19fV0sImJ1ZmZlcnMiOlt7",
        "ImJ5dGVMZW5ndGgiOjQ0MDh9LHsiYnl0ZUxlbmd0aCI6NTEyOCwiZXh0ZW5zaW9ucyI6eyJFWFRfbWVzaG9wdF9jb21wcmVz",
        "c2lvbiI6eyJmYWxsYmFjayI6dHJ1ZX19fV0sImV4dGVuc2lvbnMiOnsiRVhUX3N0cnVjdHVyYWxfbWV0YWRhdGEiOnsicHJv",
        "cGVydHlUYWJsZXMiOlt7ImNsYXNzIjoiZmVhdHVyZSIsImNvdW50IjozOSwibmFtZSI6ImZlYXR1cmVzIiwicHJvcGVydGll",
        "cyI6eyJfc291cmNlX2lkIjp7InN0cmluZ09mZnNldFR5cGUiOiJVSU5UMzIiLCJzdHJpbmdPZmZzZXRzIjoxLCJ2YWx1ZXMi",
        "OjB9LCJfc291cmNlX2xheWVyIjp7InN0cmluZ09mZnNldFR5cGUiOiJVSU5UMzIiLCJzdHJpbmdPZmZzZXRzIjozLCJ2YWx1",
        "ZXMiOjJ9LCJnaXJ0aCI6eyJ2YWx1ZXMiOjR9LCJoZWlnaHQiOnsidmFsdWVzIjo1fSwiaWQiOnsidmFsdWVzIjo2fSwia2lu",
        "ZCI6eyJzdHJpbmdPZmZzZXRUeXBlIjoiVUlOVDMyIiwic3RyaW5nT2Zmc2V0cyI6OCwidmFsdWVzIjo3fSwibmFtZSI6eyJz",
        "dHJpbmdPZmZzZXRUeXBlIjoiVUlOVDMyIiwic3RyaW5nT2Zmc2V0cyI6MTAsInZhbHVlcyI6OX19fV0sInNjaGVtYSI6eyJj",
        "bGFzc2VzIjp7ImZlYXR1cmUiOnsicHJvcGVydGllcyI6eyJfc291cmNlX2lkIjp7InR5cGUiOiJTVFJJTkcifSwiX3NvdXJj",
        "ZV9sYXllciI6eyJ0eXBlIjoiU1RSSU5HIn0sImdpcnRoIjp7ImNvbXBvbmVudFR5cGUiOiJGTE9BVDY0Iiwibm9EYXRhIjot",
        "MS43OTc2OTMxMzQ4NjIzMTU3ZSszMDgsInR5cGUiOiJTQ0FMQVIifSwiaGVpZ2h0Ijp7ImNvbXBvbmVudFR5cGUiOiJJTlQ2",
        "NCIsIm5vRGF0YSI6LTkwMDcxOTkyNTQ3NDA5OTIsInR5cGUiOiJTQ0FMQVIifSwiaWQiOnsiY29tcG9uZW50VHlwZSI6IklO",
        "VDY0IiwidHlwZSI6IlNDQUxBUiJ9LCJraW5kIjp7InR5cGUiOiJTVFJJTkcifSwibmFtZSI6eyJub0RhdGEiOiJfX1JVU1RZ",
        "X1RJTEVTX01JU1NJTkdfXyIsInR5cGUiOiJTVFJJTkcifX19fSwiaWQiOiJydXN0eV90aWxlc192ZWN0b3IifX19LCJleHRl",
        "bnNpb25zVXNlZCI6WyJFWFRfbWVzaF9mZWF0dXJlcyIsIkVYVF9zdHJ1Y3R1cmFsX21ldGFkYXRhIiwiRVhUX21lc2hfcG9s",
        "eWdvbiIsIkVYVF9tZXNob3B0X2NvbXByZXNzaW9uIl0sIm1lc2hlcyI6W3sicHJpbWl0aXZlcyI6W3siYXR0cmlidXRlcyI6",
        "eyJQT1NJVElPTiI6MCwiX0ZFQVRVUkVfSURfMCI6MX0sImV4dGVuc2lvbnMiOnsiRVhUX21lc2hfZmVhdHVyZXMiOnsiZmVh",
        "dHVyZUlkcyI6W3siYXR0cmlidXRlIjowLCJmZWF0dXJlQ291bnQiOjM5LCJwcm9wZXJ0eVRhYmxlIjowfV19fSwiaW5kaWNl",
        "cyI6MiwibW9kZSI6MH0seyJhdHRyaWJ1dGVzIjp7IlBPU0lUSU9OIjo2LCJfRkVBVFVSRV9JRF8wIjo3fSwiZXh0ZW5zaW9u",
        "cyI6eyJFWFRfbWVzaF9mZWF0dXJlcyI6eyJmZWF0dXJlSWRzIjpbeyJhdHRyaWJ1dGUiOjAsImZlYXR1cmVDb3VudCI6Mzks",
        "InByb3BlcnR5VGFibGUiOjB9XX0sIkVYVF9tZXNoX3BvbHlnb24iOnsiY291bnQiOjEwLCJpbmRpY2VzT2Zmc2V0cyI6Mywi",
        "bG9vcEluZGljZXMiOjQsImxvb3BJbmRpY2VzT2Zmc2V0cyI6NX19LCJpbmRpY2VzIjo4LCJtb2RlIjo0fV19XSwibm9kZXMi",
        "Olt7Im1lc2giOjB9XSwic2NlbmUiOjAsInNjZW5lcyI6W3sibm9kZXMiOlswXX1dLCJleHRlbnNpb25zUmVxdWlyZWQiOlsi",
        "RVhUX21lc2hvcHRfY29tcHJlc3Npb24iXX0gICAgICA4EQAAQklOADExNTYxMTU3MTIxNjEyMTcxMjE4MTI3NjEyNzcxMjc4",
        "MTMzNjEzMzc0MDE0NDAxNTQxMjY0MTc0NDYyOTQ5MzY1MjY1NTk4NzYyMDI2MjczNjQyMDY1MTE2NTg2NjgxOTY4NjA2OTc1",
        "NzMwMDgwNzQ4MjEwODI5NDg2NDg4NzkwODgxMjg5Mzg4OTM5OTA0OTk3MTA5NzY3OTkxNAAAAAAAAAAABAAAAAgAAAAMAAAA",
        "EAAAABQAAAAYAAAAHAAAACAAAAAkAAAAKAAAACwAAAAwAAAANAAAADgAAAA8AAAAQAAAAEQAAABIAAAATAAAAFAAAABUAAAA",
        "WAAAAFwAAABgAAAAZAAAAGgAAABsAAAAcAAAAHQAAAB4AAAAfAAAAIAAAACEAAAAiAAAAIwAAACQAAAAlAAAAJgAAACcAAAA",
        "dmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9y",
        "dmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9y",
        "dmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9ydmVjdG9y",
        "dmVjdG9ydmVjdG9ydmVjdG9yAAAAAAAAAAAAAAYAAAAMAAAAEgAAABgAAAAeAAAAJAAAACoAAAAwAAAANgAAADwAAABCAAAA",
        "SAAAAE4AAABUAAAAWgAAAGAAAABmAAAAbAAAAHIAAAB4AAAAfgAAAIQAAACKAAAAkAAAAJYAAACcAAAAogAAAKgAAACuAAAA",
        "tAAAALoAAADAAAAAxgAAAMwAAADSAAAA2AAAAN4AAADkAAAA6gAAAP///////+//////////7//////////v/////////+//",
        "////////7//////////v/////////+//////////7//////////v/////////+//SOF6FK5H4T8UrkfhehT2P8P1KFyPwgVA",
        "16NwPQrX0z8UrkfhehQEQPYoXI/C9QZAexSuR+F6tD8fhetRuB71P4/C9Shcj9I/FK5H4XoUBkApXI/C9SjMP4/C9Shcj9I/",
        "CtejcD0KA0DD9Shcj8L5P65H4XoUrgdA4XoUrkfhyj+PwvUoXI8GQMP1KFyPwgVAXI/C9Shc+z9I4XoUrkf9P5qZmZmZmfE/",
        "XI/C9Shc8z8K16NwPQq3Pylcj8L1KPQ/CtejcD0Ktz+kcD0K16PAP7gehetRuARAzczMzMzMBEB7FK5H4XrsPxQAAAAAAAAA",
        "CQAAAAAAAAAWAAAAAAAAACYAAAAAAAAAFwAAAAAAAAARAAAAAAAAABQAAAAAAAAABAAAAAAAAAAMAAAAAAAAACgAAAAAAAAA",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/4QEAAAAAAAAhQQAAAAAAADABAAAAAAAAMEEAAAAAAAAwgQAAAAAAAD8BAAAAAAAAP0EAAAAAAAA",
        "/gQAAAAAAAA4BQAAAAAAADkFAAAAAAAArg8AAAAAAACvDwAAAAAAAB4QAAAAAAAAThAAAAAAAAAVEgAAAAAAAEgTAAAAAAAA",
        "kRQAAAAAAABjFwAAAAAAADoYAAAAAAAAgRgAAAAAAAAUGQAAAAAAAG8ZAAAAAAAAuhkAAAAAAACjGgAAAAAAAMwaAAAAAAAA",
        "PxsAAAAAAACEHAAAAAAAAIofAAAAAAAAEiAAAAAAAABmIAAAAAAAAMghAAAAAAAAViIAAAAAAABsIgAAAAAAAOoiAAAAAAAA",
        "6yIAAAAAAABZIwAAAAAAAO4lAAAAAAAAJyYAAAAAAAC6JgAAAAAAAGJ1aWxkaW5nYnVpbGRpbmdidWlsZGluZ2J1aWxkaW5n",
        "YnVpbGRpbmdidWlsZGluZ2J1aWxkaW5nYnVpbGRpbmdidWlsZGluZ2J1aWxkaW5ndHJlZXRyZWV0cmVldHJlZXRyZWV0cmVl",
        "dHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVl",
        "dHJlZXRyZWV0cmVldHJlZXRyZWUAAAAAAAAAAAgAAAAQAAAAGAAAACAAAAAoAAAAMAAAADgAAABAAAAASAAAAFAAAABUAAAA",
        "WAAAAFwAAABgAAAAZAAAAGgAAABsAAAAcAAAAHQAAAB4AAAAfAAAAIAAAACEAAAAiAAAAIwAAACQAAAAlAAAAJgAAACcAAAA",
        "oAAAAKQAAACoAAAArAAAALAAAAC0AAAAuAAAALwAAADAAAAAxAAAAGIxOS0xNmIxOS0xN2IyMC0xNmIyMC0xN2IyMC0xOGIy",
        "MS0xNmIyMS0xN2IyMS0xOGIyMi0xNmIyMi0xN19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5H",
        "X19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19f",
        "X1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JV",
        "U1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZ",
        "X1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJ",
        "TEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVT",
        "X01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01J",
        "U1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJ",
        "TkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdf",
        "X19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX18AAAAAAAYAAAAMAAAAEgAAABgAAAAeAAAA",
        "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",
    ),
};

/// `perf-quant-meshopt.3tz` `t/d5d8c0a850f2420d2ab035cca4f08ed98291055a7858a480e27d1672e9a00d82.glb` (evidence/v3 conversions), δ from `vector-build.json` `explicitRoot`.
const QUANTIZE_MESHOPT: Payload = Payload {
    name: "t/d5d8c0a850f2420d2ab035cca4f08ed98291055a7858a480e27d1672e9a00d82.glb",
    delta: [-3997.0360376601798, 4574.8828832519575, 0.6623462713405388],
    base64: concat!(
        "Z2xURgIAAAD4IwAAxBMAAEpTT057ImFjY2Vzc29ycyI6W3siYnVmZmVyVmlldyI6MTEsImNvbXBvbmVudFR5cGUiOjUxMjMs",
        "ImNvdW50IjoyNywibWF4IjpbNjU1MzUsNjU1MzUsNjU1MzVdLCJtaW4iOls2NTI5LDkzNDksNzM0OF0sIm5vcm1hbGl6ZWQi",
        "OnRydWUsInR5cGUiOiJWRUMzIn0seyJidWZmZXJWaWV3IjoxMiwiY29tcG9uZW50VHlwZSI6NTEyMywiY291bnQiOjI3LCJ0",
        "eXBlIjoiU0NBTEFSIn0seyJidWZmZXJWaWV3IjoxMywiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjI3LCJ0eXBlIjoi",
        "U0NBTEFSIn0seyJidWZmZXJWaWV3IjoxNCwiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjEyLCJ0eXBlIjoiU0NBTEFS",
        "In0seyJidWZmZXJWaWV3IjoxNSwiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjgxLCJ0eXBlIjoiU0NBTEFSIn0seyJi",
        "dWZmZXJWaWV3IjoxNiwiY29tcG9uZW50VHlwZSI6NTEyNSwiY291bnQiOjEyLCJ0eXBlIjoiU0NBTEFSIn0seyJidWZmZXJW",
        "aWV3IjoxNywiY29tcG9uZW50VHlwZSI6NTEyMywiY291bnQiOjY4LCJtYXgiOls1OTMwMyw2MDk4NCw1NzUwOF0sIm1pbiI6",
        "WzAsMCwwXSwibm9ybWFsaXplZCI6dHJ1ZSwidHlwZSI6IlZFQzMifSx7ImJ1ZmZlclZpZXciOjE4LCJjb21wb25lbnRUeXBl",
        "Ijo1MTIzLCJjb3VudCI6NjgsInR5cGUiOiJTQ0FMQVIifSx7ImJ1ZmZlclZpZXciOjE5LCJjb21wb25lbnRUeXBlIjo1MTI1",
        "LCJjb3VudCI6MTQ0LCJ0eXBlIjoiU0NBTEFSIn1dLCJhc3NldCI6eyJnZW5lcmF0b3IiOiJydXN0eS10aWxlcyBuYXRpdmUg",
        "dmVjdG9yIiwidmVyc2lvbiI6IjIuMCJ9LCJidWZmZXJWaWV3cyI6W3siYnVmZmVyIjowLCJieXRlTGVuZ3RoIjoxNTYsImJ5",
        "dGVPZmZzZXQiOjB9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjoxNjAsImJ5dGVPZmZzZXQiOjE2MH0seyJidWZmZXIiOjAs",
        "ImJ5dGVMZW5ndGgiOjIzNCwiYnl0ZU9mZnNldCI6MzIwfSx7ImJ1ZmZlciI6MCwiYnl0ZUxlbmd0aCI6MTYwLCJieXRlT2Zm",
        "c2V0Ijo1NjB9LHsiYnVmZmVyIjowLCJieXRlTGVuZ3RoIjozMTIsImJ5dGVPZmZzZXQiOjcyMH0seyJidWZmZXIiOjAsImJ5",
        "dGVMZW5ndGgiOjMxMiwiYnl0ZU9mZnNldCI6MTAzMn0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjMxMiwiYnl0ZU9mZnNl",
        "dCI6MTM0NH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjIwNCwiYnl0ZU9mZnNldCI6MTY1Nn0seyJidWZmZXIiOjAsImJ5",
        "dGVMZW5ndGgiOjE2MCwiYnl0ZU9mZnNldCI6MTg2NH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjY4MSwiYnl0ZU9mZnNl",
        "dCI6MjAyNH0seyJidWZmZXIiOjAsImJ5dGVMZW5ndGgiOjE2MCwiYnl0ZU9mZnNldCI6MjcxMn0seyJidWZmZXIiOjEsImJ5",
        "dGVMZW5ndGgiOjIxNiwiYnl0ZU9mZnNldCI6Mjg3MiwiYnl0ZVN0cmlkZSI6OCwiZXh0ZW5zaW9ucyI6eyJFWFRfbWVzaG9w",
        "dF9jb21wcmVzc2lvbiI6eyJidWZmZXIiOjAsImJ5dGVPZmZzZXQiOjI4NzIsImJ5dGVMZW5ndGgiOjIyNiwiYnl0ZVN0cmlk",
        "ZSI6OCwiY291bnQiOjI3LCJtb2RlIjoiQVRUUklCVVRFUyIsImZpbHRlciI6Ik5PTkUifX19LHsiYnVmZmVyIjoxLCJieXRl",
        "TGVuZ3RoIjoxMDgsImJ5dGVPZmZzZXQiOjMwODgsImJ5dGVTdHJpZGUiOjQsImV4dGVuc2lvbnMiOnsiRVhUX21lc2hvcHRf",
        "Y29tcHJlc3Npb24iOnsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjozMTA0LCJieXRlTGVuZ3RoIjo0NSwiYnl0ZVN0cmlkZSI6",
        "NCwiY291bnQiOjI3LCJtb2RlIjoiQVRUUklCVVRFUyIsImZpbHRlciI6Ik5PTkUifX19LHsiYnVmZmVyIjoxLCJieXRlTGVu",
        "Z3RoIjoxMDgsImJ5dGVPZmZzZXQiOjMyMDAsImV4dGVuc2lvbnMiOnsiRVhUX21lc2hvcHRfY29tcHJlc3Npb24iOnsiYnVm",
        "ZmVyIjowLCJieXRlT2Zmc2V0IjozMTUyLCJieXRlTGVuZ3RoIjo0NSwiYnl0ZVN0cmlkZSI6NCwiY291bnQiOjI3LCJtb2Rl",
        "IjoiQVRUUklCVVRFUyIsImZpbHRlciI6Ik5PTkUifX19LHsiYnVmZmVyIjoxLCJieXRlTGVuZ3RoIjo0OCwiYnl0ZU9mZnNl",
        "dCI6MzMxMiwiZXh0ZW5zaW9ucyI6eyJFWFRfbWVzaG9wdF9jb21wcmVzc2lvbiI6eyJidWZmZXIiOjAsImJ5dGVPZmZzZXQi",
        "OjMyMDAsImJ5dGVMZW5ndGgiOjUyLCJieXRlU3RyaWRlIjo0LCJjb3VudCI6MTIsIm1vZGUiOiJBVFRSSUJVVEVTIiwiZmls",
        "dGVyIjoiTk9ORSJ9fX0seyJidWZmZXIiOjEsImJ5dGVMZW5ndGgiOjMyNCwiYnl0ZU9mZnNldCI6MzM2MCwiZXh0ZW5zaW9u",
        "cyI6eyJFWFRfbWVzaG9wdF9jb21wcmVzc2lvbiI6eyJidWZmZXIiOjAsImJ5dGVPZmZzZXQiOjMyNTYsImJ5dGVMZW5ndGgi",
        "OjE1MSwiYnl0ZVN0cmlkZSI6NCwiY291bnQiOjgxLCJtb2RlIjoiQVRUUklCVVRFUyIsImZpbHRlciI6Ik5PTkUifX19LHsi",
        "YnVmZmVyIjoxLCJieXRlTGVuZ3RoIjo0OCwiYnl0ZU9mZnNldCI6MzY4OCwiZXh0ZW5zaW9ucyI6eyJFWFRfbWVzaG9wdF9j",
        "b21wcmVzc2lvbiI6eyJidWZmZXIiOjAsImJ5dGVPZmZzZXQiOjM0MDgsImJ5dGVMZW5ndGgiOjQ3LCJieXRlU3RyaWRlIjo0",
        "LCJjb3VudCI6MTIsIm1vZGUiOiJBVFRSSUJVVEVTIiwiZmlsdGVyIjoiTk9ORSJ9fX0seyJidWZmZXIiOjEsImJ5dGVMZW5n",
        "dGgiOjU0NCwiYnl0ZU9mZnNldCI6MzczNiwiYnl0ZVN0cmlkZSI6OCwiZXh0ZW5zaW9ucyI6eyJFWFRfbWVzaG9wdF9jb21w",
        "cmVzc2lvbiI6eyJidWZmZXIiOjAsImJ5dGVPZmZzZXQiOjM0NTYsImJ5dGVMZW5ndGgiOjQ2OSwiYnl0ZVN0cmlkZSI6OCwi",
        "Y291bnQiOjY4LCJtb2RlIjoiQVRUUklCVVRFUyIsImZpbHRlciI6Ik5PTkUifX19LHsiYnVmZmVyIjoxLCJieXRlTGVuZ3Ro",
        "IjoyNzIsImJ5dGVPZmZzZXQiOjQyODAsImJ5dGVTdHJpZGUiOjQsImV4dGVuc2lvbnMiOnsiRVhUX21lc2hvcHRfY29tcHJl",
        "c3Npb24iOnsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjozOTI4LCJieXRlTGVuZ3RoIjo1NywiYnl0ZVN0cmlkZSI6NCwiY291",
        "bnQiOjY4LCJtb2RlIjoiQVRUUklCVVRFUyIsImZpbHRlciI6Ik5PTkUifX19LHsiYnVmZmVyIjoxLCJieXRlTGVuZ3RoIjo1",
        "NzYsImJ5dGVPZmZzZXQiOjQ1NTIsImV4dGVuc2lvbnMiOnsiRVhUX21lc2hvcHRfY29tcHJlc3Npb24iOnsiYnVmZmVyIjow",
        "LCJieXRlT2Zmc2V0IjozOTkyLCJieXRlTGVuZ3RoIjoxMjMsImJ5dGVTdHJpZGUiOjQsImNvdW50IjoxNDQsIm1vZGUiOiJB",
        "VFRSSUJVVEVTIiwiZmlsdGVyIjoiTk9ORSJ9fX1dLCJidWZmZXJzIjpbeyJieXRlTGVuZ3RoIjo0MTIwfSx7ImJ5dGVMZW5n",
        "dGgiOjUxMjgsImV4dGVuc2lvbnMiOnsiRVhUX21lc2hvcHRfY29tcHJlc3Npb24iOnsiZmFsbGJhY2siOnRydWV9fX1dLCJl",
        "eHRlbnNpb25zIjp7IkVYVF9zdHJ1Y3R1cmFsX21ldGFkYXRhIjp7InByb3BlcnR5VGFibGVzIjpbeyJjbGFzcyI6ImZlYXR1",
        "cmUiLCJjb3VudCI6MzksIm5hbWUiOiJmZWF0dXJlcyIsInByb3BlcnRpZXMiOnsiX3NvdXJjZV9pZCI6eyJzdHJpbmdPZmZz",
        "ZXRUeXBlIjoiVUlOVDMyIiwic3RyaW5nT2Zmc2V0cyI6MSwidmFsdWVzIjowfSwiX3NvdXJjZV9sYXllciI6eyJzdHJpbmdP",
        "ZmZzZXRUeXBlIjoiVUlOVDMyIiwic3RyaW5nT2Zmc2V0cyI6MywidmFsdWVzIjoyfSwiZ2lydGgiOnsidmFsdWVzIjo0fSwi",
        "aGVpZ2h0Ijp7InZhbHVlcyI6NX0sImlkIjp7InZhbHVlcyI6Nn0sImtpbmQiOnsic3RyaW5nT2Zmc2V0VHlwZSI6IlVJTlQz",
        "MiIsInN0cmluZ09mZnNldHMiOjgsInZhbHVlcyI6N30sIm5hbWUiOnsic3RyaW5nT2Zmc2V0VHlwZSI6IlVJTlQzMiIsInN0",
        "cmluZ09mZnNldHMiOjEwLCJ2YWx1ZXMiOjl9fX1dLCJzY2hlbWEiOnsiY2xhc3NlcyI6eyJmZWF0dXJlIjp7InByb3BlcnRp",
        "ZXMiOnsiX3NvdXJjZV9pZCI6eyJ0eXBlIjoiU1RSSU5HIn0sIl9zb3VyY2VfbGF5ZXIiOnsidHlwZSI6IlNUUklORyJ9LCJn",
        "aXJ0aCI6eyJjb21wb25lbnRUeXBlIjoiRkxPQVQ2NCIsIm5vRGF0YSI6LTEuNzk3NjkzMTM0ODYyMzE1N2UrMzA4LCJ0eXBl",
        "IjoiU0NBTEFSIn0sImhlaWdodCI6eyJjb21wb25lbnRUeXBlIjoiSU5UNjQiLCJub0RhdGEiOi05MDA3MTk5MjU0NzQwOTky",
        "LCJ0eXBlIjoiU0NBTEFSIn0sImlkIjp7ImNvbXBvbmVudFR5cGUiOiJJTlQ2NCIsInR5cGUiOiJTQ0FMQVIifSwia2luZCI6",
        "eyJ0eXBlIjoiU1RSSU5HIn0sIm5hbWUiOnsibm9EYXRhIjoiX19SVVNUWV9USUxFU19NSVNTSU5HX18iLCJ0eXBlIjoiU1RS",
        "SU5HIn19fX0sImlkIjoicnVzdHlfdGlsZXNfdmVjdG9yIn19fSwiZXh0ZW5zaW9uc1JlcXVpcmVkIjpbIktIUl9tZXNoX3F1",
        "YW50aXphdGlvbiIsIkVYVF9tZXNob3B0X2NvbXByZXNzaW9uIl0sImV4dGVuc2lvbnNVc2VkIjpbIkVYVF9tZXNoX2ZlYXR1",
        "cmVzIiwiRVhUX3N0cnVjdHVyYWxfbWV0YWRhdGEiLCJFWFRfbWVzaF9wb2x5Z29uIiwiS0hSX21lc2hfcXVhbnRpemF0aW9u",
        "IiwiRVhUX21lc2hvcHRfY29tcHJlc3Npb24iXSwibWVzaGVzIjpbeyJwcmltaXRpdmVzIjpbeyJhdHRyaWJ1dGVzIjp7IlBP",
        "U0lUSU9OIjowLCJfRkVBVFVSRV9JRF8wIjoxfSwiZXh0ZW5zaW9ucyI6eyJFWFRfbWVzaF9mZWF0dXJlcyI6eyJmZWF0dXJl",
        "SWRzIjpbeyJhdHRyaWJ1dGUiOjAsImZlYXR1cmVDb3VudCI6MzksInByb3BlcnR5VGFibGUiOjB9XX19LCJpbmRpY2VzIjoy",
        "LCJtb2RlIjowfSx7ImF0dHJpYnV0ZXMiOnsiUE9TSVRJT04iOjYsIl9GRUFUVVJFX0lEXzAiOjd9LCJleHRlbnNpb25zIjp7",
        "IkVYVF9tZXNoX2ZlYXR1cmVzIjp7ImZlYXR1cmVJZHMiOlt7ImF0dHJpYnV0ZSI6MCwiZmVhdHVyZUNvdW50IjozOSwicHJv",
        "cGVydHlUYWJsZSI6MH1dfSwiRVhUX21lc2hfcG9seWdvbiI6eyJjb3VudCI6MTIsImluZGljZXNPZmZzZXRzIjozLCJsb29w",
        "SW5kaWNlcyI6NCwibG9vcEluZGljZXNPZmZzZXRzIjo1fX0sImluZGljZXMiOjgsIm1vZGUiOjR9XX1dLCJub2RlcyI6W3si",
        "bWVzaCI6MCwic2NhbGUiOls2NDEuMDA2NTkxNzk2ODc1LDEuNTY3MjQyMjY0NzQ3NjE5Niw5MDAuNzc1ODc4OTA2MjVdLCJ0",
        "cmFuc2xhdGlvbiI6Wy0zMjAuNTAzMjk1ODk4NDM3NSwtMC43ODM2MjExMzIzNzM4MDk4LC00NTAuMzg3OTM5NDUzMTI1XX1d",
        "LCJzY2VuZSI6MCwic2NlbmVzIjpbeyJub2RlcyI6WzBdfV19ICAgIBgQAABCSU4AMjk0NTI5NDYyOTQ3MzAwNDMwMDUzMDA2",
        "MzAwNzMwNjUzMDY2MzA2NzMxMjUzMTI3NDgxODQ4ODQ1MTIzNTIyNDUzODA1NDE0NTQ4ODYyODA2Mzc1NjYyNTY3MzQ3MDA5",
        "NzAzMTcyMjg3MjY2NzUxODc2NDI4MDIxODM1MDg0MjU4NzA4OTEyODkxNDA5NjExOTc4ODk4MDM5ODI5AAAAAAAAAAAEAAAA",
        "CAAAAAwAAAAQAAAAFAAAABgAAAAcAAAAIAAAACQAAAAoAAAALAAAADAAAAA0AAAAOAAAADwAAABAAAAARAAAAEgAAABMAAAA",
        "UAAAAFQAAABYAAAAXAAAAGAAAABkAAAAaAAAAGwAAABwAAAAdAAAAHgAAAB8AAAAgAAAAIQAAACIAAAAjAAAAJAAAACUAAAA",
        "mAAAAJwAAAB2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0",
        "b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0",
        "b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0",
        "b3J2ZWN0b3J2ZWN0b3J2ZWN0b3J2ZWN0b3IAAAAAAAAAAAAABgAAAAwAAAASAAAAGAAAAB4AAAAkAAAAKgAAADAAAAA2AAAA",
        "PAAAAEIAAABIAAAATgAAAFQAAABaAAAAYAAAAGYAAABsAAAAcgAAAHgAAAB+AAAAhAAAAIoAAACQAAAAlgAAAJwAAACiAAAA",
        "qAAAAK4AAAC0AAAAugAAAMAAAADGAAAAzAAAANIAAADYAAAA3gAAAOQAAADqAAAA////////7//////////v/////////+//",
        "////////7//////////v/////////+//////////7//////////v/////////+//////////7//////////v/////////+//",
        "uB6F61G43j9xPQrXo3ADQNejcD0K19M/16NwPQrX9z9mZmZmZmbuP5qZmZmZmek/XI/C9Shc5z8AAAAAAAAGQLgehetRuPI/",
        "16NwPQrX4z+4HoXrUbjuPwrXo3A9Cvs/SOF6FK5H8T9SuB6F61EGQLgehetRuPI/7FG4HoXrAUCF61G4HoUDQFK4HoXrUeA/",
        "exSuR+F6xD/2KFyPwvX0P1yPwvUoXPs/4XoUrkfh9j+uR+F6FK7/P6RwPQrXowZAcT0K16Nw8T+PwvUoXI/+P1yPwvUoXPc/",
        "FQAAAAAAAAAFAAAAAAAAABwAAAAAAAAAHgAAAAAAAAALAAAAAAAAAAUAAAAAAAAAJwAAAAAAAAAbAAAAAAAAAAsAAAAAAAAA",
        "CwAAAAAAAAAWAAAAAAAAAAQAAAAAAAAAAAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/",
        "AAAAAAAA4P8AAAAAAADg/wAAAAAAAOD/gQsAAAAAAACCCwAAAAAAAIMLAAAAAAAAvAsAAAAAAAC9CwAAAAAAAL4LAAAAAAAA",
        "vwsAAAAAAAD5CwAAAAAAAPoLAAAAAAAA+wsAAAAAAAA1DAAAAAAAADcMAAAAAAAA0hIAAAAAAAAUEwAAAAAAAAMUAAAAAAAA",
        "aBQAAAAAAAAEFQAAAAAAACYVAAAAAAAAcBUAAAAAAACIGAAAAAAAAOcYAAAAAAAA4RkAAAAAAABOGgAAAAAAAGEbAAAAAAAA",
        "dxsAAAAAAAA8HAAAAAAAAGIcAAAAAAAAXh0AAAAAAADaHQAAAAAAAFUfAAAAAAAAniAAAAAAAADpIAAAAAAAAAQiAAAAAAAA",
        "qCMAAAAAAAC0IwAAAAAAAIslAAAAAAAAPCYAAAAAAABLJgAAAAAAAGUmAAAAAAAAYnVpbGRpbmdidWlsZGluZ2J1aWxkaW5n",
        "YnVpbGRpbmdidWlsZGluZ2J1aWxkaW5nYnVpbGRpbmdidWlsZGluZ2J1aWxkaW5nYnVpbGRpbmdidWlsZGluZ2J1aWxkaW5n",
        "dHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVl",
        "dHJlZXRyZWV0cmVldHJlZXRyZWV0cmVldHJlZXRyZWV0cmVlAAAAAAAAAAAIAAAAEAAAABgAAAAgAAAAKAAAADAAAAA4AAAA",
        "QAAAAEgAAABQAAAAWAAAAGAAAABkAAAAaAAAAGwAAABwAAAAdAAAAHgAAAB8AAAAgAAAAIQAAACIAAAAjAAAAJAAAACUAAAA",
        "mAAAAJwAAACgAAAApAAAAKgAAACsAAAAsAAAALQAAAC4AAAAvAAAAMAAAADEAAAAyAAAAMwAAABiNDktNWI0OS02YjQ5LTdi",
        "NTAtNGI1MC01YjUwLTZiNTAtN2I1MS01YjUxLTZiNTEtN2I1Mi01YjUyLTdfX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVT",
        "VFlfVElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlf",
        "VElMRVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElM",
        "RVNfTUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNf",
        "TUlTU0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlT",
        "U0lOR19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lO",
        "R19fX19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19f",
        "X19SVVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19S",
        "VVNUWV9USUxFU19NSVNTSU5HX19fX1JVU1RZX1RJTEVTX01JU1NJTkdfX19fUlVTVFlfVElMRVNfTUlTU0lOR19fX19SVVNU",
        "WV9USUxFU19NSVNTSU5HX18AAAAAAAAAAAAAAAUAAAAKAAAADwAAABQAAAAZAAAAHgAAACMAAAAoAAAALQAAADIAAAA3AAAA",
        "PAAAAFMAAABqAAAAgQAAAJgAAACvAAAAxgAAAN0AAAD0AAAACwEAACIBAAA5AQAAUAEAAGcBAAB+AQAAlQEAAKwBAADDAQAA",
        "2gEAAPEBAAAIAgAAHwIAADYCAABNAgAAZAIAAHsCAACSAgAAqQIAAKAHACDXrnLSDl/Y75lH6Dks2P///ABHemBJzXe1TW5S",
        "nQcAODNVkKJ6qnS+mep2aUm3//98AE3IUDi788eUgWwHAOiu9TaKohU7U1D1fUEBA////AAbKqGvni2wGVpdGQcAvJAo35sG",
        "OBASmb21189g///8AFxXZddWyudwIiGlBwBFsLq9H+Kxm1fzswFASrX///wAlHep2lrPsJJZEpQHALqORvuNECAWHp+zy73X",
        "VP///ABYS2HNRrrReCIrmQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMCivRW04AAAAAAAAAACgBSqqqqqqqqgAAAAAAAAA",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwAAAAAAACgBSqqqqqqqqgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "AAAAAAAAAACgAT///wASNhISNhISEhISEgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAKBVBSq+q+oJ",
        "DBMW+q+q+hseJSgvMq+r6r45PEFES06r6r6rVVhfYmnqvqvqbHN2fYCAAAAAVQEAGAGAYAYAYAYBgBgBgBgBgBgBgFUBABgB",
        "gGAGAGAGAYAYAYAYAYAYAYBVAQAYAYBgBgBgBgGAGAGAGAGAGAGAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "oAIM/M/MzMwAABYWAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAoP0BN/38zwlsYXib1Y1aWDsgAbjXnr8B",
        "DLR2MAGw344AjACj9gEtx3hzAYf66WgB9KJ22gFI3nYJA2xh56kBZEbm3wAAAGn1of0BM/z8z0InGYQqGQ8KCnA+ACMZdD4A",
        "JReEKgAZDwoACgBwOAAhFYQwABsT+DoAIxWENAAdFYRCACcZ+DoAIRf3zwAAAD4lF/8BAFJYuRFiteQI4IxIrUUdx7+ZW1jZ",
        "kaurblFZ1fcvG5EcgzxIUHp1XnkfmpzCRP7Eh4Sipz2Y7FTl/+e0oDTLXv8AAAC91Cg6/gED/+8/2PkaIzEVNh0eLRUOKwEx",
        "FQ42AR8PCh4JAQoEAysRDC4DIxEMQQMrFQwyAycTDCwFMxUOLwMvEwwq/wAAAAMxEw79AR/z/z1N2pTebjgZaSQjJaBSJ/4A",
        "Ox0QSAErEwoqDQAOCgCQSN5GAEMf7+cBi0PbVQBNJRReAU3alKQBwJ+tin8AAAD3hEH+AQD/7w/4+QqgMRk+Hw8eLxUMQwAv",
        "Fww8AB8PCB4JAAoKACsVDDYAIxEITwArFww4ACcTCjIAMRkMPQAtFQo6PwAAAC8VCgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "AAAAAAA+QjjupOAAAAAAAKBVAAAgAAgCAIAAIAgCAIAgCAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "AAAAAAAAAACgqqoCAjEjBEp8ErQoOfdFPv52TRAPyavyMSOHTCMWEjh058ErQoOfdFPv52TcmhAPvyMSMESCMSMWh0wjEjh0",
        "wjEjBEgjEjBEgjEjh0wjEjh0AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMAAAAAAAAAAA==",
    ),
};
