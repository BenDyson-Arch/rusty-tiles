//! The streamed bounded implicit forest: the caller partitions its hierarchy into documents
//! and the writer emits one bounded document at a time.
//!
//! [`write_forest_records`](super::write_forest_records) holds a whole [`TileTree`] and
//! chooses its own partition (skeleton height [`SKELETON_HEIGHT`](super::SKELETON_HEIGHT),
//! breadth-first packing within [`DOC_LEVELS`](super::DOC_LEVELS) and the document budget).
//! [`ForestWriter`] instead takes the partition from the caller, one document per call:
//!
//! - every external document is written by [`ForestWriter::external`] before the document
//!   that links it (children before parents; any such order gives the same bytes), and
//!   returns the [`DocumentLink`] its parent places as a [`ForestTile::Link`];
//! - `tileset.json` is written last by [`ForestWriter::root`].
//!
//! A document is a tree of [`ForestTile`]s: explicit tiles (skeleton tiles with children in
//! the document, or explicit leaves), implicit roots (one bounded [`TileTree`] each, written
//! as one subtree with `subtreeLevels == availableLevels`) and links. Memory is bounded by
//! the largest document the caller passes: the writer keeps no state between documents.
//!
//! Fed the whole-tree writer's own partition (its roles, `r{preorder id}` root names and
//! `tileset-d{m}.json` document names), the streamed forest is byte-identical to
//! `write_forest_records`. The writer enforces only hard limits: the per-document budget
//! ([`DOC_NODES`], [`DOC_BYTES`], [`DOC_DEPTH`]), at most [`MAX_LEVELS`] levels, the
//! subtree availability limit, and explicit tiles of at most the scheme's branch count of
//! children. Which tiles are skeleton, implicit or linked is the caller's frozen plan.
//!
//! Raw payloads that implicit tiles copy into `implicit-content/` stay in place: each
//! document reports them ([`ChunkOutput::implicit_sources`]) and the caller deletes those
//! that no explicit tile references once the forest is complete, as the whole-tree writer
//! does.
//!
//! The large-mesh tiler's delivery (H16) is the intended caller; until it lands, the tests
//! below are the only one.
#![cfg_attr(not(test), allow(dead_code))]

use super::{
    check_document, check_implicit_tile, collect_records, coordinate_template, data, key, widen,
    ImplicitRoot, Records, TileRole, WrittenContent, WrittenTile, MAX_LEVELS,
};
#[cfg(doc)]
use super::{DOC_BYTES, DOC_DEPTH, DOC_NODES};
use crate::implicit::{Coordinates, SubdivisionScheme};
use crate::tileset_node::{
    GeometricError, MemberName, ObjectRole, TileRecord, TileTree, TilesetDocument, TilesetExtension,
};
use crate::Error;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{collections::BTreeSet, io::Write, path::Path};

/// The member prefix of a streamed implicit root: 1 to 64 ASCII letters, digits and `_`.
/// Its subtree is `subtrees/{name}-{level}-{x}-{y}[-{z}].subtree` and its payloads
/// `implicit-content/{name}-{level}-{x}-{y}[-{z}]-{slot}.{suffix}`; `-` is excluded so
/// distinct names never share a member. The whole-tree writer's names are `r{preorder id}`.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RootName(String);

impl RootName {
    pub(crate) fn new(name: impl Into<String>) -> Result<Self, Error> {
        let name = name.into();
        if name.is_empty()
            || name.len() > 64
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(data(format!(
                "implicit root name {name:?} must be 1 to 64 ASCII letters, digits or '_'"
            )));
        }
        Ok(Self(name))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// An implicit root's availability: its levels (`subtreeLevels == availableLevels`), its
/// available tiles and its available contents over every content slot (one payload member
/// each).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Availability {
    pub(crate) levels: u32,
    pub(crate) tiles: u64,
    pub(crate) contents: u64,
}

/// One tile of a streamed document.
pub(crate) enum ForestTile<X> {
    /// An explicit tile: a skeleton tile whose `children` are in this document, or (without
    /// children) an explicit leaf.
    Explicit {
        record: TileRecord<X>,
        children: Vec<ForestTile<X>>,
    },
    /// An implicit root and its bounded hierarchy, written as one subtree named `name`. Its
    /// children's slots ([`TileTree::with_slots`]) give the subtree availability, which
    /// must equal `expected` when the caller planned it.
    Implicit {
        name: RootName,
        tree: TileTree<X>,
        expected: Option<Availability>,
    },
    /// A document written earlier.
    Link(DocumentLink),
}

