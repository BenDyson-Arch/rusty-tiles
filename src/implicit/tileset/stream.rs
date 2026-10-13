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
//! the largest document the caller passes. The only state kept between documents is the set
//! of explicit payload members and document names, so explicit payloads are listed once and
//! raw sources are checked against them.
//!
//! Fed the whole-tree writer's own partition (its roles, `r{preorder id}` root names and
//! `tileset-d{m}.json` document names), the streamed forest is byte-identical to
//! `write_forest_records`. The writer enforces only hard limits: the per-document budget
//! ([`DOC_NODES`], [`DOC_BYTES`], [`DOC_DEPTH`]), at most [`MAX_LEVELS`] levels, the
//! subtree availability limit, an optional implicit-root height cap
//! ([`ForestWriter::with_max_implicit_levels`]) and explicit tiles of at most the scheme's
//! branch count of children. Which tiles are skeleton, implicit or linked is the caller's
//! frozen plan; a planned [`Availability`] is checked position by position.
//!
//! Members are always created new, never overwritten. A refused document removes every file
//! and directory it created, so a refusal leaves the staging directory as it was.
//!
//! Payloads: an implicit payload whose bytes do not change (no translation to bake in) is
//! never copied. Staged at its final member name ([`ImplicitNames`]), it stays in place;
//! staged elsewhere, its member is a hard link to it (one copy only on a filesystem without
//! hard links). A payload that must be rebased is written as a new member. Each document
//! reports the raw sources it consumed ([`ChunkOutput::implicit_sources`]); deleting a source
//! is safe once no tile still to be written references it and no explicit tile references
//! it: delete them after [`ForestWriter::root`] (minus every explicit member), or stage each
//! implicit payload under a name only that tile uses and delete it as soon as its document
//! is written. Staging at the final name needs neither.
//!
//! The large-mesh tiler's delivery (H16) is the intended caller; until it lands, the tests
//! are the only one.
#![cfg_attr(not(test), allow(dead_code))]

use super::{
    check_document, check_implicit_tile, collect_records, coordinate_template, data, widen,
    Created, ImplicitRoot, Publish, Records, RootNames, TileRole, WrittenContent, WrittenTile,
    MAX_LEVELS,
};
#[cfg(doc)]
use super::{DOC_BYTES, DOC_DEPTH, DOC_NODES};
use crate::implicit::{Coordinates, SubdivisionScheme};
use crate::tileset_node::{
    GeometricError, MemberName, ObjectRole, TileBox, TileRecord, TileTree, TilesetDocument,
    TilesetExtension, UriTemplate,
};
use crate::Error;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{collections::BTreeSet, path::Path};

/// The member prefix of a streamed implicit root: 1 to 256 bytes of URI unreserved ASCII
/// in `/`-separated non-empty segments, none starting with `.`. Its subtree is
/// `subtrees/{name}-{level}-{x}-{y}[-{z}].subtree` and its payloads
/// `implicit-content/{name}-{level}-{x}-{y}[-{z}]-{slot}.{suffix}` (`r{preorder id}` in the
/// whole-tree writer). Two names whose members coincide are refused when the second is
/// written.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RootName(String);

impl RootName {
    pub(crate) fn new(name: impl Into<String>) -> Result<Self, Error> {
        let name = name.into();
        if name.is_empty()
            || name.len() > 256
            || name
                .split('/')
                .any(|segment| segment.is_empty() || segment.starts_with('.'))
        {
            return Err(data(format!(
                "implicit root name {name:?} must be 1 to 256 bytes of non-empty segments, none \
                 starting with '.'"
            )));
        }
        MemberName::new(format!("subtrees/{name}-0.subtree")).map_err(|_| {
            data(format!(
                "implicit root name {name:?} is not a member prefix"
            ))
        })?;
        Ok(Self(name))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// How a streamed implicit root names its members.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ImplicitNames {
    /// The whole-tree writer's form under a [`RootName`] prefix.
    Prefix(RootName),
    /// The caller's templates: one content template per content slot, in slot order (content
    /// index, then suffix), each ending in its slot's `.{suffix}`, and the subtree template;
    /// for example `implicit-content/f12-d3/{level}/{x}/{y}/{z}.glb` and
    /// `subtrees/f12-d3/{level}/{x}/{y}/{z}.subtree`.
    Templates {
        contents: Vec<UriTemplate>,
        subtree: UriTemplate,
    },
}

/// An implicit root's availability, position by position: its levels
/// (`subtreeLevels == availableLevels`), its available tiles and, per content slot, the
/// tiles whose content is available, each in subtree bit order (level-major, Morton order
/// within a level; the metadata row order).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Availability {
    pub(crate) levels: u32,
    pub(crate) tiles: Vec<Coordinates>,
    pub(crate) contents: Vec<Vec<Coordinates>>,
}

