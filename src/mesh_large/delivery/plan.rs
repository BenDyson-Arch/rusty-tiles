//! The streamed hierarchy: H8's leaf rows, in rank order, rebuilt into H1's tile tree with a
//! stack of at most 32 open tiles, partitioned into documents bottom-up as tiles close.
//!
//! Every proper prefix of a leaf address is an H1 internal tile (the cell tree opens a cell
//! at every depth on the way down, and a run hierarchy a node at every run length), and a
//! tile's rank range is the union of its leaves', so the leaf rows alone determine the tree.
//! A tile closes when the next leaf leaves its subtree. A closed tile's role follows its
//! height (P-IMPL §4): height ≥ 4 is an explicit skeleton tile; its leaf children are explicit
//! leaves and its other children (height 1 to 3) implicit roots, each written as one subtree.
//! A skeleton tile whose inline document fragment grows past [`Limits`] is written as an
//! external document at once and replaced by its link, so resident state is the open stack
//! plus fragments below the limits, whatever the input size.
use super::source::{add_down, add_up, Aabb, LeafRow, ProxyRow};
use crate::{tileset_node::TileBox, JobError, JobErrorKind};

/// The fixed partition. Inline fragments stay below these; the root document and every
/// external document therefore hold at most `1 + 8 · objects` tile objects and `levels`
/// tile levels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Tile objects (explicit tiles, implicit roots, links) of an inline fragment.
    pub objects: u64,
    /// Records (tiles, including those inside implicit roots) of an inline fragment.
    pub records: u64,
    /// Tile levels of a document.
    pub levels: u32,
}

pub(crate) const LIMITS: Limits = Limits {
    objects: 32,
    records: 1024,
    levels: 8,
};

/// Below this height a non-leaf tile is (inside) an implicit root (P-IMPL `S`).
pub(crate) const SKELETON_HEIGHT: u32 = 4;

/// An empty parent's geometric error is this multiple of its box diagonal: until its whole
/// box projects below about one pixel at CesiumJS's default maximum screen-space error of
/// 16, the viewer refines it towards the leaves that hold the geometry.
pub(crate) const EMPTY_PARENT_SCALE: f64 = 16.;

pub(crate) fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// The box written for `aabb`: centre and outward half lengths, and the closed extent of
/// that box (rounded outward), which is what its parent encloses.
pub(crate) fn tile_box(aabb: Aabb) -> Result<(TileBox, Aabb), JobError> {
    let center: [f64; 3] = std::array::from_fn(|i| aabb.lo[i] * 0.5 + aabb.hi[i] * 0.5);
    let half: [f64; 3] = std::array::from_fn(|i| {
        add_up(aabb.hi[i], -center[i])
            .max(add_up(center[i], -aabb.lo[i]))
            .max(0.)
    });
    let tile = TileBox::offset(center, half).map_err(|e| state(e.to_string()))?;
    let extent = Aabb {
        lo: std::array::from_fn(|i| add_down(center[i], -half[i])),
        hi: std::array::from_fn(|i| add_up(center[i], half[i])),
    };
    if (0..3).any(|i| extent.lo[i] > aabb.lo[i] || extent.hi[i] < aabb.hi[i]) {
        return Err(state("a tile box does not enclose its bounds"));
    }
    Ok((tile, extent))
}

/// A link to a written external document; `None` while planning.
pub(crate) type Linked = Option<crate::implicit::DocumentLink>;

/// A closed tile.
pub(crate) struct Node {
    pub first: u64,
    pub depth: u32,
    pub slot: u8,
    pub axes: [u32; 3],
    pub tile: TileBox,
    pub extent: Aabb,
    pub error: f64,
    pub height: u32,
    pub leaf: Option<Box<LeafRow>>,
    /// The H17 coarse tile of an internal tile.
    pub proxy: Option<Box<ProxyRow>>,
    /// Inline children in slot order (empty for leaves and links).
    pub children: Vec<Node>,
    /// Set when this skeleton tile was written as an external document.
    pub link: Option<Linked>,
    pub objects: u64,
    pub records: u64,
    pub levels: u32,
}