impl<X> ForestTile<X> {
    pub(crate) fn explicit(record: TileRecord<X>, children: Vec<ForestTile<X>>) -> Self {
        Self::Explicit { record, children }
    }

    pub(crate) fn implicit(name: RootName, tree: TileTree<X>) -> Self {
        Self::Implicit {
            name,
            tree,
            expected: None,
        }
    }

    /// An implicit root whose derived availability must equal `expected`.
    pub(crate) fn planned(name: RootName, tree: TileTree<X>, expected: Availability) -> Self {
        Self::Implicit {
            name,
            tree,
            expected: Some(expected),
        }
    }
}

/// A written external document, to be linked exactly once by its parent document: its link
/// tile, the drift by which the parent's explicit ancestors grow, and its levels.
#[derive(Debug)]
pub(crate) struct DocumentLink {
    name: MemberName,
    object: Map<String, Value>,
    reach: [f64; 3],
    levels: u32,
}

impl DocumentLink {
    pub(crate) fn name(&self) -> &MemberName {
        &self.name
    }
}

/// What one streamed document wrote.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ChunkOutput {
    /// Members written or kept, in order: explicit payloads (kept in place), implicit
    /// payloads and subtrees, then the external document itself. `tileset.json` is not
    /// listed.
    pub(crate) members: Vec<String>,
    /// One record per tile of the document in preorder, links excluded.
    pub(crate) tiles: Vec<WrittenTile>,
    /// Raw payloads copied into `implicit-content/`, sorted; the caller deletes those no
    /// explicit tile references.
    pub(crate) implicit_sources: Vec<String>,
    /// Each implicit root's derived availability, in preorder.
    pub(crate) implicit_roots: Vec<(RootName, Availability)>,
}

/// Writes a forest one caller-partitioned document at a time (see the module notes).
pub(crate) struct ForestWriter<'a> {
    document: &'a TilesetDocument,
    directory: &'a Path,
    scheme: SubdivisionScheme,
}

/// A tile object as placed in its document: the object, the drift by which its explicit
/// ancestors grow, and its height.
type Placed = (Map<String, Value>, [f64; 3], u32);

/// One document's tile objects as they are assembled.
#[derive(Default)]
struct Assembly {
    output: ChunkOutput,
    explicit_sources: BTreeSet<String>,
    implicit_sources: BTreeSet<String>,
    holds_implicit: bool,
}

impl<'a> ForestWriter<'a> {
    /// A writer for the forest whose `tileset.json` members other than `root` are
    /// `document`, under `directory`, in which every content is staged.
    pub(crate) fn new(
        document: &'a TilesetDocument,
        directory: &'a Path,
        scheme: SubdivisionScheme,
    ) -> Result<Self, Error> {
        if !document.is_root() {
            return Err(data("a forest is written from tileset.json's members"));
        }
        if document.build_state().is_some() {
            return Err(data(
                "the build state is recorded after the forest is written",
            ));
        }
        Ok(Self {
            document,
            directory,
            scheme,
        })
    }

