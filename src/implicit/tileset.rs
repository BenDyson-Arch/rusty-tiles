//! The bounded implicit forest (P-IMPL design v3 §4) and its expander.
//!
//! The writer turns a producer's finished explicit hierarchy H into:
//! - an explicit skeleton (tiles whose subtree height is at least [`SKELETON_HEIGHT`]),
//!   packed breadth-first into `tileset.json` and explicit external documents
//!   `tileset-d{m}.json`, reached through explicit link tiles;
//! - explicit leaves (leaf children of skeleton tiles), published in place;
//! - implicit roots (non-leaf children of skeleton tiles, or the root of a short tree), each
//!   with exactly one subtree, `subtreeLevels == availableLevels`, no child subtrees and a
//!   TILE_BOUNDING_BOX / TILE_GEOMETRIC_ERROR row on every tile.
//!
//! No implicit root ever carries a JSON content template. Implicit payloads are rebased by
//! replacing only the GLB scene-root node object's text ([`rebase_payload`]), which
//! [`restore_payload`] inverts byte for byte.

use super::{invalid, Coordinates, SubdivisionScheme, Subtree, TileMetadata};
use crate::tileset_node::{sum_error, sum_up};
use crate::Error;
use serde::Deserialize;
use serde_json::{json, value::RawValue, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Component, Path},
};

/// S: a tile whose subtree height is at least S is an explicit skeleton tile (design §4.1).
const SKELETON_HEIGHT: u32 = 4;
/// P9: the writer refuses hierarchies with more levels than this.
const MAX_LEVELS: u32 = 32;
/// P8: value nodes per JSON document (validator semantics: the root, containers and scalars).
const DOC_NODES: usize = 16_384;
/// P8: compact serialized bytes per JSON document.
const DOC_BYTES: usize = 2 * 1024 * 1024;
/// Skeleton levels below a document root (review D-3; lowered from v3's 12).
///
/// A tile object at skeleton level k of a document sits at JSON depth 1 + 2k (document
/// object 0, `root` 1, then a `children` array and an element per level). A document holds
/// skeleton levels 0..=DOC_LEVELS, so unit children (explicit leaves, implicit roots and
/// links) sit at level DOC_LEVELS + 1, depth 1 + 2·11 = 23. The deepest producer member
/// below a tile object is `content.extensions.3DTILES_content_gltf_vector.vector` (+4), or
/// +5 inside a `contents` array, giving 27 and 28: at least 4 below [`DOC_DEPTH`] and 36
/// below the I1/C1 limit of 64. [`DOC_DEPTH`] is also a packing criterion, so deeper extras
/// link earlier instead of exceeding it.
const DOC_LEVELS: u32 = 10;
/// P8: JSON depth per document (the document object has depth 0).
const DOC_DEPTH: usize = 32;
/// Reserved in `tileset.json` for producer edits after the writer (for example vector's
/// `asset.extras.vectorBuildStateSha256`).
const DOC_SLACK_NODES: usize = 64;
const DOC_SLACK_BYTES: usize = 4096;
/// A link name used to reserve link cost before the document number is known.
const LINK_RESERVATION: &str = "tileset-d18446744073709551615.json";

/// The role of a tile of H in the forest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TileRole {
    /// An explicit tile whose subtree height is at least S.
    Skeleton,
    /// A leaf child of a skeleton tile, written as an explicit tile.
    ExplicitLeaf,
    /// An explicit tile carrying `implicitTiling` with exactly one subtree.
    ImplicitRoot,
    /// A tile below an implicit root, available in its subtree.
    Implicit,
}

/// How to recover a raw payload from its published copy (design §4.7).
#[derive(Clone, Debug, PartialEq)]
pub struct RebaseRecord {
    pub raw_sha256: [u8; 32],
    /// The z-up translation that was baked into the GLB scene root as `(δ0, δ2, −δ1)`.
    /// Zero means the published payload is a byte copy; then `raw_root_node` is empty and
    /// `raw_json_length` is 0.
    pub delta: [f64; 3],
    /// The exact JSON text of the raw scene-root node object.
    pub raw_root_node: String,
    /// The raw GLB JSON chunk byte length, including its padding.
    pub raw_json_length: u32,
}

/// One content of a tile as published.
#[derive(Clone, Debug, PartialEq)]
pub struct WrittenContent {
    pub member: String,
    pub bytes: u64,
    /// `Some` for every payload published under `implicit-content/`.
    pub rebase: Option<RebaseRecord>,
}

/// One tile of H as written. `extras` is the written object (empty when absent), without
/// `implicitChildIndex`; for implicit roots and implicit tiles an `encodedBytes` member is the
/// published byte length.
#[derive(Clone, Debug, PartialEq)]
pub struct WrittenTile {
    pub role: TileRole,
    pub extras: Value,
    pub contents: Vec<WrittenContent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForestOutput {
    /// Every member the writer wrote or kept, except `tileset.json`: external documents,
    /// subtrees, `implicit-content/` payloads and explicit-tile payloads (published in place).
    pub members: Vec<String>,
    /// Exactly one record per tile of H, in preorder. Link tiles have no record.
    pub tiles: Vec<WrittenTile>,
    /// JSON tileset documents, including `tileset.json`.
    pub documents: usize,
    pub implicit_roots: usize,
}

fn data(message: impl std::fmt::Display) -> Error {
    Error::Data(format!("implicit output: {message}"))
}

/// One tile of H, as collected.
struct Node {
    /// The producer's tile object without `children`; `extras` without `implicitChildIndex`.
    tile: Map<String, Value>,
    children: Vec<usize>,
    parent: Option<usize>,
    slot: usize,
    translation: [f64; 3],
    bounding_box: [f64; 12],
    error: f64,
    level: u32,
    height: u32,
    /// One past the last preorder index of this tile's subtree.
    end: usize,
    refine: &'static str,
    contents: Vec<Content>,
}

struct Content {
    uri: String,
    suffix: String,
    header: Map<String, Value>,
}

fn numbers<const N: usize>(value: &Value) -> Option<[f64; N]> {
    let values = value.as_array().filter(|v| v.len() == N)?;
    let mut out = [0.; N];
    for (lane, value) in out.iter_mut().zip(values) {
        *lane = value.as_f64().filter(|v| v.is_finite())?;
    }
    Some(out)
}

fn content_suffix(uri: &str) -> Result<String, Error> {
    let suffix = Path::new(uri)
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|s| matches!(s.as_str(), "glb" | "gltf"))
        .ok_or_else(|| data(format!("content {uri:?} must be glb or gltf")))?;
    let relative = Path::new(uri)
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !relative || uri.contains('\\') {
        return Err(data(format!(
            "content {uri:?} must be a relative member name"
        )));
    }
    Ok(suffix)
}

fn parse_tile(value: &Value, id: usize, root: bool) -> Result<Node, Error> {
    let object = value
        .as_object()
        .ok_or_else(|| data(format!("tile {id} must be an object")))?;
    let bounding_box = numbers::<12>(&value["boundingVolume"]["box"])
        .ok_or_else(|| data(format!("tile {id} needs a box of 12 finite numbers")))?;
    let error = value["geometricError"]
        .as_f64()
        .filter(|e| e.is_finite() && *e >= 0.)
        .ok_or_else(|| {
            data(format!(
                "tile {id} needs a finite nonnegative geometricError"
            ))
        })?;
    let refine = match object.get("refine") {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("ADD") => "ADD",
            Some("REPLACE") => "REPLACE",
            _ => return Err(data(format!("tile {id} has an invalid refine"))),
        }),
    };
    let mut translation = [0.; 3];
    if let Some(transform) = object.get("transform") {
        let matrix = numbers::<16>(transform)
            .ok_or_else(|| data(format!("tile {id} needs a transform of 16 finite numbers")))?;
        if !root {
            if (0..12).any(|i| matrix[i] != if i % 5 == 0 { 1. } else { 0. }) || matrix[15] != 1. {
                return Err(data(format!(
                    "tile {id}: a child transform must be a translation"
                )));
            }
            translation = [matrix[12], matrix[13], matrix[14]];
        }
    }
    let raw_contents: Vec<&Value> = match (object.get("content"), object.get("contents")) {
        (Some(_), Some(_)) => return Err(data(format!("tile {id} has both content and contents"))),
        (Some(content), None) => vec![content],
        (None, Some(contents)) => contents
            .as_array()
            .ok_or_else(|| data(format!("tile {id}: contents must be an array")))?
            .iter()
            .collect(),
        (None, None) => Vec::new(),
    };
    let mut contents = Vec::with_capacity(raw_contents.len());
    for content in raw_contents {
        let mut header = content
            .as_object()
            .cloned()
            .ok_or_else(|| data(format!("tile {id}: content must be an object")))?;
        let uri = header
            .remove("uri")
            .and_then(|uri| uri.as_str().map(str::to_owned))
            .ok_or_else(|| data(format!("tile {id}: content needs a URI")))?;
        let suffix = content_suffix(&uri)?;
        contents.push(Content {
            uri,
            suffix,
            header,
        });
    }
    let mut tile: Map<String, Value> = object
        .iter()
        .filter(|(key, _)| key.as_str() != "children")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if let Some(Value::Object(extras)) = tile.get_mut("extras") {
        extras.remove("implicitChildIndex");
        if extras.is_empty() {
            tile.remove("extras");
        }
    }
    Ok(Node {
        tile,
        children: Vec::new(),
        parent: None,
        slot: 0,
        translation,
        bounding_box,
        error,
        level: 0,
        height: 0,
        end: id + 1,
        refine: refine.unwrap_or(""),
        contents,
    })
}

