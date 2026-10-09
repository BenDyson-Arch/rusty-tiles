use super::{invalid, Coordinates, SubdivisionScheme, Subtree, TileMetadata};
use crate::Error;
use serde_json::{json, Value};
use std::{
    collections::{btree_map::Entry, BTreeMap, BTreeSet},
    path::Path,
};

const LEVELS: u32 = 4;

fn content_suffix(uri: &str) -> Result<String, Error> {
    let suffix = Path::new(uri)
        .extension()
        .and_then(|s| s.to_str())
        .ok_or_else(|| invalid("content has no extension"))?
        .to_ascii_lowercase();
    if !matches!(suffix.as_str(), "glb" | "gltf" | "b3dm") {
        return Err(invalid("implicit converter content must be glTF or b3dm"));
    }
    Ok(suffix)
}

fn key(c: Coordinates, scheme: SubdivisionScheme) -> String {
    if scheme == SubdivisionScheme::Octree {
        format!("{}-{}-{}-{}", c.level, c.x, c.y, c.z)
    } else {
        format!("{}-{}-{}", c.level, c.x, c.y)
    }
}

fn child(c: Coordinates, slot: usize, scheme: SubdivisionScheme) -> Result<Coordinates, Error> {
    if c.level >= 31 || slot >= scheme.branches() {
        return Err(invalid("hierarchy exceeds implicit coordinate limits"));
    }
    Ok(Coordinates {
        level: c.level + 1,
        x: c.x * 2 + (slot as u32 & 1),
        y: c.y * 2 + ((slot as u32 >> 1) & 1),
        z: if scheme == SubdivisionScheme::Octree {
            c.z * 2 + ((slot as u32 >> 2) & 1)
        } else {
            0
        },
    })
}

fn local(c: Coordinates, root: Coordinates) -> Coordinates {
    let level = c.level - root.level;
    Coordinates {
        level,
        x: c.x - (root.x << level),
        y: c.y - (root.y << level),
        z: c.z - (root.z << level),
    }
}

fn ancestor(c: Coordinates, level: u32) -> Coordinates {
    let shift = c.level - level;
    Coordinates {
        level,
        x: c.x >> shift,
        y: c.y >> shift,
        z: c.z >> shift,
    }
}

fn contents(node: &Value) -> Vec<Value> {
    if let Some(values) = node["contents"].as_array() {
        values.clone()
    } else {
        node.get("content").cloned().into_iter().collect()
    }
}

#[derive(Clone)]
struct Tile {
    coordinates: Coordinates,
    delta: [f64; 3],
    metadata: TileMetadata,
    contents: Vec<Value>,
}

fn collect(
    node: &Value,
    coordinates: Coordinates,
    delta: [f64; 3],
    scheme: SubdivisionScheme,
    out: &mut Vec<Tile>,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<(), Error> {
    checkpoint()?;
    let mut bounds: [f64; 12] = serde_json::from_value(node["boundingVolume"]["box"].clone())?;
    for i in 0..3 {
        bounds[i] += delta[i];
    }
    let mut extras = node.get("extras").cloned().unwrap_or_else(|| json!({}));
    if let Some(extras) = extras.as_object_mut() {
        extras.remove("implicitChildIndex");
    }
    out.push(Tile {
        coordinates,
        delta,
        metadata: TileMetadata {
            bounding_box: bounds,
            geometric_error: node["geometricError"]
                .as_f64()
                .ok_or_else(|| invalid("missing geometric error"))?,
            extras,
        },
        contents: contents(node),
    });
    let mut slots = BTreeSet::new();
    for (index, node) in node["children"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let slot = node["extras"]["implicitChildIndex"]
            .as_u64()
            .unwrap_or(index as u64) as usize;
        if !slots.insert(slot) {
            return Err(invalid("duplicate child cell"));
        }
        let mut offset = delta;
        if node.get("transform").is_some() {
            let matrix: [f64; 16] = serde_json::from_value(node["transform"].clone())?;
            for (i, value) in matrix.iter().enumerate().take(12) {
                if *value != if i % 5 == 0 { 1. } else { 0. } {
                    return Err(invalid("child transform must be a translation"));
                }
            }
            for i in 0..3 {
                offset[i] += matrix[12 + i];
            }
        }
        collect(
            node,
            child(coordinates, slot, scheme)?,
            offset,
            scheme,
            out,
            checkpoint,
        )?;
    }
    Ok(())
}

fn translate_content(bytes: &[u8], delta: [f64; 3]) -> Result<Vec<u8>, Error> {
    let b3dm = bytes.starts_with(b"b3dm");
    if delta == [0.; 3] && (!b3dm || bytes.len().is_multiple_of(8)) {
        return Ok(bytes.to_vec());
    }
    let offset = if b3dm {
        if bytes.len() < 28 {
            return Err(invalid("truncated b3dm"));
        }
        (3..7)
            .try_fold(28usize, |offset, i| {
                offset.checked_add(
                    u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()) as usize,
                )
            })
            .ok_or_else(|| invalid("b3dm table size overflow"))?
    } else {
        0
    };
    let source = gltf::Glb::from_slice(
        bytes
            .get(offset..)
            .ok_or_else(|| invalid("b3dm tables exceed content"))?,
    )?;
    let mut doc: Value = serde_json::from_slice(&source.json)?;
    let mut nodes = doc["nodes"]
        .as_array()
        .cloned()
        .ok_or_else(|| invalid("GLB has no nodes"))?;
    let mut roots = BTreeSet::new();
    for scene in doc["scenes"]
        .as_array()
        .ok_or_else(|| invalid("GLB has no scenes"))?
    {
        for index in scene["nodes"]
            .as_array()
            .ok_or_else(|| invalid("scene has no root nodes"))?
        {
            roots.insert(integer(index, "node index")?);
        }
    }
    let translation = [delta[0], delta[2], -delta[1]];
    for index in roots {
        let node = nodes
            .get_mut(index)
            .ok_or_else(|| invalid("invalid scene root node"))?;
        if let Some(matrix) = node.get_mut("matrix") {
            let mut m: [f64; 16] = serde_json::from_value(matrix.clone())?;
            for i in 0..3 {
                m[12 + i] += translation[i];
            }
            *matrix = json!(m);
        } else {
            let mut t: [f64; 3] = node
                .get("translation")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?
                .unwrap_or([0.; 3]);
            for i in 0..3 {
                t[i] += translation[i];
            }
            node["translation"] = json!(t);
        }
    }
    doc["nodes"] = Value::Array(nodes);
    let mut glb = crate::glb::encode_glb(&doc, source.bin.as_deref().unwrap_or(&[]))?;
    if !b3dm {
        return Ok(glb);
    }
    crate::glb::align_glb_eight(&mut glb)?;
    let mut result = bytes[..offset].to_vec();
    result.extend_from_slice(&glb);
    let size = u32::try_from(result.len()).map_err(|_| invalid("b3dm exceeds format limit"))?;
    result[8..12].copy_from_slice(&size.to_le_bytes());
    Ok(result)
}

