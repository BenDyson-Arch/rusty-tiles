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
use crate::tileset_node::{
    self, sum_error, sum_up, ContentRef, ContentTemplate, GeometricError, ImplicitTiling,
    MemberName, ObjectRole, TileBox, TileRecord, TileTree, TilesetDocument, TilesetExtension,
    UriTemplate,
};
use crate::Error;
use serde::{Deserialize, Serialize};
use serde_json::{json, value::RawValue, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Component, Path},
};

mod stream;
#[cfg_attr(not(test), allow(unused_imports))]
pub(crate) use stream::{
    Availability, ChunkOutput, DocumentLink, ForestTile, ForestWriter, ImplicitNames, RootName,
};

/// S: a tile whose subtree height is at least S is an explicit skeleton tile (design §4.1).
const SKELETON_HEIGHT: u32 = 4;
/// P9: the writer refuses hierarchies with more levels than this.
pub(crate) const MAX_LEVELS: u32 = 32;
/// P8: value nodes per JSON document (validator semantics: the root, containers and scalars).
pub(crate) const DOC_NODES: usize = 16_384;
/// P8: compact serialized bytes per JSON document.
pub(crate) const DOC_BYTES: usize = 2 * 1024 * 1024;
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
pub(crate) const DOC_DEPTH: usize = 32;
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
    /// Empty for typed records, whose objects [`Records`] emits.
    tile: Map<String, Value>,
    /// The written `extras` object (empty when absent).
    extras: Value,
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
    /// Declares `3DTILES_content_gltf_vector` (typed records; the header carries it).
    vector: bool,
    /// The typed record's declaration, checked against the staged payload when the forest
    /// reads it (length and sha256). `None` for the JSON entry.
    declared: Option<ContentRef>,
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
            vector: false,
            declared: None,
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
    let extras = tile.get("extras").cloned().unwrap_or_else(|| json!({}));
    Ok(Node {
        tile,
        extras,
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

/// Link the next preorder tile `node` (id `nodes.len()`) to its parent at `slot`, inheriting
/// the parent's refine when the tile has none.
fn adopt(
    nodes: &mut [Node],
    node: &mut Node,
    parent: Option<usize>,
    slot: usize,
    scheme: SubdivisionScheme,
) -> Result<(), Error> {
    let id = nodes.len();
    if let Some(parent) = parent {
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
    Ok(())
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
        let slot = match (parent, value["extras"].get("implicitChildIndex")) {
            (Some(_), Some(slot)) => slot
                .as_u64()
                .and_then(|s| usize::try_from(s).ok())
                .ok_or_else(|| data(format!("tile {id} has an invalid implicitChildIndex")))?,
            _ => index,
        };
        adopt(&mut nodes, &mut node, parent, slot, scheme)?;
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
    measure(&mut nodes)?;
    Ok(nodes)
}

/// Collect typed H in preorder. A child's slot is its index among its parent's children.
fn collect_records<'a, X: Serialize>(
    tree: &'a TileTree<X>,
    scheme: SubdivisionScheme,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<(Vec<Node>, Vec<&'a TileRecord<X>>), Error> {
    let order = tree.preorder();
    let mut nodes: Vec<Node> = Vec::with_capacity(order.len());
    let mut records = Vec::with_capacity(order.len());
    for (record, parent, index) in order {
        checkpoint()?;
        let id = nodes.len();
        let translation = match parent {
            None => [0.; 3],
            Some(_) => record.placement().child_translation().ok_or_else(|| {
                data(format!(
                    "tile {id}: a child transform must be a translation"
                ))
            })?,
        };
        let contents = record
            .contents()
            .iter()
            .map(|content| {
                if !content.uri().is_member() {
                    return Err(data(format!(
                        "tile {id}: content {:?} is not an archive member name",
                        content.uri().as_str()
                    )));
                }
                Ok(Content {
                    uri: content.uri().as_str().to_owned(),
                    suffix: content.kind().suffix().to_owned(),
                    header: content.header(),
                    vector: content.vector(),
                    declared: Some(content.clone()),
                })
            })
            .collect::<Result<_, Error>>()?;
        let mut node = Node {
            tile: Map::new(),
            extras: record
                .extras_object()?
                .map_or_else(|| json!({}), Value::Object),
            children: Vec::new(),
            parent: None,
            slot: 0,
            translation,
            bounding_box: record.bounds().lanes(),
            error: record.geometric_error().metres(),
            level: 0,
            height: 0,
            end: id + 1,
            refine: record.refine().as_str(),
            contents,
        };
        adopt(&mut nodes, &mut node, parent, index, scheme)?;
        nodes.push(node);
        records.push(record);
    }
    measure(&mut nodes)?;
    Ok((nodes, records))
}

/// Check the level count and child slots, and set every tile's height and subtree end.
fn measure(nodes: &mut [Node]) -> Result<(), Error> {
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
    Ok(())
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
    if node
        .tile
        .get("boundingVolume")
        .and_then(Value::as_object)
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

fn coordinate_template(scheme: SubdivisionScheme) -> &'static str {
    if scheme == SubdivisionScheme::Octree {
        "{level}-{x}-{y}-{z}"
    } else {
        "{level}-{x}-{y}"
    }
}

fn schema() -> Value {
    tileset_node::tile_schema()
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

/// An implicit root's content template for one slot, as the layout names it.
struct Template {
    uri: String,
    header: Map<String, Value>,
    vector: bool,
}

/// An implicit root's single-subtree tiling, as the layout names it.
struct Tiling {
    scheme: SubdivisionScheme,
    levels: u32,
    subtrees: String,
}

/// How the forest emits the JSON objects it lays out. [`Loose`] copies a producer's JSON
/// hierarchy (the migration shim for callers not yet on typed records); [`Records`] emits
/// every tile object and document through `tileset_node` and reads no JSON back.
trait Emit {
    /// A skeleton tile or explicit leaf without `children`, at the tile's (widened) bounds.
    fn explicit(&self, id: usize, node: &Node) -> Result<Map<String, Value>, Error>;
    /// An assembled skeleton tile as the root of an external document: no `transform`
    /// (its link carries it) and an explicit `refine`.
    fn document_root(
        &self,
        id: usize,
        node: &Node,
        root: Map<String, Value>,
    ) -> Result<Map<String, Value>, Error>;
    /// The explicit link tile to the document `name` that this skeleton tile roots.
    fn link(&self, id: usize, node: &Node, name: &str) -> Result<Map<String, Value>, Error>;
    /// The implicit root tile object at bounds `widened`.
    fn implicit_root(
        &self,
        id: usize,
        node: &Node,
        widened: &[f64; 12],
        templates: &[Template],
        tiling: &Tiling,
    ) -> Result<Map<String, Value>, Error>;
    /// `tileset.json` without `root`, with `schema`.
    fn root_head(&self) -> Result<Map<String, Value>, Error>;
    /// An external document without `root`.
    fn external_head(&self, error: f64, schema: bool) -> Result<Map<String, Value>, Error>;
}

/// The producer's JSON hierarchy and its other `tileset.json` members.
struct Loose<'a> {
    manifest: &'a Value,
}

impl Emit for Loose<'_> {
    fn explicit(&self, _: usize, node: &Node) -> Result<Map<String, Value>, Error> {
        Ok(node.tile.clone())
    }

    fn document_root(
        &self,
        _: usize,
        node: &Node,
        mut root: Map<String, Value>,
    ) -> Result<Map<String, Value>, Error> {
        root.remove("transform");
        root.insert("refine".into(), json!(node.refine));
        Ok(root)
    }

    fn link(&self, _: usize, node: &Node, name: &str) -> Result<Map<String, Value>, Error> {
        Ok(link_tile(node, name))
    }

    fn implicit_root(
        &self,
        _: usize,
        node: &Node,
        widened: &[f64; 12],
        templates: &[Template],
        tiling: &Tiling,
    ) -> Result<Map<String, Value>, Error> {
        let mut object = Map::new();
        object.insert("boundingVolume".into(), node.tile["boundingVolume"].clone());
        if *widened != node.bounding_box {
            set_halves(&mut object["boundingVolume"], &node.bounding_box, widened);
        }
        object.insert("geometricError".into(), node.tile["geometricError"].clone());
        object.insert("refine".into(), json!(node.refine));
        if let Some(transform) = node.tile.get("transform") {
            object.insert("transform".into(), transform.clone());
        }
        set_contents(
            &mut object,
            templates
                .iter()
                .map(|template| {
                    let mut header = template.header.clone();
                    header.insert("uri".into(), json!(template.uri));
                    Value::Object(header)
                })
                .collect(),
        );
        object.insert(
            "implicitTiling".into(),
            json!({
                "subdivisionScheme": tiling.scheme.as_str(),
                "subtreeLevels": tiling.levels,
                "availableLevels": tiling.levels,
                "subtrees": {"uri": tiling.subtrees},
            }),
        );
        Ok(object)
    }

    fn root_head(&self) -> Result<Map<String, Value>, Error> {
        let mut top: Map<String, Value> = self
            .manifest
            .as_object()
            .ok_or_else(|| data("tileset must be an object"))?
            .iter()
            .filter(|(key, _)| key.as_str() != "root")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        top.insert("schema".into(), schema());
        Ok(top)
    }

    fn external_head(&self, error: f64, with_schema: bool) -> Result<Map<String, Value>, Error> {
        let mut head = Map::new();
        head.insert("asset".into(), self.manifest["asset"].clone());
        head.insert("geometricError".into(), json!(error));
        for key in ["extensionsUsed", "extensionsRequired"] {
            if let Some(value) = self.manifest.get(key) {
                head.insert(key.into(), value.clone());
            }
        }
        if with_schema {
            head.insert("schema".into(), schema());
        }
        Ok(head)
    }
}

/// Typed H in preorder and its `tileset.json` members.
struct Records<'a, X> {
    records: Vec<&'a TileRecord<X>>,
    document: &'a TilesetDocument,
}

impl<X: Serialize> Records<'_, X> {
    fn bounds(&self, id: usize, lanes: &[f64; 12]) -> Result<TileBox, Error> {
        self.records[id].bounds().with_half_lanes(lanes)
    }
}