/// Collect H in preorder (children in array order) without recursion.
fn collect(
    root: &Value,
    scheme: SubdivisionScheme,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<Vec<Node>, Error> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack = vec![(root, None::<usize>, 0usize)];
    while let Some((value, parent, index)) = stack.pop() {
        checkpoint()?;
        let id = nodes.len();
        let mut node = parse_tile(value, id, parent.is_none())?;
        if let Some(parent) = parent {
            let slot = match value["extras"].get("implicitChildIndex") {
                None => index,
                Some(slot) => slot
                    .as_u64()
                    .and_then(|s| usize::try_from(s).ok())
                    .ok_or_else(|| data(format!("tile {id} has an invalid implicitChildIndex")))?,
            };
            if slot >= scheme.branches() {
                return Err(data(format!(
                    "tile {id}: child slot {slot} is outside the {} scheme",
                    scheme.as_str()
                )));
            }
            node.parent = Some(parent);
            node.slot = slot;
            node.level = nodes[parent].level + 1;
            if node.refine.is_empty() {
                node.refine = nodes[parent].refine;
            }
            nodes[parent].children.push(id);
        } else if node.refine.is_empty() {
            node.refine = "REPLACE";
        }
        match value.get("children") {
            None => {}
            Some(Value::Array(children)) => {
                for (index, child) in children.iter().enumerate().rev() {
                    stack.push((child, Some(id), index));
                }
            }
            Some(_) => return Err(data(format!("tile {id}: children must be an array"))),
        }
        nodes.push(node);
    }
    let levels = nodes.iter().map(|n| n.level).max().unwrap_or(0) + 1;
    if levels > MAX_LEVELS {
        return Err(Error::Data(format!(
            "hierarchy has {levels} levels; implicit output admits at most {MAX_LEVELS}"
        )));
    }
    for id in (0..nodes.len()).rev() {
        let mut slots = BTreeSet::new();
        for &child in &nodes[id].children {
            if !slots.insert(nodes[child].slot) {
                return Err(data(format!("tile {id} has a duplicate child slot")));
            }
        }
        if let Some(&last) = nodes[id].children.last() {
            let height = nodes[id]
                .children
                .iter()
                .map(|&c| nodes[c].height + 1)
                .max()
                .unwrap_or(0);
            let end = nodes[last].end;
            nodes[id].height = height;
            nodes[id].end = end;
        }
    }
    Ok(nodes)
}

fn roles(nodes: &[Node]) -> Vec<TileRole> {
    let mut roles = Vec::with_capacity(nodes.len());
    for node in nodes {
        let role = match node.parent.map(|p| roles[p]) {
            None if node.height < SKELETON_HEIGHT => TileRole::ImplicitRoot,
            None => TileRole::Skeleton,
            Some(TileRole::Skeleton) if node.height >= SKELETON_HEIGHT => TileRole::Skeleton,
            Some(TileRole::Skeleton) if node.height == 0 => TileRole::ExplicitLeaf,
            Some(TileRole::Skeleton) => TileRole::ImplicitRoot,
            Some(_) => TileRole::Implicit,
        };
        roles.push(role);
    }
    roles
}

/// Keys a tile at or below an implicit root may carry: everything else has no place in a
/// subtree row or the root's templates.
const IMPLICIT_TILE_KEYS: [&str; 7] = [
    "boundingVolume",
    "geometricError",
    "refine",
    "transform",
    "content",
    "contents",
    "extras",
];

fn check_implicit_tile(id: usize, node: &Node, root: &Node, at_root: bool) -> Result<(), Error> {
    if let Some(key) = node
        .tile
        .keys()
        .find(|k| !IMPLICIT_TILE_KEYS.contains(&k.as_str()))
    {
        return Err(data(format!(
            "tile {id}: {key:?} cannot be expressed at or below an implicit root"
        )));
    }
    if node.tile["boundingVolume"]
        .as_object()
        .is_some_and(|v| v.len() != 1)
    {
        return Err(data(format!(
            "tile {id}: an implicit tile's boundingVolume must hold only a box"
        )));
    }
    if node.refine != root.refine {
        return Err(data(format!(
            "tile {id}: refine {} differs from its implicit root's {}",
            node.refine, root.refine
        )));
    }
    for content in &node.contents {
        if content.header.contains_key("boundingVolume") {
            return Err(data(format!(
                "tile {id}: a content boundingVolume cannot be expressed in an implicit template"
            )));
        }
        if content.suffix == "gltf" && !at_root {
            return Err(data(format!(
                "tile {id}: .gltf content is admitted only at an implicit root or an explicit tile"
            )));
        }
    }
    Ok(())
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

fn key(c: Coordinates, scheme: SubdivisionScheme) -> String {
    if scheme == SubdivisionScheme::Octree {
        format!("{}-{}-{}-{}", c.level, c.x, c.y, c.z)
    } else {
        format!("{}-{}-{}", c.level, c.x, c.y)
    }
}

fn coordinate_template(scheme: SubdivisionScheme) -> &'static str {
    if scheme == SubdivisionScheme::Octree {
        "{level}-{x}-{y}-{z}"
    } else {
        "{level}-{x}-{y}"
    }
}

fn schema() -> Value {
    serde_json::to_value(crate::metadata::tile_schema()).expect("tile schema serializes")
}

fn set_contents(tile: &mut Map<String, Value>, mut contents: Vec<Value>) {
    tile.remove("content");
    tile.remove("contents");
    if contents.len() == 1 {
        tile.insert("content".into(), contents.remove(0));
    } else if !contents.is_empty() {
        tile.insert("contents".into(), Value::Array(contents));
    }
}

/// Counts serialized bytes without allocating the serialization.
struct Count(usize);
impl std::io::Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Cost {
    nodes: usize,
    bytes: usize,
    /// The deepest value below the object, relative to it.
    depth: usize,
}

fn cost(value: &Value) -> Result<Cost, Error> {
    let mut nodes = 0;
    let mut depth = 0;
    let mut stack = vec![(value, 0usize)];
    while let Some((value, d)) = stack.pop() {
        nodes += 1;
        depth = depth.max(d);
        match value {
            Value::Array(values) => stack.extend(values.iter().map(|v| (v, d + 1))),
            Value::Object(values) => stack.extend(values.values().map(|v| (v, d + 1))),
            _ => {}
        }
    }
    let mut bytes = Count(0);
    serde_json::to_writer(&mut bytes, value)?;
    Ok(Cost {
        nodes,
        bytes: bytes.0,
        depth,
    })
}

/// `,"children":[` and `]`.
const CHILDREN_BYTES: usize = 14;

