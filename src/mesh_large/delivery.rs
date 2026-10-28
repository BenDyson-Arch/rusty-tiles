//! H16: H1's tile tree and H8's full-detail leaves, published as a Tiles 1.1 implicit forest
//! in one `.3tz` through the shared typed serializer (`implicit::ForestWriter`).
//!
//! Input is H8's published leaf directory. Two passes stream its rows in rank order through
//! the same [`plan::Builder`]: the first only counts (exact member count, so the 3TZ ceiling
//! refuses before anything is staged, and the root error the root document needs); the
//! second stages payloads by hard link at their final member names and writes one document
//! at a time. The archive is then written from the staged members in a fixed order. See
//! `docs/architecture/h16-delivery-contract.md`.
mod archive;
pub(crate) mod plan;
pub(crate) mod source;
#[cfg(test)]
pub(crate) mod tests;

use self::{
    plan::{document_members, state, Builder, Limits, Linked, Node, Shape, Sink, LIMITS},
    source::{LeafDirectory, LeafRow, ProxyRow},
};
use super::config::MEMBER_CEILING_3TZ;
use crate::{
    implicit::{
        Availability, ChunkOutput, Coordinates, ForestTile, ForestWriter, ImplicitNames,
        SubdivisionScheme,
    },
    mesh_archive::{MeshPlacement, MeshPlacementReport, ResolvedPlacement},
    runtime::Staging,
    tileset_node::{
        top_level_error, ContentKind, ContentRef, GeometricError, MemberName, NodePlacement,
        Refine, SourceIdentity, TileRecord, TileTree, TilesetDocument, UriTemplate,
    },
    CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl, RunEvent,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

const OCTREE: SubdivisionScheme = SubdivisionScheme::Octree;

/// Publish an H8 leaf directory as a `.3tz` tileset.
#[derive(Clone, Debug)]
pub struct MeshTilesetRequest {
    leaves: PathBuf,
    output: PathBuf,
    placement: MeshPlacement,
    policy: OutputPolicy,
    member_ceiling: u64,
}

impl MeshTilesetRequest {
    /// `leaves` is a directory published by the internal leaf producer.
    pub fn new(leaves: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            leaves: leaves.into(),
            output: output.into(),
            placement: MeshPlacement::Local,
            policy: OutputPolicy::CreateNew,
            member_ceiling: MEMBER_CEILING_3TZ,
        }
    }
    pub fn with_placement(mut self, placement: MeshPlacement) -> Self {
        self.placement = placement;
        self
    }
    pub fn with_output_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
    #[cfg(test)]
    pub(crate) fn with_member_ceiling(mut self, ceiling: u64) -> Self {
        self.member_ceiling = ceiling;
        self
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshTilesetReport {
    pub schema_version: u32,
    pub leaves: u64,
    pub triangles: u64,
    pub tiles: u64,
    pub explicit_tiles: u64,
    pub implicit_roots: u64,
    pub implicit_tiles: u64,
    pub documents: u64,
    pub max_depth: u32,
    pub synthetic_leaves: u64,
    pub packed_leaves: u64,
    /// Internal tiles with an HLOD coarse tile (none for a full-detail-only directory).
    pub proxies: u64,
    /// Every 3TZ member, `tileset.json` and the index included.
    pub members: u64,
    pub member_ceiling: u64,
    pub archive_bytes: u64,
    pub root_geometric_error: f64,
    pub tileset_geometric_error: f64,
    pub peak_held_records: u64,
    pub placement: MeshPlacementReport,
}

#[derive(Debug)]
pub struct MeshTilesetResult {
    pub output: PathBuf,
    pub report: MeshTilesetReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}

/// A tile's `extras`: its H1 identity and leaf context. Rank strings keep u64 exact in JSON.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct TileExtras {
    first: String,
    depth: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<String>,
    #[serde(rename = "lastOctant", skip_serializing_if = "Option::is_none")]
    last_octant: Option<u8>,
    #[serde(rename = "syntheticLevels", skip_serializing_if = "Option::is_none")]
    synthetic_levels: Option<u8>,
}

fn record(node: &Node, contents: Vec<ContentRef>) -> Result<TileRecord<TileExtras>, JobError> {
    let leaf = node.leaf.as_deref();
    let extras = TileExtras {
        first: node.first.to_string(),
        depth: node.depth,
        end: leaf.map(|l| l.end.to_string()),
        last_octant: leaf.filter(|l| l.packed).map(|l| l.last_octant),
        synthetic_levels: leaf.filter(|l| l.synthetic > 0).map(|l| l.synthetic),
    };
    let error = GeometricError::new(node.error).map_err(|e| state(e.to_string()))?;
    TileRecord::new(
        NodePlacement::INHERIT,
        node.tile,
        error,
        Refine::Replace,
        contents,
        Some(extras),
    )
    .map_err(|e| state(e.to_string()))
}

/// The content of a leaf published as `member`. Its source identities are the GLB's
/// `source_primitive`/`source_triangle` property tables; the accounting digest binds the
/// rank range to the payload that carries them.
fn leaf_content(row: &LeafRow, member: String) -> Result<ContentRef, JobError> {
    let mut digest = Sha256::new();
    digest.update(b"rusty-tiles/h16/source-identity/v1\0");
    digest.update(row.first.to_le_bytes());
    digest.update(row.end.to_le_bytes());
    digest.update(row.sha256);
    let count = u32::try_from(row.triangles()).map_err(|_| state("leaf triangle count"))?;
    ContentRef::new(
        MemberName::new(member).map_err(|e| state(e.to_string()))?,
        row.sha256,
        row.bytes,
        ContentKind::Glb,
        false,
        SourceIdentity::Features {
            count,
            digest: digest.finalize().into(),
        },
    )
    .map_err(|e| state(e.to_string()))
}

/// The content of an HLOD coarse tile published as `member`: its source identities are the
/// rows of its representative `source_primitive` table (D4). The digest binds the tile's
/// rank range to the payload. Its domain string is a published accounting constant, kept
/// from the coarse tiles' first producer so the identity digests do not change.
fn proxy_content(row: &ProxyRow, member: String) -> Result<ContentRef, JobError> {
    let mut digest = Sha256::new();
    digest.update(b"rusty-tiles/h17/proxy-identity/v1\0");
    digest.update(row.first.to_le_bytes());
    digest.update(row.end.to_le_bytes());
    digest.update(row.depth.to_le_bytes());
    digest.update(row.sha256);
    ContentRef::new(
        MemberName::new(member).map_err(|e| state(e.to_string()))?,
        row.sha256,
        row.bytes,
        ContentKind::Glb,
        false,
        SourceIdentity::Features {
            count: row.primitives,
            digest: digest.finalize().into(),
        },
    )
    .map_err(|e| state(e.to_string()))
}

fn morton(c: &Coordinates) -> u128 {
    let mut index = 0u128;
    for bit in 0..c.level {
        for (k, axis) in [c.x, c.y, c.z].into_iter().enumerate() {
            index |= u128::from((axis >> bit) & 1) << (3 * bit as usize + k);
        }
    }
    index
}

/// Planning: counts members and documents; nothing is written.
#[derive(Default)]
struct PlanSink {
    members: u64,
    root: Option<(f64, f64)>,
}

impl Sink for PlanSink {
    fn external(&mut self, node: Node) -> Result<Linked, JobError> {
        self.members += document_members(&node) + 1;
        Ok(None)
    }
    fn root(&mut self, node: Node) -> Result<(), JobError> {
        self.members += document_members(&node) + 1;
        self.root = Some((node.error, node.tile.diagonal()));
        Ok(())
    }
}

/// Explicit children with every chain or omitted cell replaced by its children
/// (recursively): a coarse tile's children are then its renderable replacements, each with
/// its own box and error (no greater than the coarse tile's).
///
/// Below content (`under`), a content-less explicit cell is spliced out as well (see
/// `Node::barrier`); its children then sit below that content too.
fn spliced(children: Vec<Node>, under: bool) -> Vec<Node> {
    let mut out = Vec::with_capacity(children.len());
    for child in children {
        if (child.is_chain_cell() || child.is_spliced_barrier(under)) && !child.is_implicit_root() {
            out.extend(spliced(child.children, true));
        } else {
            out.push(child);
        }
    }
    out
}

/// Run a forest-writer call with the job's cancellation as its checkpoint.
fn forest<T>(
    staging: &Path,
    check: &mut dyn FnMut() -> Result<(), JobError>,
    call: impl FnOnce(&mut dyn FnMut() -> Result<(), crate::Error>) -> Result<T, crate::Error>,
) -> Result<T, JobError> {
    let mut stopped = None;
    let result = call(&mut || {
        check().map_err(|error| {
            let message = error.message().to_owned();
            stopped = Some(error);
            crate::Error::Message(message)
        })
    });
    if let Some(error) = stopped {
        return Err(error);
    }
    result.map_err(|error| match error {
        crate::Error::Io(source) => JobError::io("write forest member", staging, source),
        other => state(format!(
            "the forest writer refused a planned document: {other}"
        )),
    })
}

/// Publishing: stages payloads at their final names and writes each document.
struct WriteSink<'a> {
    writer: Option<ForestWriter<'a>>,
    staging: &'a Path,
    leaves: &'a LeafDirectory,
    placement: NodePlacement,
    names: BufWriter<crate::storage_meter::Metered<File>>,
    names_path: PathBuf,
    members: u64,
    top: Option<serde_json::Value>,
    check: &'a mut dyn FnMut() -> Result<(), JobError>,
}