/// Encode a converter's hierarchy into implicit coordinates. Semantic metadata
/// preserves actual tile boxes and errors; child translations move into GLB scene
/// nodes without changing attribute, texture, or feature metadata buffers.
pub(crate) fn write_tileset(
    manifest: &mut Value,
    directory: &Path,
    scheme: SubdivisionScheme,
    retain_sources: bool,
) -> Result<(), Error> {
    write_tileset_recorded(manifest, directory, scheme, retain_sources, &mut || Ok(())).map(drop)
}

pub(crate) fn write_tileset_recorded(
    manifest: &mut Value,
    directory: &Path,
    scheme: SubdivisionScheme,
    retain_sources: bool,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<Vec<String>, Error> {
    checkpoint()?;
    let mut members = Vec::new();
    let mut tiles = Vec::new();
    collect(
        &manifest["root"],
        Coordinates {
            level: 0,
            x: 0,
            y: 0,
            z: 0,
        },
        [0.; 3],
        scheme,
        &mut tiles,
        checkpoint,
    )?;
    let mut slot_keys = BTreeSet::new();
    for tile in &tiles {
        for (slot, content) in tile.contents.iter().enumerate() {
            let uri = content["uri"]
                .as_str()
                .ok_or_else(|| invalid("content has no URI"))?;
            slot_keys.insert((slot, content_suffix(uri)?));
        }
    }
    let slot_keys: Vec<_> = slot_keys.into_iter().collect();
    let headers: Vec<_> = slot_keys
        .iter()
        .map(|(index, suffix)| {
            tiles
                .iter()
                .find_map(|tile| {
                    let content = tile.contents.get(*index)?;
                    (content_suffix(content["uri"].as_str()?).ok()?.as_str() == suffix)
                        .then(|| content.clone())
                })
                .unwrap()
        })
        .collect();
    // Metadata may enlarge a tile beyond its regular cell. Cesium must cull an
    // unloaded child subtree using that cell, including during a narrow pick.
    // External tileset roots expose the actual boundary box before that cull.
    let root_box = tiles[0].metadata.bounding_box;
    if tiles.iter().any(|tile| {
        tile.coordinates.level > 0
            && tile.coordinates.level.is_multiple_of(LEVELS)
            && crate::bbox::Obb::from_box(tile.metadata.bounding_box)
                .corners()
                .iter()
                .any(|&corner| {
                    !crate::bbox::Obb::from_box(derived_box(root_box, tile.coordinates, scheme))
                        .contains(corner, 1e-6)
                })
    }) {
        return write_chunks(
            manifest,
            directory,
            scheme,
            &tiles,
            (&slot_keys, &headers),
            retain_sources,
            checkpoint,
        );
    }
    let template = if scheme == SubdivisionScheme::Octree {
        "{level}-{x}-{y}-{z}"
    } else {
        "{level}-{x}-{y}"
    };
    let templates: Vec<_> = slot_keys
        .iter()
        .enumerate()
        .map(|(slot, (_, suffix))| {
            let mut content = headers[slot].clone();
            content["uri"] = json!(format!("implicit-content/{template}-{slot}.{suffix}"));
            content
        })
        .collect();
    std::fs::create_dir_all(directory.join("implicit-content"))?;
    std::fs::create_dir_all(directory.join("subtrees"))?;
    let mut trees: BTreeMap<Coordinates, Subtree> = BTreeMap::new();
    let mut source_names = BTreeSet::new();
    let available_levels = tiles
        .iter()
        .map(|tile| tile.coordinates.level + 1)
        .max()
        .unwrap();
    for mut tile in tiles {
        checkpoint()?;
        let root = ancestor(tile.coordinates, tile.coordinates.level / LEVELS * LEVELS);
        let mut flags = vec![false; slot_keys.len()];
        let mut bytes_total = 0usize;
        for (index, content) in tile.contents.iter().enumerate() {
            let uri = content["uri"].as_str().unwrap();
            let suffix = content_suffix(uri)?;
            let slot = slot_keys
                .iter()
                .position(|(i, s)| *i == index && s == &suffix)
                .unwrap();
            let bytes = translate_content(&std::fs::read(directory.join(uri))?, tile.delta)?;
            bytes_total += bytes.len();
            let name = format!(
                "implicit-content/{}-{slot}.{suffix}",
                key(tile.coordinates, scheme)
            );
            std::fs::write(directory.join(&name), bytes)?;
            members.push(name);
            source_names.insert(uri.to_owned());
            flags[slot] = true;
        }
        if tile.metadata.extras.get("encodedBytes").is_some() {
            tile.metadata.extras["encodedBytes"] = json!(bytes_total);
        }
        if let Entry::Vacant(entry) = trees.entry(root) {
            entry.insert(Subtree::new(scheme, LEVELS, slot_keys.len())?);
        }
        let tree = trees.get_mut(&root).unwrap();
        tree.set_tile(local(tile.coordinates, root), &flags)?;
        tree.set_metadata(local(tile.coordinates, root), tile.metadata)?;
        if tile.coordinates == root && root.level > 0 {
            let parent = ancestor(root, root.level - LEVELS);
            if let Entry::Vacant(entry) = trees.entry(parent) {
                entry.insert(Subtree::new(scheme, LEVELS, slot_keys.len())?);
            }
            trees
                .get_mut(&parent)
                .unwrap()
                .set_child_subtree(local(root, parent))?;
        }
    }
    for (coordinates, tree) in trees {
        checkpoint()?;
        let name = format!("subtrees/{}.subtree", key(coordinates, scheme));
        std::fs::write(directory.join(&name), tree.to_bytes()?)?;
        members.push(name);
    }
    let root = manifest["root"].as_object_mut().unwrap();
    root.remove("children");
    root.remove("content");
    root.remove("contents");
    if templates.len() == 1 {
        root.insert("content".into(), templates[0].clone());
    } else if !templates.is_empty() {
        root.insert("contents".into(), json!(templates));
    }
    root.insert("implicitTiling".into(),json!({"subdivisionScheme":scheme.as_str(),"subtreeLevels":LEVELS,"availableLevels":available_levels,"subtrees":{"uri":format!("subtrees/{template}.subtree")}}));
    manifest["schema"] = schema();
    if retain_sources {
        manifest["extras"]["rustyTilesSourceContents"] = json!(source_names);
    } else {
        for uri in source_names {
            checkpoint()?;
            std::fs::remove_file(directory.join(uri))?;
        }
    }
    Ok(members)
}

fn derived_box(root: [f64; 12], c: Coordinates, scheme: SubdivisionScheme) -> [f64; 12] {
    let mut bounds = root;
    let width = 2f64.powi(c.level as i32);
    for axis in 0..if scheme == SubdivisionScheme::Octree {
        3
    } else {
        2
    } {
        let value = [c.x, c.y, c.z][axis] as f64;
        for lane in 0..3 {
            bounds[lane] += root[3 + axis * 3 + lane] * ((2. * value + 1.) / width - 1.);
            bounds[3 + axis * 3 + lane] /= width;
        }
    }
    bounds
}

fn schema() -> Value {
    serde_json::to_value(crate::metadata::tile_schema()).expect("tile schema serializes")
}

fn chunk_document(
    root: &Tile,
    scheme: SubdivisionScheme,
    slots: &[(usize, String)],
    headers: &[Value],
    links: bool,
    available: u32,
) -> Value {
    let prefix = key(root.coordinates, scheme);
    let template = if scheme == SubdivisionScheme::Octree {
        "{level}-{x}-{y}-{z}"
    } else {
        "{level}-{x}-{y}"
    };
    let mut templates: Vec<_> = slots
        .iter()
        .enumerate()
        .map(|(slot, (_, suffix))| {
            let mut content = headers[slot].clone();
            content["uri"] = json!(format!(
                "implicit-content/{prefix}-{template}-{slot}.{suffix}"
            ));
            content
        })
        .collect();
    if links {
        templates.push(json!({"uri":format!("implicit-tileset-{prefix}-{template}.json")}));
    }
    let mut tile = json!({"boundingVolume":{"box":root.metadata.bounding_box},"geometricError":root.metadata.geometric_error,
        "refine":"REPLACE","extras":root.metadata.extras,
        "implicitTiling":{"subdivisionScheme":scheme.as_str(),"subtreeLevels":LEVELS,"availableLevels":available,
            "subtrees":{"uri":format!("subtrees/{prefix}-{template}.subtree")}}});
    if templates.len() == 1 {
        tile["content"] = templates.remove(0);
    } else if !templates.is_empty() {
        tile["contents"] = json!(templates);
    }
    let mut document = json!({"asset":{"version":"1.1"},"schema":schema(),"geometricError":root.metadata.geometric_error,"root":tile});
    let extensions: BTreeSet<_> = headers
        .iter()
        .filter_map(|content| content["extensions"].as_object())
        .flat_map(|extensions| extensions.keys().cloned())
        .collect();
    if !extensions.is_empty() {
        document["extensionsUsed"] = json!(extensions);
    }
    document
}

fn write_chunks(
    manifest: &mut Value,
    directory: &Path,
    scheme: SubdivisionScheme,
    tiles: &[Tile],
    content_layout: (&[(usize, String)], &[Value]),
    retain_sources: bool,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<Vec<String>, Error> {
    let (slots, headers) = content_layout;
    let mut members = Vec::new();
    checkpoint()?;
    let stride = LEVELS - 1;
    let parents: BTreeSet<_> = tiles
        .iter()
        .filter(|t| t.coordinates.level > 0)
        .map(|t| ancestor(t.coordinates, t.coordinates.level - 1))
        .collect();
    let mut groups: BTreeMap<Coordinates, Vec<(Tile, bool)>> = BTreeMap::new();
    for tile in tiles {
        let c = tile.coordinates;
        let previous = ancestor(c, c.level.saturating_sub(1) / stride * stride);
        let link = c.level > 0 && c.level.is_multiple_of(stride) && parents.contains(&c);
        groups
            .entry(previous)
            .or_default()
            .push((tile.clone(), link));
        if link {
            groups.entry(c).or_default().push((tile.clone(), false));
        }
    }
    std::fs::create_dir_all(directory.join("implicit-content"))?;
    std::fs::create_dir_all(directory.join("subtrees"))?;
    let mut source_names = BTreeSet::new();
    for (root, tiles) in groups {
        checkpoint()?;
        let prefix = key(root, scheme);
        let links = tiles.iter().any(|(_, link)| *link);
        let count = slots.len() + usize::from(links);
        let mut tree = Subtree::new(scheme, LEVELS, count)?;
        let first = tiles
            .iter()
            .find(|(t, _)| t.coordinates == root)
            .unwrap()
            .0
            .clone();
        let available = tiles
            .iter()
            .map(|(t, _)| t.coordinates.level - root.level + 1)
            .max()
            .unwrap();
        for (mut tile, link) in tiles {
            checkpoint()?;
            let coord = local(tile.coordinates, root);
            let mut flags = vec![false; count];
            if link {
                flags[slots.len()] = true;
            } else {
                let mut size = 0;
                for (index, content) in tile.contents.iter().enumerate() {
                    let uri = content["uri"].as_str().unwrap();
                    let suffix = content_suffix(uri)?;
                    let slot = slots
                        .iter()
                        .position(|(i, s)| *i == index && s == &suffix)
                        .unwrap();
                    let bytes =
                        translate_content(&std::fs::read(directory.join(uri))?, tile.delta)?;
                    size += bytes.len();
                    let name = format!(
                        "implicit-content/{prefix}-{}-{slot}.{suffix}",
                        key(coord, scheme)
                    );
                    std::fs::write(directory.join(&name), bytes)?;
                    members.push(name);
                    source_names.insert(uri.to_owned());
                    flags[slot] = true;
                }
                if tile.metadata.extras.get("encodedBytes").is_some() {
                    tile.metadata.extras["encodedBytes"] = json!(size);
                }
            }
            tree.set_tile(coord, &flags)?;
            tree.set_metadata(coord, tile.metadata)?;
        }
        checkpoint()?;
        let name = format!(
            "subtrees/{prefix}-{}.subtree",
            key(
                Coordinates {
                    level: 0,
                    x: 0,
                    y: 0,
                    z: 0
                },
                scheme
            )
        );
        std::fs::write(directory.join(&name), tree.to_bytes()?)?;
        members.push(name);
        let document = chunk_document(&first, scheme, slots, headers, links, available);
        if root.level == 0 {
            let transform = manifest["root"].get("transform").cloned();
            manifest["root"] = document["root"].clone();
            if let Some(transform) = transform {
                manifest["root"]["transform"] = transform;
            }
            manifest["schema"] = schema();
        } else {
            let parent = ancestor(root, root.level - stride);
            let name = format!(
                "implicit-tileset-{}-{}.json",
                key(parent, scheme),
                key(local(root, parent), scheme)
            );
            checkpoint()?;
            std::fs::write(directory.join(&name), serde_json::to_vec(&document)?)?;
            members.push(name);
        }
    }
    if retain_sources {
        manifest["extras"]["rustyTilesSourceContents"] = json!(source_names);
    } else {
        for name in source_names {
            checkpoint()?;
            std::fs::remove_file(directory.join(name))?;
        }
    }
    Ok(members)
}

/// Expand implicit converter output for audits and built-in validation. Resource
/// reads go through the caller, so archive paths and size limits remain enforced.
pub fn expand_tileset(
    manifest: &Value,
    mut read: impl FnMut(&str) -> Result<Vec<u8>, Error>,
) -> Result<Value, Error> {
    expand_tileset_bounded(
        manifest,
        &mut read,
        &mut ExpansionBudget::new(1_000_000, 1024 * 1024 * 1024),
    )
}

/// Caller-supplied admission for structural presentation, not an addressing proof.
pub(crate) fn expand_tileset_bounded(
    manifest: &Value,
    mut read: impl FnMut(&str) -> Result<Vec<u8>, Error>,
    budget: &mut ExpansionBudget,
) -> Result<Value, Error> {
    expand_document(manifest, &mut read, &mut BTreeSet::new(), 0, budget)
}

pub(crate) struct ExpansionBudget {
    nodes: usize,
    bytes: usize,
    max_nodes: usize,
    max_bytes: usize,
    parse_json: fn(&[u8]) -> Result<Value, Error>,
}
impl ExpansionBudget {
    /// Select JSON admission for every document interpreted during expansion.
    pub(crate) fn with_json_parser(mut self, parser: fn(&[u8]) -> Result<Value, Error>) -> Self {
        self.parse_json = parser;
        self
    }
    pub(crate) fn new(max_nodes: usize, max_bytes: usize) -> Self {
        Self {
            nodes: 0,
            bytes: 0,
            max_nodes,
            max_bytes,
            parse_json: |bytes| Ok(serde_json::from_slice(bytes)?),
        }
    }
}
fn expansion_limit(message: &str) -> Error {
    Error::Validation(crate::validate::ValidationFailure::ResourceLimit(
        message.into(),
    ))
}

fn expand_document<F: FnMut(&str) -> Result<Vec<u8>, Error>>(
    manifest: &Value,
    read: &mut F,
    active: &mut BTreeSet<String>,
    depth: usize,
    budget: &mut ExpansionBudget,
) -> Result<Value, Error> {
    if depth > 32 {
        return Err(invalid("implicit tileset chain exceeds 32 roots"));
    }
    if manifest["root"].get("implicitTiling").is_none() {
        return Ok(manifest.clone());
    }
    let root = &manifest["root"];
    if root.get("children").is_some() {
        return Err(invalid("implicit root cannot have explicit children"));
    }
    let tiling = &root["implicitTiling"];
    let scheme = match tiling["subdivisionScheme"].as_str() {
        Some("OCTREE") => SubdivisionScheme::Octree,
        Some("QUADTREE") => SubdivisionScheme::Quadtree,
        _ => return Err(invalid("unsupported subdivisionScheme")),
    };
    let levels = u32::try_from(integer(&tiling["subtreeLevels"], "subtreeLevels")?)
        .map_err(|_| invalid("invalid subtreeLevels"))?;
    let available = u32::try_from(integer(&tiling["availableLevels"], "availableLevels")?)
        .map_err(|_| invalid("invalid availableLevels"))?;
    if available == 0 || available > 32 {
        return Err(invalid("availableLevels must be between 1 and 32"));
    }
    // Apply the same bounded allocation policy as the writer before parsing.
    let templates = contents(root);
    let slots = scheme
        .branches()
        .checked_pow(levels)
        .filter(|&n| n <= budget.max_nodes)
        .ok_or_else(|| expansion_limit("implicit subtree exceeds admitted node capacity"))?;
    slots
        .checked_mul(templates.len().saturating_add(2))
        .filter(|&n| n <= budget.max_bytes)
        .ok_or_else(|| expansion_limit("implicit availability exceeds admitted allocation"))?;
    let bounded = Subtree::new(scheme, levels, templates.len())?;
    let tile_count = bounded.tiles.len();
    let child_count = bounded.children.len();
    let subtree_template = tiling["subtrees"]["uri"]
        .as_str()
        .ok_or_else(|| invalid("missing subtree URI template"))?;
    let root_box: [f64; 12] = serde_json::from_value(root["boundingVolume"]["box"].clone())?;
    let root_error = root["geometricError"]
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.)
        .ok_or_else(|| invalid("root error must be finite and nonnegative"))?;
    if root_box.iter().any(|v| !v.is_finite()) {
        return Err(invalid("root box must be finite"));
    }
    let metadata_required = manifest["schema"]["classes"].get("rustyTile").is_some();
    if metadata_required {
        let properties = &manifest["schema"]["classes"]["rustyTile"]["properties"];
        let bounds = &properties["boundingBox"];
        let error = &properties["geometricError"];
        if bounds["type"] != "SCALAR"
            || bounds["componentType"] != "FLOAT64"
            || bounds["array"] != true
            || bounds["count"] != 12
            || bounds["semantic"] != "TILE_BOUNDING_BOX"
            || error["type"] != "SCALAR"
            || error["componentType"] != "FLOAT64"
            || error["semantic"] != "TILE_GEOMETRIC_ERROR"
            || properties["extras"]["type"] != "STRING"
        {
            return Err(invalid("unsupported tile metadata schema"));
        }
    }
    let mut reader = Reader {
        read,
        scheme,
        levels,
        available,
        tile_count,
        child_count,
        templates,
        subtree_template,
        root_box,
        root_error,
        metadata_required,
        cache: BTreeMap::new(),
        budget,
        cache_bytes: 0,
    };
    let mut expanded = reader.node(Coordinates {
        level: 0,
        x: 0,
        y: 0,
        z: 0,
    })?;
    if metadata_required {
        let bounds: [f64; 12] = serde_json::from_value(expanded["boundingVolume"]["box"].clone())?;
        let placeholder = crate::bbox::Obb::from_box(root_box);
        if crate::bbox::Obb::from_box(bounds)
            .corners()
            .iter()
            .any(|&corner| !placeholder.contains(corner, 1e-6))
            || expanded["geometricError"].as_f64().unwrap() > root_error
        {
            return Err(invalid(
                "semantic root bounds or error exceed the implicit placeholder",
            ));
        }
    }
    expand_links(&mut expanded, reader.read, active, depth, reader.budget)?;
    if let Some(transform) = root.get("transform") {
        expanded["transform"] = transform.clone();
    }
    expanded["refine"] = root
        .get("refine")
        .cloned()
        .unwrap_or_else(|| json!("REPLACE"));
    let mut result = manifest.clone();
    result["root"] = expanded;
    Ok(result)
}

fn expand_links<F: FnMut(&str) -> Result<Vec<u8>, Error>>(
    node: &mut Value,
    read: &mut F,
    active: &mut BTreeSet<String>,
    depth: usize,
    budget: &mut ExpansionBudget,
) -> Result<(), Error> {
    if let Some(uri) = node["content"]["uri"]
        .as_str()
        .filter(|uri| uri.starts_with("implicit-tileset-") && uri.ends_with(".json"))
    {
        let uri = uri.to_owned();
        if uri.contains(['/', '\\', ':']) || !active.insert(uri.clone()) {
            return Err(invalid("invalid or cyclic implicit tileset link"));
        }
        let doc = (budget.parse_json)(&read(&uri)?)?;
        if doc["asset"]["version"] != "1.1"
            || doc["root"].get("implicitTiling").is_none()
            || doc["root"].get("transform").is_some()
            || doc["root"]["boundingVolume"] != node["boundingVolume"]
            || doc["root"]["geometricError"] != node["geometricError"]
        {
            return Err(invalid(
                "implicit tileset link changes boundary bounds, error, or placement",
            ));
        }
        let mut expanded = expand_document(&doc, read, active, depth + 1, budget)?;
        *node = expanded["root"].take();
        active.remove(&uri);
    } else if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            expand_links(child, read, active, depth, budget)?;
        }
    }
    Ok(())
}