/// The forest's per-tile JSON pieces and their costs, before packing.
struct Pieces {
    /// The tile object without children, for skeleton tiles, explicit leaves and implicit roots.
    objects: Vec<Option<Map<String, Value>>>,
    costs: Vec<Cost>,
    /// The link reservation for skeleton tiles.
    links: Vec<Cost>,
}

fn link_tile(node: &Node, name: &str) -> Map<String, Value> {
    let mut link = Map::new();
    link.insert("boundingVolume".into(), node.tile["boundingVolume"].clone());
    link.insert("geometricError".into(), node.tile["geometricError"].clone());
    link.insert("refine".into(), json!(node.refine));
    if let Some(transform) = node.tile.get("transform") {
        link.insert("transform".into(), transform.clone());
    }
    link.insert("content".into(), json!({ "uri": name }));
    link
}

struct Unit {
    nodes: usize,
    bytes: usize,
    /// Absolute JSON depth of the unit at its level.
    depth: usize,
}

/// A skeleton tile with its explicit leaves and implicit roots, plus link reservations for
/// its skeleton children, at skeleton level `level` of a document.
fn unit(id: usize, level: u32, nodes: &[Node], roles: &[TileRole], pieces: &Pieces) -> Unit {
    let own = pieces.costs[id];
    let at = |level: u32| 1 + 2 * level as usize;
    let mut unit = Unit {
        nodes: own.nodes + 1,
        bytes: own.bytes + CHILDREN_BYTES,
        depth: at(level) + own.depth,
    };
    for &child in &nodes[id].children {
        let cost = if roles[child] == TileRole::Skeleton {
            pieces.links[child]
        } else {
            pieces.costs[child]
        };
        unit.nodes += cost.nodes;
        unit.bytes += cost.bytes + 1;
        unit.depth = unit.depth.max(at(level + 1) + cost.depth);
    }
    unit
}

struct Document {
    root: usize,
    name: String,
    holds_root: bool,
}