impl WriteSink<'_> {
    /// Hard-link a leaf payload into the staging directory at `member` (one copy only where
    /// the filesystem has no hard links).
    fn stage(&mut self, row: &LeafRow, member: &str) -> Result<(), JobError> {
        self.stage_payload(&row.uri, row.bytes, row.sha256, member)
    }

    /// Stage a tile's coarse content; with `chain`, a chain cell repeats its coarse tile.
    fn stage_proxy(
        &mut self,
        node: &Node,
        member: &str,
        chain: bool,
    ) -> Result<Vec<ContentRef>, JobError> {
        let row = match (node.proxy.as_deref(), node.chain.as_deref()) {
            (Some(row), _) => row,
            (None, Some(row)) if chain => row,
            _ => return Ok(Vec::new()),
        };
        self.stage_payload(&row.uri, row.bytes, row.sha256, member)?;
        Ok(vec![proxy_content(row, member.to_owned())?])
    }

    fn stage_payload(
        &mut self,
        uri: &str,
        length: u64,
        sha256: [u8; 32],
        member: &str,
    ) -> Result<(), JobError> {
        let source = self.leaves.member(uri);
        // The row's digest is checked here, so a payload that contradicts its row is the
        // leaf directory's fault (InvalidInput); the writer re-checks the staged member.
        let bytes =
            std::fs::read(&source).map_err(|e| JobError::io("read leaf payload", &source, e))?;
        if bytes.len() as u64 != length || <[u8; 32]>::from(Sha256::digest(&bytes)) != sha256 {
            return Err(source::input(format!(
                "{uri}: payload bytes differ from its manifest row (length or SHA-256)"
            )));
        }
        drop(bytes);
        let target = self.staging.join(member);
        if let Some(parent) = target.parent() {
            crate::storage_meter::create_dir_all(parent)
                .map_err(|e| JobError::io("create staging folder", parent, e))?;
        }
        match crate::storage_meter::hard_link(&source, &target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(state(format!("member {member} is staged twice")))
            }
            Err(_) => crate::storage_meter::copy(&source, &target)
                .map(|_| ())
                .map_err(|e| JobError::io("copy leaf payload", &source, e)),
        }
    }

    /// A document's root tile (a chain cell there repeats its coarse tile).
    fn tile(&mut self, node: Node) -> Result<ForestTile<TileExtras>, JobError> {
        self.tile_in(node, true)
    }

    fn tile_in(&mut self, node: Node, root: bool) -> Result<ForestTile<TileExtras>, JobError> {
        if let Some(link) = node.link {
            return link
                .map(ForestTile::Link)
                .ok_or_else(|| state("a planned link reached the writer"));
        }
        if let Some(row) = node.leaf.as_deref() {
            let member = format!("content/f{}-d{}.glb", node.first, node.depth);
            self.stage(row, &member)?;
            let content = leaf_content(row, member)?;
            return Ok(ForestTile::explicit(
                record(&node, vec![content])?,
                Vec::new(),
            ));
        }
        if node.is_implicit_root() {
            return self.implicit(node);
        }
        // Skeleton tiles and the tiles of a shallow tree written explicitly.
        let contents = self.stage_proxy(
            &node,
            &format!("content/f{}-d{}.glb", node.first, node.depth),
            root,
        )?;
        let under = !contents.is_empty();
        let own = record(&node, contents)?;
        let mut children = Vec::with_capacity(node.children.len());
        for child in spliced(node.children, under) {
            children.push(self.tile_in(child, false)?);
        }
        Ok(ForestTile::explicit(own, children))
    }

    fn implicit(&mut self, node: Node) -> Result<ForestTile<TileExtras>, JobError> {
        let key = format!("f{}-d{}", node.first, node.depth);
        let template =
            |pattern: String| UriTemplate::new(pattern, OCTREE).map_err(|e| state(e.to_string()));
        let names = ImplicitNames::Templates {
            contents: vec![template(format!(
                "implicit-content/{key}-{{level}}-{{x}}-{{y}}-{{z}}.glb"
            ))?],
            subtree: template(format!(
                "subtrees/{key}-{{level}}-{{x}}-{{y}}-{{z}}.subtree"
            ))?,
        };
        let mut planned = Availability {
            levels: node.height + 1,
            tiles: Vec::new(),
            contents: vec![Vec::new()],
        };
        let (depth, axes) = (node.depth, node.axes);
        let tree = self.implicit_tree(node, depth, axes, &key, &mut planned)?;
        planned.tiles.sort_by_key(|c| (c.level, morton(c)));
        planned.contents[0].sort_by_key(|c| (c.level, morton(c)));
        Ok(ForestTile::named(names, tree).planned(planned))
    }

    /// The implicit hierarchy below `root_depth`, its planned availability taken from each
    /// tile's H1 address rather than from the slots the writer derives it from.
    fn implicit_tree(
        &mut self,
        node: Node,
        root_depth: u32,
        root_axes: [u32; 3],
        key: &str,
        planned: &mut Availability,
    ) -> Result<TileTree<TileExtras>, JobError> {
        let level = node.depth - root_depth;
        if (0..3).any(|a| node.axes[a] >> level != root_axes[a]) {
            return Err(state("an implicit tile is outside its root's address"));
        }
        let mask = (1u32 << level) - 1;
        let c = Coordinates {
            level,
            x: node.axes[0] & mask,
            y: node.axes[1] & mask,
            z: node.axes[2] & mask,
        };
        planned.tiles.push(c);
        if let Some(row) = node.leaf.as_deref() {
            let member = format!(
                "implicit-content/{key}-{}-{}-{}-{}.glb",
                c.level, c.x, c.y, c.z
            );
            self.stage(row, &member)?;
            planned.contents[0].push(c);
            let content = leaf_content(row, member)?;
            return Ok(TileTree::new(record(&node, vec![content])?, Vec::new()));
        }
        let member = format!(
            "implicit-content/{key}-{}-{}-{}-{}.glb",
            c.level, c.x, c.y, c.z
        );
        let contents = self.stage_proxy(&node, &member, true)?;
        if !contents.is_empty() {
            planned.contents[0].push(c);
        }
        if !contents.is_empty() && node.children.iter().any(Node::is_content_less) {
            // Cesium would let this tile stop drawing before that child's content loads.
            return Err(state(
                "a content-less tile sits below content inside an implicit tree",
            ));
        }
        let own = record(&node, contents)?;
        let mut children = Vec::with_capacity(node.children.len());
        for child in node.children {
            let slot = child.slot;
            children.push((
                slot,
                self.implicit_tree(child, root_depth, root_axes, key, planned)?,
            ));
        }
        TileTree::with_slots(own, children).map_err(|e| state(e.to_string()))
    }

    /// Check one written document against its plan and list its members.
    fn record(&mut self, output: ChunkOutput, expected: u64) -> Result<(), JobError> {
        if output.members.len() as u64 != expected {
            return Err(state(format!(
                "a document wrote {} members; its plan counted {expected}",
                output.members.len()
            )));
        }
        if let Some(source) = output.implicit_sources.first() {
            return Err(state(format!(
                "payload {source} was not staged at its final member"
            )));
        }
        for member in &output.members {
            writeln!(self.names, "{member}")
                .map_err(|e| JobError::io("write member list", &self.names_path, e))?;
        }
        self.members += expected;
        Ok(())
    }
}