impl Node {
    pub(crate) fn is_skeleton(&self) -> bool {
        self.height >= SKELETON_HEIGHT
    }
}

/// Where closed fragments go: counted while planning, written while publishing.
pub(crate) trait Sink {
    /// Write `node` (a skeleton tile with inline children) as an external document.
    fn external(&mut self, node: Node) -> Result<Linked, JobError>;
    /// Write `tileset.json` around the root.
    fn root(&mut self, node: Node) -> Result<(), JobError>;
}

/// Members one document adds: explicit payloads, implicit payloads and subtrees in it.
pub(crate) fn document_members(node: &Node) -> u64 {
    fn implicit_leaves(node: &Node) -> u64 {
        if node.leaf.is_some() {
            1
        } else {
            u64::from(node.proxy.is_some()) + node.children.iter().map(implicit_leaves).sum::<u64>()
        }
    }
    fn walk(node: &Node) -> u64 {
        if node.link.is_some() {
            0
        } else if node.leaf.is_some() {
            1
        } else if !node.is_skeleton() {
            implicit_leaves(node) + 1
        } else {
            u64::from(node.proxy.is_some()) + node.children.iter().map(walk).sum::<u64>()
        }
    }
    walk(node)
}

/// Counts of the published forest.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Shape {
    pub leaves: u64,
    pub tiles: u64,
    pub explicit_tiles: u64,
    pub implicit_roots: u64,
    pub implicit_tiles: u64,
    pub documents: u64,
    pub max_depth: u32,
    pub synthetic_leaves: u64,
    pub packed_leaves: u64,
    /// Internal tiles published with an H17 coarse tile.
    pub proxies: u64,
    /// The largest number of tile records held at once by the open stack and its inline
    /// fragments (the planner's resident hierarchy state).
    pub peak_held_records: u64,
}

struct Frame {
    first: u64,
    depth: u32,
    slot: u8,
    axes: [u32; 3],
    children: Vec<Node>,
    /// The least slot the next child may take (a packed leaf covers up to its last octant).
    next_slot: u8,
    held: u64,
}

impl Frame {
    fn new(first: u64, depth: u32, slot: u8, axes: [u32; 3]) -> Self {
        Self {
            first,
            depth,
            slot,
            axes,
            children: Vec::new(),
            next_slot: 0,
            held: 0,
        }
    }
}

/// The streaming builder. Feed leaves in row order, then [`Builder::finish`].
/// H17 proxy rows in postorder, with their decoded bounds.
pub(crate) type Proxies<'p> = Box<dyn Iterator<Item = Result<(ProxyRow, Aabb), JobError>> + 'p>;

pub(crate) struct Builder<'s, S: Sink> {
    sink: &'s mut S,
    proxies: Option<Proxies<'s>>,
    limits: Limits,
    stack: Vec<Frame>,
    next_rank: u64,
    held: u64,
    pub shape: Shape,
    finished_root: bool,
}

fn child_axes(axes: [u32; 3], slot: u8) -> [u32; 3] {
    std::array::from_fn(|a| (axes[a] << 1) | u32::from((slot >> a) & 1))
}

impl<'s, S: Sink> Builder<'s, S> {
    pub(crate) fn new(sink: &'s mut S, limits: Limits) -> Self {
        Self::with_proxies(sink, limits, None)
    }

    /// A builder that gives every internal tile the next H17 proxy row (postorder).
    pub(crate) fn with_proxies(sink: &'s mut S, limits: Limits, proxies: Option<Proxies<'s>>) -> Self {
        Self {
            sink,
            proxies,
            limits,
            stack: Vec::new(),
            next_rank: 0,
            held: 0,
            shape: Shape::default(),
            finished_root: false,
        }
    }

    pub(crate) fn sink(&mut self) -> &mut S {
        self.sink
    }