/// Breadth-first packing (design §4.1, Documents). Returns the documents in creation order
/// and, for every linked skeleton tile, the index of the document it roots.
fn pack(
    nodes: &[Node],
    roles: &[TileRole],
    pieces: &Pieces,
    top: Cost,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<(Vec<Document>, BTreeMap<usize, usize>), Error> {
    let mut documents = vec![Document {
        root: 0,
        name: "tileset.json".into(),
        holds_root: false,
    }];
    let mut linked = BTreeMap::new();
    let mut index = 0;
    while index < documents.len() {
        checkpoint()?;
        let root = documents[index].root;
        let first = unit(root, 0, nodes, roles, pieces);
        let mut used_nodes = top.nodes + first.nodes + DOC_SLACK_NODES;
        let mut used_bytes = top.bytes + first.bytes + DOC_SLACK_BYTES;
        if used_nodes > DOC_NODES || used_bytes > DOC_BYTES || first.depth > DOC_DEPTH {
            return Err(data(format!(
                "tile {root}: a single skeleton unit exceeds the document budget \
                 ({DOC_NODES} value nodes, {DOC_BYTES} bytes, JSON depth {DOC_DEPTH})"
            )));
        }
        let mut holds_root = false;
        let mut frontier = VecDeque::from([(root, 0u32)]);
        while let Some((tile, level)) = frontier.pop_front() {
            for &child in &nodes[tile].children {
                match roles[child] {
                    TileRole::ImplicitRoot => holds_root = true,
                    TileRole::Skeleton => {
                        let candidate = unit(child, level + 1, nodes, roles, pieces);
                        let link = pieces.links[child];
                        let nodes_after = (used_nodes + candidate.nodes).checked_sub(link.nodes);
                        let bytes_after = (used_bytes + candidate.bytes).checked_sub(link.bytes);
                        if level < DOC_LEVELS
                            && candidate.depth <= DOC_DEPTH
                            && nodes_after.is_some_and(|n| n <= DOC_NODES)
                            && bytes_after.is_some_and(|b| b <= DOC_BYTES)
                        {
                            used_nodes = nodes_after.unwrap_or(used_nodes);
                            used_bytes = bytes_after.unwrap_or(used_bytes);
                            frontier.push_back((child, level + 1));
                        } else {
                            linked.insert(child, documents.len());
                            documents.push(Document {
                                root: child,
                                name: format!("tileset-d{}.json", documents.len()),
                                holds_root: false,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        documents[index].holds_root = holds_root;
        index += 1;
    }
    Ok((documents, linked))
}

fn assemble(
    id: usize,
    nodes: &[Node],
    roles: &[TileRole],
    pieces: &Pieces,
    linked: &BTreeMap<usize, usize>,
    documents: &[Document],
) -> Map<String, Value> {
    let mut tile = pieces.objects[id].clone().expect("explicit tile object");
    if roles[id] == TileRole::Skeleton {
        let children = nodes[id]
            .children
            .iter()
            .map(|&child| match (roles[child], linked.get(&child)) {
                (TileRole::Skeleton, Some(&document)) => {
                    Value::Object(link_tile(&nodes[child], &documents[document].name))
                }
                (TileRole::Skeleton, None) => {
                    Value::Object(assemble(child, nodes, roles, pieces, linked, documents))
                }
                _ => Value::Object(pieces.objects[child].clone().expect("unit child object")),
            })
            .collect();
        tile.insert("children".into(), Value::Array(children));
    }
    tile
}

/// Grow an axis-aligned box's half lengths outward by `by` (z-up), keeping each lane's sign.
/// Returns whether any lane changed. Rotated boxes are left as they are: C2 evaluates
/// implicit rows only when they are axis aligned.
fn widen(bounding_box: &mut [f64; 12], by: [f64; 3]) -> bool {
    if by == [0.; 3] || [4, 5, 6, 8, 9, 10].iter().any(|&k| bounding_box[k] != 0.) {
        return false;
    }
    let before = *bounding_box;
    for (i, step) in by.into_iter().enumerate() {
        let lane = &mut bounding_box[3 + 4 * i];
        *lane = sum_up(lane.abs(), step).copysign(*lane);
    }
    before != *bounding_box
}

/// Write the half lanes of `after` that differ from `before` into a `boundingVolume`.
fn set_halves(volume: &mut Value, before: &[f64; 12], after: &[f64; 12]) {
    for k in [3, 7, 11] {
        if before[k] != after[k] {
            volume["box"][k] = json!(after[k]);
        }
    }
}

/// Per z-up axis, the magnitude of the error in a rebased payload's scene-root translation:
/// the raw root translation (or matrix column) plus `(δ0, δ2, −δ1)`, which
/// [`translate_node`] rounds once in f64.
fn rebase_error(record: &RebaseRecord) -> Result<[f64; 3], Error> {
    if record.raw_root_node.is_empty() {
        return Ok([0.; 3]);
    }
    let node: Value = serde_json::from_str(&record.raw_root_node)?;
    let raw = match node.get("matrix") {
        Some(matrix) => numbers::<16>(matrix).map(|m| [m[12], m[13], m[14]]),
        None => node.get("translation").map_or(Some([0.; 3]), numbers::<3>),
    }
    .ok_or_else(|| data("rebased scene root has no finite translation"))?;
    let d = record.delta;
    let step = [d[0], d[2], -d[1]];
    let e: [f64; 3] = std::array::from_fn(|i| sum_error(raw[i], step[i]).abs());
    Ok([e[0], e[2], e[1]])
}

/// Exact P8 check on a finished document.
fn check_document(name: &str, document: &Value) -> Result<Vec<u8>, Error> {
    let measured = cost(document)?;
    if measured.nodes > DOC_NODES || measured.bytes > DOC_BYTES || measured.depth > DOC_DEPTH {
        return Err(data(format!(
            "{name} has {} value nodes, {} bytes and JSON depth {}; the document budget is \
             {DOC_NODES}, {DOC_BYTES} and {DOC_DEPTH}",
            measured.nodes, measured.bytes, measured.depth
        )));
    }
    Ok(serde_json::to_vec(document)?)
}

/// Write the bounded implicit forest for a producer's explicit hierarchy.
///
/// `manifest` holds the producer's explicit hierarchy on entry and the root `tileset.json`
/// document on success; the producer serializes it. Payload URIs are relative to
/// `directory`. Raw sources of payloads rewritten into `implicit-content/` are deleted unless
/// an explicit tile still references them. Every failure is a typed error raised before the
/// producer publishes; files already written stay in the producer's staging directory.
pub fn write_forest(
    manifest: &mut Value,
    directory: &Path,
    scheme: SubdivisionScheme,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<ForestOutput, Error> {
    checkpoint()?;
    let mut nodes = collect(&manifest["root"], scheme, checkpoint)?;
    let roles = roles(&nodes);
    for (id, node) in nodes.iter().enumerate() {
        if roles[id] == TileRole::ImplicitRoot {
            for (t, tile) in nodes.iter().enumerate().take(node.end).skip(id) {
                check_implicit_tile(t, tile, node, t == id)?;
            }
        }
    }
    let template = coordinate_template(scheme);
    let mut members = Vec::new();
    let mut written: BTreeSet<String> = BTreeSet::new();
    let mut tiles: Vec<Option<WrittenTile>> = vec![None; nodes.len()];
    let mut objects: Vec<Option<Map<String, Value>>> = vec![None; nodes.len()];
    let mut implicit_sources = BTreeSet::new();
    let mut explicit_sources = BTreeSet::new();
    let mut implicit_roots = 0;
    // Drift of implicit content from its exact place, carried up to explicit ancestors.
    let mut drift = vec![[0f64; 3]; nodes.len()];
    for (id, node) in nodes.iter().enumerate() {
        match roles[id] {
            TileRole::Skeleton | TileRole::ExplicitLeaf => {
                let mut contents = Vec::with_capacity(node.contents.len());
                for content in &node.contents {
                    let bytes = std::fs::metadata(directory.join(&content.uri))?.len();
                    if explicit_sources.insert(content.uri.clone()) {
                        members.push(content.uri.clone());
                    }
                    contents.push(WrittenContent {
                        member: content.uri.clone(),
                        bytes,
                        rebase: None,
                    });
                }
                tiles[id] = Some(WrittenTile {
                    role: roles[id],
                    extras: node
                        .tile
                        .get("extras")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                    contents,
                });
                objects[id] = Some(node.tile.clone());
            }
            TileRole::ImplicitRoot => {
                checkpoint()?;
                implicit_roots += 1;
                std::fs::create_dir_all(directory.join("implicit-content"))?;
                std::fs::create_dir_all(directory.join("subtrees"))?;
                let root = ImplicitRoot {
                    id,
                    scheme,
                    directory,
                    template,
                };
                let (object, subtree, reach) = root.write(
                    &nodes,
                    &mut tiles,
                    &mut members,
                    &mut implicit_sources,
                    checkpoint,
                )?;
                written.insert(subtree);
                objects[id] = Some(object);
                let mut ancestor = node.parent;
                while let Some(a) = ancestor {
                    for (lane, step) in drift[a].iter_mut().zip(reach) {
                        *lane = lane.max(step);
                    }
                    ancestor = nodes[a].parent;
                }
            }
            TileRole::Implicit => {}
        }
    }
    // Explicit ancestors of an implicit root contain its content at its exact place; widen
    // them by the drift so they also contain it where it is published.
    for (id, by) in drift.into_iter().enumerate() {
        let before = nodes[id].bounding_box;
        let mut after = before;
        if widen(&mut after, by) {
            let node = &mut nodes[id];
            node.bounding_box = after;
            set_halves(&mut node.tile["boundingVolume"], &before, &after);
            if let Some(object) = objects[id].as_mut() {
                set_halves(&mut object["boundingVolume"], &before, &after);
            }
        }
    }
    written.extend(members.iter().cloned());
    let mut links = vec![Cost::default(); nodes.len()];
    let mut costs = vec![Cost::default(); nodes.len()];
    for id in 0..nodes.len() {
        if let Some(object) = &objects[id] {
            costs[id] = cost(&Value::Object(object.clone()))?;
        }
        if roles[id] == TileRole::Skeleton {
            links[id] = cost(&Value::Object(link_tile(&nodes[id], LINK_RESERVATION)))?;
        }
    }
    let pieces = Pieces {
        objects,
        costs,
        links,
    };
    let mut top: Map<String, Value> = manifest
        .as_object()
        .ok_or_else(|| data("tileset must be an object"))?
        .iter()
        .filter(|(key, _)| key.as_str() != "root")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    top.insert("schema".into(), schema());
    top.insert("root".into(), Value::Null);
    let mut documents = 1;
    if roles[0] == TileRole::ImplicitRoot {
        top.insert(
            "root".into(),
            Value::Object(pieces.objects[0].clone().expect("root object")),
        );
        check_document("tileset.json", &Value::Object(top.clone()))?;
    } else {
        let top_cost = cost(&Value::Object(top.clone()))?;
        let mut document_top = Map::new();
        document_top.insert("asset".into(), manifest["asset"].clone());
        document_top.insert("geometricError".into(), json!(f64::MAX));
        for key in ["extensionsUsed", "extensionsRequired"] {
            if let Some(value) = manifest.get(key) {
                document_top.insert(key.into(), value.clone());
            }
        }
        document_top.insert("schema".into(), schema());
        document_top.insert("root".into(), Value::Null);
        let reserve = cost(&Value::Object(document_top.clone()))?;
        let reserve = Cost {
            nodes: reserve.nodes.max(top_cost.nodes),
            bytes: reserve.bytes.max(top_cost.bytes),
            depth: reserve.depth.max(top_cost.depth),
        };
        let (packed, linked) = pack(&nodes, &roles, &pieces, reserve, checkpoint)?;
        documents = packed.len();
        for (index, document) in packed.iter().enumerate() {
            checkpoint()?;
            let mut root = assemble(document.root, &nodes, &roles, &pieces, &linked, &packed);
            if index == 0 {
                top.insert("root".into(), Value::Object(root));
                check_document("tileset.json", &Value::Object(top.clone()))?;
                continue;
            }
            root.remove("transform");
            root.insert("refine".into(), json!(nodes[document.root].refine));
            let mut body = Map::new();
            body.insert("asset".into(), manifest["asset"].clone());
            body.insert("geometricError".into(), json!(nodes[document.root].error));
            for key in ["extensionsUsed", "extensionsRequired"] {
                if let Some(value) = manifest.get(key) {
                    body.insert(key.into(), value.clone());
                }
            }
            if document.holds_root {
                body.insert("schema".into(), schema());
            }
            body.insert("root".into(), Value::Object(root));
            let bytes = check_document(&document.name, &Value::Object(body))?;
            std::fs::write(directory.join(&document.name), bytes)?;
            written.insert(document.name.clone());
            members.push(document.name.clone());
        }
    }
    for source in implicit_sources.difference(&explicit_sources) {
        checkpoint()?;
        if written.contains(source) {
            return Err(data(format!(
                "raw source {source:?} collides with a written member"
            )));
        }
        std::fs::remove_file(directory.join(source))?;
    }
    *manifest = Value::Object(top);
    Ok(ForestOutput {
        members,
        tiles: tiles
            .into_iter()
            .map(|t| t.expect("every tile is written"))
            .collect(),
        documents,
        implicit_roots,
    })
}

/// An implicit root as written: its explicit tile object, its subtree member name and its
/// drift bound (z-up), by which its explicit ancestors grow.
type WrittenRoot = (Map<String, Value>, String, [f64; 3]);

struct ImplicitRoot<'a> {
    id: usize,
    scheme: SubdivisionScheme,
    directory: &'a Path,
    template: &'static str,
}

impl ImplicitRoot<'_> {
    /// Write one implicit root's payloads and its single subtree. Returns the explicit root
    /// tile object, the subtree member name and the root's drift bound (see below), by which
    /// its explicit ancestors must grow.
    ///
    /// Implicit tile `t` is published at `δ(t) = δ(parent) + translation(t)`, rounded per
    /// edge, so its row centre `B(t) + δ(t)` and its payload's rebased scene root (each
    /// rounded once more) drift from the producer's exact placement by a few ulps. Every
    /// row grows by an outward-rounded bound on the drift of its own and its descendants'
    /// content relative to it, so a vertex the producer's hierarchy contains exactly stays
    /// inside every volume C2 tests. Exact sums give a zero bound and unchanged rows.
    fn write(
        &self,
        nodes: &[Node],
        tiles: &mut [Option<WrittenTile>],
        members: &mut Vec<String>,
        sources: &mut BTreeSet<String>,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<WrittenRoot, Error> {
        let (id, scheme, template) = (self.id, self.scheme, self.template);
        let root = &nodes[id];
        let range = id..root.end;
        // R-4: the slot list drives the templates and the content layers, in the same order.
        let mut slots: BTreeMap<(usize, String), Map<String, Value>> = BTreeMap::new();
        for t in range.clone() {
            for (index, content) in nodes[t].contents.iter().enumerate() {
                match slots.get(&(index, content.suffix.clone())) {
                    None => {
                        slots.insert((index, content.suffix.clone()), content.header.clone());
                    }
                    Some(header) if *header != content.header => {
                        return Err(data(format!(
                            "tile {t}: content {index} header differs from its implicit root's \
                             slot header"
                        )));
                    }
                    Some(_) => {}
                }
            }
        }
        let slot_list: Vec<_> = slots.keys().cloned().collect();
        let mut subtree = Subtree::new(scheme, root.height + 1, slot_list.len())?;
        let mut coordinates = BTreeMap::new();
        let mut deltas: BTreeMap<usize, [f64; 3]> = BTreeMap::new();
        coordinates.insert(
            id,
            Coordinates {
                level: 0,
                x: 0,
                y: 0,
                z: 0,
            },
        );
        deltas.insert(id, [0.; 3]);
        // |error| of each edge's δ sum, and per tile the drift bound G: first the largest
        // payload rebase error, then max(own, |edge(c)| + G(c)) over the children.
        let mut edges: BTreeMap<usize, [f64; 3]> = BTreeMap::new();
        let mut drift: BTreeMap<usize, [f64; 3]> = BTreeMap::new();
        let mut layers = Vec::with_capacity(range.len());
        for t in range.clone() {
            checkpoint()?;
            let node = &nodes[t];
            let c = coordinates[&t];
            let delta = deltas[&t];
            for &next in &node.children {
                coordinates.insert(next, child(c, nodes[next].slot, scheme)?);
                let step = nodes[next].translation;
                deltas.insert(next, std::array::from_fn(|i| delta[i] + step[i]));
                edges.insert(
                    next,
                    std::array::from_fn(|i| sum_error(delta[i], step[i]).abs()),
                );
            }
            let mut own = [0f64; 3];
            let mut flags = vec![false; slot_list.len()];
            let mut written = Vec::with_capacity(node.contents.len());
            let mut total = 0u64;
            for (index, content) in node.contents.iter().enumerate() {
                let slot = slot_list
                    .iter()
                    .position(|(i, s)| *i == index && *s == content.suffix)
                    .expect("slot list covers every content");
                let raw = std::fs::read(self.directory.join(&content.uri))?;
                let (bytes, record) = rebase_payload(&raw, delta)?;
                for (lane, error) in own.iter_mut().zip(rebase_error(&record)?) {
                    *lane = lane.max(error);
                }
                let name = format!(
                    "implicit-content/r{id}-{}-{slot}.{}",
                    key(c, scheme),
                    content.suffix
                );
                std::fs::write(self.directory.join(&name), &bytes)?;
                total += bytes.len() as u64;
                members.push(name.clone());
                sources.insert(content.uri.clone());
                flags[slot] = true;
                written.push(WrittenContent {
                    member: name,
                    bytes: bytes.len() as u64,
                    rebase: Some(record),
                });
            }
            let mut extras = node
                .tile
                .get("extras")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if extras.get("encodedBytes").is_some() {
                extras["encodedBytes"] = json!(total);
            }
            drift.insert(t, own);
            layers.push((c, flags));
            tiles[t] = Some(WrittenTile {
                role: if t == id {
                    TileRole::ImplicitRoot
                } else {
                    TileRole::Implicit
                },
                extras,
                contents: written,
            });
        }
        // Reverse preorder visits every child before its parent.
        for t in range.clone().rev() {
            let mut bound = drift[&t];
            for &next in &nodes[t].children {
                let (edge, below) = (edges[&next], drift[&next]);
                for i in 0..3 {
                    bound[i] = bound[i].max(sum_up(edge[i], below[i]));
                }
            }
            drift.insert(t, bound);
        }
        for (t, (c, flags)) in range.zip(layers) {
            checkpoint()?;
            let node = &nodes[t];
            // P3: the level-0 row is the root's box; below it, B(t) + δ(t) in f64. Each
            // grows by its drift bound plus the rounding error of its own centre.
            let mut bounding_box = node.bounding_box;
            let mut by = drift[&t];
            if t != id {
                let delta = deltas[&t];
                for i in 0..3 {
                    let error = sum_error(bounding_box[i], delta[i]).abs();
                    bounding_box[i] += delta[i];
                    by[i] = sum_up(by[i], error);
                }
            }
            widen(&mut bounding_box, by);
            subtree.set_tile(c, &flags)?;
            subtree.set_metadata(
                c,
                TileMetadata {
                    bounding_box,
                    geometric_error: node.error,
                    extras: tiles[t].as_ref().expect("written above").extras.clone(),
                },
            )?;
        }
        let origin = Coordinates {
            level: 0,
            x: 0,
            y: 0,
            z: 0,
        };
        let name = format!("subtrees/r{id}-{}.subtree", key(origin, scheme));
        std::fs::write(self.directory.join(&name), subtree.to_bytes()?)?;
        members.push(name.clone());
        let mut object = Map::new();
        object.insert("boundingVolume".into(), root.tile["boundingVolume"].clone());
        // V_R is the level-0 row, widened alike.
        let reach = drift[&id];
        let mut bounding_box = root.bounding_box;
        if widen(&mut bounding_box, reach) {
            set_halves(
                &mut object["boundingVolume"],
                &root.bounding_box,
                &bounding_box,
            );
        }
        object.insert("geometricError".into(), root.tile["geometricError"].clone());
        object.insert("refine".into(), json!(root.refine));
        if let Some(transform) = root.tile.get("transform") {
            object.insert("transform".into(), transform.clone());
        }
        set_contents(
            &mut object,
            slots
                .into_iter()
                .enumerate()
                .map(|(slot, ((_, suffix), mut header))| {
                    header.insert(
                        "uri".into(),
                        json!(format!("implicit-content/r{id}-{template}-{slot}.{suffix}")),
                    );
                    Value::Object(header)
                })
                .collect(),
        );
        object.insert(
            "implicitTiling".into(),
            json!({
                "subdivisionScheme": scheme.as_str(),
                "subtreeLevels": root.height + 1,
                "availableLevels": root.height + 1,
                "subtrees": {"uri": format!("subtrees/r{id}-{template}.subtree")},
            }),
        );
        Ok((object, name, reach))
    }
}

// ---- Reversible payload rebase (design §4.7) ----

/// A GLB split around its JSON chunk.
struct Container<'a> {
    /// The JSON chunk data, including padding.
    json: &'a [u8],
    /// Every GLB byte after the JSON chunk (BIN and other chunks).
    rest: &'a [u8],
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<usize> {
    let word = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes(word.try_into().ok()?) as usize)
}

fn container(glb: &[u8]) -> Result<Container<'_>, String> {
    if !glb.starts_with(b"glTF") {
        return Err("payload is not a GLB".into());
    }
    if glb.len() < 20 || u32_at(glb, 4) != Some(2) {
        return Err("GLB header is not glTF 2.0 with a JSON chunk".into());
    }
    let length = u32_at(glb, 8).expect("GLB header");
    if length != glb.len() {
        return Err("GLB length differs from the payload".into());
    }
    let json_length = u32_at(glb, 12).expect("GLB header");
    if &glb[16..20] != b"JSON" {
        return Err("the first GLB chunk is not JSON".into());
    }
    let json_end = json_length
        .checked_add(20)
        .filter(|&end| end <= length)
        .ok_or("GLB JSON chunk exceeds the GLB")?;
    Ok(Container {
        json: &glb[20..json_end],
        rest: &glb[json_end..length],
    })
}

impl Container<'_> {
    fn with_json(&self, json: &[u8]) -> Result<Vec<u8>, String> {
        let too_large = || "payload exceeds the format's 4 GiB limit".to_owned();
        let glb_length = 20 + json.len() + self.rest.len();
        let glb_word = u32::try_from(glb_length).map_err(|_| too_large())?;
        let json_word = u32::try_from(json.len()).map_err(|_| too_large())?;
        let mut out = Vec::with_capacity(glb_length);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&glb_word.to_le_bytes());
        out.extend_from_slice(&json_word.to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(json);
        out.extend_from_slice(self.rest);
        Ok(out)
    }

    /// The JSON chunk without its trailing space padding.
    fn text(&self) -> &[u8] {
        let end = self
            .json
            .iter()
            .rposition(|&b| b != b' ')
            .map_or(0, |i| i + 1);
        &self.json[..end]
    }
}

#[derive(Deserialize)]
struct SceneRoots<'a> {
    #[serde(borrow, default)]
    nodes: Vec<&'a RawValue>,
    #[serde(default)]
    scenes: Vec<SceneNodes>,
}

#[derive(Deserialize)]
struct SceneNodes {
    #[serde(default)]
    nodes: Vec<u64>,
}

/// The byte span of the single scene-root node object within the JSON chunk text.
fn root_node_span(text: &[u8]) -> Result<std::ops::Range<usize>, String> {
    let text = std::str::from_utf8(text).map_err(|_| "GLB JSON is not UTF-8".to_owned())?;
    let document: SceneRoots =
        serde_json::from_str(text).map_err(|e| format!("GLB JSON cannot be read: {e}"))?;
    let roots: BTreeSet<u64> = document
        .scenes
        .iter()
        .flat_map(|scene| scene.nodes.iter().copied())
        .collect();
    if roots.len() != 1 {
        return Err(format!(
            "payload has {} scene root nodes; the reversible rebase admits exactly one",
            roots.len()
        ));
    }
    let root = roots.into_iter().next().expect("one root");
    let node = usize::try_from(root)
        .ok()
        .and_then(|index| document.nodes.get(index))
        .ok_or("the scene root node does not exist")?
        .get();
    let start = (node.as_ptr() as usize)
        .checked_sub(text.as_ptr() as usize)
        .filter(|&start| text.get(start..start + node.len()) == Some(node))
        .ok_or("the scene root node is not a span of the JSON chunk")?;
    Ok(start..start + node.len())
}

/// The root node text after adding a glTF (y-up) translation to its `translation` (absent
/// means zero) or to the translation column of its affine `matrix`.
fn translate_node(text: &[u8], translation: [f64; 3]) -> Result<String, String> {
    let mut node: Map<String, Value> =
        serde_json::from_slice(text).map_err(|e| format!("scene root node: {e}"))?;
    let (key, mut values, offset) = if let Some(matrix) = node.get("matrix") {
        if ["translation", "rotation", "scale"]
            .iter()
            .any(|k| node.contains_key(*k))
        {
            return Err("scene root node has both matrix and TRS properties".into());
        }
        let m = numbers::<16>(matrix).ok_or("scene root matrix is not 16 finite numbers")?;
        if m[3] != 0. || m[7] != 0. || m[11] != 0. || m[15] != 1. {
            return Err("scene root matrix is not affine".into());
        }
        ("matrix", m.to_vec(), 12)
    } else {
        let t = match node.get("translation") {
            None => [0.; 3],
            Some(value) => {
                numbers::<3>(value).ok_or("scene root translation is not 3 finite numbers")?
            }
        };
        ("translation", t.to_vec(), 0)
    };
    for (value, step) in values[offset..offset + 3].iter_mut().zip(translation) {
        *value += step;
        if !value.is_finite() {
            return Err(format!("rebased scene root {key} is not finite"));
        }
    }
    node.insert(key.into(), json!(values));
    serde_json::to_string(&node).map_err(|e| e.to_string())
}

fn splice(text: &[u8], span: std::ops::Range<usize>, node: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() - span.len() + node.len() + 8);
    out.extend_from_slice(&text[..span.start]);
    out.extend_from_slice(node);
    out.extend_from_slice(&text[span.end..]);
    out
}

fn is_zero(delta: [f64; 3]) -> bool {
    delta.iter().all(|&v| v == 0.)
}

/// Bake the z-up translation `delta` into a GLB payload's scene root as
/// `(δ0, δ2, −δ1)`, changing only the root node object's text in the JSON chunk.
///
/// A zero `delta` is a byte copy (any suffix, including `.gltf`). Otherwise the payload must
/// be a GLB with exactly one scene root node, which may carry a
/// translation or an affine `matrix`; anything else is a typed error. The JSON chunk is
/// padded with spaces to a length congruent to the raw length modulo 8, so the chunk and the
/// payload keep their alignment. Every other byte is unchanged.
pub fn rebase_payload(raw: &[u8], delta: [f64; 3]) -> Result<(Vec<u8>, RebaseRecord), Error> {
    let fail = |message: String| Error::Data(format!("payload rebase: {message}"));
    let raw_sha256: [u8; 32] = Sha256::digest(raw).into();
    if delta.iter().any(|v| !v.is_finite()) {
        return Err(fail("translation is not finite".into()));
    }
    if is_zero(delta) {
        return Ok((
            raw.to_vec(),
            RebaseRecord {
                raw_sha256,
                delta: [0.; 3],
                raw_root_node: String::new(),
                raw_json_length: 0,
            },
        ));
    }
    let source = container(raw).map_err(fail)?;
    let text = source.text();
    let span = root_node_span(text).map_err(fail)?;
    let raw_root_node = std::str::from_utf8(&text[span.clone()])
        .expect("span of UTF-8 text")
        .to_owned();
    let node =
        translate_node(&text[span.clone()], [delta[0], delta[2], -delta[1]]).map_err(fail)?;
    let mut json = splice(text, span, node.as_bytes());
    let raw_json_length = source.json.len();
    while json.len() % 8 != raw_json_length % 8 {
        json.push(b' ');
    }
    let bytes = source.with_json(&json).map_err(fail)?;
    Ok((
        bytes,
        RebaseRecord {
            raw_sha256,
            delta,
            raw_root_node,
            raw_json_length: u32::try_from(raw_json_length)
                .map_err(|_| fail("JSON chunk exceeds 4 GiB".into()))?,
        },
    ))
}

/// The exact inverse of [`rebase_payload`]: replace the published scene-root node text with
/// the recorded raw text and pad the JSON chunk with spaces to the recorded raw length.
///
/// Malformed records and payloads are typed errors ("invalid previous payload record"). The
/// caller checks the result's sha256 against `record.raw_sha256`; a stale record or an
/// altered member yields bytes whose digest differs.
pub fn restore_payload(published: &[u8], record: &RebaseRecord) -> Result<Vec<u8>, Error> {
    let fail = |message: String| Error::Data(format!("invalid previous payload record: {message}"));
    if record.delta.iter().any(|v| !v.is_finite()) {
        return Err(fail("translation is not finite".into()));
    }
    if is_zero(record.delta) {
        if !record.raw_root_node.is_empty() || record.raw_json_length != 0 {
            return Err(fail(
                "a byte copy records no root node or JSON length".into(),
            ));
        }
        return Ok(published.to_vec());
    }
    if record.raw_root_node.is_empty() {
        return Err(fail("a rebased payload records its raw root node".into()));
    }
    let source = container(published).map_err(fail)?;
    let text = source.text();
    let span = root_node_span(text).map_err(fail)?;
    let mut json = splice(text, span, record.raw_root_node.as_bytes());
    let length = record.raw_json_length as usize;
    if json.len() > length {
        return Err(fail(format!(
            "recorded JSON length {length} is shorter than the restored JSON ({} bytes)",
            json.len()
        )));
    }
    // The raw chunk differs from the restored text only by its space padding, which chunk
    // alignment keeps below 8 bytes. Refuse a larger recorded length before allocating it:
    // the record comes from the previous archive and is untrusted.
    if length - json.len() > 7 {
        return Err(fail(format!(
            "recorded JSON length {length} exceeds the restored JSON ({} bytes) by more than \
             alignment padding",
            json.len()
        )));
    }
    json.resize(length, b' ');
    source.with_json(&json).map_err(fail)
}

// ---- Expansion (PD-8): documents, links and implicit roots, one shared budget ----

fn contents(node: &Value) -> Vec<Value> {
    if let Some(values) = node["contents"].as_array() {
        values.clone()
    } else {
        node.get("content").cloned().into_iter().collect()
    }
}

/// Expand forest output (or any explicit tileset with single-document implicit roots) into
/// one explicit hierarchy, for audits and built-in validation. External documents are
/// followed through explicit link tiles; implicit JSON content templates are refused. Resource
/// reads go through the caller, so archive paths and size limits remain enforced.
pub fn expand_tileset(
    manifest: &Value,
    mut read: impl FnMut(&str) -> Result<Vec<u8>, Error>,
) -> Result<Value, Error> {
    expand_tileset_bounded(
        manifest,
        &mut read,
        &mut ExpansionBudget::new(1_000_000, 1024 * 1024 * 1024),
        &producer_json,
    )
}

/// Caller-supplied admission for structural presentation, not an addressing proof. The
/// budget is charged once across every document and implicit root.
pub(crate) fn expand_tileset_bounded(
    manifest: &Value,
    mut read: impl FnMut(&str) -> Result<Vec<u8>, Error>,
    budget: &mut ExpansionBudget,
    parse_json: &dyn Fn(&[u8]) -> Result<Value, Error>,
) -> Result<Value, Error> {
    let mut expander = Expander {
        read: &mut read,
        budget,
        parse_json,
        active: BTreeSet::new(),
    };
    let root = expander.tile(&manifest["root"], &manifest["schema"], "REPLACE", 0)?;
    let mut result: Map<String, Value> = manifest
        .as_object()
        .ok_or_else(|| invalid("tileset must be an object"))?
        .iter()
        .filter(|(key, _)| key.as_str() != "root")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    result.insert("root".into(), root);
    Ok(Value::Object(result))
}

// The non-C1 producer explicitly selects its existing document representation.
fn producer_json(bytes: &[u8]) -> Result<Value, Error> {
    Ok(serde_json::from_slice(bytes)?)
}

pub(crate) struct ExpansionBudget {
    nodes: usize,
    bytes: usize,
    max_nodes: usize,
    max_bytes: usize,
}
impl ExpansionBudget {
    pub(crate) fn new(max_nodes: usize, max_bytes: usize) -> Self {
        Self {
            nodes: 0,
            bytes: 0,
            max_nodes,
            max_bytes,
        }
    }

    fn charge(&mut self, nodes: usize, bytes: usize) -> Result<(), Error> {
        self.nodes = self
            .nodes
            .checked_add(nodes)
            .filter(|&n| n <= self.max_nodes)
            .ok_or_else(|| expansion_limit("expansion exceeds node limit"))?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|&n| n <= self.max_bytes)
            .ok_or_else(|| expansion_limit("expansion exceeds byte limit"))?;
        Ok(())
    }
}
fn expansion_limit(message: &str) -> Error {
    Error::Validation(crate::validate::ValidationFailure::ResourceLimit(
        message.into(),
    ))
}

/// Explicit external documents may nest at most this deep along one path.
const MAX_DOCUMENT_CHAIN: usize = 32;

struct Expander<'a, F> {
    read: &'a mut F,
    budget: &'a mut ExpansionBudget,
    parse_json: &'a dyn Fn(&[u8]) -> Result<Value, Error>,
    active: BTreeSet<String>,
}

impl<F: FnMut(&str) -> Result<Vec<u8>, Error>> Expander<'_, F> {
    fn tile(
        &mut self,
        node: &Value,
        schema: &Value,
        inherited: &str,
        documents: usize,
    ) -> Result<Value, Error> {
        let object = node
            .as_object()
            .ok_or_else(|| invalid("tile must be an object"))?;
        let refine = match object.get("refine") {
            None => inherited,
            Some(value) => value
                .as_str()
                .filter(|r| matches!(*r, "ADD" | "REPLACE"))
                .ok_or_else(|| invalid("invalid refine value"))?,
        }
        .to_owned();
        if object.contains_key("implicitTiling") {
            return self.implicit(node, schema, &refine);
        }
        if contents(node).iter().any(|c| is_json(&c["uri"])) {
            return self.link(node, schema, &refine, documents);
        }
        let mut tile: Map<String, Value> = object
            .iter()
            .filter(|(key, _)| key.as_str() != "children")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let own = cost(&Value::Object(tile.clone()))?;
        self.budget.charge(1, own.bytes)?;
        if let Some(children) = object.get("children") {
            let children = children
                .as_array()
                .ok_or_else(|| invalid("children must be an array"))?;
            let mut expanded = Vec::with_capacity(children.len());
            for child in children {
                expanded.push(self.tile(child, schema, &refine, documents)?);
            }
            tile.insert("children".into(), Value::Array(expanded));
        }
        Ok(Value::Object(tile))
    }

    /// An explicit tile whose content is an explicit external tileset document.
    fn link(
        &mut self,
        node: &Value,
        schema: &Value,
        refine: &str,
        documents: usize,
    ) -> Result<Value, Error> {
        let uri = node["content"]["uri"]
            .as_str()
            .filter(|uri| !uri.contains(['/', '\\', ':']))
            .ok_or_else(|| invalid("an external tileset link must be one archive-root content"))?
            .to_owned();
        if node["children"].as_array().is_some_and(|c| !c.is_empty())
            || node.get("implicitTiling").is_some()
        {
            return Err(invalid("an external tileset link cannot have children"));
        }
        if documents >= MAX_DOCUMENT_CHAIN || !self.active.insert(uri.clone()) {
            return Err(invalid("external tileset chain is too deep or cyclic"));
        }
        let bytes = (self.read)(&uri)?;
        self.budget.charge(0, bytes.len())?;
        let document = (self.parse_json)(&bytes)?;
        let root = &document["root"];
        if document["asset"]["version"] != "1.1"
            || root.get("transform").is_some()
            || root["boundingVolume"] != node["boundingVolume"]
            || root["geometricError"] != node["geometricError"]
            || root.get("refine").is_some_and(|r| r != refine)
        {
            return Err(invalid(
                "external tileset link changes its root's bounds, error, refine or placement",
            ));
        }
        let schema = if document.get("schema").is_some() {
            &document["schema"]
        } else {
            schema
        };
        let mut expanded = self.tile(root, schema, refine, documents + 1)?;
        if let Some(transform) = node.get("transform") {
            expanded["transform"] = transform.clone();
        }
        self.active.remove(&uri);
        Ok(expanded)
    }

    fn implicit(&mut self, root: &Value, schema: &Value, refine: &str) -> Result<Value, Error> {
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
        let templates = contents(root);
        if templates.iter().any(|t| is_json(&t["uri"])) {
            return Err(invalid(
                "JSON content templates on implicit roots are not supported",
            ));
        }
        // Apply the same bounded allocation policy as the writer before parsing.
        let slots = scheme
            .branches()
            .checked_pow(levels)
            .filter(|&n| n <= self.budget.max_nodes)
            .ok_or_else(|| expansion_limit("implicit subtree exceeds admitted node capacity"))?;
        slots
            .checked_mul(templates.len().saturating_add(2))
            .filter(|&n| n <= self.budget.max_bytes)
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
        let metadata_required = schema["classes"].get("rustyTile").is_some();
        if metadata_required {
            let properties = &schema["classes"]["rustyTile"]["properties"];
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
            read: &mut *self.read,
            scheme,
            levels,
            available,
            tile_count,
            child_count,
            templates,
            subtree_template,
            root_box,
            root_error,
            refine,
            metadata_required,
            cache: BTreeMap::new(),
            budget: &mut *self.budget,
            cache_bytes: 0,
            parse_json: self.parse_json,
        };
        let mut expanded = reader.node(Coordinates {
            level: 0,
            x: 0,
            y: 0,
            z: 0,
        })?;
        if metadata_required {
            let bounds: [f64; 12] =
                serde_json::from_value(expanded["boundingVolume"]["box"].clone())?;
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
        if let Some(transform) = root.get("transform") {
            expanded["transform"] = transform.clone();
        }
        Ok(expanded)
    }
}

fn is_json(uri: &Value) -> bool {
    uri.as_str()
        .is_some_and(|uri| uri.to_ascii_lowercase().ends_with(".json"))
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

/// The regular cell of `c`, for implicit tiles without semantic rows.
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
    parse_json: &dyn Fn(&[u8]) -> Result<Value, Error>,
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
    refine: &'a str,
    metadata_required: bool,
    cache: BTreeMap<Coordinates, Parsed>,
    budget: &'a mut ExpansionBudget,
    cache_bytes: usize,
    parse_json: &'a dyn Fn(&[u8]) -> Result<Value, Error>,
}

impl<F: FnMut(&str) -> Result<Vec<u8>, Error>> Reader<'_, F> {
    fn node(&mut self, c: Coordinates) -> Result<Value, Error> {
        self.budget.charge(1, 0)?;
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
                    self.parse_json,
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
        let mut node = json!({"boundingVolume":{"box":metadata.map_or(bounds,|m|m.bounding_box)},"geometricError":metadata.map_or(self.root_error/width,|m|m.geometric_error),"refine":self.refine});
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
        self.budget.charge(0, serde_json::to_vec(&node)?.len())?;
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
    fn selected_capturing_parser_reaches_subtree_and_nested_extras() {
        use crate::content_integrity::{json, FormatError, JsonLimits};
        use std::cell::Cell;
        // Independent one-row binary columns. Nested extras exceed the same
        // root-inclusive node ceiling while the subtree document remains below it.
        let extras = format!("[{}]", vec!["0"; 128].join(",")).into_bytes();
        let string_start = 104usize;
        let offset_start = (string_start + extras.len()).next_multiple_of(8);
        let logical_length = offset_start + 8;
        let mut binary = vec![0u8; logical_length];
        for (lane, value) in [0f64, 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1.]
            .into_iter()
            .enumerate()
        {
            binary[lane * 8..lane * 8 + 8].copy_from_slice(&value.to_le_bytes());
        }
        binary[96..104].copy_from_slice(&1f64.to_le_bytes());
        binary[string_start..string_start + extras.len()].copy_from_slice(&extras);
        binary[offset_start + 4..offset_start + 8]
            .copy_from_slice(&(extras.len() as u32).to_le_bytes());
        let doc = json!({"buffers":[{"byteLength":logical_length}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":96},
                {"buffer":0,"byteOffset":96,"byteLength":8},
                {"buffer":0,"byteOffset":104,"byteLength":extras.len()},
                {"buffer":0,"byteOffset":offset_start,"byteLength":8}],
            "tileAvailability":{"constant":1},"contentAvailability":[],
            "childSubtreeAvailability":{"constant":0},"tileMetadata":0,
            "propertyTables":[{"class":"rustyTile","count":1,"properties":{
                "boundingBox":{"values":0},"geometricError":{"values":1},
                "extras":{"values":2,"stringOffsets":3}}}]});
        let mut encoded = serde_json::to_vec(&doc).unwrap();
        while !encoded.len().is_multiple_of(8) {
            encoded.push(b' ');
        }
        let mut bytes = b"subt".to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&(encoded.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(binary.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&encoded);
        bytes.extend_from_slice(&binary);
        let calls = Cell::new(0);
        let limits = JsonLimits {
            bytes: 4096,
            depth: 64,
            value_nodes: 128,
        };
        let parser = |bytes: &[u8]| {
            calls.set(calls.get() + 1);
            json::parse(bytes, limits).map_err(|error| match error {
                FormatError::ResourceLimit(message) => expansion_limit(&message),
                other => invalid(&other.to_string()),
            })
        };
        assert!(matches!(
            parse(&bytes, 1, 4, 0, 4, true, &parser),
            Err(Error::Validation(
                crate::validate::ValidationFailure::ResourceLimit(_)
            ))
        ));
        assert_eq!(calls.get(), 2);
        // A capturing parser also reaches a regular subtree through Reader.
        calls.set(0);
        expand_tileset_bounded(
            &manifest(1),
            |_| Ok(subtree()),
            &mut ExpansionBudget::new(4, 4096),
            &parser,
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn selected_parser_reaches_linked_external_document() {
        use crate::content_integrity::{json, FormatError, JsonLimits};
        use std::cell::Cell;
        // An explicit link tile names an explicit external document whose extras exceed
        // the parser's node ceiling.
        let initial = json!({"asset":{"version":"1.1"},"geometricError":1,
            "root":{"boundingVolume":{"box":[0,0,0,1,0,0,0,1,0,0,0,1]},"geometricError":1,
                "refine":"REPLACE","content":{"uri":"tileset-d1.json"}}});
        let mut external = manifest(1);
        external["extras"] = json!(vec![0; 128]);
        let external_bytes = serde_json::to_vec(&external).unwrap();
        let calls = Cell::new(0);
        let limits = JsonLimits {
            bytes: 4096,
            depth: 64,
            value_nodes: 128,
        };
        let parser = |bytes: &[u8]| {
            calls.set(calls.get() + 1);
            json::parse(bytes, limits).map_err(|error| match error {
                FormatError::ResourceLimit(message) => expansion_limit(&message),
                other => invalid(&other.to_string()),
            })
        };
        let mut reads = Vec::new();
        let result = expand_tileset_bounded(
            &initial,
            |name| {
                reads.push(name.to_owned());
                Ok(external_bytes.clone())
            },
            &mut ExpansionBudget::new(4, 4096),
            &parser,
        );
        assert!(is_limit(result));
        assert_eq!(calls.get(), 1);
        assert_eq!(reads, ["tileset-d1.json"]);
    }

    #[test]
    fn json_content_templates_on_implicit_roots_are_not_followed() {
        let mut templated = manifest(1);
        templated["root"]["content"] = json!({"uri":"implicit-tileset-{level}-{x}-{y}.json"});
        let result = expand_tileset_bounded(
            &templated,
            |_| panic!("no resource is read for a JSON template"),
            &mut ExpansionBudget::new(64, 4096),
            &producer_json,
        );
        assert!(
            matches!(&result, Err(Error::Data(m)) if m.contains("JSON content templates")),
            "{result:?}"
        );
    }

    #[test]
    fn node_capacity_is_shared_across_documents() {
        let mut budget = ExpansionBudget::new(4, 4096);
        for _ in 0..4 {
            expand_tileset_bounded(&manifest(1), |_| Ok(subtree()), &mut budget, &producer_json)
                .unwrap();
        }
        assert!(is_limit(expand_tileset_bounded(
            &manifest(1),
            |_| panic!("fail before another resource read"),
            &mut budget,
            &producer_json
        )));
    }

    #[test]
    fn one_budget_spans_linked_documents_and_their_implicit_roots() {
        // tileset.json: an explicit root linking to a document whose root is implicit.
        let initial = json!({"asset":{"version":"1.1"},"geometricError":1,
            "root":{"boundingVolume":{"box":[0,0,0,1,0,0,0,1,0,0,0,1]},"geometricError":1,
                "refine":"REPLACE","children":[
                    {"boundingVolume":{"box":[0,0,0,1,0,0,0,1,0,0,0,1]},"geometricError":1,
                     "content":{"uri":"tileset-d1.json"}}]}});
        let document = serde_json::to_vec(&manifest(1)).unwrap();
        let read = |name: &str| {
            Ok(if name.ends_with(".json") {
                document.clone()
            } else {
                subtree()
            })
        };
        // Each expansion charges two nodes to the one budget: the explicit root and the
        // single implicit tile of the linked document.
        let mut budget = ExpansionBudget::new(5, 4096);
        expand_tileset_bounded(&initial, read, &mut budget, &producer_json).unwrap();
        assert_eq!(budget.nodes, 2);
        expand_tileset_bounded(&initial, read, &mut budget, &producer_json).unwrap();
        assert_eq!(budget.nodes, 4);
        assert!(is_limit(expand_tileset_bounded(
            &initial,
            read,
            &mut budget,
            &producer_json,
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
            &mut budget,
            &producer_json
        )));
        assert_eq!(reads, 1);
        assert!(is_limit(expand_tileset_bounded(
            &manifest(16),
            |_| panic!("admission before resource read"),
            &mut ExpansionBudget::new(64, 4096),
            &producer_json
        )));
    }
}