fn integer(value: &Value, label: &str) -> Result<usize, Error> {
    value
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| invalid(&format!("invalid {label}")))
}

fn template(uri: &str, c: Coordinates, scheme: SubdivisionScheme) -> Result<String, Error> {
    if scheme == SubdivisionScheme::Quadtree && uri.contains("{z}") {
        return Err(invalid("quadtree URI contains {z}"));
    }
    let result = uri
        .replace("{level}", &c.level.to_string())
        .replace("{x}", &c.x.to_string())
        .replace("{y}", &c.y.to_string())
        .replace("{z}", &c.z.to_string());
    if result.contains(['{', '}']) {
        return Err(invalid("unsupported URI template expression"));
    }
    Ok(result)
}

struct Parsed {
    tiles: Vec<bool>,
    contents: Vec<Vec<bool>>,
    children: Vec<bool>,
    metadata: BTreeMap<usize, TileMetadata>,
}

fn parse(
    bytes: &[u8],
    tile_count: usize,
    child_count: usize,
    content_count: usize,
    branches: usize,
    metadata_required: bool,
    parse_json: fn(&[u8]) -> Result<Value, Error>,
) -> Result<Parsed, Error> {
    if bytes.len() < 24 || &bytes[..4] != b"subt" || bytes[4..8] != 1u32.to_le_bytes() {
        return Err(invalid("invalid binary subtree header"));
    }
    let length = |start| {
        usize::try_from(u64::from_le_bytes(
            bytes[start..start + 8].try_into().unwrap(),
        ))
        .map_err(|_| invalid("subtree length exceeds platform limits"))
    };
    let json_len = length(8)?;
    let bin_len = length(16)?;
    let json_end = 24usize
        .checked_add(json_len)
        .ok_or_else(|| invalid("subtree length overflow"))?;
    let total = json_end
        .checked_add(bin_len)
        .ok_or_else(|| invalid("subtree length overflow"))?;
    if json_len == 0 || json_len % 8 != 0 || bin_len % 8 != 0 || total != bytes.len() {
        return Err(invalid("invalid subtree chunk lengths or alignment"));
    }
    let doc = parse_json(&bytes[24..json_end])?;
    let binary = &bytes[json_end..];
    let buffer_length = if binary.is_empty() {
        0
    } else {
        let buffers = doc["buffers"]
            .as_array()
            .filter(|b| b.len() == 1 && b[0].get("uri").is_none())
            .ok_or_else(|| invalid("only one internal subtree buffer is supported"))?;
        let len = integer(&buffers[0]["byteLength"], "buffer length")?;
        if len > binary.len() || binary.len() - len >= 8 || binary[len..].iter().any(|&v| v != 0) {
            return Err(invalid("invalid binary buffer padding"));
        }
        len
    };
    let view = |index: &Value| -> Result<&[u8], Error> {
        let index = integer(index, "buffer view index")?;
        let view = doc["bufferViews"]
            .get(index)
            .ok_or_else(|| invalid("missing buffer view"))?;
        if view["buffer"] != 0 {
            return Err(invalid("invalid buffer index"));
        }
        let offset = view
            .get("byteOffset")
            .map(|v| integer(v, "buffer view offset"))
            .transpose()?
            .unwrap_or(0);
        let len = integer(&view["byteLength"], "buffer view length")?;
        let end = offset
            .checked_add(len)
            .ok_or_else(|| invalid("buffer view overflows"))?;
        if offset % 8 != 0 || len == 0 || end > buffer_length {
            return Err(invalid("buffer view is unaligned or outside its buffer"));
        }
        Ok(&binary[offset..end])
    };
    let availability = |value: &Value, len: usize| -> Result<Vec<bool>, Error> {
        let bits = if let Some(constant) = value.get("constant") {
            if value.get("bitstream").is_some() || !matches!(constant.as_u64(), Some(0 | 1)) {
                return Err(invalid("invalid availability constant"));
            }
            vec![constant == 1; len]
        } else {
            let bytes = view(&value["bitstream"])?;
            if bytes.len() != len.div_ceil(8) {
                return Err(invalid("availability bitstream has the wrong length"));
            }
            for index in len..bytes.len() * 8 {
                if bytes[index / 8] & (1 << (index % 8)) != 0 {
                    return Err(invalid("nonzero trailing availability bits"));
                }
            }
            (0..len)
                .map(|i| bytes[i / 8] & (1 << (i % 8)) != 0)
                .collect()
        };
        if let Some(count) = value.get("availableCount") {
            if integer(count, "availableCount")? != bits.iter().filter(|&&v| v).count() {
                return Err(invalid("incorrect availableCount"));
            }
        }
        Ok(bits)
    };
    for index in 0..doc["bufferViews"].as_array().map_or(0, Vec::len) {
        view(&json!(index))?;
    }
    let tiles = availability(&doc["tileAvailability"], tile_count)?;
    let children = availability(&doc["childSubtreeAvailability"], child_count)?;
    if !tiles[0] {
        return Err(invalid("root tile is unavailable"));
    }
    let mut width = branches;
    let mut offset = 1;
    let mut parent_offset = 0;
    while offset < tile_count {
        for index in 0..width {
            if tiles[offset + index] && !tiles[parent_offset + index / branches] {
                return Err(invalid("tile has an unavailable parent"));
            }
        }
        parent_offset = offset;
        offset += width;
        width *= branches;
    }
    for (index, &available) in children.iter().enumerate() {
        if available && !tiles[parent_offset + index / branches] {
            return Err(invalid("child subtree has an unavailable parent"));
        }
    }
    let slots = doc["contentAvailability"].as_array();
    if slots.map_or(0, |s| s.len()) != content_count {
        return Err(invalid(
            "content availability count differs from URI templates",
        ));
    }
    let mut contents = Vec::new();
    for slot in slots.into_iter().flatten() {
        let bits = availability(slot, tile_count)?;
        if bits.iter().zip(&tiles).any(|(&c, &t)| c && !t) {
            return Err(invalid("content belongs to an unavailable tile"));
        }
        contents.push(bits);
    }
    let mut metadata = BTreeMap::new();
    if metadata_required && doc.get("tileMetadata").is_none() {
        return Err(invalid("missing semantic tile metadata"));
    }
    if let Some(table) = doc.get("tileMetadata") {
        if !metadata_required {
            return Err(invalid("unsupported tile metadata schema"));
        }
        let table = doc["propertyTables"]
            .get(integer(table, "tileMetadata")?)
            .ok_or_else(|| invalid("missing metadata table"))?;
        let rows = tiles.iter().filter(|&&v| v).count();
        if table["class"] != "rustyTile" || integer(&table["count"], "metadata count")? != rows {
            return Err(invalid("unsupported or inconsistent tile metadata table"));
        }
        let props = &table["properties"];
        if !matches!(
            props["extras"]["stringOffsetType"].as_str(),
            None | Some("UINT32")
        ) {
            return Err(invalid("unsupported metadata offset type"));
        }
        let boxes = view(&props["boundingBox"]["values"])?;
        let errors = view(&props["geometricError"]["values"])?;
        let strings = view(&props["extras"]["values"])?;
        let offsets = view(&props["extras"]["stringOffsets"])?;
        if boxes.len() != rows * 96 || errors.len() != rows * 8 || offsets.len() != (rows + 1) * 4 {
            return Err(invalid("invalid metadata column lengths"));
        }
        let mut previous = 0;
        for (row, index) in tiles
            .iter()
            .enumerate()
            .filter_map(|(i, &b)| b.then_some(i))
            .enumerate()
        {
            let start =
                u32::from_le_bytes(offsets[row * 4..row * 4 + 4].try_into().unwrap()) as usize;
            let end =
                u32::from_le_bytes(offsets[row * 4 + 4..row * 4 + 8].try_into().unwrap()) as usize;
            if start != previous || end < start || end > strings.len() {
                return Err(invalid("invalid metadata string offsets"));
            }
            previous = end;
            let bounds = std::array::from_fn(|lane| {
                f64::from_le_bytes(
                    boxes[row * 96 + lane * 8..row * 96 + lane * 8 + 8]
                        .try_into()
                        .unwrap(),
                )
            });
            let error = f64::from_le_bytes(errors[row * 8..row * 8 + 8].try_into().unwrap());
            if bounds.iter().any(|v| !v.is_finite()) || !error.is_finite() || error < 0. {
                return Err(invalid("invalid tile metadata values"));
            }
            let extras = parse_json(&strings[start..end])?;
            metadata.insert(
                index,
                TileMetadata {
                    bounding_box: bounds,
                    geometric_error: error,
                    extras,
                },
            );
        }
        if previous != strings.len() {
            return Err(invalid("unreferenced metadata string bytes"));
        }
    }
    Ok(Parsed {
        tiles,
        contents,
        children,
        metadata,
    })
}