impl Availability {
    /// The first way `derived` differs from this plan, if any.
    pub(crate) fn difference(&self, derived: &Availability) -> Option<String> {
        fn cells(label: &str, planned: &[Coordinates], derived: &[Coordinates]) -> Option<String> {
            match planned.iter().zip(derived).position(|(p, d)| p != d) {
                Some(row) => Some(format!(
                    "{label} row {row}: planned {:?}, the slots derive {:?}",
                    planned[row], derived[row]
                )),
                None if planned.len() != derived.len() => Some(format!(
                    "{label}: planned {} positions, the slots derive {}",
                    planned.len(),
                    derived.len()
                )),
                None => None,
            }
        }
        if self.levels != derived.levels {
            return Some(format!(
                "planned levels: {}, the slots derive {}",
                self.levels, derived.levels
            ));
        }
        if self.contents.len() != derived.contents.len() {
            return Some(format!(
                "planned {} content slots, the contents derive {}",
                self.contents.len(),
                derived.contents.len()
            ));
        }
        cells("tile", &self.tiles, &derived.tiles).or_else(|| {
            self.contents
                .iter()
                .zip(&derived.contents)
                .enumerate()
                .find_map(|(slot, (p, d))| cells(&format!("content slot {slot}"), p, d))
        })
    }
}