impl Sink for WriteSink<'_> {
    fn external(&mut self, node: Node) -> Result<Linked, JobError> {
        let expected = document_members(&node) + 1;
        let name = MemberName::new(format!("tileset-f{}-d{}.json", node.first, node.depth))
            .map_err(|e| state(e.to_string()))?;
        let tile = self.tile(node)?;
        let writer = self.writer.as_mut().expect("documents precede the root");
        let (link, output) = forest(self.staging, self.check, |mut cp| {
            writer.external(name, tile, &mut cp)
        })?;
        self.record(output, expected)?;
        Ok(Some(link))
    }

    fn root(&mut self, node: Node) -> Result<(), JobError> {
        let expected = document_members(&node);
        let mut tile = self.tile(node)?;
        match &mut tile {
            ForestTile::Explicit { record, .. } => record.set_placement(self.placement),
            ForestTile::Implicit { tree, .. } => tree.record_mut().set_placement(self.placement),
            ForestTile::Link(_) => return Err(state("the root was written as a link")),
        }
        let writer = self.writer.take().expect("one root");
        let (top, output) = forest(self.staging, self.check, |mut cp| {
            writer.root(tile, &mut cp)
        })?;
        self.record(output, expected)?;
        self.top = Some(top);
        Ok(())
    }
}