    pub(crate) fn push(&mut self, row: LeafRow, bounds: Aabb) -> Result<(), JobError> {
        let bad = |why: &str| {
            JobError::new(
                JobErrorKind::InvalidInput,
                format!("leaf {} (rank {}): {why}", row.uri, row.first),
            )
        };
        if self.finished_root || row.first != self.next_rank {
            return Err(bad("rank ranges are not contiguous from 0 in row order"));
        }
        let depth = row.depth();
        if self.stack.is_empty() {
            if self.shape.leaves > 0 {
                return Err(bad("a leaf follows a root leaf"));
            }
            if depth == 0 {
                // The whole tree is one leaf.
                let node = self.leaf(row, bounds, 0, [0; 3])?;
                self.finished_root = true;
                self.shape.explicit_tiles += 1;
                self.shape.documents += 1;
                return self.sink.root(node);
            }
            self.stack.push(Frame::new(row.first, 0, 0, [0; 3]));
        } else {
            if depth == 0 {
                return Err(bad("a root leaf follows other leaves"));
            }
            // The open path is stack[0..]; frame k (k >= 1) sits at digit k - 1.
            let mut common = 1;
            while common < self.stack.len()
                && common < depth as usize
                && self.stack[common].slot == row.digits.get(common as u32 - 1)
            {
                common += 1;
            }
            while self.stack.len() > common {
                self.close_top()?;
            }
        }
        while (self.stack.len() as u32) < depth {
            let parent = self.stack.last().expect("root frame is open");
            let slot = row.digits.get(parent.depth);
            if slot < parent.next_slot {
                return Err(bad("tiles are not in H1 preorder (octant order)"));
            }
            let axes = child_axes(parent.axes, slot);
            let frame = Frame::new(row.first, parent.depth + 1, slot, axes);
            self.stack.last_mut().unwrap().next_slot = slot + 1;
            self.stack.push(frame);
        }
        let parent = self.stack.last().unwrap();
        let slot = row.digits.get(depth - 1);
        if slot < parent.next_slot {
            return Err(bad("tiles are not in H1 preorder (octant order)"));
        }
        let axes = child_axes(parent.axes, slot);
        let last = row.last_octant;
        let node = self.leaf(row, bounds, slot, axes)?;
        let parent = self.stack.last_mut().unwrap();
        parent.next_slot = last + 1;
        self.attach(node);
        Ok(())
    }

    fn leaf(
        &mut self,
        row: LeafRow,
        bounds: Aabb,
        slot: u8,
        axes: [u32; 3],
    ) -> Result<Node, JobError> {
        let (tile, extent) = tile_box(bounds)?;
        self.next_rank = row.end;
        self.shape.leaves += 1;
        self.shape.tiles += 1;
        self.shape.max_depth = self.shape.max_depth.max(row.depth());
        self.shape.synthetic_leaves += u64::from(row.synthetic > 0);
        self.shape.packed_leaves += u64::from(row.packed);
        Ok(Node {
            first: row.first,
            depth: row.depth(),
            slot,
            axes,
            tile,
            extent,
            error: row.geometry_error,
            height: 0,
            leaf: Some(Box::new(row)),
            proxy: None,
            children: Vec::new(),
            link: None,
            objects: 1,
            records: 1,
            levels: 1,
        })
    }

    fn attach(&mut self, node: Node) {
        let records = node.records;
        let frame = self.stack.last_mut().expect("a parent frame is open");
        frame.held += records;
        frame.children.push(node);
        self.held += records;
        self.shape.peak_held_records = self.shape.peak_held_records.max(self.held);
    }

