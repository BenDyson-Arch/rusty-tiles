//! The streamed hierarchy: H8's leaf rows, in rank order, rebuilt into H1's tile tree with a
//! stack of at most 32 open tiles, partitioned into documents bottom-up as tiles close.
//!
//! Every proper prefix of a leaf address is an H1 internal tile (the cell tree opens a cell
//! at every depth on the way down, and a run hierarchy a node at every run length), and a
//! tile's rank range is the union of its leaves', so the leaf rows alone determine the tree.
//! A tile closes when the next leaf leaves its subtree. A closed tile's role follows its
//! height (P-IMPL §4, S = 4): height ≥ 4 is an explicit skeleton tile and its leaf children
//! are explicit leaves. Its other children (height 1 to 3) are shallow trees, written as
//! explicit tiles while their document has room and otherwise as implicit roots (one subtree
//! each); a whole tree of height below 4 is the same. When a skeleton tile closes over a
//! [`Limits`] cap its fragment is reduced (`Builder::fit`): the largest shallow trees become
//! implicit roots, then its largest skeleton children become external documents, then, if
//! records alone still exceed their cap, the tile itself does. Every fragment held after a
//! close is within the caps, so resident state is the open stack plus such fragments,
//! whatever the input size. See `docs/architecture/implicit-forest-consolidation.md`.
use super::source::{add_down, add_up, Aabb, LeafRow, ProxyRow};
use crate::{tileset_node::TileBox, JobError, JobErrorKind};

/// The fixed partition: caps on one fragment, and therefore on every document.
///
/// A held fragment is within all three caps. A skeleton tile whose records alone stay over
/// their cap once no skeleton child is left to move is written as an external document at
/// once; that document holds at most `1 + 8 · 585` records (eight implicit roots of four
/// levels) and is still within `objects` and `levels`. The root `tileset.json` never splits,
/// so it can hold up to 1 + 8 · 585 = 4,681 records, past the 4,096 records cap.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Tile objects (explicit tiles, implicit roots, links) of a fragment. A tile object is
    /// at most 26 JSON value nodes, so 480 objects stay within the writer's 16,384.
    pub objects: u64,
    /// Records (tiles, including those inside implicit roots) of a fragment: the planner's
    /// resident state, not a JSON cost.
    pub records: u64,
    /// Tile levels of a fragment. A tile at level k sits at JSON depth 1 + 2k and its
    /// deepest member 3 below, so 12 levels reach depth 26, within the writer's 32.
    pub levels: u32,
}

pub(crate) const LIMITS: Limits = Limits {
    objects: 480,
    records: 4096,
    levels: 12,
};

/// Declared resident bound of the planner's held records: the open stack of at most 32
/// tiles, each holding at most 8 closed child fragments within `LIMITS.records`, plus the
/// root fragment's exception (`1 + 8 · 585`); one record is a `Node` with its boxed row and
/// an allowance for the strings and child slot it owns.
pub(crate) fn held_bytes_bound() -> u64 {
    let records = 32 * 8 * LIMITS.records + 1 + 8 * 585;
    let per = std::mem::size_of::<Node>()
        + std::mem::size_of::<LeafRow>().max(std::mem::size_of::<ProxyRow>())
        + 128;
    records * per as u64
}

/// Below this height a non-leaf tile is (inside) an implicit root (P-IMPL `S`).
pub(crate) const SKELETON_HEIGHT: u32 = 4;

/// An empty parent's geometric error is this multiple of its box diagonal: until its whole
/// box projects below about one pixel at CesiumJS's default maximum screen-space error of
/// 16, the viewer refines it towards the leaves that hold the geometry.
pub(crate) const EMPTY_PARENT_SCALE: f64 = 16.;

pub(crate) fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// Smallest written half length (metres). A flat tile (one planar triangle) would
/// otherwise have a zero axis, which the spec allows but clients that invert the box's
/// half axes (cesium-native) cannot use.
pub(crate) const MIN_HALF_LENGTH: f64 = 1e-4;