/// The root transform: none for local output, otherwise the rigid anchor/orientation/offset
/// placement as a rotation and translation.
fn root_placement(placement: &ResolvedPlacement) -> Result<NodePlacement, JobError> {
    if matches!(placement.report(), MeshPlacementReport::Local) {
        return Ok(NodePlacement::INHERIT);
    }
    let t = placement.transform();
    let axes = [[t[0], t[1], t[2]], [t[4], t[5], t[6]], [t[8], t[9], t[10]]];
    NodePlacement::rigid(axes, [t[12], t[13], t[14]]).map_err(|e| state(e.to_string()))
}

/// The planned forest.
struct Planned {
    shape: Shape,
    members: u64,
    root_error: f64,
    tileset_error: f64,
}

fn plan_pass(
    leaves: &LeafDirectory,
    limits: Limits,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Planned, JobError> {
    let mut sink = PlanSink::default();
    let proxies = leaves.proxies.then(|| leaves.proxy_rows()).transpose()?;
    let mut builder = Builder::with_proxies(&mut sink, limits, proxies);
    let mut triangles = 0u64;
    for (index, item) in leaves.leaves()?.enumerate() {
        if index % 256 == 0 {
            check()?;
        }
        let (row, bounds) = item?;
        triangles = row.end;
        builder.push(row, bounds)?;
    }
    let shape = builder.finish()?;
    leaves.check_omissions(&shape)?;
    if shape.leaves != leaves.totals.leaves || triangles != leaves.totals.triangles {
        return Err(source::input(format!(
            "leaves.jsonl holds {} leaves and {triangles} triangles; manifest.json reports {} \
             and {}",
            shape.leaves, leaves.totals.leaves, leaves.totals.triangles
        )));
    }
    let (root_error, diagonal) = sink.root.ok_or_else(|| state("no root was planned"))?;
    Ok(Planned {
        shape,
        members: sink.members,
        root_error,
        tileset_error: top_level_error(diagonal, root_error, 1.),
    })
}

fn ceiling(members: u64, ceiling: u64, what: &str) -> Result<(), JobError> {
    if members > ceiling {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            format!(
                "3TZ member ceiling: {what} {members} members (tileset.json and the index \
                 included) exceed {ceiling}"
            ),
        ));
    }
    Ok(())
}