    /// Close the top frame into a node: its role, box, error and partition.
    fn close_top(&mut self) -> Result<(), JobError> {
        let frame = self.stack.pop().expect("a frame is open");
        self.held -= frame.held;
        if frame.children.is_empty() {
            return Err(state("an internal tile closed without children"));
        }
        let mut extent = frame
            .children
            .iter()
            .map(|c| c.extent)
            .reduce(Aabb::union)
            .unwrap();
        let children_error = frame.children.iter().map(|c| c.error).fold(0., f64::max);
        let proxy = match self.proxies.as_mut() {
            None => None,
            Some(rows) => {
                let bad = |why: String| JobError::new(JobErrorKind::InvalidInput, why);
                let (row, bounds) = rows.next().transpose()?.ok_or_else(|| {
                    bad(format!(
                        "proxies.jsonl ends before internal tile f{}-d{}",
                        frame.first, frame.depth
                    ))
                })?;
                if (row.first, row.end, row.depth) != (frame.first, self.next_rank, frame.depth) {
                    return Err(bad(format!(
                        "proxies.jsonl row {} is not internal tile f{}-d{} (postorder)",
                        row.uri, frame.first, frame.depth
                    )));
                }
                if row.geometric_error < children_error {
                    return Err(bad(format!(
                        "proxy {} has a geometric error below its children's",
                        row.uri
                    )));
                }
                // The proxy's snapped vertices may leave the children's boxes.
                extent = extent.union(bounds);
                self.shape.proxies += 1;
                Some(Box::new(row))
            }
        };
        let (tile, extent) = tile_box(extent)?;
        let height = 1 + frame.children.iter().map(|c| c.height).max().unwrap();
        let diagonal = tile.diagonal();
        let error = match &proxy {
            Some(row) => row.geometric_error,
            None => children_error.max(EMPTY_PARENT_SCALE * diagonal),
        };
        if !error.is_finite() {
            return Err(state("an empty parent's geometric error is not finite"));
        }
        self.shape.tiles += 1;
        let skeleton = height >= SKELETON_HEIGHT;
        let (objects, records, levels) = if skeleton {
            for child in &frame.children {
                if child.leaf.is_none() && !child.is_skeleton() {
                    self.shape.implicit_roots += 1;
                    self.shape.implicit_tiles += child.records;
                } else if child.leaf.is_some() {
                    self.shape.explicit_tiles += 1;
                }
            }
            self.shape.explicit_tiles += 1;
            (
                1 + frame.children.iter().map(|c| c.objects).sum::<u64>(),
                1 + frame.children.iter().map(|c| c.records).sum::<u64>(),
                1 + frame.children.iter().map(|c| c.levels).max().unwrap(),
            )
        } else {
            (
                1,
                1 + frame.children.iter().map(|c| c.records).sum::<u64>(),
                1,
            )
        };
        let mut node = Node {
            first: frame.first,
            depth: frame.depth,
            slot: frame.slot,
            axes: frame.axes,
            tile,
            extent,
            error,
            height,
            leaf: None,
            proxy,
            children: frame.children,
            link: None,
            objects,
            records,
            levels,
        };
        if frame.depth == 0 {
            if !skeleton {
                self.shape.implicit_roots += 1;
                self.shape.implicit_tiles += node.records;
            }
            self.finished_root = true;
            self.shape.documents += 1;
            return self.sink.root(node);
        }
        if skeleton
            && (objects > self.limits.objects
                || records > self.limits.records
                || levels >= self.limits.levels)
        {
            self.shape.documents += 1;
            let (first, depth, slot, axes, tile, extent, error, height) = (
                node.first,
                node.depth,
                node.slot,
                node.axes,
                node.tile,
                node.extent,
                node.error,
                node.height,
            );
            let link = self.sink.external(node)?;
            node = Node {
                first,
                depth,
                slot,
                axes,
                tile,
                extent,
                error,
                height,
                leaf: None,
                // The proxy is the external document's root content, not the link's.
                proxy: None,
                children: Vec::new(),
                link: Some(link),
                objects: 1,
                records: 0,
                levels: 1,
            };
        }
        self.attach(node);
        Ok(())
    }

    /// Close every open tile; the root is written last.
    pub(crate) fn finish(mut self) -> Result<Shape, JobError> {
        while !self.stack.is_empty() {
            self.close_top()?;
        }
        if !self.finished_root {
            return Err(JobError::new(
                JobErrorKind::InvalidInput,
                "the leaf directory has no leaf rows",
            ));
        }
        if let Some(rows) = self.proxies.as_mut() {
            if rows.next().is_some() {
                return Err(JobError::new(
                    JobErrorKind::InvalidInput,
                    "proxies.jsonl has rows beyond the internal tiles",
                ));
            }
        }
        Ok(self.shape)
    }
}