struct Reader<'a, F> {
    read: &'a mut F,
    scheme: SubdivisionScheme,
    levels: u32,
    available: u32,
    tile_count: usize,
    child_count: usize,
    templates: Vec<Value>,
    subtree_template: &'a str,
    root_box: [f64; 12],
    root_error: f64,
    metadata_required: bool,
    cache: BTreeMap<Coordinates, Parsed>,
    budget: &'a mut ExpansionBudget,
    cache_bytes: usize,
}

impl<F: FnMut(&str) -> Result<Vec<u8>, Error>> Reader<'_, F> {
    fn node(&mut self, c: Coordinates) -> Result<Value, Error> {
        self.budget.nodes = self
            .budget
            .nodes
            .checked_add(1)
            .filter(|&n| n <= self.budget.max_nodes)
            .ok_or_else(|| expansion_limit("implicit expansion exceeds node limit"))?;
        let root = ancestor(c, c.level / self.levels * self.levels);
        if !self.cache.contains_key(&root) {
            let bytes = (self.read)(&template(self.subtree_template, root, self.scheme)?)?;
            self.cache_bytes = self
                .cache_bytes
                .checked_add(bytes.len())
                .filter(|&n| n <= self.budget.max_bytes)
                .ok_or_else(|| expansion_limit("implicit cache exceeds byte admission"))?;
            self.cache.insert(
                root,
                parse(
                    &bytes,
                    self.tile_count,
                    self.child_count,
                    self.templates.len(),
                    self.scheme.branches(),
                    self.metadata_required,
                    self.budget.parse_json,
                )?,
            );
        }
        let coord = local(c, root);
        let morton = match self.scheme {
            SubdivisionScheme::Quadtree => {
                morton_encoding::morton_encode([coord.y, coord.x]) as usize
            }
            SubdivisionScheme::Octree => {
                morton_encoding::morton_encode([coord.z, coord.y, coord.x]) as usize
            }
        };
        let index =
            (self.scheme.branches().pow(coord.level) - 1) / (self.scheme.branches() - 1) + morton;
        let tree = &self.cache[&root];
        if !tree.tiles[index] {
            return Err(invalid("unavailable tile was traversed"));
        }
        let width = 2f64.powi(c.level as i32);
        let bounds = derived_box(self.root_box, c, self.scheme);
        let metadata = tree.metadata.get(&index);
        let mut node = json!({"boundingVolume":{"box":metadata.map_or(bounds,|m|m.bounding_box)},"geometricError":metadata.map_or(self.root_error/width,|m|m.geometric_error),"refine":"REPLACE"});
        if let Some(metadata) = metadata {
            if metadata.extras != json!({}) {
                node["extras"] = metadata.extras.clone();
            }
        }
        let mut contents = Vec::new();
        for (slot, flags) in tree.contents.iter().enumerate() {
            if flags[index] {
                let mut content = self.templates[slot].clone();
                content["uri"] = json!(template(
                    content["uri"]
                        .as_str()
                        .ok_or_else(|| invalid("content template has no URI"))?,
                    c,
                    self.scheme
                )?);
                contents.push(content);
            }
        }
        if contents.len() == 1 {
            node["content"] = contents.remove(0);
        } else if !contents.is_empty() {
            node["contents"] = json!(contents);
        }
        // Charge this node before retaining child results; do not repeatedly
        // serialize its already-built descendants.
        let own_bytes = serde_json::to_vec(&node)?.len();
        self.budget.bytes = self
            .budget
            .bytes
            .checked_add(own_bytes)
            .filter(|&n| n <= self.budget.max_bytes)
            .ok_or_else(|| expansion_limit("implicit presentation exceeds byte limit"))?;
        let mut child_coords = Vec::new();
        for slot in 0..self.scheme.branches() {
            let next_level = coord.level + 1;
            let code = morton * self.scheme.branches() + slot;
            let exists = if next_level == self.levels {
                tree.children[code]
            } else {
                tree.tiles[(self.scheme.branches().pow(next_level) - 1)
                    / (self.scheme.branches() - 1)
                    + code]
            };
            if exists {
                if c.level + 1 >= self.available {
                    return Err(invalid("availability exceeds availableLevels"));
                }
                child_coords.push(child(c, slot, self.scheme)?);
            }
        }
        let children = child_coords
            .into_iter()
            .map(|c| self.node(c))
            .collect::<Result<Vec<_>, _>>()?;
        if !children.is_empty() {
            node["children"] = json!(children);
        }
        Ok(node)
    }
}