pub(crate) fn publish(
    request: &MeshTilesetRequest,
    attempt_check: &mut dyn FnMut() -> Result<(), JobError>,
    out: &mut File,
    work_parent: &Path,
) -> Result<(MeshTilesetReport, u64), JobError> {
    publish_with(request, LIMITS, attempt_check, out, work_parent)
}

/// [`publish`] under other fragment caps (tests forcing implicit roots and documents).
#[cfg(test)]
pub(crate) fn publish_with_limits(
    request: &MeshTilesetRequest,
    limits: Limits,
    out: &mut File,
    work_parent: &Path,
) -> Result<(MeshTilesetReport, u64), JobError> {
    publish_with(request, limits, &mut || Ok(()), out, work_parent)
}

fn publish_with(
    request: &MeshTilesetRequest,
    limits: Limits,
    check: &mut dyn FnMut() -> Result<(), JobError>,
    out: &mut File,
    work_parent: &Path,
) -> Result<(MeshTilesetReport, u64), JobError> {
    let prepared = prepare(request, limits, check, false)?;
    write_prepared(
        prepared,
        check,
        out,
        work_parent,
        &StorageCharges::default(),
    )
    .map(|(report, bytes, _)| (report, bytes))
}

/// D8 meters of the publication's memory-backed (tmpfs) storage, `None` for an owner on
/// disk: the private work directory (registered over `work_parent` by the caller, so leaf
/// and coarse payloads, forest staging and archive scratch are admitted as they are
/// written) and the archive staged beside the output (admitted by the archive writer).
#[derive(Clone, Debug, Default)]
pub(crate) struct StorageCharges {
    pub work: Option<std::sync::Arc<crate::storage_meter::Meter>>,
    pub archive: Option<std::sync::Arc<crate::storage_meter::Meter>>,
}