/// The box written for `aabb`: centre and outward half lengths (at least
/// [`MIN_HALF_LENGTH`]), and the closed extent of that box (rounded outward), which is
/// what its parent encloses.
pub(crate) fn tile_box(aabb: Aabb) -> Result<(TileBox, Aabb), JobError> {
    let center: [f64; 3] = std::array::from_fn(|i| aabb.lo[i] * 0.5 + aabb.hi[i] * 0.5);
    let half: [f64; 3] = std::array::from_fn(|i| {
        add_up(aabb.hi[i], -center[i])
            .max(add_up(center[i], -aabb.lo[i]))
            .max(MIN_HALF_LENGTH)
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
    /// A shallow tree (an internal tile of height below 4 whose parent is a skeleton tile, or
    /// the root) published as an implicit root rather than as explicit tiles.
    pub implicit: bool,
    pub leaf: Option<Box<LeafRow>>,
    /// The HLOD coarse tile of an internal tile.
    pub proxy: Option<Box<ProxyRow>>,
    /// An HLOD single-child chain cell below its chain's coarse tile (this row) has no
    /// content of its own. Cesium's empty traversal would let the coarse tile stop drawing
    /// on reaching it, before the tiles that replace it are loaded, so it is never published
    /// empty: explicit output splices it out (its child replaces it), and where the layout
    /// cannot skip a cell (an implicit tree, a document's root) it repeats the coarse tile.
    pub chain: Option<Box<ProxyRow>>,
    /// An HLOD cell whose coarse tile was omitted: it, or a descendant, could not keep its
    /// exact interface within the cap, and omission propagates to the root. It publishes no
    /// content, and every ancestor is omitted too, so it never sits between content levels
    /// (no coarse tile above it can stop drawing on reaching it); it is plain availability,
    /// explicit or implicit, and may be a document's root. Checked: a coarse tile above an
    /// omitted cell is refused.
    pub omitted: bool,
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

    /// An implicit root: a shallow tree demoted from explicit tiles.
    pub(crate) fn is_implicit_root(&self) -> bool {
        self.implicit && self.leaf.is_none() && !self.is_skeleton()
    }

    /// A chain cell (see `chain`): spliced out of explicit output.
    pub(crate) fn is_chain_cell(&self) -> bool {
        self.chain.is_some() && self.proxy.is_none() && self.leaf.is_none() && self.link.is_none()
    }

    /// A skeleton tile whose fragment is still inline (not a link).
    fn is_inline_skeleton(&self) -> bool {
        self.is_skeleton() && self.link.is_none()
    }

    /// Recompute a tile's fragment totals from its children (explicit tiles throughout).
    fn aggregate(&mut self) {
        self.objects = 1 + self.children.iter().map(|c| c.objects).sum::<u64>();
        self.records = 1 + self.children.iter().map(|c| c.records).sum::<u64>();
        self.levels = 1 + self.children.iter().map(|c| c.levels).max().unwrap_or(0);
    }

    /// The path (child indices) to the explicit shallow tree of this fragment to demote
    /// first: most tiles, then greatest height, then lowest rank.
    fn largest_inline_tree(&self) -> Option<Vec<usize>> {
        type Best = Option<((u64, u32, std::cmp::Reverse<u64>), Vec<usize>)>;
        fn visit(node: &Node, path: &mut Vec<usize>, best: &mut Best) {
            for (i, child) in node.children.iter().enumerate() {
                path.push(i);
                if child.is_inline_skeleton() {
                    visit(child, path, best);
                } else if child.leaf.is_none() && child.link.is_none() && !child.implicit {
                    let key = (child.records, child.height, std::cmp::Reverse(child.first));
                    if best.as_ref().is_none_or(|(k, _)| key > *k) {
                        *best = Some((key, path.clone()));
                    }
                }
                path.pop();
            }
        }
        let mut best = None;
        visit(self, &mut Vec::new(), &mut best);
        best.map(|(_, path)| path)
    }

    /// Demote the shallow tree at `path` to an implicit root and update the totals above it.
    fn demote(&mut self, path: &[usize]) {
        let child = &mut self.children[path[0]];
        if let [_] = path {
            child.implicit = true;
            child.objects = 1;
            child.levels = 1;
        } else {
            child.demote(&path[1..]);
        }
        self.aggregate();
    }

    fn over(&self, limits: &Limits) -> bool {
        self.objects > limits.objects
            || self.records > limits.records
            || self.levels > limits.levels
    }
}

/// Add the published roles of one document's fragment to `shape`.
fn tally(shape: &mut Shape, node: &Node) {
    fn visit(shape: &mut Shape, node: &Node, root: bool) {
        if node.link.is_some() {
            return;
        }
        if node.is_implicit_root() {
            shape.implicit_roots += 1;
            shape.implicit_tiles += node.records;
            return;
        }
        if node.is_chain_cell() && !root {
            // Spliced out: its child takes its place.
            shape.tiles -= 1;
            shape.spliced_chain_cells += 1;
        } else {
            shape.explicit_tiles += 1;
        }
        for child in &node.children {
            visit(shape, child, false);
        }
    }
    visit(shape, node, true);
}

/// Where closed fragments go: counted while planning, written while publishing.
pub(crate) trait Sink {
    /// Write `node` (a skeleton tile with inline children) as an external document.
    fn external(&mut self, node: Node) -> Result<Linked, JobError>;
    /// Write `tileset.json` around the root.
    fn root(&mut self, node: Node) -> Result<(), JobError>;
}

/// Members one document adds: explicit payloads, implicit payloads and subtrees in it. A
/// chain cell repeats its coarse tile in an implicit tree and as a document's root, and is
/// spliced out (no payload) elsewhere.
pub(crate) fn document_members(node: &Node) -> u64 {
    fn payloads(node: &Node, chains: bool) -> u64 {
        if node.leaf.is_some() {
            1
        } else {
            u64::from(node.proxy.is_some() || (chains && node.chain.is_some()))
                + node
                    .children
                    .iter()
                    .map(|c| payloads(c, chains))
                    .sum::<u64>()
        }
    }
    fn walk(node: &Node, root: bool) -> u64 {
        if node.link.is_some() {
            0
        } else if node.leaf.is_some() {
            1
        } else if node.is_implicit_root() {
            // Its payloads and its subtree.
            payloads(node, true) + 1
        } else if !node.is_skeleton() {
            // A shallow tree written as explicit tiles: its payloads only.
            u64::from(root && node.is_chain_cell()) + payloads(node, false)
        } else {
            u64::from(node.proxy.is_some() || (root && node.is_chain_cell()))
                + node.children.iter().map(|c| walk(c, false)).sum::<u64>()
        }
    }
    walk(node, true)
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
    /// Internal tiles published with an HLOD coarse tile.
    pub proxies: u64,
    /// HLOD chain cells spliced out of explicit output (not in `tiles`).
    pub spliced_chain_cells: u64,
    /// HLOD cells whose coarse tile was omitted (content-less, every ancestor too).
    pub omitted_cells: u64,
    /// Logical omitted cells, including pass-through cells, by ownership depth.
    pub omitted_by_depth: Vec<u64>,
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
/// HLOD coarse rows in postorder, with their decoded bounds.
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
    /// The next proxy row, read but not yet matched.
    lookahead: Option<(ProxyRow, Aabb)>,
}

fn child_axes(axes: [u32; 3], slot: u8) -> [u32; 3] {
    std::array::from_fn(|a| (axes[a] << 1) | u32::from((slot >> a) & 1))
}

impl<'s, S: Sink> Builder<'s, S> {
    pub(crate) fn new(sink: &'s mut S, limits: Limits) -> Self {
        Self::with_proxies(sink, limits, None)
    }

    /// A builder that matches the HLOD coarse rows (postorder) to internal tiles. Rows may
    /// skip internal tiles: single-child pass-through cells and omitted cells publish no
    /// content.
    pub(crate) fn with_proxies(
        sink: &'s mut S,
        limits: Limits,
        proxies: Option<Proxies<'s>>,
    ) -> Self {
        Self {
            sink,
            proxies,
            limits,
            stack: Vec::new(),
            next_rank: 0,
            held: 0,
            shape: Shape::default(),
            finished_root: false,
            lookahead: None,
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
            implicit: false,
            leaf: Some(Box::new(row)),
            proxy: None,
            chain: None,
            omitted: false,
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
        // An HLOD cell without content sits below its single-child chain's coarse tile.
        let mut chain_error = None;
        let mut chain = None;
        let mut omitted = false;
        let proxy = if let Some(rows) = self.proxies.as_mut() {
            let bad = |why: String| JobError::new(JobErrorKind::InvalidInput, why);
            let next = match self.lookahead.take() {
                Some(row) => Some(row),
                None => rows.next().transpose()?,
            };
            let key = (frame.first, self.next_rank, frame.depth);
            match next {
                Some((row, bounds)) if (row.first, row.end, row.depth) == key => {
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
                // A cell of a single-child chain publishes no content below the chain's
                // coarse tile, which is the next row (same rank range, shallower). It takes that tile's error, so refinement through it is
                // monotone and happens exactly where the coarse tile's own cell would refine.
                Some((row, bounds))
                    if (row.first, row.end) == (frame.first, self.next_rank)
                        && row.depth < frame.depth =>
                {
                    chain_error = Some(row.geometric_error);
                    chain = Some(Box::new(row.clone()));
                    // Where it cannot be spliced the cell repeats that payload, so its box
                    // holds the payload's bounds too (as a coarse tile's own box does).
                    extent = extent.union(bounds);
                    self.lookahead = Some((row, bounds));
                    None
                }
                // A cell whose coarse tile was omitted has no row (the next row is a later
                // tile's in postorder, or there is none).
                next => {
                    omitted = true;
                    self.shape.omitted_cells += 1;
                    let depth = frame.depth as usize;
                    self.shape
                        .omitted_by_depth
                        .resize(self.shape.omitted_by_depth.len().max(depth + 1), 0);
                    self.shape.omitted_by_depth[depth] += 1;
                    self.lookahead = next;
                    None
                }
            }
        } else {
            None
        };
        // Omission reaches the root: no coarse content above an omitted cell.
        if (proxy.is_some() || chain.is_some()) && frame.children.iter().any(|c| c.omitted) {
            return Err(JobError::new(
                JobErrorKind::InvalidInput,
                format!(
                    "coarse content of f{}-d{} sits above an omitted cell (omission must reach \
                     the root)",
                    frame.first, frame.depth
                ),
            ));
        }
        let (tile, extent) = tile_box(extent)?;
        let height = 1 + frame.children.iter().map(|c| c.height).max().unwrap();
        let diagonal = tile.diagonal();
        let error = match (&proxy, chain_error) {
            (Some(row), _) => row.geometric_error,
            (None, Some(error)) if error >= children_error => error,
            (None, Some(_)) => {
                return Err(JobError::new(
                    JobErrorKind::InvalidInput,
                    format!(
                        "coarse tile above f{}-d{} has a geometric error below its children's",
                        frame.first, frame.depth
                    ),
                ))
            }
            // Content-less (an empty parent, or an omitted HLOD cell): it must refine
            // whenever it is visible, or Cesium stops at it and draws nothing there (an empty
            // tile traverses only while its own screen-space error passes the maximum, and an
            // omitted cell has no content ancestor to fall back on). Written the same way
            // whether it ends up explicit, inside an implicit tree or as a document's root.
            (None, None) => children_error.max(EMPTY_PARENT_SCALE * diagonal),
        };
        if !error.is_finite() {
            return Err(state("an empty parent's geometric error is not finite"));
        }
        self.shape.tiles += 1;
        let skeleton = height >= SKELETON_HEIGHT;
        let mut node = Node {
            first: frame.first,
            depth: frame.depth,
            slot: frame.slot,
            axes: frame.axes,
            tile,
            extent,
            error,
            height,
            implicit: false,
            leaf: None,
            proxy,
            chain,
            omitted,
            children: frame.children,
            link: None,
            objects: 0,
            records: 0,
            levels: 0,
        };
        // Every tile starts explicit: a shallow tree's totals are its own explicit tiles.
        node.aggregate();
        if frame.depth == 0 {
            if skeleton {
                self.fit(&mut node)?;
            } else if node.objects > self.limits.objects || node.levels > self.limits.levels {
                node.implicit = true;
            }
            tally(&mut self.shape, &node);
            self.finished_root = true;
            self.shape.documents += 1;
            return self.sink.root(node);
        }
        if skeleton {
            self.fit(&mut node)?;
            if node.over(&self.limits) {
                node = self.externalize(node)?;
            }
        }
        self.attach(node);
        Ok(())
    }

    /// Reduce a closed skeleton tile's fragment towards the caps, one step at a time:
    ///
    /// 1. tile objects over: demote the largest explicit shallow tree anywhere in it to an
    ///    implicit root;
    /// 2. levels over: the child reaching deepest (lowest slot on ties) is demoted if it is a
    ///    shallow tree or moved into its own external document if it is a skeleton tile;
    /// 3. objects (with no shallow tree left) or records over: move the inline skeleton child
    ///    contributing most (lowest slot on ties) into its own external document.
    ///
    /// What is left over (records only, once no skeleton child remains inline) is the
    /// caller's.
    fn fit(&mut self, node: &mut Node) -> Result<(), JobError> {
        let limits = self.limits;
        while node.over(&limits) {
            if node.objects > limits.objects {
                if let Some(path) = node.largest_inline_tree() {
                    node.demote(&path);
                    continue;
                }
            }
            let largest = |measure: &dyn Fn(&Node) -> u64, movable: &dyn Fn(&Node) -> bool| {
                node.children
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| movable(c))
                    .max_by(|a, b| measure(a.1).cmp(&measure(b.1)).then(b.0.cmp(&a.0)))
                    .map(|(i, _)| i)
            };
            let index = if node.levels > limits.levels {
                let shallow = |c: &Node| c.leaf.is_none() && c.link.is_none() && !c.implicit;
                match largest(&|c| u64::from(c.levels), &shallow) {
                    Some(i) if !node.children[i].is_skeleton() => {
                        node.demote(&[i]);
                        continue;
                    }
                    found => found,
                }
            } else if node.objects > limits.objects {
                largest(&|c| c.objects, &Node::is_inline_skeleton)
            } else {
                largest(&|c| c.records, &Node::is_inline_skeleton)
            };
            let Some(index) = index else {
                break;
            };
            let child = node.children.remove(index);
            let link = self.externalize(child)?;
            node.children.insert(index, link);
            node.aggregate();
        }
        Ok(())
    }

    /// Write a skeleton tile's fragment as an external document; returns its link tile.
    fn externalize(&mut self, node: Node) -> Result<Node, JobError> {
        tally(&mut self.shape, &node);
        self.shape.documents += 1;
        // The link keeps its document root's omission, so the content-above-omitted rule
        // holds across documents too.
        let omitted = node.omitted;
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
        Ok(Node {
            first,
            depth,
            slot,
            axes,
            tile,
            extent,
            error,
            height,
            implicit: false,
            leaf: None,
            // The proxy is the external document's root content, not the link's.
            proxy: None,
            chain: None,
            omitted,
            children: Vec::new(),
            link: Some(link),
            objects: 1,
            records: 0,
            levels: 1,
        })
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
            if self.lookahead.is_some() || rows.next().transpose()?.is_some() {
                return Err(JobError::new(
                    JobErrorKind::InvalidInput,
                    "proxies.jsonl has rows beyond the internal tiles",
                ));
            }
        }
        Ok(self.shape)
    }
}

#[cfg(test)]
mod omission_tests {
    use super::*;
    use crate::mesh_large::delivery::source::Digits;

    #[derive(Default)]
    struct Collect {
        /// (objects, records, levels) of each document, the root last.
        docs: Vec<(u64, u64, u32)>,
        /// Omitted cells written (document roots included) and those whose error is below
        /// the empty-parent floor.
        omitted: u64,
        below_floor: u64,
    }

    impl Collect {
        fn errors(&mut self, node: &Node) {
            if node.omitted {
                self.omitted += 1;
                if node.error < EMPTY_PARENT_SCALE * node.tile.diagonal() {
                    self.below_floor += 1;
                }
            }
            for c in &node.children {
                if c.link.is_none() {
                    self.errors(c);
                }
            }
        }
    }

    impl Sink for Collect {
        fn external(&mut self, node: Node) -> Result<Linked, JobError> {
            self.docs.push((node.objects, node.records, node.levels));
            self.errors(&node);
            Ok(None)
        }
        fn root(&mut self, node: Node) -> Result<(), JobError> {
            self.docs.push((node.objects, node.records, node.levels));
            self.errors(&node);
            Ok(())
        }
    }

    /// A complete octree of `depth` levels of capped leaves, planned against `proxies`.
    fn plan(depth: u32, proxies: Vec<ProxyRow>) -> (Result<Shape, JobError>, Collect) {
        plan_with(depth, proxies, LIMITS)
    }

    fn plan_with(
        depth: u32,
        proxies: Vec<ProxyRow>,
        limits: Limits,
    ) -> (Result<Shape, JobError>, Collect) {
        let mut sink = Collect::default();
        let rows = proxies.into_iter().map(|r| {
            Ok((
                r,
                Aabb {
                    lo: [0.; 3],
                    hi: [1.; 3],
                },
            ))
        });
        let mut builder = Builder::with_proxies(&mut sink, limits, Some(Box::new(rows)));
        let leaves = 8u64.pow(depth);
        let result = (|| {
            for i in 0..leaves {
                let mut digits = Digits::ROOT;
                for k in (0..depth).rev() {
                    digits.push(((i >> (k * 3)) & 7) as u8);
                }
                let row = LeafRow {
                    uri: format!("leaves/{i}.glb"),
                    sha256: [0; 32],
                    bytes: 100,
                    first: i * 8192,
                    end: (i + 1) * 8192,
                    digits,
                    last_octant: (i & 7) as u8,
                    packed: false,
                    synthetic: 0,
                    geometry_error: 0.,
                };
                let bounds = Aabb {
                    lo: [0.; 3],
                    hi: [1.; 3],
                };
                builder.push(row, bounds)?;
            }
            builder.finish()
        })();
        (result, sink)
    }

    /// Sol round 8: a complete all-omitted octree (every internal cell content-less, as
    /// upward propagation leaves it when a leaf's parent cannot fit) is plain availability:
    /// the planner partitions it like any other, so every document stays within its caps,
    /// and nothing is cloned (each leaf is one payload).
    #[test]
    fn an_all_omitted_octree_keeps_documents_bounded() {
        for depth in [4u32, 5] {
            let (shape, sink) = plan(depth, Vec::new());
            let shape = shape.unwrap();
            let internal = (8u64.pow(depth) - 1) / 7;
            assert_eq!(shape.omitted_cells, internal, "{shape:?}");
            assert_eq!(shape.proxies, 0);
            assert!(
                shape.implicit_roots + shape.documents > 1,
                "partitioned: {shape:?}"
            );
            // Round 9: every omitted cell, document roots included, refines while visible.
            assert_eq!(sink.omitted, internal, "{shape:?}");
            assert_eq!(sink.below_floor, 0, "{shape:?}");
            // Objects (the JSON cost) and levels within the caps in every document; records
            // within the documented `1 + 8 · 585` exception (see `Limits`).
            for &(objects, records, levels) in &sink.docs {
                assert!(
                    objects <= LIMITS.objects && records <= 1 + 8 * 585 && levels <= LIMITS.levels,
                    "document ({objects}, {records}, {levels}) past {LIMITS:?}: {shape:?}"
                );
            }
        }
    }

    /// No content-less cell has a content ancestor: a coarse tile above an omitted cell is
    /// refused (omission must reach the root).
    #[test]
    fn a_coarse_tile_above_an_omitted_cell_is_refused() {
        let root = ProxyRow {
            uri: "proxies/f0-d0.glb".into(),
            sha256: [0; 32],
            bytes: 100,
            first: 0,
            end: 64 * 8192,
            depth: 0,
            geometric_error: 1e9,
            primitives: 1,
            sampling: (0, 0, crate::mesh_large::hlod::error::Schedule::PRODUCTION),
            encoded_depth: 0,
        };
        let (shape, _) = plan(2, vec![root]);
        let err = shape.unwrap_err();
        assert!(err.to_string().contains("above an omitted cell"), "{err}");
    }

    /// Round 9: the rule holds across documents. Tight caps write omitted skeleton cells
    /// below the root as external documents; a coarse root above those links is refused (a
    /// link keeps its document root's omission).
    #[test]
    fn a_coarse_tile_above_an_omitted_document_root_is_refused() {
        let depth = 5;
        let root = ProxyRow {
            uri: "proxies/f0-d0.glb".into(),
            sha256: [0; 32],
            bytes: 100,
            first: 0,
            end: 8u64.pow(depth) * 8192,
            depth: 0,
            geometric_error: 1e9,
            primitives: 1,
            sampling: (0, 0, crate::mesh_large::hlod::error::Schedule::PRODUCTION),
            encoded_depth: 0,
        };
        let tight = Limits {
            objects: 4,
            records: 24,
            levels: 3,
        };
        let (shape, sink) = plan_with(depth, vec![root], tight);
        assert!(!sink.docs.is_empty(), "external documents were written");
        let err = shape.unwrap_err();
        assert!(err.to_string().contains("above an omitted cell"), "{err}");
    }
}