#[cfg(test)]
mod expansion_budget_tests {
    use super::*;

    // Literal subtree framing and constant availability, independent of the writer.
    fn subtree() -> Vec<u8> {
        let mut json = br#"{"tileAvailability":{"constant":1},"contentAvailability":[],"childSubtreeAvailability":{"constant":0}}"#.to_vec();
        while !json.len().is_multiple_of(8) {
            json.push(b' ');
        }
        let mut bytes = b"subt".to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&json);
        bytes
    }
    fn manifest(levels: u32) -> Value {
        json!({"asset":{"version":"1.1"},"geometricError":1,
            "root":{"boundingVolume":{"box":[0,0,0,1,0,0,0,1,0,0,0,1]},"geometricError":1,
                "implicitTiling":{"subdivisionScheme":"QUADTREE","subtreeLevels":levels,
                    "availableLevels":levels,"subtrees":{"uri":"{level}-{x}-{y}.subtree"}}}})
    }
    fn is_limit(result: Result<Value, Error>) -> bool {
        matches!(
            result,
            Err(Error::Validation(
                crate::validate::ValidationFailure::ResourceLimit(_)
            ))
        )
    }
    #[test]
    fn node_capacity_is_shared_across_documents() {
        let mut budget = ExpansionBudget::new(4, 4096);
        for _ in 0..4 {
            expand_tileset_bounded(&manifest(1), |_| Ok(subtree()), &mut budget).unwrap();
        }
        assert!(is_limit(expand_tileset_bounded(
            &manifest(1),
            |_| panic!("fail before another resource read"),
            &mut budget
        )));
    }
    #[test]
    fn byte_capacity_stops_child_presentation_and_large_subtree_before_reads() {
        let mut budget = ExpansionBudget::new(64, 300);
        let mut reads = 0;
        assert!(is_limit(expand_tileset_bounded(
            &manifest(2),
            |_| {
                reads += 1;
                Ok(subtree())
            },
            &mut budget
        )));
        assert_eq!(reads, 1);
        assert!(is_limit(expand_tileset_bounded(
            &manifest(16),
            |_| panic!("admission before resource read"),
            &mut ExpansionBudget::new(64, 4096)
        )));
    }
}