/// Semantic delivery facts known before archive serialization. No runtime measurements.
#[derive(Clone, Debug)]
pub(crate) struct DeliveryFacts {
    pub tiles: u64,
    pub explicit_tiles: u64,
    pub implicit_roots: u64,
    pub implicit_tiles: u64,
    pub documents: u64,
    pub max_depth: u32,
    pub synthetic_leaves: u64,
    pub packed_leaves: u64,
    pub members: u64,
    pub member_ceiling: u64,
    pub root_geometric_error: f64,
    pub tileset_geometric_error: f64,
    pub placement: MeshPlacementReport,
    pub root_transform: Option<[f64; 16]>,
}

pub(crate) struct PreparedDelivery {
    limits: Limits,
    leaves: LeafDirectory,
    planned: Planned,
    placement: ResolvedPlacement,
    root: NodePlacement,
    members: u64,
    member_ceiling: u64,
    conversion: Option<Vec<u8>>,
}
impl PreparedDelivery {
    pub(crate) fn facts(&self) -> DeliveryFacts {
        let shape = &self.planned.shape;
        DeliveryFacts {
            tiles: shape.tiles,
            explicit_tiles: shape.explicit_tiles,
            implicit_roots: shape.implicit_roots,
            implicit_tiles: shape.implicit_tiles,
            documents: shape.documents,
            max_depth: shape.max_depth,
            synthetic_leaves: shape.synthetic_leaves,
            packed_leaves: shape.packed_leaves,
            members: self.members,
            member_ceiling: self.member_ceiling,
            root_geometric_error: self.planned.root_error,
            tileset_geometric_error: self.planned.tileset_error,
            placement: self.placement.report(),
            root_transform: self.root.matrix(),
        }
    }
}

fn prepare(
    request: &MeshTilesetRequest,
    limits: Limits,
    check: &mut dyn FnMut() -> Result<(), JobError>,
    conversion_member: bool,
) -> Result<PreparedDelivery, JobError> {
    let placement = ResolvedPlacement::resolve(&request.placement)?;
    prepare_resolved(
        request,
        placement,
        limits,
        check,
        conversion_member,
        source::MAX_GLB_JSON,
    )
}

fn prepare_resolved(
    request: &MeshTilesetRequest,
    placement: ResolvedPlacement,
    limits: Limits,
    check: &mut dyn FnMut() -> Result<(), JobError>,
    conversion_member: bool,
    max_glb_json: u64,
) -> Result<PreparedDelivery, JobError> {
    let root = root_placement(&placement)?;
    let leaves = LeafDirectory::open(&request.leaves)?.with_max_glb_json(max_glb_json);
    // Early: every leaf is a member, as are tileset.json and the index.
    ceiling(
        leaves
            .totals
            .leaves
            .saturating_add(2 + u64::from(conversion_member)),
        request.member_ceiling,
        "at least",
    )?;
    let planned = plan_pass(&leaves, limits, check)?;
    let members = planned.members + 1 + u64::from(conversion_member);
    ceiling(members, request.member_ceiling, "exactly")?;
    Ok(PreparedDelivery {
        limits,
        leaves,
        planned,
        placement,
        root,
        members,
        member_ceiling: request.member_ceiling,
        conversion: None,
    })
}