    /// Write the external document `name` rooted at the explicit tile `root`, and return
    /// the link its parent document places. The name must be a new `.json` member.
    pub(crate) fn external<X: Serialize>(
        &self,
        name: MemberName,
        root: ForestTile<X>,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<(DocumentLink, ChunkOutput), Error> {
        checkpoint()?;
        if name.as_str() == "tileset.json" || !name.is_member() || !name.as_str().ends_with(".json")
        {
            return Err(data(format!(
                "external document {:?} must be an archive member named *.json, not tileset.json",
                name.as_str()
            )));
        }
        let ForestTile::Explicit { record, children } = root else {
            return Err(data(format!(
                "external document {:?} must be rooted at an explicit tile",
                name.as_str()
            )));
        };
        if self.directory.join(name.as_str()).exists() {
            return Err(data(format!(
                "external document {:?} is already written",
                name.as_str()
            )));
        }
        let mut assembly = Assembly::default();
        let (root, reach, height) = self.explicit(
            &record,
            children,
            ObjectRole::DocumentRoot,
            0,
            &mut assembly,
            checkpoint,
        )?;
        let widened = widened(&record, reach)?;
        let link = DocumentLink {
            object: record.link_object(&widened, &name),
            name,
            reach,
            levels: height + 1,
        };
        let extensions = self.document.extensions_used().iter().copied();
        let mut body = TilesetDocument::external(
            GeometricError::new(record.geometric_error().metres())?,
            extensions,
        )
        .with_schema(assembly.holds_implicit)
        .head();
        body.insert("root".into(), Value::Object(root));
        let bytes = check_document(link.name.as_str(), &Value::Object(body))?;
        let path = self.directory.join(link.name.as_str());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?
            .write_all(&bytes)?;
        let mut output = assembly.finish();
        output.members.push(link.name.as_str().to_owned());
        Ok((link, output))
    }

    /// Write `tileset.json`'s document rooted at `root` (an explicit tile or an implicit
    /// root) and return `tileset.json` (with `schema`), which the producer serializes.
    pub(crate) fn root<X: Serialize>(
        self,
        root: ForestTile<X>,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<(Value, ChunkOutput), Error> {
        checkpoint()?;
        let mut assembly = Assembly::default();
        let object = match root {
            ForestTile::Explicit { record, children } => {
                self.explicit(
                    &record,
                    children,
                    ObjectRole::Tile,
                    0,
                    &mut assembly,
                    checkpoint,
                )?
                .0
            }
            ForestTile::Implicit {
                name,
                tree,
                expected,
            } => {
                self.implicit(&name, &tree, expected, true, &mut assembly, checkpoint)?
                    .0
            }
            ForestTile::Link(link) => {
                return Err(data(format!(
                    "tileset.json cannot be rooted at the link to {:?}",
                    link.name.as_str()
                )))
            }
        };
        let mut top = self.document.clone().with_schema(true).head();
        top.insert("root".into(), Value::Object(object));
        let top = Value::Object(top);
        check_document("tileset.json", &top)?;
        Ok((top, assembly.finish()))
    }

    /// Check a tile's records for the document's declared extensions.
    fn check_vector<X: Serialize>(&self, record: &TileRecord<X>) -> Result<(), Error> {
        let vector = TilesetExtension::ContentGltfVector;
        if !self.document.extensions_used().contains(&vector)
            && record.contents().iter().any(|c| c.vector())
        {
            return Err(data(format!(
                "vector content needs {} in extensionsUsed",
                vector.as_str()
            )));
        }
        Ok(())
    }

    /// An explicit tile at document `depth` (0 for the document root), its children
    /// assembled, at bounds widened by the drift below it. Returns the object, the drift and
    /// the tile's height.
    fn explicit<X: Serialize>(
        &self,
        record: &TileRecord<X>,
        children: Vec<ForestTile<X>>,
        role: ObjectRole,
        depth: u32,
        assembly: &mut Assembly,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<Placed, Error> {
        checkpoint()?;
        if depth >= MAX_LEVELS {
            return Err(levels_error(depth + 1));
        }
        // Every tile below tileset.json's root, an external root included, is a child.
        let child = depth > 0 || role == ObjectRole::DocumentRoot;
        if child && record.placement().child_translation().is_none() {
            return Err(data(format!(
                "document level {depth}: a child transform must be a translation"
            )));
        }
        if children.len() > self.scheme.branches() {
            return Err(data(format!(
                "document level {depth}: {} children exceed the {} scheme's {}",
                children.len(),
                self.scheme.as_str(),
                self.scheme.branches()
            )));
        }
        self.check_vector(record)?;
        // The tile's own record precedes its descendants'.
        let index = assembly.output.tiles.len();
        let mut contents = Vec::with_capacity(record.contents().len());
        for content in record.contents() {
            if !content.uri().is_member() {
                return Err(data(format!(
                    "content {:?} is not an archive member name",
                    content.uri().as_str()
                )));
            }
            let bytes = content.verify_staged(self.directory)?;
            let member = content.uri().as_str().to_owned();
            if assembly.explicit_sources.insert(member.clone()) {
                assembly.output.members.push(member.clone());
            }
            contents.push(WrittenContent {
                member,
                bytes,
                rebase: None,
            });
        }
        assembly.output.tiles.push(WrittenTile {
            role: if children.is_empty() {
                TileRole::ExplicitLeaf
            } else {
                TileRole::Skeleton
            },
            extras: record
                .extras_object()?
                .map_or_else(|| json!({}), Value::Object),
            contents,
        });
        debug_assert_eq!(assembly.output.tiles.len(), index + 1);
        let mut objects = Vec::with_capacity(children.len());
        let mut reach = [0f64; 3];
        let mut height = 0;
        let has_children = !children.is_empty();
        for child in children {
            let (object, below, child_height) = match child {
                ForestTile::Explicit { record, children } => self.explicit(
                    &record,
                    children,
                    ObjectRole::Tile,
                    depth + 1,
                    assembly,
                    checkpoint,
                )?,
                ForestTile::Implicit {
                    name,
                    tree,
                    expected,
                } => {
                    if depth + 1 >= MAX_LEVELS {
                        return Err(levels_error(depth + 2));
                    }
                    self.implicit(&name, &tree, expected, false, assembly, checkpoint)?
                }
                ForestTile::Link(link) => (link.object, link.reach, link.levels - 1),
            };
            for (lane, step) in reach.iter_mut().zip(below) {
                *lane = lane.max(step);
            }
            height = height.max(child_height + 1);
            objects.push(Value::Object(object));
        }
        if depth + height >= MAX_LEVELS {
            return Err(levels_error(depth + height + 1));
        }
        let mut object = record.explicit_object(&widened(record, reach)?, role)?;
        if has_children {
            object.insert("children".into(), Value::Array(objects));
        }
        Ok((object, reach, height))
    }

    /// One implicit root: its payloads and subtree, and its tile object. Returns the
    /// object, its drift and its height.
    fn implicit<X: Serialize>(
        &self,
        name: &RootName,
        tree: &TileTree<X>,
        expected: Option<Availability>,
        at_tileset_root: bool,
        assembly: &mut Assembly,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<Placed, Error> {
        checkpoint()?;
        if !at_tileset_root && tree.record().placement().child_translation().is_none() {
            return Err(data(format!(
                "implicit root {}: a child transform must be a translation",
                name.as_str()
            )));
        }
        let (nodes, records) = collect_records(tree, self.scheme, checkpoint)?;
        let root = &nodes[0];
        for (t, node) in nodes.iter().enumerate() {
            check_implicit_tile(t, node, root, t == 0)?;
        }
        for record in &records {
            self.check_vector(record)?;
        }
        let derived = Availability {
            levels: root.height + 1,
            tiles: nodes.len() as u64,
            contents: nodes.iter().map(|n| n.contents.len() as u64).sum(),
        };
        if let Some(expected) = expected {
            if expected != derived {
                return Err(data(format!(
                    "implicit root {}: availability mismatch: planned {expected:?}, the slots \
                     derive {derived:?}",
                    name.as_str()
                )));
            }
        }
        let origin = Coordinates {
            level: 0,
            x: 0,
            y: 0,
            z: 0,
        };
        let subtree = format!(
            "subtrees/{}-{}.subtree",
            name.as_str(),
            key(origin, self.scheme)
        );
        if self.directory.join(&subtree).exists() {
            return Err(data(format!(
                "implicit root {} is already written",
                name.as_str()
            )));
        }
        std::fs::create_dir_all(self.directory.join("implicit-content"))?;
        std::fs::create_dir_all(self.directory.join("subtrees"))?;
        let writer = ImplicitRoot {
            id: 0,
            name: name.as_str(),
            scheme: self.scheme,
            directory: self.directory,
            template: coordinate_template(self.scheme),
        };
        let height = root.height;
        let emit = Records {
            records,
            document: self.document,
        };
        let mut tiles = vec![None; nodes.len()];
        let (object, _, reach) = writer.write(
            &nodes,
            &emit,
            &mut tiles,
            &mut assembly.output.members,
            &mut assembly.implicit_sources,
            checkpoint,
        )?;
        assembly.output.tiles.extend(
            tiles
                .into_iter()
                .map(|tile| tile.expect("every implicit tile is written")),
        );
        assembly.output.implicit_roots.push((name.clone(), derived));
        assembly.holds_implicit = true;
        Ok((object, reach, height))
    }
}

impl Assembly {
    fn finish(self) -> ChunkOutput {
        let mut output = self.output;
        output.implicit_sources = self.implicit_sources.into_iter().collect();
        output
    }
}

/// `record`'s bounds grown by `reach`, as the whole-tree writer widens skeleton tiles.
fn widened<X: Serialize>(
    record: &TileRecord<X>,
    reach: [f64; 3],
) -> Result<crate::tileset_node::TileBox, Error> {
    let mut lanes = record.bounds().lanes();
    widen(&mut lanes, reach);
    record.bounds().with_half_lanes(&lanes)
}

fn levels_error(levels: u32) -> Error {
    Error::Data(format!(
        "hierarchy has at least {levels} levels; implicit output admits at most {MAX_LEVELS}"
    ))
}