impl<X: Serialize> Emit for Records<'_, X> {
    fn explicit(&self, id: usize, node: &Node) -> Result<Map<String, Value>, Error> {
        self.records[id].explicit_object(&self.bounds(id, &node.bounding_box)?, ObjectRole::Tile)
    }

    fn document_root(
        &self,
        id: usize,
        node: &Node,
        mut root: Map<String, Value>,
    ) -> Result<Map<String, Value>, Error> {
        let mut object = self.records[id].explicit_object(
            &self.bounds(id, &node.bounding_box)?,
            ObjectRole::DocumentRoot,
        )?;
        if let Some(children) = root.remove("children") {
            object.insert("children".into(), children);
        }
        Ok(object)
    }

    fn link(&self, id: usize, node: &Node, name: &str) -> Result<Map<String, Value>, Error> {
        Ok(self.records[id].link_object(
            &self.bounds(id, &node.bounding_box)?,
            &MemberName::new(name)?,
        ))
    }

    fn implicit_root(
        &self,
        id: usize,
        _: &Node,
        widened: &[f64; 12],
        templates: &[Template],
        tiling: &Tiling,
    ) -> Result<Map<String, Value>, Error> {
        let templates = templates
            .iter()
            .map(|t| {
                Ok(ContentTemplate::new(
                    UriTemplate::new(t.uri.as_str(), tiling.scheme)?,
                    t.vector,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let tiling = ImplicitTiling::single_subtree(
            tiling.scheme,
            tiling.levels,
            UriTemplate::new(tiling.subtrees.as_str(), tiling.scheme)?,
        )?;
        self.records[id].implicit_root_object(&self.bounds(id, widened)?, &templates, &tiling)
    }

    fn root_head(&self) -> Result<Map<String, Value>, Error> {
        Ok(self.document.clone().with_schema(true).head())
    }

    fn external_head(&self, error: f64, schema: bool) -> Result<Map<String, Value>, Error> {
        let extensions = self.document.extensions_used().iter().copied();
        Ok(
            TilesetDocument::external(GeometricError::new(error)?, extensions)
                .with_schema(schema)
                .head(),
        )
    }
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

struct Layout<'a> {
    nodes: &'a [Node],
    roles: &'a [TileRole],
    pieces: &'a Pieces,
    linked: &'a BTreeMap<usize, usize>,
    documents: &'a [Document],
}

fn assemble(id: usize, layout: &Layout, emit: &impl Emit) -> Result<Map<String, Value>, Error> {
    let Layout {
        nodes,
        roles,
        pieces,
        linked,
        documents,
    } = layout;
    let mut tile = pieces.objects[id].clone().expect("explicit tile object");
    if roles[id] == TileRole::Skeleton {
        let children = nodes[id]
            .children
            .iter()
            .map(|&child| {
                Ok(Value::Object(match (roles[child], linked.get(&child)) {
                    (TileRole::Skeleton, Some(&document)) => {
                        emit.link(child, &nodes[child], &documents[document].name)?
                    }
                    (TileRole::Skeleton, None) => assemble(child, layout, emit)?,
                    _ => pieces.objects[child].clone().expect("unit child object"),
                }))
            })
            .collect::<Result<_, Error>>()?;
        tile.insert("children".into(), Value::Array(children));
    }
    Ok(tile)
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
    let nodes = collect(&manifest["root"], scheme, checkpoint)?;
    let (top, output) = forest(nodes, &Loose { manifest }, directory, scheme, checkpoint)?;
    *manifest = Value::Object(top);
    Ok(output)
}

/// Write the bounded implicit forest for a producer's typed hierarchy `tree`, whose
/// `tileset.json` members other than `root` are `document`.
///
/// The layout, payloads, subtrees and documents are those [`write_forest`] writes for the
/// same hierarchy serialized by `tileset_node`; every JSON object is emitted from the
/// records. Returns `tileset.json` (with `schema`), which the producer serializes.
pub(crate) fn write_forest_records<X: Serialize>(
    tree: &TileTree<X>,
    document: &TilesetDocument,
    directory: &Path,
    scheme: SubdivisionScheme,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<(Value, ForestOutput), Error> {
    checkpoint()?;
    if !document.is_root() {
        return Err(data("a forest is written from tileset.json's members"));
    }
    // External documents copy no asset extras: the digest covers the written forest, so it
    // is stamped afterwards, as the JSON entry's producers do.
    if document.build_state().is_some() {
        return Err(data(
            "the build state is recorded after the forest is written",
        ));
    }
    let (nodes, records) = collect_records(tree, scheme, checkpoint)?;
    let vector = TilesetExtension::ContentGltfVector;
    if !document.extensions_used().contains(&vector)
        && records
            .iter()
            .any(|record| record.contents().iter().any(|c| c.vector()))
    {
        return Err(data(format!(
            "vector content needs {} in extensionsUsed",
            vector.as_str()
        )));
    }
    let emit = Records { records, document };
    let (top, output) = forest(nodes, &emit, directory, scheme, checkpoint)?;
    Ok((Value::Object(top), output))
}

/// The forest core: lay out H, write payloads, subtrees and external documents, and return
/// `tileset.json` with the per-tile record.
fn forest(
    mut nodes: Vec<Node>,
    emit: &impl Emit,
    directory: &Path,
    scheme: SubdivisionScheme,
    checkpoint: &mut impl FnMut() -> Result<(), Error>,
) -> Result<(Map<String, Value>, ForestOutput), Error> {
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
                    let bytes = match &content.declared {
                        Some(declared) => declared.verify_staged(directory)?,
                        None => std::fs::metadata(directory.join(&content.uri))?.len(),
                    };
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
                    extras: node.extras.clone(),
                    contents,
                });
            }
            TileRole::ImplicitRoot => {
                checkpoint()?;
                implicit_roots += 1;
                std::fs::create_dir_all(directory.join("implicit-content"))?;
                std::fs::create_dir_all(directory.join("subtrees"))?;
                let name = format!("r{id}");
                let root = ImplicitRoot {
                    id,
                    names: RootNames::Prefix(&name),
                    scheme,
                    directory,
                    template,
                };
                let (object, subtree, reach, _) = root.write(
                    &nodes,
                    emit,
                    &mut tiles,
                    &mut members,
                    &mut implicit_sources,
                    Publish::Write,
                    None,
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
    // them by the drift so they also contain it where it is published. Only skeleton tiles
    // have drift: an implicit root has no implicit root below it.
    for (id, by) in drift.into_iter().enumerate() {
        let before = nodes[id].bounding_box;
        let mut after = before;
        if widen(&mut after, by) {
            debug_assert_eq!(roles[id], TileRole::Skeleton);
            let node = &mut nodes[id];
            node.bounding_box = after;
            if let Some(volume) = node.tile.get_mut("boundingVolume") {
                set_halves(volume, &before, &after);
            }
        }
    }
    for (id, node) in nodes.iter().enumerate() {
        if matches!(roles[id], TileRole::Skeleton | TileRole::ExplicitLeaf) {
            objects[id] = Some(emit.explicit(id, node)?);
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
            links[id] = cost(&Value::Object(emit.link(
                id,
                &nodes[id],
                LINK_RESERVATION,
            )?))?;
        }
    }
    let pieces = Pieces {
        objects,
        costs,
        links,
    };
    let mut top = emit.root_head()?;
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
        let mut document_top = emit.external_head(f64::MAX, true)?;
        document_top.insert("root".into(), Value::Null);
        let reserve = cost(&Value::Object(document_top))?;
        let reserve = Cost {
            nodes: reserve.nodes.max(top_cost.nodes),
            bytes: reserve.bytes.max(top_cost.bytes),
            depth: reserve.depth.max(top_cost.depth),
        };
        let (packed, linked) = pack(&nodes, &roles, &pieces, reserve, checkpoint)?;
        documents = packed.len();
        let layout = Layout {
            nodes: &nodes,
            roles: &roles,
            pieces: &pieces,
            linked: &linked,
            documents: &packed,
        };
        for (index, document) in packed.iter().enumerate() {
            checkpoint()?;
            let root = assemble(document.root, &layout, emit)?;
            if index == 0 {
                top.insert("root".into(), Value::Object(root));
                check_document("tileset.json", &Value::Object(top.clone()))?;
                continue;
            }
            let node = &nodes[document.root];
            let root = emit.document_root(document.root, node, root)?;
            let mut body = emit.external_head(node.error, document.holds_root)?;
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
    Ok((
        top,
        ForestOutput {
            members,
            tiles: tiles
                .into_iter()
                .map(|t| t.expect("every tile is written"))
                .collect(),
            documents,
            implicit_roots,
        },
    ))
}

/// An implicit root as written: its explicit tile object, its subtree member name, its
/// drift bound (z-up), by which its explicit ancestors grow, and its derived availability.
type WrittenRoot = (Map<String, Value>, String, [f64; 3], Availability);

/// How an implicit root names its members.
pub(crate) enum RootNames<'a> {
    /// `implicit-content/{prefix}-{level}-{x}-{y}[-{z}]-{slot}.{suffix}` and
    /// `subtrees/{prefix}-{level}-{x}-{y}[-{z}].subtree` (`r{id}` in a whole-tree forest).
    Prefix(&'a str),
    /// The caller's content templates, one per content slot in slot order, and subtree
    /// template.
    Templates {
        contents: &'a [UriTemplate],
        subtree: &'a UriTemplate,
    },
}

/// How an implicit root's members are published.
enum Publish<'p> {
    /// The whole-tree forest: write every member.
    Write,
    /// The streamed forest: create every member new and never copy an unchanged payload.
    /// A payload staged at its own final member stays in place; an unchanged payload staged
    /// elsewhere is hard-linked (copied only where the filesystem has no hard links).
    /// Everything created is recorded, so a refused document can be removed.
    Fresh(&'p mut Created),
}

/// Files and directories a streamed document created, in creation order.
#[derive(Debug, Default)]
pub(crate) struct Created {
    files: Vec<std::path::PathBuf>,
    dirs: Vec<std::path::PathBuf>,
}

impl Created {
    /// Create the missing parent directories of `member` under `directory`.
    fn parents(&mut self, directory: &Path, member: &str) -> Result<(), Error> {
        let mut missing = Vec::new();
        let mut parent = directory.join(member);
        while parent.pop() && parent != directory && !parent.exists() {
            missing.push(parent.clone());
        }
        for dir in missing.into_iter().rev() {
            std::fs::create_dir(&dir)?;
            self.dirs.push(dir);
        }
        Ok(())
    }

    /// Create `member` with `bytes`, refusing an existing member.
    fn file(&mut self, directory: &Path, member: &str, bytes: &[u8]) -> Result<(), Error> {
        self.parents(directory, member)?;
        let path = directory.join(member);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| exists(member, error))?;
        self.files.push(path);
        std::io::Write::write_all(&mut file, bytes)?;
        Ok(())
    }

    /// Publish the unchanged payload `raw` as `member` without a second copy of its bytes.
    fn link(
        &mut self,
        directory: &Path,
        raw: &str,
        member: &str,
        bytes: &[u8],
    ) -> Result<(), Error> {
        self.parents(directory, member)?;
        match std::fs::hard_link(directory.join(raw), directory.join(member)) {
            Ok(()) => {
                self.files.push(directory.join(member));
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(exists(member, error))
            }
            // A filesystem without hard links: one copy.
            Err(_) => self.file(directory, member, bytes),
        }
    }

    /// Remove everything created, newest first (best effort: a refusal is already being
    /// reported).
    pub(crate) fn remove(self) {
        for file in self.files.iter().rev() {
            let _ = std::fs::remove_file(file);
        }
        for dir in self.dirs.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

fn exists(member: &str, error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        data(format!("member {member:?} is already written"))
    } else {
        error.into()
    }
}

/// The Morton index of `c` within its level: x, y (and z) bits interleaved from the least
/// significant up.
fn morton(c: Coordinates, scheme: SubdivisionScheme) -> u128 {
    let mut index = 0u128;
    let axes: &[u32] = match scheme {
        SubdivisionScheme::Quadtree => &[c.x, c.y],
        SubdivisionScheme::Octree => &[c.x, c.y, c.z],
    };
    for bit in 0..c.level {
        for (k, axis) in axes.iter().enumerate() {
            index |= u128::from((axis >> bit) & 1) << (bit as usize * axes.len() + k);
        }
    }
    index
}

struct ImplicitRoot<'a> {
    id: usize,
    names: RootNames<'a>,
    scheme: SubdivisionScheme,
    directory: &'a Path,
    template: &'static str,
}

impl ImplicitRoot<'_> {
    /// Write one implicit root's payloads and its single subtree. Returns the explicit root
    /// tile object, the subtree member name, the root's drift bound (see below), by which
    /// its explicit ancestors must grow, and the availability its slots derive. A planned
    /// availability (`expected`) that differs is refused before anything is read or written.
    ///
    /// Implicit tile `t` is published at `δ(t) = δ(parent) + translation(t)`, rounded per
    /// edge, so its row centre `B(t) + δ(t)` and its payload's rebased scene root (each
    /// rounded once more) drift from the producer's exact placement by a few ulps. Every
    /// row grows by an outward-rounded bound on the drift of its own and its descendants'
    /// content relative to it, so a vertex the producer's hierarchy contains exactly stays
    /// inside every volume C2 tests. Exact sums give a zero bound and unchanged rows.
    #[allow(clippy::too_many_arguments)]
    fn write(
        &self,
        nodes: &[Node],
        emit: &impl Emit,
        tiles: &mut [Option<WrittenTile>],
        members: &mut Vec<String>,
        sources: &mut BTreeSet<String>,
        mut publish: Publish<'_>,
        expected: Option<&Availability>,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<WrittenRoot, Error> {
        let (id, scheme, template) = (self.id, self.scheme, self.template);
        let root = &nodes[id];
        let range = id..root.end;
        // R-4: the slot list drives the templates and the content layers, in the same order.
        let mut slots: BTreeMap<(usize, String), (Map<String, Value>, bool)> = BTreeMap::new();
        for t in range.clone() {
            for (index, content) in nodes[t].contents.iter().enumerate() {
                match slots.get(&(index, content.suffix.clone())) {
                    None => {
                        slots.insert(
                            (index, content.suffix.clone()),
                            (content.header.clone(), content.vector),
                        );
                    }
                    Some((header, vector))
                        if *header != content.header || *vector != content.vector =>
                    {
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
        let (content_templates, subtree_template) = match &self.names {
            RootNames::Prefix(name) => (
                slot_list
                    .iter()
                    .enumerate()
                    .map(|(slot, (_, suffix))| {
                        format!("implicit-content/{name}-{template}-{slot}.{suffix}")
                    })
                    .collect::<Vec<_>>(),
                format!("subtrees/{name}-{template}.subtree"),
            ),
            RootNames::Templates { contents, subtree } => {
                if contents.len() != slot_list.len() {
                    return Err(data(format!(
                        "tile {id}: {} content templates for {} content slots",
                        contents.len(),
                        slot_list.len()
                    )));
                }
                for (uri, (_, suffix)) in contents.iter().zip(&slot_list) {
                    if !uri.as_str().ends_with(&format!(".{suffix}")) {
                        return Err(data(format!(
                            "tile {id}: content template {:?} does not name a .{suffix} slot",
                            uri.as_str()
                        )));
                    }
                }
                (
                    contents.iter().map(|t| t.as_str().to_owned()).collect(),
                    subtree.as_str().to_owned(),
                )
            }
        };
        let origin = Coordinates {
            level: 0,
            x: 0,
            y: 0,
            z: 0,
        };
        // Coordinates from the slots, and the availability they derive, before any I/O.
        let mut coordinates = BTreeMap::new();
        coordinates.insert(id, origin);
        for t in range.clone() {
            let c = coordinates[&t];
            for &next in &nodes[t].children {
                coordinates.insert(next, child(c, nodes[next].slot, scheme)?);
            }
        }
        let order = |c: &Coordinates| (c.level, morton(*c, scheme));
        let mut available: Vec<Coordinates> = range.clone().map(|t| coordinates[&t]).collect();
        available.sort_by_key(order);
        let mut contents_available = vec![Vec::new(); slot_list.len()];
        for t in range.clone() {
            for (index, content) in nodes[t].contents.iter().enumerate() {
                let slot = slot_list
                    .iter()
                    .position(|(i, s)| *i == index && *s == content.suffix)
                    .expect("slot list covers every content");
                contents_available[slot].push(coordinates[&t]);
            }
        }
        for cells in &mut contents_available {
            cells.sort_by_key(order);
        }
        let derived = Availability {
            levels: root.height + 1,
            tiles: available,
            contents: contents_available,
        };
        if let Some(expected) = expected {
            if let Some(difference) = expected.difference(&derived) {
                return Err(data(format!(
                    "implicit root {subtree_template:?}: availability mismatch: {difference}"
                )));
            }
        }
        let mut subtree = Subtree::new(scheme, root.height + 1, slot_list.len())?;
        let mut deltas: BTreeMap<usize, [f64; 3]> = BTreeMap::new();
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
                let raw = match &content.declared {
                    Some(declared) => declared.read_staged(self.directory)?,
                    None => std::fs::read(self.directory.join(&content.uri))?,
                };
                let raw_sha256: [u8; 32] = Sha256::digest(&raw).into();
                if let Some(declared) = &content.declared {
                    declared.check_staged(raw.len() as u64, &raw_sha256)?;
                }
                let (bytes, record) = rebase_hashed(&raw, raw_sha256, delta)?;
                for (lane, error) in own.iter_mut().zip(rebase_error(&record)?) {
                    *lane = lane.max(error);
                }
                let member = template_uri(&content_templates[slot], c, scheme)?;
                match &mut publish {
                    Publish::Write => {
                        std::fs::write(self.directory.join(&member), &bytes)?;
                        sources.insert(content.uri.clone());
                    }
                    Publish::Fresh(created) => {
                        let unchanged = record.raw_root_node.is_empty();
                        if member == content.uri {
                            if !unchanged {
                                return Err(data(format!(
                                    "tile {t}: payload {member:?} is staged at its final member \
                                     but needs a rebase"
                                )));
                            }
                        } else {
                            if unchanged {
                                created.link(self.directory, &content.uri, &member, &bytes)?;
                            } else {
                                created.file(self.directory, &member, &bytes)?;
                            }
                            sources.insert(content.uri.clone());
                        }
                    }
                }
                total += bytes.len() as u64;
                members.push(member.clone());
                flags[slot] = true;
                written.push(WrittenContent {
                    member,
                    bytes: bytes.len() as u64,
                    rebase: Some(record),
                });
            }
            let mut extras = node.extras.clone();
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
        for (t, (c, flags)) in range.clone().zip(layers) {
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
        // Availability is derived from the slots: every tile is one available position.
        let (available, contents) = subtree.available();
        let declared: usize = derived.contents.iter().map(Vec::len).sum();
        if available != range.len() || contents != declared {
            return Err(data(format!(
                "tile {id}: {} tiles and {declared} contents map to {available} available tiles \
                 and {contents} available contents",
                range.len()
            )));
        }
        let subtree_member = template_uri(&subtree_template, origin, scheme)?;
        let bytes = subtree.to_bytes()?;
        match &mut publish {
            Publish::Write => std::fs::write(self.directory.join(&subtree_member), bytes)?,
            Publish::Fresh(created) => created.file(self.directory, &subtree_member, &bytes)?,
        }
        members.push(subtree_member.clone());
        // V_R is the level-0 row, widened alike.
        let reach = drift[&id];
        let mut bounding_box = root.bounding_box;
        widen(&mut bounding_box, reach);
        let templates: Vec<Template> = slots
            .into_iter()
            .zip(content_templates)
            .map(|((_, (header, vector)), uri)| Template {
                uri,
                header,
                vector,
            })
            .collect();
        let tiling = Tiling {
            scheme,
            levels: root.height + 1,
            subtrees: subtree_template,
        };
        let object = emit.implicit_root(id, root, &bounding_box, &templates, &tiling)?;
        Ok((object, subtree_member, reach, derived))
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
    rebase_hashed(raw, Sha256::digest(raw).into(), delta)
}

/// [`rebase_payload`] for a payload whose sha256 is already known.
fn rebase_hashed(
    raw: &[u8],
    raw_sha256: [u8; 32],
    delta: [f64; 3],
) -> Result<(Vec<u8>, RebaseRecord), Error> {
    let fail = |message: String| Error::Data(format!("payload rebase: {message}"));
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

fn template_uri(uri: &str, c: Coordinates, scheme: SubdivisionScheme) -> Result<String, Error> {
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
            let bytes = (self.read)(&template_uri(self.subtree_template, root, self.scheme)?)?;
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
                content["uri"] = json!(template_uri(
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

/// The typed forest entry against the JSON entry on the same hierarchy: the vector producer's
/// JSON forms are rebuilt here as `store.rs` and `encoding.rs` write them today, and the typed
/// records as the producer will build them. Fixtures A, B and C are design §4's tiny vector
/// fixtures at the record level.
#[cfg(test)]
mod records_tests {
    use super::*;
    use crate::tileset_node::{
        enclose_children, top_level_error, translation, ContentKind, ContentRef, NodePlacement,
        Refine, SourceIdentity,
    };
    use crate::vec3::{add, mul, norm, sub, Vec3};

    // ---- The producer's typed extras (the vector key set) ----

    #[derive(Clone, Debug, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Statistics {
        feature_fragments: usize,
        vertices: usize,
        primitives: usize,
        encoded_bytes: usize,
        geometry_error_metres: f64,
        position_rounding_metres: f64,
        quantization_error_metres: f64,
        uncompressed_bytes: usize,
        tolerance_metres: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        point_aggregation: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        build_path: Option<String>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Routing {
        routing: bool,
        routing_reason: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        build_path: Option<String>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct BuildPath {
        build_path: String,
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    #[serde(untagged)]
    enum Extras {
        Content(Statistics),
        Routing(Routing),
        Empty(BuildPath),
    }

    // ---- Fixture hierarchy ----

    #[derive(Clone)]
    enum Kind {
        /// `tag` names the payload; `level` > 0 is a coarse candidate.
        Content {
            tag: u64,
            level: u32,
            error: f64,
            rounding: f64,
            tolerance: f64,
            fragments: usize,
            aggregation: Option<Value>,
            /// A second, non-vector content (a `contents` array).
            second: Option<u64>,
        },
        Routing {
            reason: Option<&'static str>,
            error: f64,
        },
    }

    /// One tile as `Build::attach` sees it: the z-up centre and half extent of its
    /// original points, its candidate, children and the build path `remember` records.
    #[derive(Clone)]
    struct Spec {
        center: Vec3,
        half: Vec3,
        kind: Kind,
        path: Option<&'static str>,
        children: Vec<Spec>,
    }

    fn content(tag: u64, level: u32, error: f64, center: Vec3, half: Vec3) -> Spec {
        Spec {
            center,
            half,
            kind: Kind::Content {
                tag,
                level,
                error,
                rounding: 2f64.powi(-17),
                tolerance: 0.5 * 2f64.powi(level as i32),
                fragments: 1,
                aggregation: None,
                second: None,
            },
            path: None,
            children: Vec::new(),
        }
    }

    impl Spec {
        fn with(mut self, children: Vec<Spec>) -> Self {
            self.children = children;
            self
        }
        fn path(mut self, path: &'static str) -> Self {
            self.path = Some(path);
            self
        }
    }

    /// A GLB whose single scene root carries `translation` (odd tags), so non-zero rebases
    /// change its JSON and inexact ones drift.
    fn payload(tag: u64) -> Vec<u8> {
        let mut node = json!({"name": format!("n{tag}")});
        if tag % 2 == 1 {
            node["translation"] = json!([0.5, 1.5, -2.0]);
        }
        let document = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[node],"buffers":[{"byteLength":8}]});
        let mut json = serde_json::to_vec(&document).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(((28 + json.len() + 8) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(&json);
        out.extend(8u32.to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(tag.to_le_bytes());
        out
    }

    fn member(bytes: &[u8]) -> String {
        format!("t/{:x}.glb", Sha256::digest(bytes))
    }

    /// The frame at anchor A (lon 7.4385°, lat 46.948°, h 540 m): WGS84 ECEF and ENU axes.
    fn frame() -> (Vec3, [Vec3; 3]) {
        let (lon, lat, h) = (7.4385f64.to_radians(), 46.948f64.to_radians(), 540.);
        let (a, f) = (6_378_137., 1. / 298.257_223_563);
        let e2 = f * (2. - f);
        let n = a / (1. - e2 * lat.sin().powi(2)).sqrt();
        let anchor = [
            (n + h) * lat.cos() * lon.cos(),
            (n + h) * lat.cos() * lon.sin(),
            (n * (1. - e2) + h) * lat.sin(),
        ];
        let east = [-lon.sin(), lon.cos(), 0.];
        let north = [-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos()];
        let up = [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()];
        (anchor, [east, north, up])
    }

    // ---- Today's JSON writers (`encoding.rs` candidate, `store.rs` attach and root) ----

    fn legacy_tile(spec: &Spec, payloads: &mut BTreeMap<String, Vec<u8>>) -> (Value, Vec3, f64) {
        let children: Vec<_> = spec
            .children
            .iter()
            .map(|child| legacy_tile(child, payloads))
            .collect();
        let mut value = match &spec.kind {
            Kind::Content {
                tag,
                level,
                error,
                rounding,
                tolerance,
                fragments,
                aggregation,
                second,
            } => {
                let bytes = payload(*tag);
                let uri = member(&bytes);
                let byte_count = bytes.len();
                payloads.insert(uri.clone(), bytes);
                let (quantization, vertex_count, primitives) = (0f64, 2 * fragments, 1);
                let mut node = json!({"extras":{"featureFragments":fragments,"vertices":vertex_count,"primitives":primitives,"encodedBytes":byte_count,
                    "geometryErrorMetres":error,"positionRoundingMetres":rounding,"quantizationErrorMetres":quantization,"uncompressedBytes":byte_count * 2,"toleranceMetres":tolerance},
                    "geometricError":if *level>0 {error+rounding+quantization}else{0.}});
                if let Some(summary) = aggregation {
                    node["extras"]["pointAggregation"] = summary.clone();
                }
                node["content"] =
                    json!({"uri":uri,"extensions":{"3DTILES_content_gltf_vector":{"vector":true}}});
                if let Some(tag) = second {
                    let bytes = payload(*tag);
                    let plain = json!({"uri": member(&bytes)});
                    payloads.insert(member(&bytes), bytes);
                    node["contents"] = json!([node["content"].take(), plain]);
                    node.as_object_mut().unwrap().remove("content");
                }
                node
            }
            Kind::Routing { reason, error } => {
                json!({"geometricError":error,"extras":{"routing":true,"routingReason":reason}})
            }
        };
        let mut half = spec.half.map(|v| v.max(0.001));
        let mut padding = value["extras"]["positionRoundingMetres"]
            .as_f64()
            .unwrap_or(0.)
            + value["extras"]["quantizationErrorMetres"]
                .as_f64()
                .unwrap_or(0.);
        for child in &children {
            padding = padding.max(child.2);
        }
        half = half.map(|v| v + padding);
        let mut deltas = Vec::new();
        let mut values = Vec::new();
        for (mut child, center, _) in children {
            let delta = sub(center, spec.center);
            child["transform"] = translation(delta);
            deltas.push(delta);
            values.push(child);
        }
        let error = enclose_children(
            &mut half,
            value["geometricError"].as_f64().unwrap_or(0.),
            deltas.into_iter().zip(values.iter()),
        )
        .unwrap();
        value["boundingVolume"] = json!({"box":crate::tileset_node::box_json(0, half)});
        value["refine"] = json!("REPLACE");
        value["geometricError"] = json!(error);
        if !values.is_empty() {
            value["children"] = Value::Array(values);
        }
        if let Some(path) = spec.path {
            value["extras"]["buildPath"] = json!(path);
        }
        (value, spec.center, padding)
    }

    fn legacy(spec: Option<&Spec>, placed: bool) -> (Value, BTreeMap<String, Vec<u8>>) {
        let mut payloads = BTreeMap::new();
        let (anchor, axes) = frame();
        let (mut root, center) = match spec {
            Some(spec) => {
                let (root, center, _) = legacy_tile(spec, &mut payloads);
                (root, center)
            }
            None => (
                json!({"boundingVolume":{"box":[0,0,0,0.001,0,0,0,0.001,0,0,0,0.001]},"geometricError":0,"refine":"REPLACE"}),
                [0.; 3],
            ),
        };
        if spec.is_none() {
            root["extras"]["buildPath"] = json!("");
        }
        let anchor = (0..3).fold(anchor, |p, i| add(p, mul(axes[i], center[i])));
        if placed {
            root["transform"] = json!([
                axes[0][0], axes[0][1], axes[0][2], 0, axes[1][0], axes[1][1], axes[1][2], 0,
                axes[2][0], axes[2][1], axes[2][2], 0, anchor[0], anchor[1], anchor[2], 1
            ]);
        }
        let diagonal =
            norm(crate::tileset_node::box_half(&root["boundingVolume"]["box"]).unwrap()) * 2.;
        let top_error = top_level_error(diagonal, root["geometricError"].as_f64().unwrap(), 2.);
        let manifest = json!({"asset":{"version":"1.1"},"extensionsUsed":["3DTILES_content_gltf_vector"],"geometricError":top_error,"root":root});
        (manifest, payloads)
    }

    // ---- The typed records ----

    fn typed_tile(spec: &Spec) -> (TileTree<Extras>, Vec3, f64) {
        let mut children: Vec<_> = spec.children.iter().map(typed_tile).collect();
        let build_path = spec.path.map(str::to_owned);
        let (contents, extras, own, padding) = match &spec.kind {
            Kind::Content {
                tag,
                level,
                error,
                rounding,
                tolerance,
                fragments,
                aggregation,
                second,
            } => {
                let bytes = payload(*tag);
                let content = ContentRef::new(
                    MemberName::new(member(&bytes)).unwrap(),
                    Sha256::digest(&bytes).into(),
                    bytes.len() as u64,
                    ContentKind::Glb,
                    true,
                    match aggregation {
                        None => SourceIdentity::Features {
                            count: *fragments as u32,
                            digest: [*tag as u8; 32],
                        },
                        Some(_) => SourceIdentity::Aggregate,
                    },
                )
                .unwrap();
                let quantization = 0.;
                let extras = Extras::Content(Statistics {
                    feature_fragments: *fragments,
                    vertices: 2 * fragments,
                    primitives: 1,
                    encoded_bytes: bytes.len(),
                    geometry_error_metres: *error,
                    position_rounding_metres: *rounding,
                    quantization_error_metres: quantization,
                    uncompressed_bytes: bytes.len() * 2,
                    tolerance_metres: *tolerance,
                    point_aggregation: aggregation.clone(),
                    build_path,
                });
                let own = if *level > 0 {
                    error + rounding + quantization
                } else {
                    0.
                };
                let mut contents = vec![content];
                if let Some(tag) = second {
                    let bytes = payload(*tag);
                    contents.push(
                        ContentRef::new(
                            MemberName::new(member(&bytes)).unwrap(),
                            Sha256::digest(&bytes).into(),
                            bytes.len() as u64,
                            ContentKind::Glb,
                            false,
                            SourceIdentity::Aggregate,
                        )
                        .unwrap(),
                    );
                }
                (contents, extras, own, rounding + quantization)
            }
            Kind::Routing { reason, error } => (
                Vec::new(),
                Extras::Routing(Routing {
                    routing: true,
                    routing_reason: *reason,
                    build_path,
                }),
                *error,
                0.,
            ),
        };
        let padding = children.iter().fold(padding, |p, c| p.max(c.2));
        let mut bounds = TileBox::centred(spec.half.map(|v| v.max(0.001) + padding)).unwrap();
        let mut error = GeometricError::new(own).unwrap();
        for (child, center, _) in &mut children {
            let delta = sub(*center, spec.center);
            child
                .record_mut()
                .set_placement(NodePlacement::translation(delta).unwrap());
            bounds
                .grow_to_enclose(delta, child.record().bounds())
                .unwrap();
            error = error.max(child.record().geometric_error());
        }
        let record = TileRecord::new(
            NodePlacement::INHERIT,
            bounds,
            error,
            Refine::Replace,
            contents,
            Some(extras),
        )
        .unwrap();
        let children = children.into_iter().map(|c| c.0).collect();
        (TileTree::new(record, children), spec.center, padding)
    }

    fn typed(spec: Option<&Spec>, placed: bool) -> (TileTree<Extras>, TilesetDocument) {
        let (anchor, axes) = frame();
        let (mut tree, center) = match spec {
            Some(spec) => {
                let (tree, center, _) = typed_tile(spec);
                (tree, center)
            }
            None => {
                // The empty dataset's root records only its build path.
                let record = TileRecord::empty_root(Some(Extras::Empty(BuildPath {
                    build_path: String::new(),
                })))
                .unwrap();
                (TileTree::new(record, Vec::new()), [0.; 3])
            }
        };
        let anchor = (0..3).fold(anchor, |p, i| add(p, mul(axes[i], center[i])));
        if placed {
            tree.record_mut()
                .set_placement(NodePlacement::rigid(axes, anchor).unwrap());
        }
        let root = tree.record();
        let top = top_level_error(
            root.bounds().diagonal(),
            root.geometric_error().metres(),
            2.,
        );
        let document = TilesetDocument::root(
            GeometricError::new(top).unwrap(),
            [TilesetExtension::ContentGltfVector],
        );
        (tree, document)
    }

    // ---- Comparison ----

    fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, out);
                } else {
                    let name = path.strip_prefix(root).unwrap().to_string_lossy();
                    out.insert(name.replace('\\', "/"), std::fs::read(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        visit(dir, dir, &mut out);
        out
    }

    fn stage(payloads: &BTreeMap<String, Vec<u8>>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("t")).unwrap();
        for (name, bytes) in payloads {
            std::fs::create_dir_all(dir.path().join(name).parent().unwrap()).unwrap();
            std::fs::write(dir.path().join(name), bytes).unwrap();
        }
        dir
    }

    struct Written {
        manifest: Value,
        h: Value,
        output: ForestOutput,
        files: BTreeMap<String, Vec<u8>>,
        raw: BTreeMap<String, Vec<u8>>,
    }

    /// Write `spec` through both entries and require identical bytes everywhere: H, the
    /// root document, every written member, and the per-tile record.
    fn both(spec: Option<&Spec>, scheme: SubdivisionScheme) -> Written {
        both_placed(spec, scheme, true)
    }

    /// [`both`], with the root rigidly placed or (`placed` false) without a transform.
    fn both_placed(spec: Option<&Spec>, scheme: SubdivisionScheme, placed: bool) -> Written {
        let (h, raw) = legacy(spec, placed);
        let (tree, document) = typed(spec, placed);
        let typed_h = document.to_value(tree.to_object().unwrap());
        assert_eq!(
            String::from_utf8(serde_json::to_vec(&typed_h).unwrap()).unwrap(),
            String::from_utf8(serde_json::to_vec(&h).unwrap()).unwrap(),
            "H"
        );
        let loose_dir = stage(&raw);
        let mut manifest = h.clone();
        let loose = write_forest(&mut manifest, loose_dir.path(), scheme, &mut || Ok(())).unwrap();
        let typed_dir = stage(&raw);
        let (top, output) =
            write_forest_records(&tree, &document, typed_dir.path(), scheme, &mut || Ok(()))
                .unwrap();
        assert_eq!(
            String::from_utf8(serde_json::to_vec(&top).unwrap()).unwrap(),
            String::from_utf8(serde_json::to_vec(&manifest).unwrap()).unwrap(),
            "tileset.json"
        );
        assert_eq!(output, loose);
        let written = files(typed_dir.path());
        assert_eq!(written, files(loose_dir.path()));
        // Every implicit payload restores to its raw source, and the expansion has H's tiles.
        for tile in &output.tiles {
            for content in &tile.contents {
                let published = &written[&content.member];
                assert_eq!(content.bytes, published.len() as u64);
                if let Some(rebase) = &content.rebase {
                    let raw_bytes = restore_payload(published, rebase).unwrap();
                    assert_eq!(
                        <[u8; 32]>::from(Sha256::digest(&raw_bytes)),
                        rebase.raw_sha256
                    );
                    assert!(raw.contains_key(&member(&raw_bytes)));
                }
            }
        }
        let expanded = expand_tileset(&top, |name| {
            written
                .get(name)
                .cloned()
                .ok_or_else(|| Error::Data(format!("missing {name}")))
        })
        .unwrap();
        fn count(tile: &Value) -> usize {
            1 + tile["children"]
                .as_array()
                .map_or(0, |c| c.iter().map(count).sum())
        }
        assert_eq!(count(&expanded["root"]), count(&h["root"]));
        assert_eq!(output.tiles.len(), count(&h["root"]));
        Written {
            manifest: top,
            h,
            output,
            files: written,
            raw,
        }
    }

    fn roles(output: &ForestOutput) -> Vec<TileRole> {
        output.tiles.iter().map(|t| t.role).collect()
    }

    const R: f64 = 7.62939453125e-6; // 2^-17, the fixtures' f32 position rounding.

    /// Fixture A: `pt-alpha` at the anchor and `ln-bravo` from (E 120, N −45, U 7) to
    /// (E 180, N 60, U 3), one level of LOD: root → {point leaf, line L1 → line leaf}.
    fn fixture_a() -> Spec {
        let line = [150., 7.5, 5.];
        let line_half = [30.3, 52.5, 2.];
        content(4, 2, 0.3, [90., 7.5, 3.5], [90.3, 52.5, 3.5])
            .with(vec![
                content(1, 0, 0., [0.; 3], [0.; 3]).path("0"),
                content(3, 1, 0.3, line, line_half)
                    .with(vec![content(2, 0, 0., line, line_half)])
                    .path("1"),
            ])
            .path("")
    }

    /// Fixture B: four LOD levels under the line: root (height 5) → {point leaf, L4 → L3 →
    /// L2 → L1 → leaf}.
    fn fixture_b() -> Spec {
        let line = [150., 7.5, 5.];
        let half = [31.5, 52.5, 2.];
        let mut chain = content(12, 0, 0., line, half);
        for (level, (tag, error)) in [(13, 0.3), (14, 0.75), (15, 1.5), (16, 3.)]
            .into_iter()
            .enumerate()
        {
            chain = content(tag, level as u32 + 1, error, line, half).with(vec![chain]);
        }
        content(17, 5, 3., [90., 7.5, 3.5], [91.5, 52.5, 3.5])
            .with(vec![
                content(11, 0, 0., [0.; 3], [0.; 3]).path("0"),
                chain.path("1"),
            ])
            .path("")
    }

    /// C6: each content record's length and sha256 are checked against its staged payload
    /// when the forest reads it, for implicit (A: every tile) and explicit-leaf (B: the
    /// point leaf) contents alike. A missing, lengthened or altered payload is a typed
    /// failure before `tileset.json` exists.
    #[test]
    fn staged_payloads_must_match_their_content_records() {
        use crate::tileset_node::StagedContentError;
        for spec in [fixture_a(), fixture_b()] {
            let (tree, document) = typed(Some(&spec), true);
            let (_, raw) = legacy(Some(&spec), true);
            for member in raw.keys() {
                type Tamper = (&'static str, fn(&Path));
                let cases: [Tamper; 3] = [
                    ("missing", |path| std::fs::remove_file(path).unwrap()),
                    ("length", |path| {
                        let mut bytes = std::fs::read(path).unwrap();
                        bytes.push(0);
                        std::fs::write(path, bytes).unwrap();
                    }),
                    ("digest", |path| {
                        let mut bytes = std::fs::read(path).unwrap();
                        *bytes.last_mut().unwrap() ^= 1;
                        std::fs::write(path, bytes).unwrap();
                    }),
                ];
                for (label, tamper) in cases {
                    let dir = stage(&raw);
                    tamper(&dir.path().join(member));
                    let error = write_forest_records(
                        &tree,
                        &document,
                        dir.path(),
                        SubdivisionScheme::Quadtree,
                        &mut || Ok(()),
                    )
                    .unwrap_err();
                    let bytes = raw[member].len() as u64;
                    let expected = match label {
                        "missing" => StagedContentError::Missing {
                            member: member.clone(),
                        },
                        "length" => StagedContentError::Length {
                            member: member.clone(),
                            declared: bytes,
                            staged: bytes + 1,
                        },
                        _ => {
                            let mut altered = raw[member].clone();
                            *altered.last_mut().unwrap() ^= 1;
                            StagedContentError::Digest {
                                member: member.clone(),
                                declared: Sha256::digest(&raw[member]).into(),
                                staged: Sha256::digest(&altered).into(),
                            }
                        }
                    };
                    assert_eq!(
                        error.to_string(),
                        Error::from(expected).to_string(),
                        "{label} {member}"
                    );
                    assert!(!dir.path().join("tileset.json").exists());
                }
            }
        }
    }

    #[test]
    fn fixture_a_is_one_implicit_root_at_tileset_json() {
        let a = both(Some(&fixture_a()), SubdivisionScheme::Quadtree);
        assert_eq!(
            roles(&a.output),
            [
                TileRole::ImplicitRoot,
                TileRole::Implicit,
                TileRole::Implicit,
                TileRole::Implicit
            ]
        );
        assert_eq!((a.output.documents, a.output.implicit_roots), (1, 1));
        let root = &a.manifest["root"];
        assert_eq!(
            root["implicitTiling"],
            json!({"subdivisionScheme":"QUADTREE","subtreeLevels":3,"availableLevels":3,
                "subtrees":{"uri":"subtrees/r0-{level}-{x}-{y}.subtree"}})
        );
        assert_eq!(
            root["content"],
            json!({"uri":"implicit-content/r0-{level}-{x}-{y}-0.glb",
                "extensions":{"3DTILES_content_gltf_vector":{"vector":true}}})
        );
        assert!(
            root.get("extras").is_none(),
            "implicit roots carry no extras"
        );
        assert_eq!(root["transform"], a.h["root"]["transform"]);
        assert_eq!(a.manifest["schema"], tileset_node::tile_schema());
        // Point leaf at slot 0, the line's L1 at slot 1, its leaf at slot 0 below that.
        let members: Vec<_> = a.files.keys().map(String::as_str).collect();
        assert_eq!(
            members,
            [
                "implicit-content/r0-0-0-0-0.glb",
                "implicit-content/r0-1-0-0-0.glb",
                "implicit-content/r0-1-1-0-0.glb",
                "implicit-content/r0-2-2-0-0.glb",
                "subtrees/r0-0-0-0.subtree",
            ]
        );
        // Non-zero rebase deltas at depths 1 and 2; the implicit root's is zero.
        let deltas: Vec<_> = a
            .output
            .tiles
            .iter()
            .map(|t| t.contents[0].rebase.as_ref().unwrap().delta)
            .collect();
        assert_eq!(deltas[0], [0.; 3]);
        assert_eq!(deltas[1], [-90., -7.5, -3.5]);
        assert_eq!(deltas[2], [60., 0., 1.5]);
        assert_eq!(deltas[3], deltas[2]);
        // Hand-derived root half: (90.3, 52.5, 3.5) plus the f32 rounding pad, grown on x by
        // the line L1 (60 + its half, rounded up) and on z by the point's 1 mm floor.
        let half = &a.h["root"]["boundingVolume"]["box"];
        assert_eq!(
            [half[3].as_f64(), half[7].as_f64(), half[11].as_f64()],
            [
                Some(sum_up(60., 30.3 + R)),
                Some(52.5 + R),
                Some(sum_up(3.5, 0.001 + R))
            ]
        );
        assert_eq!(sum_up(60., 30.3 + R), (90.3 + R).next_up());
        // Implicit tiles keep their statistics in the subtree row, with the published bytes.
        let line_leaf = &a.output.tiles[3];
        assert_eq!(
            line_leaf.extras["encodedBytes"],
            json!(line_leaf.contents[0].bytes)
        );
        assert_eq!(line_leaf.extras["toleranceMetres"], json!(0.5));
    }

    #[test]
    fn fixture_b_crosses_the_subtree_boundary_under_a_skeleton() {
        let b = both(Some(&fixture_b()), SubdivisionScheme::Quadtree);
        assert_eq!(
            roles(&b.output),
            [
                TileRole::Skeleton,
                TileRole::ExplicitLeaf,
                TileRole::Skeleton,
                TileRole::ImplicitRoot,
                TileRole::Implicit,
                TileRole::Implicit,
                TileRole::Implicit
            ]
        );
        let root = &b.manifest["root"];
        assert_eq!(b.manifest["schema"], tileset_node::tile_schema());
        let point = &root["children"][0];
        let point_uri = point["content"]["uri"].as_str().unwrap();
        assert!(
            point_uri.starts_with("t/"),
            "explicit leaves publish in place"
        );
        assert_eq!(point["extras"]["buildPath"], "0");
        let l4 = &root["children"][1];
        assert_eq!(l4["transform"], json!(translation([60., 0., 1.5])));
        let l3 = &l4["children"][0];
        assert_eq!(
            l3["transform"].to_string(),
            "[1,0,0,0,0,1,0,0,0,0,1,0,0.0,0.0,0.0,1]"
        );
        assert_eq!(
            l3["implicitTiling"],
            json!({"subdivisionScheme":"QUADTREE","subtreeLevels":4,"availableLevels":4,
                "subtrees":{"uri":"subtrees/r3-{level}-{x}-{y}.subtree"}})
        );
        assert_eq!(
            l3["content"]["uri"],
            "implicit-content/r3-{level}-{x}-{y}-0.glb"
        );
        let implicit: Vec<_> = b
            .files
            .keys()
            .filter(|name| !name.starts_with("t/"))
            .map(String::as_str)
            .collect();
        assert_eq!(
            implicit,
            [
                "implicit-content/r3-0-0-0-0.glb",
                "implicit-content/r3-1-0-0-0.glb",
                "implicit-content/r3-2-0-0-0.glb",
                "implicit-content/r3-3-0-0-0.glb",
                "subtrees/r3-0-0-0.subtree",
            ]
        );
        // δ = 0 along the LOD chain: the rebased payloads are byte copies, and raw sources
        // only implicit tiles referenced are deleted; explicit payloads stay.
        for tile in &b.output.tiles[3..] {
            let content = &tile.contents[0];
            assert_eq!(content.rebase.as_ref().unwrap().delta, [0.; 3]);
            assert!(b.raw.contains_key(&member(&b.files[&content.member])));
        }
        let kept: Vec<_> = b.files.keys().filter(|n| n.starts_with("t/")).collect();
        assert_eq!(
            kept.len(),
            3,
            "root, point leaf and L4 payloads stay in place"
        );
        assert_eq!(b.output.documents, 1);
    }

    #[test]
    fn fixture_c_links_an_external_document() {
        // A 17-tile LOD chain: the skeleton is deeper than DOC_LEVELS, so it links.
        let line = [10_000., -2_000., 30.];
        let half = [12_000., 9_000., 40.];
        let mut chain = content(100, 0, 0., line, half);
        for level in 1..=16u32 {
            let error = 0.3 * 2f64.powi(level as i32 - 1);
            chain = content(100 + level as u64, level, error, line, half).with(vec![chain]);
        }
        let c = both(Some(&chain.path("")), SubdivisionScheme::Quadtree);
        assert_eq!(c.output.documents, 2);
        assert!(c.files.contains_key("tileset-d1.json"));
        let document: Value = serde_json::from_slice(&c.files["tileset-d1.json"]).unwrap();
        assert!(document["root"].get("transform").is_none());
        assert_eq!(document["root"]["refine"], "REPLACE");
        assert_eq!(document["asset"], json!({"version":"1.1"}));
        assert_eq!(
            document["extensionsUsed"],
            json!(["3DTILES_content_gltf_vector"])
        );
        assert_eq!(document["schema"], tileset_node::tile_schema());
        let mut tile = &c.manifest["root"];
        while tile["content"]["uri"] != "tileset-d1.json" {
            tile = &tile["children"][0];
        }
        assert_eq!(
            tile.as_object().unwrap().keys().collect::<Vec<_>>(),
            [
                "boundingVolume",
                "content",
                "geometricError",
                "refine",
                "transform"
            ]
        );
    }

    #[test]
    fn drift_widens_explicit_ancestors_identically() {
        // Centres a tenth of a metre apart near the origin, with translated payloads: the
        // payloads' scene-root sums (0.5 + 0.1) and the δ sums round, so the implicit root
        // and its skeleton ancestors grow outward.
        let half = [2., 1., 0.5];
        let mut chain = content(21, 0, 0., [0.6, 0., 0.], half);
        for (i, (tag, x)) in [(23u64, 0.3), (25, 0.1), (27, 0.), (29, 0.)]
            .into_iter()
            .enumerate()
        {
            chain = content(tag, i as u32 + 1, 0.3 * (i + 1) as f64, [x, 0., 0.], half)
                .with(vec![chain]);
        }
        let spec = content(31, 5, 3., [0.05, 0., 0.], [3., 2., 1.])
            .with(vec![content(33, 0, 0., [1.5, 1., 0.5], [0.; 3]), chain]);
        let d = both(Some(&spec), SubdivisionScheme::Quadtree);
        assert_eq!(roles(&d.output)[3], TileRole::ImplicitRoot);
        let x_half = |tile: &Value| tile["boundingVolume"]["box"][3].as_f64().unwrap();
        let (root, h) = (&d.manifest["root"], &d.h["root"]);
        let (l4, l3) = (&root["children"][1], &root["children"][1]["children"][0]);
        let (h4, h3) = (&h["children"][1], &h["children"][1]["children"][0]);
        assert!(x_half(l3) > x_half(h3), "the implicit root grows");
        assert!(x_half(l4) > x_half(h4), "its skeleton parent grows");
        assert!(x_half(root) >= x_half(h));
    }

    #[test]
    fn routing_aggregates_octrees_and_the_empty_root_match() {
        // Routing tiles (null and named reasons) and an aggregate content.
        let mut aggregate = content(41, 1, 0.5, [5., 5., 1.], [5., 5., 1.]);
        if let Kind::Content { aggregation, .. } = &mut aggregate.kind {
            *aggregation = Some(json!({"layers":{"pts":{"voxels":3}},"pointsAggregated":12}));
        }
        let routing = |reason, children| Spec {
            center: [3., -4., 0.5],
            half: [8., 9., 1.5],
            kind: Kind::Routing {
                reason,
                error: 12.5,
            },
            path: Some(""),
            children,
        };
        let spec = routing(
            Some("bytes"),
            vec![
                aggregate
                    .clone()
                    .with(vec![content(42, 0, 0., [5., 5., 1.], [5., 5., 1.])]),
                Spec {
                    path: Some("1"),
                    ..routing(None, vec![content(43, 0, 0., [-2., -8., 0.], [1., 1., 0.])])
                },
            ],
        );
        for scheme in [SubdivisionScheme::Quadtree, SubdivisionScheme::Octree] {
            let r = both(Some(&spec), scheme);
            assert_eq!(r.output.tiles[0].extras["routingReason"], "bytes");
            assert_eq!(r.output.tiles[3].extras["routingReason"], Value::Null);
            assert!(r.output.tiles[0].contents.is_empty());
            assert_eq!(
                r.output.tiles[1].extras["pointAggregation"]["pointsAggregated"],
                12
            );
        }
        let o = both(Some(&fixture_a()), SubdivisionScheme::Octree);
        assert_eq!(
            o.manifest["root"]["content"]["uri"],
            "implicit-content/r0-{level}-{x}-{y}-{z}-0.glb"
        );
        // The empty dataset: the structural integer error survives the forest.
        let empty = both(None, SubdivisionScheme::Quadtree);
        assert_eq!(empty.manifest["root"]["geometricError"].to_string(), "0");
        assert_eq!(roles(&empty.output), [TileRole::ImplicitRoot]);
    }

    #[test]
    fn drifting_links_mixed_slots_and_unplaced_roots_match() {
        // A 17-tile chain a tenth of a metre apart near the origin, with translated payloads
        // and a second plain content per coarse tile: widened skeleton tiles, a widened link
        // and external-document root, two content slots (one without the vector marker).
        let half = [2., 1., 0.5];
        let mut chain = content(300, 0, 0., [0.; 3], half);
        for level in 1..=16u32 {
            let x = 0.1 * level as f64;
            chain = content(
                301 + 2 * level as u64,
                level,
                0.1 * level as f64,
                [x, 0., 0.],
                half,
            )
            .with(vec![chain]);
            if let Kind::Content { second, .. } = &mut chain.kind {
                *second = Some(1001 + 2 * level as u64);
            }
        }
        let chain = chain.path("");
        for placed in [true, false] {
            let w = both_placed(Some(&chain), SubdivisionScheme::Quadtree, placed);
            assert_eq!(w.output.documents, 2);
            assert_eq!(w.manifest["root"].get("transform").is_some(), placed);
            let mut tile = &w.manifest["root"];
            let mut h = &w.h["root"];
            while tile["content"]["uri"] != "tileset-d1.json" {
                assert_eq!(tile["contents"].as_array().unwrap().len(), 2);
                assert_eq!(tile["contents"][1].as_object().unwrap().len(), 1);
                tile = &tile["children"][0];
                h = &h["children"][0];
            }
            let x_half = |t: &Value| t["boundingVolume"]["box"][3].as_f64().unwrap();
            assert!(
                x_half(tile) > x_half(h),
                "the link grows with its subtree's drift"
            );
            let document: Value = serde_json::from_slice(&w.files["tileset-d1.json"]).unwrap();
            assert_eq!(x_half(&document["root"]), x_half(tile));
            let mut tile = &document["root"];
            while tile.get("implicitTiling").is_none() {
                tile = &tile["children"][0];
            }
            let slots = tile["contents"].as_array().unwrap();
            assert_eq!(slots.len(), 2);
            assert!(slots[1].get("extensions").is_none());
        }
    }

    #[test]
    fn typed_refusals_match_or_precede_the_json_entry() {
        let dir = tempfile::tempdir().unwrap();
        let none = &mut || Ok(());
        let quad = SubdivisionScheme::Quadtree;
        let refused = |result: Result<(Value, ForestOutput), Error>| match result {
            Err(Error::Data(message)) => message,
            other => panic!("expected a typed data error, got {other:?}"),
        };
        // 33 levels: the same refusal as the JSON entry.
        let mut chain = content(200, 0, 0., [0.; 3], [1.; 3]);
        for level in 1..33u32 {
            chain = content(200 + level as u64, level, 0.1, [0.; 3], [1.; 3]).with(vec![chain]);
        }
        let (mut h, _) = legacy(Some(&chain), true);
        let loose = write_forest(&mut h, dir.path(), quad, none).unwrap_err();
        let (tree, document) = typed(Some(&chain), true);
        let typed_message = refused(write_forest_records(
            &tree,
            &document,
            dir.path(),
            quad,
            none,
        ));
        assert_eq!(typed_message, loose.to_string());
        // A rigid child placement is not a translation.
        let (mut tree, document) = typed(Some(&fixture_a()), true);
        let (_, axes) = frame();
        tree.children_mut()[0]
            .record_mut()
            .set_placement(NodePlacement::rigid(axes, [0.; 3]).unwrap());
        assert!(refused(write_forest_records(
            &tree,
            &document,
            dir.path(),
            quad,
            none
        ))
        .contains("a child transform must be a translation"));
        // Vector content needs the declared extension; only tileset.json roots a forest.
        let (tree, document) = typed(Some(&fixture_a()), true);
        let undeclared = TilesetDocument::root(document.geometric_error(), []);
        assert!(refused(write_forest_records(
            &tree,
            &undeclared,
            dir.path(),
            quad,
            none
        ))
        .contains("extensionsUsed"));
        let external = TilesetDocument::external(
            document.geometric_error(),
            [TilesetExtension::ContentGltfVector],
        );
        assert!(refused(write_forest_records(
            &tree,
            &external,
            dir.path(),
            quad,
            none
        ))
        .contains("tileset.json"));
        let stamped = document.clone().with_build_state([1; 32]).unwrap();
        assert!(refused(write_forest_records(
            &tree,
            &stamped,
            dir.path(),
            quad,
            none
        ))
        .contains("build state"));
        // A cancelled checkpoint stops before anything is written.
        let cancelled = write_forest_records(&tree, &document, dir.path(), quad, &mut || {
            Err(Error::Message("cancelled".into()))
        });
        assert!(matches!(cancelled, Err(Error::Message(_))));
        assert!(files(dir.path()).is_empty());
    }

    // ---- The streamed forest ----

    type Ptr<X> = *const TileTree<X>;

    /// The whole-tree writer's partition of `tree`: roles by height, skeleton tiles packed
    /// breadth-first within DOC_LEVELS (no fixture reaches the node, byte or depth budget),
    /// implicit roots `r{preorder id}` and documents `tileset-d{m}.json` in creation order.
    struct Plan<'t, X> {
        ids: std::collections::HashMap<Ptr<X>, usize>,
        heights: std::collections::HashMap<Ptr<X>, u32>,
        linked: std::collections::HashMap<Ptr<X>, usize>,
        documents: Vec<&'t TileTree<X>>,
    }

    impl<'t, X: Serialize + Clone> Plan<'t, X> {
        fn new(tree: &'t TileTree<X>) -> Self {
            fn index<X: Serialize>(t: &TileTree<X>, plan: &mut Plan<'_, X>) -> u32 {
                plan.ids.insert(t as Ptr<X>, plan.ids.len());
                let height = t
                    .children()
                    .iter()
                    .map(|c| index(c, plan) + 1)
                    .max()
                    .unwrap_or(0);
                plan.heights.insert(t as Ptr<X>, height);
                height
            }
            let mut plan = Plan {
                ids: Default::default(),
                heights: Default::default(),
                linked: Default::default(),
                documents: vec![tree],
            };
            index(tree, &mut plan);
            let mut next = 0;
            while next < plan.documents.len() && plan.skeleton(tree) {
                let mut frontier = VecDeque::from([(plan.documents[next], 0u32)]);
                while let Some((tile, level)) = frontier.pop_front() {
                    for child in tile.children() {
                        if !plan.skeleton(child) {
                        } else if level < DOC_LEVELS {
                            frontier.push_back((child, level + 1));
                        } else {
                            plan.linked.insert(child as Ptr<X>, plan.documents.len());
                            plan.documents.push(child);
                        }
                    }
                }
                next += 1;
            }
            plan
        }

        fn skeleton(&self, t: &TileTree<X>) -> bool {
            self.heights[&(t as Ptr<X>)] >= SKELETON_HEIGHT
        }

        fn implicit(&self, t: &TileTree<X>) -> ForestTile<X> {
            let name = RootName::new(format!("r{}", self.ids[&(t as Ptr<X>)])).unwrap();
            ForestTile::implicit(name, t.clone())
        }

        /// The document rooted at skeleton tile `t`, its links taken from `links`.
        fn document(
            &self,
            t: &TileTree<X>,
            links: &mut BTreeMap<usize, DocumentLink>,
        ) -> ForestTile<X> {
            let children = t
                .children()
                .iter()
                .map(|c| match self.linked.get(&(c as Ptr<X>)) {
                    Some(d) => ForestTile::Link(links.remove(d).expect("linked document")),
                    None if self.skeleton(c) => self.document(c, links),
                    None if c.children().is_empty() => {
                        ForestTile::explicit(c.record().clone(), Vec::new())
                    }
                    None => self.implicit(c),
                })
                .collect();
            ForestTile::explicit(t.record().clone(), children)
        }
    }

    /// Stream `tree` through [`ForestWriter`] in the whole-tree writer's partition, deepest
    /// documents first.
    fn write_chunked<X: Serialize + Clone>(
        tree: &TileTree<X>,
        document: &TilesetDocument,
        directory: &Path,
        scheme: SubdivisionScheme,
    ) -> Result<(Value, Vec<ChunkOutput>), Error> {
        let plan = Plan::new(tree);
        let mut writer = ForestWriter::new(document, directory, scheme)?;
        let none = &mut || Ok(());
        if !plan.skeleton(tree) {
            let (top, output) = writer.root(plan.implicit(tree), none)?;
            return Ok((top, vec![output]));
        }
        let mut links = BTreeMap::new();
        let mut outputs = Vec::new();
        for d in (1..plan.documents.len()).rev() {
            let name = MemberName::new(format!("tileset-d{d}.json"))?;
            let chunk = plan.document(plan.documents[d], &mut links);
            let (link, output) = writer.external(name, chunk, none)?;
            assert_eq!(link.name().as_str(), format!("tileset-d{d}.json"));
            links.insert(d, link);
            outputs.push(output);
        }
        let (top, output) = writer.root(plan.document(tree, &mut links), none)?;
        assert!(links.is_empty(), "every document is linked once");
        outputs.push(output);
        Ok((top, outputs))
    }

    /// Delete the raw sources the streamed documents copied into `implicit-content/` that
    /// no explicit tile keeps, as the whole-tree writer does.
    fn delete_consumed(directory: &Path, outputs: &[ChunkOutput]) {
        let mut kept = BTreeSet::new();
        let mut written = BTreeSet::new();
        let mut consumed = BTreeSet::new();
        for output in outputs {
            written.extend(output.members.iter().cloned());
            consumed.extend(output.implicit_sources.iter().cloned());
            for tile in &output.tiles {
                for content in &tile.contents {
                    if content.rebase.is_none() {
                        kept.insert(content.member.clone());
                    }
                }
            }
        }
        for source in consumed.difference(&kept) {
            assert!(!written.contains(source));
            std::fs::remove_file(directory.join(source)).unwrap();
        }
    }

    /// Write `tree` whole and streamed from the same staged payloads and require identical
    /// bytes: `tileset.json`, every member, the per-tile records and the member list.
    fn whole_and_streamed<X: Serialize + Clone>(
        tree: &TileTree<X>,
        document: &TilesetDocument,
        raw: &BTreeMap<String, Vec<u8>>,
        scheme: SubdivisionScheme,
    ) -> (Value, ForestOutput, BTreeMap<String, Vec<u8>>) {
        let whole_dir = stage(raw);
        let (top, output) =
            write_forest_records(tree, document, whole_dir.path(), scheme, &mut || Ok(())).unwrap();
        let stream_dir = stage(raw);
        let (streamed, outputs) = write_chunked(tree, document, stream_dir.path(), scheme).unwrap();
        delete_consumed(stream_dir.path(), &outputs);
        assert_eq!(
            serde_json::to_vec(&streamed).unwrap(),
            serde_json::to_vec(&top).unwrap(),
            "tileset.json"
        );
        let written = files(whole_dir.path());
        assert_eq!(files(stream_dir.path()), written, "members");
        assert_eq!(outputs.len(), output.documents);
        let roots: usize = outputs.iter().map(|o| o.implicit_roots.len()).sum();
        assert_eq!(roots, output.implicit_roots);
        let mut tiles: Vec<_> = outputs.iter().flat_map(|o| &o.tiles).collect();
        let mut whole: Vec<_> = output.tiles.iter().collect();
        let order = |t: &&WrittenTile| format!("{t:?}");
        tiles.sort_by_key(order);
        whole.sort_by_key(order);
        assert_eq!(tiles, whole, "per-tile records");
        let mut members: Vec<_> = outputs.iter().flat_map(|o| o.members.clone()).collect();
        let mut expected = output.members.clone();
        members.sort();
        expected.sort();
        assert_eq!(members, expected);
        (top, output, written)
    }

    fn chain(levels: u32, tag: u64, step: f64, second: bool) -> Spec {
        let half = [2., 1., 0.5];
        let mut chain = content(tag, 0, 0., [0.; 3], half);
        for level in 1..levels {
            let x = step * level as f64;
            chain = content(
                tag + 2 * level as u64 + 1,
                level,
                0.1 * level as f64,
                [x, 0., 0.],
                half,
            )
            .with(vec![chain]);
            if second {
                if let Kind::Content { second, .. } = &mut chain.kind {
                    *second = Some(tag + 1000 + 2 * level as u64 + 1);
                }
            }
        }
        chain.path("")
    }

    /// The streamed forest, fed the whole-tree partition, is byte-identical to the whole
    /// tree: one implicit root at tileset.json, skeletons with explicit leaves, drifting
    /// links and external roots, nested external documents and the empty root.
    #[test]
    fn streamed_forest_is_byte_identical_to_the_whole_tree() {
        let mut specs = vec![
            ("a", Some(fixture_a())),
            ("b", Some(fixture_b())),
            ("chain17", Some(chain(17, 500, 1000., false))),
            ("drift17", Some(chain(17, 600, 0.1, true))),
            ("nested30", Some(chain(30, 700, 0.1, true))),
            ("empty", None),
        ];
        // Several links under one document root: a wide skeleton of deep chains.
        let wide = content(900, 20, 50., [0.; 3], [60., 60., 5.]).with(
            (0..4u64)
                .map(|i| {
                    let mut c = chain(18, 1000 + 100 * i, 0.1, i % 2 == 0);
                    c.center = [i as f64, 0., 0.];
                    c.path = Some(["0", "1", "2", "3"][i as usize]);
                    c
                })
                .collect(),
        );
        specs.push(("wide", Some(wide.path(""))));
        let mut documents = Vec::new();
        for (label, spec) in &specs {
            for scheme in [SubdivisionScheme::Quadtree, SubdivisionScheme::Octree] {
                for placed in [true, false] {
                    let (tree, document) = typed(spec.as_ref(), placed);
                    let (_, raw) = legacy(spec.as_ref(), placed);
                    let (_, output, _) = whole_and_streamed(&tree, &document, &raw, scheme);
                    documents.push((*label, output.documents));
                }
            }
        }
        let count = |label| documents.iter().find(|(l, _)| *l == label).unwrap().1;
        assert_eq!(
            (
                count("a"),
                count("chain17"),
                count("nested30"),
                count("wide")
            ),
            (1, 2, 3, 5)
        );
    }

    fn slot_tile(tag: u64, children: Vec<(u8, TileTree<Extras>)>) -> TileTree<Extras> {
        let bytes = payload(tag);
        let content = ContentRef::new(
            MemberName::new(member(&bytes)).unwrap(),
            Sha256::digest(&bytes).into(),
            bytes.len() as u64,
            ContentKind::Glb,
            true,
            SourceIdentity::Features {
                count: 1,
                digest: [tag as u8; 32],
            },
        )
        .unwrap();
        let record = TileRecord::new(
            NodePlacement::INHERIT,
            TileBox::centred([4.; 3]).unwrap(),
            GeometricError::new(children.len() as f64).unwrap(),
            Refine::Replace,
            vec![content],
            Some(Extras::Empty(BuildPath {
                build_path: format!("t{tag}"),
            })),
        )
        .unwrap();
        TileTree::with_slots(record, children).unwrap()
    }

    fn slot_raw(tags: &[u64]) -> BTreeMap<String, Vec<u8>> {
        tags.iter()
            .map(|&tag| (member(&payload(tag)), payload(tag)))
            .collect()
    }

    fn vector_document() -> TilesetDocument {
        TilesetDocument::root(
            GeometricError::new(10.).unwrap(),
            [TilesetExtension::ContentGltfVector],
        )
    }

    /// Sparse octree slots 3 and 6, and slot 7 below slot 6: the availability and payload
    /// coordinates are the slots' Morton cells, whole and streamed alike.
    #[test]
    fn explicit_slots_place_sparse_octants() {
        let octree = SubdivisionScheme::Octree;
        let tree = slot_tile(
            60,
            vec![
                (3, slot_tile(62, Vec::new())),
                (6, slot_tile(64, vec![(7, slot_tile(66, Vec::new()))])),
            ],
        );
        let raw = slot_raw(&[60, 62, 64, 66]);
        let (top, output, written) = whole_and_streamed(&tree, &vector_document(), &raw, octree);
        assert_eq!(roles(&output)[0], TileRole::ImplicitRoot);
        assert_eq!(top["root"]["implicitTiling"]["availableLevels"], 3);
        let mut payloads: Vec<_> = written
            .keys()
            .filter(|k| k.starts_with("implicit-content/"))
            .cloned()
            .collect();
        payloads.sort();
        // (level, x, y, z): slot 3 is (1, 1, 1, 0), slot 6 (1, 0, 1, 1); slot 7 below it is
        // (2, 1, 3, 3).
        assert_eq!(
            payloads,
            [
                "implicit-content/r0-0-0-0-0-0.glb",
                "implicit-content/r0-1-0-1-1-0.glb",
                "implicit-content/r0-1-1-1-0-0.glb",
                "implicit-content/r0-2-1-3-3-0.glb",
            ]
        );
        // The subtree's tile availability is exactly those cells: level 0 bit 0, level 1
        // Morton 3 and 6 (bits 1 + 3, 1 + 6), level 2 Morton 0b110_111 (bit 9 + 55).
        let subtree = &written["subtrees/r0-0-0-0-0.subtree"];
        let json_length = u64::from_le_bytes(subtree[8..16].try_into().unwrap()) as usize;
        let header: Value = serde_json::from_slice(&subtree[24..24 + json_length]).unwrap();
        let view = &header["bufferViews"]
            [header["tileAvailability"]["bitstream"].as_u64().unwrap() as usize];
        let offset = 24 + json_length + view["byteOffset"].as_u64().unwrap() as usize;
        let bits = &subtree[offset..offset + view["byteLength"].as_u64().unwrap() as usize];
        let ones: Vec<usize> = (0..bits.len() * 8)
            .filter(|i| bits[i / 8] >> (i % 8) & 1 == 1)
            .collect();
        assert_eq!(ones, [0, 4, 7, 9 + 55]);
        assert_eq!(header["tileAvailability"]["availableCount"], 4);
        // Sibling-index slots of the same children would be cells 0 and 1 instead.
        let (a, b) = (slot_tile(62, Vec::new()), slot_tile(64, Vec::new()));
        let sibling = TileTree::new(tree.record().clone(), vec![a, b]);
        let dir = stage(&raw);
        write_forest_records(
            &sibling,
            &vector_document(),
            dir.path(),
            octree,
            &mut || Ok(()),
        )
        .unwrap();
        assert!(dir
            .path()
            .join("implicit-content/r0-1-1-0-0-0.glb")
            .exists());
    }

    fn cell(level: u32, x: u32, y: u32, z: u32) -> Coordinates {
        Coordinates { level, x, y, z }
    }

    /// Every cell of a complete subtree of `levels` levels, in subtree bit order.
    fn complete_cells(levels: u32, scheme: SubdivisionScheme) -> Vec<Coordinates> {
        let mut cells = Vec::new();
        for level in 0..levels {
            let width = 1u32 << level;
            let depth = if scheme == SubdivisionScheme::Octree {
                width
            } else {
                1
            };
            for z in 0..depth {
                for y in 0..width {
                    for x in 0..width {
                        cells.push(cell(level, x, y, z));
                    }
                }
            }
        }
        cells.sort_by_key(|c| (c.level, morton(*c, scheme)));
        cells
    }

    fn refused<T: std::fmt::Debug>(result: Result<T, Error>, why: &str) -> String {
        let message = result.unwrap_err().to_string();
        assert!(message.contains(why), "{why:?} not in {message:?}");
        message
    }

    /// Duplicate and out-of-range slots are typed refusals, whole and streamed.
    #[test]
    fn explicit_slot_validation() {
        let leaf = || slot_tile(70, Vec::new());
        let record = || leaf().record().clone();
        refused(
            TileTree::with_slots(record(), vec![(2, leaf()), (2, leaf())]),
            "duplicate child slot 2",
        );
        refused(
            TileTree::with_slots(record(), vec![(8, leaf())]),
            "child slot 8 is outside every subdivision scheme",
        );
        let raw = slot_raw(&[70]);
        let quad = SubdivisionScheme::Quadtree;
        // Slot 5 is an octant, outside a quadtree: whole and streamed refuse it.
        let tree = TileTree::with_slots(record(), vec![(5, leaf())]).unwrap();
        let dir = stage(&raw);
        let none = &mut || Ok(());
        let why = "child slot 5 is outside the QUADTREE scheme";
        refused(
            write_forest_records(&tree, &vector_document(), dir.path(), quad, none),
            why,
        );
        refused(
            write_chunked(&tree, &vector_document(), dir.path(), quad),
            why,
        );
        assert_eq!(files(dir.path()), raw);
    }

    /// A planned availability is checked position by position before anything is written,
    /// including the H16 controls: swapped Morton axes and a packed leaf duplicated into
    /// its covered octants.
    #[test]
    fn planned_availability_is_positional() {
        let octree = SubdivisionScheme::Octree;
        let leaf = |tag| slot_tile(tag, Vec::new());
        // Slot 1 is x = 1 and slot 6 is y = z = 1; the packed leaf covering octants 4 and 5
        // occupies its first slot (4) only.
        let tree = slot_tile(70, vec![(1, leaf(72)), (4, leaf(74)), (6, leaf(76))]);
        let raw = slot_raw(&[70, 72, 74, 76]);
        let tiles = vec![
            cell(0, 0, 0, 0),
            cell(1, 1, 0, 0),
            cell(1, 0, 0, 1),
            cell(1, 0, 1, 1),
        ];
        let derived = Availability {
            levels: 2,
            tiles: tiles.clone(),
            contents: vec![tiles.clone()],
        };
        let write = |planned: Availability| {
            let dir = stage(&raw);
            let document = vector_document();
            let writer = ForestWriter::new(&document, dir.path(), octree).unwrap();
            let name = RootName::new("f0-d0").unwrap();
            let tile = ForestTile::implicit(name, tree.clone()).planned(planned);
            let result = writer.root(tile, &mut || Ok(()));
            (result, files(dir.path()))
        };
        let swapped = |c: &Coordinates| cell(c.level, c.y, c.x, c.z);
        let mut axes = derived.clone();
        axes.tiles = tiles.iter().map(swapped).collect();
        axes.tiles.sort_by_key(|c| (c.level, morton(*c, octree)));
        let mut packed = derived.clone();
        packed.tiles.push(cell(1, 1, 0, 1));
        packed.tiles.sort_by_key(|c| (c.level, morton(*c, octree)));
        let mut content = derived.clone();
        content.contents[0].pop();
        let mut order = derived.clone();
        order.tiles.swap(1, 2);
        for (planned, why) in [
            (
                Availability {
                    levels: 3,
                    ..derived.clone()
                },
                "planned levels: 3",
            ),
            (axes, "tile row 1: planned"),
            (packed, "tile row 3"),
            (
                content,
                "content slot 0: planned 3 positions, the slots derive 4",
            ),
            (order, "tile row 1"),
            (
                Availability {
                    contents: Vec::new(),
                    ..derived.clone()
                },
                "content slots",
            ),
        ] {
            let (result, after) = write(planned);
            let message = refused(result, why);
            assert!(message.contains("availability mismatch"), "{message}");
            assert_eq!(after, raw, "nothing is written");
        }
        let (result, after) = write(derived.clone());
        let (top, output) = result.unwrap();
        assert_eq!(
            output.implicit_roots,
            [("subtrees/f0-d0-0-0-0-0.subtree".to_owned(), derived)]
        );
        assert_eq!(
            top["root"]["implicitTiling"]["subtrees"]["uri"],
            "subtrees/f0-d0-{level}-{x}-{y}-{z}.subtree"
        );
        assert!(after.contains_key("implicit-content/f0-d0-1-0-0-1-0.glb"));
    }

    /// Caller templates name every member, including per-root folders; unchanged payloads
    /// are never copied (in place at their final name, otherwise hard-linked); the height
    /// cap refuses deep roots.
    #[test]
    fn caller_templates_and_payloads_without_copies() {
        let octree = SubdivisionScheme::Octree;
        let template = |t: &str| UriTemplate::new(t, octree).unwrap();
        let names = ImplicitNames::Templates {
            contents: vec![template("implicit-content/f12-d3/{level}/{x}/{y}/{z}.glb")],
            subtree: template("subtrees/f12-d3/{level}/{x}/{y}/{z}.subtree"),
        };
        // The root payload is staged at its final member; the child's elsewhere.
        let (root_bytes, child_bytes) = (payload(90), payload(92));
        let final_root = "implicit-content/f12-d3/0/0/0/0.glb";
        let record = |bytes: &[u8], uri: &str| {
            TileRecord::new(
                NodePlacement::INHERIT,
                TileBox::centred([4.; 3]).unwrap(),
                GeometricError::new(1.).unwrap(),
                Refine::Replace,
                vec![ContentRef::new(
                    MemberName::new(uri).unwrap(),
                    Sha256::digest(bytes).into(),
                    bytes.len() as u64,
                    ContentKind::Glb,
                    false,
                    SourceIdentity::Aggregate,
                )
                .unwrap()],
                None::<Extras>,
            )
            .unwrap()
        };
        let child = TileTree::new(record(&child_bytes, &member(&child_bytes)), Vec::new());
        let tree = TileTree::with_slots(record(&root_bytes, final_root), vec![(7, child)]).unwrap();
        let mut raw = slot_raw(&[92]);
        raw.insert(final_root.to_owned(), root_bytes.clone());
        let dir = stage(&raw);
        let document = TilesetDocument::root(GeometricError::new(10.).unwrap(), []);
        let capped = ForestWriter::new(&document, dir.path(), octree)
            .unwrap()
            .with_max_implicit_levels(1);
        refused(
            capped.root(ForestTile::named(names.clone(), tree.clone()), &mut || {
                Ok(())
            }),
            "an implicit root has 2 levels; this forest admits at most 1",
        );
        assert_eq!(files(dir.path()), raw);
        let writer = ForestWriter::new(&document, dir.path(), octree)
            .unwrap()
            .with_max_implicit_levels(2);
        let (top, output) = writer
            .root(ForestTile::named(names, tree), &mut || Ok(()))
            .unwrap();
        assert_eq!(
            top["root"]["content"]["uri"],
            "implicit-content/f12-d3/{level}/{x}/{y}/{z}.glb"
        );
        let child_member = "implicit-content/f12-d3/1/1/1/1.glb";
        assert_eq!(
            output.members,
            [final_root, child_member, "subtrees/f12-d3/0/0/0/0.subtree"]
        );
        // The root payload stayed in place and is no source; the child is a hard link.
        assert_eq!(output.implicit_sources, [member(&child_bytes)]);
        let written = files(dir.path());
        assert_eq!(written[final_root], root_bytes);
        assert_eq!(written[child_member], child_bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let links = |m: &str| std::fs::metadata(dir.path().join(m)).unwrap().nlink();
            assert_eq!((links(final_root), links(child_member)), (1, 2));
        }
    }

    /// #198 follow-up: a payload staged at its final implicit member cannot also be rebased
    /// in place. A translated implicit child whose payload sits at its own final name is
    /// refused, and the refused document leaves the staging directory as it was; the same
    /// child staged elsewhere is written as a new, rebased member.
    #[test]
    fn payload_at_its_final_member_that_needs_a_rebase_is_refused() {
        let octree = SubdivisionScheme::Octree;
        let template = |t: &str| UriTemplate::new(t, octree).unwrap();
        let names = ImplicitNames::Templates {
            contents: vec![template("implicit-content/f7-d2/{level}/{x}/{y}/{z}.glb")],
            subtree: template("subtrees/f7-d2/{level}/{x}/{y}/{z}.subtree"),
        };
        let child_bytes = payload(94);
        let record = |placement: NodePlacement, contents: Vec<ContentRef>| {
            TileRecord::new(
                placement,
                TileBox::centred([4.; 3]).unwrap(),
                GeometricError::new(1.).unwrap(),
                Refine::Replace,
                contents,
                None::<Extras>,
            )
            .unwrap()
        };
        let content = |uri: &str| {
            ContentRef::new(
                MemberName::new(uri).unwrap(),
                Sha256::digest(&child_bytes).into(),
                child_bytes.len() as u64,
                ContentKind::Glb,
                false,
                SourceIdentity::Aggregate,
            )
            .unwrap()
        };
        // Slot 1 is (level 1, x 1, y 0, z 0).
        let final_child = "implicit-content/f7-d2/1/1/0/0.glb";
        let tree = |uri: &str| {
            let child = TileTree::new(
                record(
                    NodePlacement::translation([2., 0., 0.]).unwrap(),
                    vec![content(uri)],
                ),
                Vec::new(),
            );
            TileTree::with_slots(record(NodePlacement::INHERIT, Vec::new()), vec![(1, child)])
                .unwrap()
        };
        let document = TilesetDocument::root(GeometricError::new(10.).unwrap(), []);
        let raw = BTreeMap::from([(final_child.to_owned(), child_bytes.clone())]);
        let dir = stage(&raw);
        let writer = ForestWriter::new(&document, dir.path(), octree).unwrap();
        refused(
            writer.root(
                ForestTile::named(names.clone(), tree(final_child)),
                &mut || Ok(()),
            ),
            "payload \"implicit-content/f7-d2/1/1/0/0.glb\" is staged at its final member but \
             needs a rebase",
        );
        assert_eq!(files(dir.path()), raw, "nothing is written or removed");
        // Staged elsewhere, the translation is baked into a new member.
        let elsewhere = slot_raw(&[94]);
        let dir = stage(&elsewhere);
        let writer = ForestWriter::new(&document, dir.path(), octree).unwrap();
        let (_, output) = writer
            .root(
                ForestTile::named(names, tree(&member(&child_bytes))),
                &mut || Ok(()),
            )
            .unwrap();
        assert_eq!(output.implicit_sources, [member(&child_bytes)]);
        let written = files(dir.path());
        assert_ne!(written[final_child], child_bytes, "the payload is rebased");
        let rebase = output.tiles[1].contents[0].rebase.as_ref().unwrap();
        assert_eq!(rebase.delta, [2., 0., 0.]);
    }

    /// A refused document removes everything it created; explicit payloads are listed once
    /// across documents; a raw source that is a written member is refused.
    #[test]
    fn refused_documents_leave_nothing_and_sources_are_checked() {
        let quad = SubdivisionScheme::Quadtree;
        let raw = slot_raw(&[80, 82]);
        let document = vector_document();
        let none = &mut || Ok(());
        let leaf = |tag| slot_tile(tag, Vec::new());
        let name = |n: &str| RootName::new(n).unwrap();
        let explicit = |children| ForestTile::explicit(leaf(80).record().clone(), children);
        let dir = stage(&raw);
        let mut writer = ForestWriter::new(&document, dir.path(), quad).unwrap();
        // The second root's members coincide with the first's: the whole document goes.
        let twice = explicit(vec![
            ForestTile::implicit(name("a/r1"), leaf(82)),
            ForestTile::implicit(name("a/r1"), leaf(82)),
        ]);
        refused(
            writer.external(MemberName::new("d-one.json").unwrap(), twice, none),
            "\"implicit-content/a/r1-0-0-0-0.glb\" is already written",
        );
        assert_eq!(files(dir.path()), raw);
        assert!(!dir.path().join("implicit-content").exists());
        // The explicit payload of the refused document is listed by the next one, once.
        let (_, one) = writer
            .external(
                MemberName::new("d-one.json").unwrap(),
                explicit(Vec::new()),
                none,
            )
            .unwrap();
        let (_, two) = writer
            .external(
                MemberName::new("d-two.json").unwrap(),
                explicit(Vec::new()),
                none,
            )
            .unwrap();
        assert_eq!(one.members, [member(&payload(80)), "d-one.json".to_owned()]);
        assert_eq!(two.members, ["d-two.json"]);
        // A root whose raw source is another root's written member.
        let first = "implicit-content/b-0-0-0-0.glb";
        let bytes = payload(82);
        let reread = TileRecord::new(
            NodePlacement::INHERIT,
            TileBox::centred([4.; 3]).unwrap(),
            GeometricError::new(1.).unwrap(),
            Refine::Replace,
            vec![ContentRef::new(
                MemberName::new(first).unwrap(),
                Sha256::digest(&bytes).into(),
                bytes.len() as u64,
                ContentKind::Glb,
                true,
                SourceIdentity::Aggregate,
            )
            .unwrap()],
            None::<Extras>,
        )
        .unwrap();
        let before = files(dir.path());
        let collides = explicit(vec![
            ForestTile::implicit(name("b"), leaf(82)),
            ForestTile::implicit(name("c"), TileTree::new(reread, Vec::new())),
        ]);
        refused(
            writer.external(MemberName::new("d-three.json").unwrap(), collides, none),
            "collides with a written member",
        );
        assert_eq!(files(dir.path()), before);
    }

    /// Partition misuse is refused: a reused document name, an external document not
    /// rooted at an explicit tile, a link as tileset.json's root, too many children and bad
    /// names.
    #[test]
    fn streamed_partition_misuse_is_refused() {
        let quad = SubdivisionScheme::Quadtree;
        let raw = slot_raw(&[80, 82]);
        let document = vector_document();
        let none = &mut || Ok(());
        let leaf = |tag| slot_tile(tag, Vec::new());
        let name = |n: &str| RootName::new(n).unwrap();
        for good in ["r1", "f12-d3", "f12/d3", "a_b.c~d"] {
            assert_eq!(RootName::new(good).unwrap().as_str(), good);
        }
        for bad in [
            "",
            "x/",
            "../x",
            ".x",
            "a b",
            "a{b}",
            "a//b",
            &"a".repeat(257),
        ] {
            assert!(RootName::new(bad).is_err(), "{bad:?}");
        }
        let explicit = |children| ForestTile::explicit(leaf(80).record().clone(), children);
        let dir = stage(&raw);
        let mut writer = ForestWriter::new(&document, dir.path(), quad).unwrap();
        let (link, output) = writer
            .external(
                MemberName::new("d-two.json").unwrap(),
                explicit(Vec::new()),
                none,
            )
            .unwrap();
        assert_eq!(output.members.last().unwrap(), "d-two.json");
        for bad in ["d-two.json", "tileset.json", "d-three.glb", "d/six.json"] {
            refused(
                writer.external(MemberName::new(bad).unwrap(), explicit(Vec::new()), none),
                bad,
            );
        }
        refused(
            writer.external(
                MemberName::new("d-four.json").unwrap(),
                ForestTile::implicit(name("r4"), leaf(82)),
                none,
            ),
            "explicit tile",
        );
        let children = (0..5).map(|_| explicit(Vec::new())).collect();
        refused(
            writer.external(
                MemberName::new("d-five.json").unwrap(),
                explicit(children),
                none,
            ),
            "5 children exceed",
        );
        refused(
            writer.root(ForestTile::<Extras>::Link(link), none),
            "d-two.json",
        );
        // Escaped (sibling) names are not forest members.
        let model = ContentRef::source_model(
            MemberName::escaped("a%20b.glb").unwrap(),
            [0; 32],
            64,
            ContentKind::Glb,
        )
        .unwrap();
        let record = leaf(80).record().clone();
        let record = TileRecord::new(
            record.placement(),
            *record.bounds(),
            record.geometric_error(),
            record.refine(),
            vec![model],
            None::<Extras>,
        )
        .unwrap();
        let dir = stage(&raw);
        let writer = ForestWriter::new(&document, dir.path(), quad).unwrap();
        refused(
            writer.root(ForestTile::explicit(record, Vec::new()), none),
            "not an archive member",
        );
    }

    fn rss_kib() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
            .unwrap_or(0)
    }

    /// A 1,198,409-tile OCTREE forest, streamed one document at a time: 64 external
    /// documents of 4 skeleton tiles over 32 complete three-level implicit roots (585 tiles
    /// each), linked from a two-level root document. Resident memory stays bounded by one
    /// document: the process's RSS grows by less than 64 MiB after the first document.
    /// Ignored by default because RSS is per process: run it alone. CI runs it alone in the
    /// debug test build (about 30 s); release takes about 2 s:
    /// `cargo test --lib streamed_forest_memory_is_bounded_by_one_document -- --ignored
    /// --exact --nocapture` (set `M1S2_MEMORY_REPORT` to keep the JSON report).
    #[test]
    #[ignore = "writes a 2^20-tile forest; run alone so its RSS series is its own"]
    fn streamed_forest_memory_is_bounded_by_one_document() {
        let octree = SubdivisionScheme::Octree;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("t")).unwrap();
        let bytes = payload(1);
        std::fs::write(dir.path().join(member(&bytes)), &bytes).unwrap();
        let content = ContentRef::new(
            MemberName::new(member(&bytes)).unwrap(),
            Sha256::digest(&bytes).into(),
            bytes.len() as u64,
            ContentKind::Glb,
            false,
            SourceIdentity::Features {
                count: 1,
                digest: [1; 32],
            },
        )
        .unwrap();
        let record = |error: f64, contents: Vec<ContentRef>| {
            TileRecord::<()>::new(
                NodePlacement::INHERIT,
                TileBox::centred([1.; 3]).unwrap(),
                GeometricError::new(error).unwrap(),
                Refine::Replace,
                contents,
                None,
            )
            .unwrap()
        };
        // A complete octree of `height` below a tile; only the top tile has content.
        fn complete(
            height: u32,
            record: &dyn Fn(f64, Vec<ContentRef>) -> TileRecord<()>,
            contents: Vec<ContentRef>,
        ) -> TileTree<()> {
            let children = if height == 0 {
                Vec::new()
            } else {
                (0..8u8)
                    .map(|slot| (slot, complete(height - 1, record, Vec::new())))
                    .collect()
            };
            TileTree::with_slots(record(height as f64, contents), children).unwrap()
        }
        let document = TilesetDocument::root(GeometricError::new(100.).unwrap(), []);
        let mut writer = ForestWriter::new(&document, dir.path(), octree)
            .unwrap()
            .with_max_implicit_levels(4);
        let none = &mut || Ok(());
        let cells = complete_cells(4, octree);
        let planned = Availability {
            levels: 4,
            tiles: cells.clone(),
            contents: vec![vec![cells[0]]],
        };
        let started = std::time::Instant::now();
        let mut samples = Vec::new();
        let mut links = Vec::new();
        let mut tiles = 0usize;
        let mut roots = 0usize;
        for d in 0..64u32 {
            let parents = (0..4u32)
                .map(|p| {
                    let implicit = (0..8u32)
                        .map(|r| {
                            let name = RootName::new(format!("d{d}/p{p}-r{r}")).unwrap();
                            let tree = complete(3, &record, vec![content.clone()]);
                            ForestTile::implicit(name, tree).planned(planned.clone())
                        })
                        .collect();
                    ForestTile::explicit(record(4., Vec::new()), implicit)
                })
                .collect();
            let chunk = ForestTile::explicit(record(5., Vec::new()), parents);
            let name = MemberName::new(format!("docs-d{d}.json")).unwrap();
            let (link, output) = writer.external(name, chunk, none).unwrap();
            tiles += output.tiles.len();
            roots += output.implicit_roots.len();
            links.push(link);
            drop(output);
            samples.push(rss_kib());
        }
        let mut links = links.into_iter();
        let children = (0..8)
            .map(|_| {
                let linked = links.by_ref().take(8).map(ForestTile::Link).collect();
                ForestTile::explicit(record(6., Vec::new()), linked)
            })
            .collect();
        let (top, output) = writer
            .root(ForestTile::explicit(record(7., Vec::new()), children), none)
            .unwrap();
        tiles += output.tiles.len();
        samples.push(rss_kib());
        assert_eq!(tiles, 1_198_409);
        assert!(tiles >= 1 << 20);
        assert_eq!(roots, 2048);
        assert_eq!(
            top["root"]["children"][7]["children"][7]["content"]["uri"],
            "docs-d63.json"
        );
        let baseline = samples[0];
        let peak = samples.iter().copied().max().unwrap();
        let whole_lower_bound = tiles * std::mem::size_of::<TileTree<()>>() / 1024;
        let report = json!({
            "tiles": tiles,
            "implicit_roots": roots,
            "documents": 65,
            "rss_after_first_document_kib": baseline,
            "rss_peak_kib": peak,
            "rss_growth_kib": peak - baseline,
            "rss_samples_kib": samples,
            "whole_tree_records_lower_bound_kib": whole_lower_bound,
            "seconds": started.elapsed().as_secs_f64(),
        });
        eprintln!("{report}");
        if let Ok(path) = std::env::var("M1S2_MEMORY_REPORT") {
            std::fs::write(path, report.to_string()).unwrap();
        }
        assert!(
            peak - baseline < 64 * 1024,
            "RSS grew {} KiB beyond the first document",
            peak - baseline
        );
    }
}