/// `max_glb_json` caps each member's GLB JSON chunk read for bounds: a budgeted run passes
/// [`source::TILER_MAX_GLB_JSON`], which its [`resident_bound`] charges.
pub(crate) fn prepare_tiler(
    request: &MeshTilesetRequest,
    placement: ResolvedPlacement,
    check: &mut dyn FnMut() -> Result<(), JobError>,
    make_report: impl FnOnce(&DeliveryFacts) -> Result<Vec<u8>, JobError>,
    max_glb_json: u64,
) -> Result<PreparedDelivery, JobError> {
    let mut prepared = prepare_resolved(request, placement, LIMITS, check, true, max_glb_json)?;
    prepared.conversion = Some(make_report(&prepared.facts())?);
    Ok(prepared)
}

/// Stage the forest under `work_parent` and write the archive into `out`. Every staged
/// entry goes through [`crate::storage_meter`], so a charged work directory admits it before
/// it is written. Once the forest is staged, the stored model of a charged work directory is
/// measured and must be within what its meter admitted (a larger figure is a write that
/// bypassed the meter); that measure is returned. A charged archive is admitted by the
/// archive writer before each part of it is written.
pub(crate) fn write_prepared(
    prepared: PreparedDelivery,
    check: &mut dyn FnMut() -> Result<(), JobError>,
    out: &mut File,
    work_parent: &Path,
    charges: &StorageCharges,
) -> Result<(MeshTilesetReport, u64, Option<u64>), JobError> {
    let PreparedDelivery {
        limits,
        leaves,
        planned,
        placement,
        root,
        members,
        member_ceiling,
        conversion,
    } = prepared;
    check()?;
    let work = crate::storage_meter::tempdir_in(work_parent, ".tiles-h16-")
        .map_err(|e| JobError::io("create forest staging", work_parent, e))?;
    let staging = work.path().join("forest");
    crate::storage_meter::create_dir(&staging)
        .map_err(|e| JobError::io("create forest staging", &staging, e))?;
    let names_path = work.path().join("members.txt");
    let names = BufWriter::new(
        crate::storage_meter::create(&names_path)
            .map_err(|e| JobError::io("create member list", &names_path, e))?,
    );
    let mut document = TilesetDocument::root(
        GeometricError::new(planned.tileset_error).map_err(|e| state(e.to_string()))?,
        [],
    );
    // The producer's HLOD output (coarse tiles with their stage record) declares itself, so
    // a validator applies the HLOD expectation without being asked.
    if leaves.proxies {
        document = document
            .declaring_hlod()
            .map_err(|e| state(e.to_string()))?;
    }
    let writer = ForestWriter::new(&document, &staging, OCTREE)
        .map_err(|e| state(e.to_string()))?
        .with_max_implicit_levels(plan::SKELETON_HEIGHT);
    let mut sink = WriteSink {
        writer: Some(writer),
        staging: &staging,
        leaves: &leaves,
        placement: root,
        names,
        names_path: names_path.clone(),
        members: 0,
        top: None,
        check,
    };
    let proxies = leaves.proxies.then(|| leaves.proxy_rows()).transpose()?;
    let mut builder = Builder::with_proxies(&mut sink, limits, proxies);
    for (index, item) in leaves.leaves()?.enumerate() {
        if index % 256 == 0 {
            (builder.sink().check)()?;
        }
        let (row, bounds) = item?;
        builder.push(row, bounds)?;
    }
    let shape = builder.finish()?;
    if shape != planned.shape {
        return Err(state("the published forest differs from its plan"));
    }
    let WriteSink {
        mut names,
        members: listed,
        top,
        check,
        ..
    } = sink;
    if let Some(bytes) = conversion {
        let member = staging.join("conversion.json");
        crate::storage_meter::write(&member, &bytes)
            .map_err(|e| JobError::io("write conversion.json", &member, e))?;
        writeln!(names, "conversion.json")
            .map_err(|e| JobError::io("write member list", &names_path, e))?;
    }
    names
        .flush()
        .map_err(|e| JobError::io("write member list", &names_path, e))?;
    drop(names);
    if listed + 1 != planned.members {
        return Err(state("the published member count differs from its plan"));
    }
    let top = top.ok_or_else(|| state("tileset.json was not written"))?;
    let bytes = serde_json::to_vec(&top).map_err(|e| state(e.to_string()))?;
    let tileset = staging.join("tileset.json");
    crate::storage_meter::write(&tileset, &bytes)
        .map_err(|e| JobError::io("write tileset.json", &tileset, e))?;
    let scratch_need =
        archive::scratch_need(members).ok_or_else(|| state("archive scratch overflow"))?;
    let work_bytes = match &charges.work {
        Some(meter) => {
            let stored = crate::storage_meter::stored_bytes(meter, work_parent, &staging)
                .map_err(|e| JobError::io("measure work files", work_parent, e))?;
            if stored > meter.used() {
                return Err(state(format!(
                    "the work directory stores {stored} bytes but its D8 meter admitted only {}: \
                     a write bypassed the meter",
                    meter.used()
                )));
            }
            // The archive's scratch directory entry; its files are admitted as written.
            meter
                .admit(crate::storage_meter::ENTRY_BYTES)
                .map_err(|e| JobError::io("create archive scratch", work.path(), e))?;
            Some(stored)
        }
        None => None,
    };
    let scratch = crate::spill::Scratch::create(Some(work.path()), scratch_need)?;
    let archive_bytes = archive::write(
        &staging,
        &names_path,
        members - 1,
        &scratch,
        out,
        check,
        charges.archive.as_deref(),
    )?;
    scratch.close()?;
    work.close()
        .map_err(|e| JobError::io("remove forest staging", work_parent, e))?;
    let report = MeshTilesetReport {
        schema_version: 1,
        leaves: shape.leaves,
        triangles: leaves.totals.triangles,
        tiles: shape.tiles,
        explicit_tiles: shape.explicit_tiles,
        implicit_roots: shape.implicit_roots,
        implicit_tiles: shape.implicit_tiles,
        documents: shape.documents,
        max_depth: shape.max_depth,
        synthetic_leaves: shape.synthetic_leaves,
        packed_leaves: shape.packed_leaves,
        proxies: shape.proxies,
        members,
        member_ceiling,
        archive_bytes,
        root_geometric_error: planned.root_error,
        tileset_geometric_error: planned.tileset_error,
        peak_held_records: shape.peak_held_records,
        placement: placement.report(),
    };
    Ok((report, archive_bytes, work_bytes))
}