/// One tile of a streamed document.
pub(crate) enum ForestTile<X> {
    /// An explicit tile: a skeleton tile whose `children` are in this document, or (without
    /// children) an explicit leaf.
    Explicit {
        record: TileRecord<X>,
        children: Vec<ForestTile<X>>,
    },
    /// An implicit root and its bounded hierarchy, written as one subtree. Its children's
    /// slots ([`TileTree::with_slots`]) give the subtree availability, which must equal
    /// `expected` position by position when the caller planned it.
    Implicit {
        names: ImplicitNames,
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

    /// An implicit root named by `name` ([`ImplicitNames::Prefix`]).
    pub(crate) fn implicit(name: RootName, tree: TileTree<X>) -> Self {
        Self::named(ImplicitNames::Prefix(name), tree)
    }

    pub(crate) fn named(names: ImplicitNames, tree: TileTree<X>) -> Self {
        Self::Implicit {
            names,
            tree,
            expected: None,
        }
    }

    /// This implicit root, whose derived availability must equal `expected`.
    pub(crate) fn planned(self, expected: Availability) -> Self {
        match self {
            Self::Implicit { names, tree, .. } => Self::Implicit {
                names,
                tree,
                expected: Some(expected),
            },
            other => other,
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
    /// Members written or kept, in order: explicit payloads (kept in place, each listed by
    /// the first document that references it), implicit payloads and subtrees, then the
    /// external document itself. `tileset.json` is not listed.
    pub(crate) members: Vec<String>,
    /// One record per tile of the document in preorder, links excluded.
    pub(crate) tiles: Vec<WrittenTile>,
    /// Raw payloads published under another implicit member name, sorted, other than
    /// explicit members known so far; see the module notes before deleting them.
    pub(crate) implicit_sources: Vec<String>,
    /// Each implicit root's subtree member and derived availability, in preorder.
    pub(crate) implicit_roots: Vec<(String, Availability)>,
}

/// Writes a forest one caller-partitioned document at a time (see the module notes).
pub(crate) struct ForestWriter<'a> {
    document: &'a TilesetDocument,
    directory: &'a Path,
    scheme: SubdivisionScheme,
    max_implicit_levels: u32,
    /// Explicit payload members listed so far.
    explicit: BTreeSet<String>,
    /// External documents written so far.
    documents: BTreeSet<String>,
}

/// A tile object as placed in its document: the object, the drift by which its explicit
/// ancestors grow, and its height.
type Placed = (Map<String, Value>, [f64; 3], u32);

/// One document as it is assembled.
#[derive(Default)]
struct Assembly {
    output: ChunkOutput,
    /// Explicit members this document listed first.
    explicit: Vec<String>,
    sources: BTreeSet<String>,
    created: Created,
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
            max_implicit_levels: MAX_LEVELS,
            explicit: BTreeSet::new(),
            documents: BTreeSet::new(),
        })
    }

    /// Refuse implicit roots of more than `levels` levels (for example the frozen plan's
    /// subtree height).
    pub(crate) fn with_max_implicit_levels(mut self, levels: u32) -> Self {
        self.max_implicit_levels = levels.min(MAX_LEVELS);
        self
    }

    /// Write the external document `name` rooted at the explicit tile `root`, and return
    /// the link its parent document places. The name must be a new `.json` member at the
    /// archive root (member names are written unchanged, and they resolve relative to the
    /// document). A refused document removes what it created.
    pub(crate) fn external<X: Serialize>(
        &mut self,
        name: MemberName,
        root: ForestTile<X>,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<(DocumentLink, ChunkOutput), Error> {
        checkpoint()?;
        // Content, template and link URIs resolve relative to the document that holds them,
        // and the forest writes them as archive-root member names: a document in a folder
        // would make every one of them resolve inside that folder.
        if name.as_str() == "tileset.json"
            || !name.is_member()
            || !name.as_str().ends_with(".json")
            || name.as_str().contains('/')
        {
            return Err(data(format!(
                "external document {:?} must be an archive-root member named *.json, not \
                 tileset.json",
                name.as_str()
            )));
        }
        let ForestTile::Explicit { record, children } = root else {
            return Err(data(format!(
                "external document {:?} must be rooted at an explicit tile",
                name.as_str()
            )));
        };
        let mut assembly = Assembly::default();
        let result = self.external_document(&name, &record, children, &mut assembly, checkpoint);
        match result {
            Ok(link) => {
                let mut output = self.finish(assembly)?;
                output.members.push(name.as_str().to_owned());
                self.documents.insert(name.as_str().to_owned());
                Ok((link, output))
            }
            Err(error) => Err(self.refuse(assembly, error)),
        }
    }

    /// Undo a refused document: remove what it created and forget its explicit members.
    fn refuse(&mut self, assembly: Assembly, error: Error) -> Error {
        for member in &assembly.explicit {
            self.explicit.remove(member);
        }
        assembly.created.remove();
        error
    }

    fn external_document<X: Serialize>(
        &mut self,
        name: &MemberName,
        record: &TileRecord<X>,
        children: Vec<ForestTile<X>>,
        assembly: &mut Assembly,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<DocumentLink, Error> {
        let (root, reach, height) = self.explicit(
            record,
            children,
            ObjectRole::DocumentRoot,
            0,
            assembly,
            checkpoint,
        )?;
        let extensions = self.document.extensions_used().iter().copied();
        let mut body = TilesetDocument::external(
            GeometricError::new(record.geometric_error().metres())?,
            extensions,
        )
        .with_schema(assembly.holds_implicit)
        .head();
        body.insert("root".into(), Value::Object(root));
        let bytes = check_document(name.as_str(), &Value::Object(body))?;
        self.check_sources(assembly)?;
        assembly
            .created
            .file(self.directory, name.as_str(), &bytes)?;
        Ok(DocumentLink {
            object: record.link_object(&widened(record, reach)?, name),
            name: name.clone(),
            reach,
            levels: height + 1,
        })
    }

    /// Write `tileset.json`'s document rooted at `root` (an explicit tile or an implicit
    /// root) and return `tileset.json` (with `schema`), which the producer serializes. A
    /// refused document removes what it created.
    pub(crate) fn root<X: Serialize>(
        mut self,
        root: ForestTile<X>,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<(Value, ChunkOutput), Error> {
        checkpoint()?;
        let mut assembly = Assembly::default();
        match self.root_document(root, &mut assembly, checkpoint) {
            Ok(top) => Ok((top, self.finish(assembly)?)),
            Err(error) => Err(self.refuse(assembly, error)),
        }
    }

    fn root_document<X: Serialize>(
        &mut self,
        root: ForestTile<X>,
        assembly: &mut Assembly,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<Value, Error> {
        let object = match root {
            ForestTile::Explicit { record, children } => {
                self.explicit(&record, children, ObjectRole::Tile, 0, assembly, checkpoint)?
                    .0
            }
            ForestTile::Implicit {
                names,
                tree,
                expected,
            } => {
                self.implicit(&names, &tree, expected, true, assembly, checkpoint)?
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
        self.check_sources(assembly)?;
        Ok(top)
    }

    /// A raw source must not be a member the forest wrote: its deletion would remove that
    /// member (the whole-tree writer's collision check). Within the document every written
    /// member is known; across documents, every document name. Every member is created new,
    /// so no later member can be written over a source that still exists.
    fn check_sources(&self, assembly: &Assembly) -> Result<(), Error> {
        let written: BTreeSet<&str> = assembly
            .output
            .members
            .iter()
            .map(String::as_str)
            .filter(|m| !self.explicit.contains(*m))
            .collect();
        for source in &assembly.sources {
            if written.contains(source.as_str()) || self.documents.contains(source) {
                return Err(data(format!(
                    "raw source {source:?} collides with a written member"
                )));
            }
        }
        Ok(())
    }

    fn finish(&self, assembly: Assembly) -> Result<ChunkOutput, Error> {
        // Success: what the document created is published.
        drop(assembly.created);
        let mut output = assembly.output;
        output.implicit_sources = assembly
            .sources
            .into_iter()
            .filter(|source| !self.explicit.contains(source))
            .collect();
        Ok(output)
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
    /// assembled, at bounds widened by the drift below it.
    fn explicit<X: Serialize>(
        &mut self,
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
            if self.explicit.insert(member.clone()) {
                assembly.output.members.push(member.clone());
                assembly.explicit.push(member.clone());
            }
            contents.push(WrittenContent {
                member,
                bytes,
                rebase: None,
            });
        }
        // The tile's own record precedes its descendants'.
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
                    names,
                    tree,
                    expected,
                } => {
                    if depth + 1 >= MAX_LEVELS {
                        return Err(levels_error(depth + 2));
                    }
                    self.implicit(&names, &tree, expected, false, assembly, checkpoint)?
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

    /// One implicit root: its payloads and subtree, and its tile object.
    fn implicit<X: Serialize>(
        &self,
        names: &ImplicitNames,
        tree: &TileTree<X>,
        expected: Option<Availability>,
        at_tileset_root: bool,
        assembly: &mut Assembly,
        checkpoint: &mut impl FnMut() -> Result<(), Error>,
    ) -> Result<Placed, Error> {
        checkpoint()?;
        if !at_tileset_root && tree.record().placement().child_translation().is_none() {
            return Err(data("an implicit root's transform must be a translation"));
        }
        if let ImplicitNames::Templates { contents, subtree } = names {
            if contents
                .iter()
                .chain([subtree])
                .any(|t| t.scheme() != self.scheme)
            {
                return Err(data(format!(
                    "an implicit root's templates must be {} templates",
                    self.scheme.as_str()
                )));
            }
        }
        let (nodes, records) = collect_records(tree, self.scheme, checkpoint)?;
        let root = &nodes[0];
        if root.height + 1 > self.max_implicit_levels {
            return Err(data(format!(
                "an implicit root has {} levels; this forest admits at most {}",
                root.height + 1,
                self.max_implicit_levels
            )));
        }
        for (t, node) in nodes.iter().enumerate() {
            check_implicit_tile(t, node, root, t == 0)?;
        }
        for record in &records {
            self.check_vector(record)?;
        }
        let writer = ImplicitRoot {
            id: 0,
            names: match names {
                ImplicitNames::Prefix(name) => RootNames::Prefix(name.as_str()),
                ImplicitNames::Templates { contents, subtree } => {
                    RootNames::Templates { contents, subtree }
                }
            },
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
        let (object, subtree, reach, derived) = writer.write(
            &nodes,
            &emit,
            &mut tiles,
            &mut assembly.output.members,
            &mut assembly.sources,
            Publish::Fresh(&mut assembly.created),
            expected.as_ref(),
            checkpoint,
        )?;
        assembly.output.tiles.extend(
            tiles
                .into_iter()
                .map(|tile| tile.expect("every implicit tile is written")),
        );
        assembly.output.implicit_roots.push((subtree, derived));
        assembly.holds_implicit = true;
        Ok((object, reach, height))
    }
}

/// `record`'s bounds grown by `reach`, as the whole-tree writer widens skeleton tiles.
fn widened<X: Serialize>(record: &TileRecord<X>, reach: [f64; 3]) -> Result<TileBox, Error> {
    let mut lanes = record.bounds().lanes();
    widen(&mut lanes, reach);
    record.bounds().with_half_lanes(&lanes)
}

fn levels_error(levels: u32) -> Error {
    Error::Data(format!(
        "hierarchy has at least {levels} levels; implicit output admits at most {MAX_LEVELS}"
    ))
}