/// Bytes of parsed `serde_json::Value` per byte of JSON text, a proven ceiling: a `Value`
/// is 32 bytes and every value takes at least two bytes of text with its separator, arrays
/// hold at most twice their length in capacity (32 per text byte), a map entry (24-byte key
/// and a value in a B-tree node at least half full, at most 128 bytes) takes at least five
/// bytes of text, and string contents are at most the text itself.
const JSON_VALUE_BYTES_PER_TEXT_BYTE: u64 = 33;

/// Declared resident bound of preparing and writing a tiler archive (D8), which runs after
/// the leaf and coarse stages have released their workspaces: the planner's held records,
/// the archive writer, one member's parsed GLB JSON (members are read one at a time, each
/// JSON chunk at most `max_glb_json` bytes, with its text), and an allowance for one
/// hierarchy document or subtree, the conversion report and row lines being serialized.
pub(crate) fn resident_bound(max_glb_json: u64) -> u64 {
    plan::held_bytes_bound()
        + archive::resident_bound()
        + (JSON_VALUE_BYTES_PER_TEXT_BYTE + 1) * max_glb_json
        + (64 << 20)
}

/// Publish an H8 leaf directory as a Tiles 1.1 `.3tz` (hidden CLI `mesh-leaves-to-3tz`).
pub fn mesh_leaves_to_3tz(
    request: MeshTilesetRequest,
    control: &RunControl,
) -> Result<MeshTilesetResult, JobFailure> {
    let attempt = control.begin()?;
    let mut staging = Staging::create(&request.output, &attempt)?;
    let parent = staging
        .path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let result = publish(&request, &mut || attempt.check(), staging.writer(), &parent);
    let report = match result {
        Ok((report, _)) => report,
        Err(error) => return Err(staging.fail(error)),
    };
    if let Err(error) = attempt
        .emit(&RunEvent::Progress {
            phase: "mesh_tileset",
            done: report.leaves,
            total: Some(report.leaves),
        })
        .and_then(|_| attempt.close_events())
    {
        return Err(staging.fail(error));
    }
    let published = staging.seal()?.publish(&request.output, request.policy)?;
    Ok(MeshTilesetResult {
        output: published.output,
        report,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}
