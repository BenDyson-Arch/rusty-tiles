//! `--weld-epsilon` / `--join-primitives same-material`: an opt-in, reported weld of the
//! source's vertex positions before capture's assembly (#349).
//!
//! **What is welded.** A source vertex is one expanded primitive instance's POSITION element
//! that a captured triangle uses (a kept triangle in a subset bake). Its position is the one
//! capture decodes: after node transforms and any source CRS mapping, in local metres, as
//! canonical f32 (`capture::canonical`). Vertices nothing captured uses take no part.
//!
//! **Pairs.** Two vertices pair when they are in one scope and each coordinate differs by at
//! most ε and their Euclidean distance is at most ε, computed in f64 from the f32
//! coordinates (`within`). At ε = 0 that is bit-equal positions. The default scope is one
//! primitive instance; `--join-primitives same-material` widens it to every primitive
//! instance with the same glTF material index (no material is one scope of its own).
//!
//! **Classes and representatives.** The classes are the connected components of the pair
//! graph (union-find over every pair found), so they do not depend on the order pairs are
//! found in, on `--jobs` or on the budget. A class's representative is its lowest global
//! vertex index (primitive instances in scene order, then POSITION index), and every member
//! takes the representative's position. A chain of pairs can move a vertex by more than ε:
//! the report gives the true largest displacement, not ε.
//!
//! **Attributes.** Only positions move. Normals, tangents, UVs and colours stay each vertex's
//! own, so a UV or normal seam stays a seam in attribute space while its geometry closes; no
//! texture coordinate is averaged or taken from another vertex, and no texture is corrupted.
//! Provenance (`source_primitive`, `source_triangle`, the authored vertex indices) is kept
//! per triangle. A triangle whose corners land on fewer than three positions becomes
//! degenerate, as the capture already defines it.
//!
//! **Finding pairs at scale.** Each vertex is a record in a column: its scope and its cell
//! of side `c = max(4ε, 2^-20 m)` in y and z (at ε = 0, its exact y and z), hashed to 64 bits.
//! A vertex is also listed in the lower neighbour cell of each of y and z that its ε-range
//! reaches (at most four records). One external sort by (column, x) and one sweep with a
//! window of the records within ε in x then meet every pair: of two vertices within ε, the
//! one with the lower cell on an axis lists it, and the other's own cell on that axis is it.
//! A hash collision only adds comparisons. An exact repeat of a window record joins it
//! without entering the window, so repeats cost one comparison each. The window
//! ([`WINDOW_RECORDS`]) and the comparisons per record on average ([`MAX_COMPARISONS`])
//! are capped, so the sweep is linear; a source denser than that within ε is refused as
//! `resource_limit`.
//!
//! **Resources.** One `u32` per expanded source vertex (the union-find), a bitmap of paired
//! vertices, one external sort at a time and the window, all declared in capture's resident
//! bound ([`MeshWeld::memory_bytes`]). Scratch: the column sort, the vertices' positions,
//! the class sort and the move list ([`scratch_bytes`]); the sorted move list (one record per
//! merged vertex) stays until the assembly has decoded every vertex. Progress is capture's
//! `capture_weld` step; cancellation is checked throughout.
//!
//! **Honest error.** Store each moved vertex's original mapped XYZ in expanded vertex
//! order. Its bit rank gives a positioned read into this sparse file. The leaf writer
//! pairs originals with final decoded corners. The direct coarse certificate reads
//! original faces for the welded tree's rank range, including collapsed slivers.
//! The weld audit independently describes the topology change and its reach.
use super::{
    assembly::decode_chunks,
    capture::{canonical, source_position, vertices},
    input::StreamSource,
    phases::{Step, StepProgress},
    source_crs::SourceFrame,
    subset::{Ranks, Selection},
    tasks::{run, Ran},
};
use crate::{
    gltf_source::source::ScenePrimitive,
    spill::{
        ByteFile, ByteWriter, ExternalSorter, Record, Scratch, Sequence, SequenceReader,
        SequenceWriter, SortConfig,
    },
    JobError, JobErrorKind,
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
};

type Result<T> = std::result::Result<T, JobError>;

fn request(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidRequest, message)
}
fn resource(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message)
}
fn invariant(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.to_owned())
}
fn stopped() -> JobError {
    JobError::new(JobErrorKind::Cancelled, "weld stopped")
}

/// Vertices one decoding task covers.
const WELD_VERTICES: usize = 1 << 14;
/// Triangles one marking task covers.
const WELD_TRIANGLES: usize = 1 << 16;
/// Records the sweep's window may hold: distinct positions of one column within ε of each
/// other in x.
pub(crate) const WINDOW_RECORDS: usize = 1 << 16;
/// Comparisons per record the sweep allows on average, beyond [`COMPARISON_SLACK`].
pub(crate) const MAX_COMPARISONS: u64 = 1024;
const COMPARISON_SLACK: u64 = 1 << 26;
/// The sweep checks for a stop at least this often, in comparisons.
const CHECK_COMPARISONS: u64 = 1 << 16;
/// Records per vertex at most: its own cell and the lower neighbour in y and in z.
const COPIES: u64 = 4;
/// The smallest cell side, so cell indices of any local coordinate fit an `i64`.
const MIN_CELL: f64 = 1. / (1u64 << 20) as f64;
/// The move reader's buffer in each decoding task.
pub(crate) const MOVE_BUFFER: usize = 64 << 10;
/// The home-record flag in a point's id.
const HOME: u32 = 1 << 31;

/// An opt-in weld of source vertex positions; see the module notes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshWeld {
    epsilon: f64,
    same_material: bool,
}

impl MeshWeld {
    /// The shared adapter grammar: `epsilon` in metres (finite, at least 0), and
    /// `join_primitives` either absent or `same-material`. `join_primitives` alone welds at
    /// ε = 0. Neither returns `None`: no weld.
    pub fn from_parameters(
        epsilon: Option<f64>,
        join_primitives: Option<&str>,
    ) -> Result<Option<Self>> {
        let same_material = match join_primitives {
            None => false,
            Some("same-material") => true,
            Some(other) => {
                return Err(request(format!(
                    "join_primitives takes `same-material`, not `{other}`"
                )))
            }
        };
        if epsilon.is_none() && !same_material {
            return Ok(None);
        }
        let weld = Self::new(epsilon.unwrap_or(0.))?;
        Ok(Some(if same_material {
            weld.joining_same_material()
        } else {
            weld
        }))
    }

    /// A weld within `epsilon` metres (finite, at least 0) of each primitive instance's own
    /// vertices.
    pub fn new(epsilon: f64) -> Result<Self> {
        if !epsilon.is_finite() || epsilon < 0. {
            return Err(request(
                "weld epsilon must be a finite number of metres, at least 0",
            ));
        }
        Ok(Self {
            // -0 is 0.
            epsilon: epsilon + 0.,
            same_material: false,
        })
    }

    /// Also weld across primitive instances with the same material.
    pub fn joining_same_material(mut self) -> Self {
        self.same_material = true;
        self
    }

    pub fn epsilon(&self) -> f64 {
        self.epsilon
    }

    pub fn joins_same_material(&self) -> bool {
        self.same_material
    }

    fn cell(&self) -> f64 {
        (4. * self.epsilon).max(MIN_CELL)
    }

    /// The weld's resident bytes beside capture's other terms, for `vertices` expanded source
    /// vertices, `workers` tasks and `sorter` (one external sort at a time): the union-find,
    /// the paired and used bitmaps, the sort, the window, a decoding wave and, during the
    /// assembly, each task's move reader.
    pub(crate) fn memory_bytes(vertices: u64, workers: usize, sorter: SortConfig) -> Option<u64> {
        let w = workers.max(1) as u64;
        let bitmap = vertices.div_ceil(64).checked_mul(8)?;
        let sort = [
            sorter.memory_bound::<Point>(),
            sorter.memory_bound::<Class>(),
            sorter.memory_bound::<Move>(),
            sorter.memory_bound::<Face>(),
        ]
        .into_iter()
        .max()?;
        let window = (WINDOW_RECORDS as u64).checked_mul(size_of::<Point>() as u64)?;
        // Two chunks per worker in flight, each at most `COPIES` records per vertex.
        let wave = (2 * w)
            .checked_mul(WELD_VERTICES as u64 * COPIES)?
            .checked_mul(size_of::<Point>() as u64)?;
        let readers = w
            .checked_mul(MOVE_BUFFER as u64 + 4096)?
            .checked_add(sorter.buffer_bytes as u64)?;
        vertices
            .checked_mul(4)?
            .checked_add(bitmap.checked_mul(2)?)?
            .checked_add(sort)?
            .checked_add(window.max(audit_memory_bytes()))?
            .checked_add(wave)?
            .checked_add(readers)
    }
}

/// The weld's peak scratch for `vertices` used source vertices: the column sort (two run
/// generations of up to [`COPIES`] records per vertex) beside the positions written during
/// the sweep, or later the positions, the class sort and the move lists.
pub(crate) fn scratch_bytes(vertices: u64) -> Option<u64> {
    let points = vertices
        .checked_mul(COPIES)?
        .checked_mul(2 * Point::SIZE as u64)?;
    let located = vertices.checked_mul(Located::SIZE as u64)?;
    let classes = vertices.checked_mul(2 * Class::SIZE as u64)?;
    let moves = vertices.checked_mul(Move::SIZE as u64)?;
    let later = located
        .checked_add(classes)?
        .max(classes.checked_add(moves)?)
        .max(moves.checked_mul(3)?)
        .max(moves.checked_mul(2)?.checked_add(located)?);
    points.checked_add(located)?.max(later).checked_add(located)
}

/// The sorted move list the assembly reads: one record per merged vertex.
pub(crate) fn moves_bytes(vertices: u64) -> Option<u64> {
    vertices.checked_mul(Located::SIZE as u64)
}

/// The published weld statement (`tilerReport.weld`, `inspectReport.weld`).
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct MeshWeldReport {
    /// The tolerance ε, in metres.
    pub epsilon_m: f64,
    /// `none` (each primitive instance welds its own vertices) or `same-material`.
    pub join_primitives: String,
    /// Source vertices the captured triangles use: those the weld considered.
    pub vertices: u64,
    /// Vertices merged into another (a class's members other than its representative).
    pub merged_vertices: u64,
    /// Of those, the ones whose position changed (not already bit-equal to the
    /// representative's).
    pub moved_vertices: u64,
    /// Classes of two or more vertices, and the largest.
    pub classes: u64,
    pub largest_class: u64,
    /// The true largest displacement of a merged vertex, in metres (may exceed ε along a
    /// chain).
    pub max_displacement_m: f64,
    /// Captured triangles with fewer than three distinct corner positions after the weld,
    /// and of those the ones the weld collapsed (degenerate with a moved corner).
    pub degenerate_triangles: u64,
    pub collapsed_triangles: u64,
    /// Collapsed triangles no corner of which lies on a surviving (non-degenerate) face:
    /// `mesh` refuses this topology change.
    pub isolated_collapses: u64,
    /// The largest bound on a collapsed triangle's source points' distance to a surviving
    /// face (see the module notes); 0 without collapses.
    pub collapse_bound_m: f64,
    /// Non-degenerate triangles in a group of coincident ones (the same three corner
    /// positions, either orientation) that holds a triangle with a moved corner.
    pub coincident_triangles: u64,
    /// Additive weld charge in delivery. Zero with original-source certification.
    pub geometric_error_term_m: f64,
}

impl MeshWeldReport {
    /// Lines for a human summary (the CLI's `mesh` and `mesh-inspect`): the statement and,
    /// when the weld collapsed, isolated or duplicated faces, a warning.
    pub fn summary_lines(&self) -> Vec<String> {
        let scope = if self.join_primitives == "same-material" {
            "within each material"
        } else {
            "within each primitive"
        };
        let mut lines = vec![format!(
            "weld: merged {} of {} source vertices within {} m {scope} ({} moved, {} classes); \
             largest displacement {} m; geometric error uses the original source",
            self.merged_vertices,
            self.vertices,
            self.epsilon_m,
            self.moved_vertices,
            self.classes,
            self.max_displacement_m,
        )];
        if self.collapsed_triangles + self.coincident_triangles + self.isolated_collapses > 0 {
            lines.push(format!(
                "warning: the weld collapsed {} triangles ({} isolated from every surviving \
                 face; {} degenerate after it) and left {} triangles coincident with another; \
                 a smaller --weld-epsilon may avoid them",
                self.collapsed_triangles,
                self.isolated_collapses,
                self.degenerate_triangles,
                self.coincident_triangles,
            ));
        }
        lines
    }
}

/// One record of a vertex in a column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Point {
    column: u64,
    /// f32 bits of the canonical position.
    position: [u32; 3],
    /// Global vertex index, with [`HOME`] set on the record in its own cell.
    id: u32,
    group: u32,
}
/// f32 bits in the order of the values (canonical positions hold no −0 or NaN).
fn ordered(bits: u32) -> u32 {
    if bits & 0x8000_0000 != 0 {
        !bits
    } else {
        bits | 0x8000_0000
    }
}
impl Ord for Point {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.column,
            ordered(self.position[0]),
            self.id,
            self.group,
            self.position,
        )
            .cmp(&(
                other.column,
                ordered(other.position[0]),
                other.id,
                other.group,
                other.position,
            ))
    }
}
impl PartialOrd for Point {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
fn put_u32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn get_u32(bytes: &[u8], at: usize) -> u32 {
    let mut b = [0; 4];
    b.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(b)
}
impl Record for Point {
    const SIZE: usize = 28;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.column.to_le_bytes());
        for (a, v) in self.position.iter().enumerate() {
            put_u32(out, 8 + 4 * a, *v);
        }
        put_u32(out, 20, self.id);
        put_u32(out, 24, self.group);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            column: {
                let mut b = [0; 8];
                b.copy_from_slice(&bytes[..8]);
                u64::from_le_bytes(b)
            },
            position: std::array::from_fn(|a| get_u32(bytes, 8 + 4 * a)),
            id: get_u32(bytes, 20),
            group: get_u32(bytes, 24),
        }
    }
}

/// A vertex and its position (the sweep's positions, the move list).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Located {
    id: u32,
    position: [u32; 3],
}
impl Record for Located {
    const SIZE: usize = 16;
    fn encode(&self, out: &mut [u8]) {
        put_u32(out, 0, self.id);
        for (a, v) in self.position.iter().enumerate() {
            put_u32(out, 4 + 4 * a, *v);
        }
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            id: get_u32(bytes, 0),
            position: std::array::from_fn(|a| get_u32(bytes, 4 + 4 * a)),
        }
    }
}

/// Both sides of a move, sorted by the vertex ID before the two logs are written.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Move {
    target: Located,
    original: [u32; 3],
}
impl Record for Move {
    const SIZE: usize = Located::SIZE + 12;
    fn encode(&self, out: &mut [u8]) {
        self.target.encode(&mut out[..Located::SIZE]);
        for (i, bits) in self.original.iter().enumerate() {
            put_u32(out, Located::SIZE + i * 4, *bits);
        }
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            target: Located::decode(&bytes[..Located::SIZE]),
            original: std::array::from_fn(|i| get_u32(bytes, Located::SIZE + i * 4)),
        }
    }
}

/// A paired vertex under its class's representative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Class {
    root: u32,
    id: u32,
    position: [u32; 3],
}
impl Record for Class {
    const SIZE: usize = 20;
    fn encode(&self, out: &mut [u8]) {
        put_u32(out, 0, self.root);
        put_u32(out, 4, self.id);
        for (a, v) in self.position.iter().enumerate() {
            put_u32(out, 8 + 4 * a, *v);
        }
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            root: get_u32(bytes, 0),
            id: get_u32(bytes, 4),
            position: std::array::from_fn(|a| get_u32(bytes, 8 + 4 * a)),
        }
    }
}

fn bits(p: [f32; 3]) -> [u32; 3] {
    p.map(f32::to_bits)
}
fn value(p: [u32; 3]) -> [f64; 3] {
    p.map(|b| f64::from(f32::from_bits(b)))
}

/// SplitMix64's finaliser: a fixed 64-bit mix.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
fn column(group: u32, y: u64, z: u64) -> u64 {
    mix(mix(mix(u64::from(group)) ^ y) ^ z)
}

/// Whether `a` and `b` pair under `epsilon` (see the module notes); symmetric.
fn within(a: [u32; 3], b: [u32; 3], epsilon: f64) -> bool {
    if epsilon == 0. {
        return a == b;
    }
    let (a, b) = (value(a), value(b));
    let d: [f64; 3] = std::array::from_fn(|k| a[k] - b[k]);
    d.iter().all(|d| d.abs() <= epsilon)
        && d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= epsilon * epsilon
}

/// The cells of `v` on one axis, as the bits of their integral index: its own and, when its
/// ε-range reaches it, the lower one.
fn cells(v: f32, epsilon: f64, cell: f64) -> Result<[Option<u64>; 2]> {
    // Integral and exact below 2^52; +0 for every zero.
    let index = |x: f64| -> Result<f64> {
        let q = (x / cell).floor() + 0.;
        if q.abs() < (1u64 << 52) as f64 {
            Ok(q)
        } else {
            Err(request(
                "weld epsilon is too small for the source's coordinate range",
            ))
        }
    };
    let v = f64::from(v);
    let (home, low) = (index(v)?, index(v - epsilon)?);
    if home == low {
        Ok([Some(home.to_bits()), None])
    } else if home - low == 1. {
        Ok([Some(home.to_bits()), Some(low.to_bits())])
    } else {
        Err(invariant("weld cell side below twice its tolerance"))
    }
}

/// The records of vertex `id` of `group` at `position`.
fn points(
    weld: &MeshWeld,
    group: u32,
    id: u32,
    position: [f32; 3],
    out: &mut Vec<Point>,
) -> Result<()> {
    let b = bits(position);
    if weld.epsilon == 0. {
        out.push(Point {
            column: column(group, u64::from(b[1]), u64::from(b[2])),
            position: b,
            id: id | HOME,
            group,
        });
        return Ok(());
    }
    let cell = weld.cell();
    let [ys, zs] = [
        cells(position[1], weld.epsilon, cell)?,
        cells(position[2], weld.epsilon, cell)?,
    ];
    for (i, y) in ys.iter().enumerate() {
        for (j, z) in zs.iter().enumerate() {
            let (Some(y), Some(z)) = (y, z) else {
                continue;
            };
            out.push(Point {
                column: column(group, *y, *z),
                position: b,
                id: if i == 0 && j == 0 { id | HOME } else { id },
                group,
            });
        }
    }
    Ok(())
}

/// Union-find over global vertex indices; a class's root is its lowest index.
struct Classes {
    parent: Vec<u32>,
}
impl Classes {
    fn new(vertices: usize) -> Result<Self> {
        let mut parent = Vec::new();
        parent
            .try_reserve_exact(vertices)
            .map_err(|_| resource("weld union-find allocation failed"))?;
        let end = u32::try_from(vertices)
            .map_err(|_| resource("the weld supports at most 2^31 - 1 source vertices"))?;
        parent.extend(0..end);
        Ok(Self { parent })
    }
    fn find(&mut self, mut x: u32) -> u32 {
        // Path halving keeps `parent[x] <= x`: roots are the lowest of their class.
        while self.parent[x as usize] != x {
            let grand = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = grand;
            x = grand;
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            let (low, high) = if a < b { (a, b) } else { (b, a) };
            self.parent[high as usize] = low;
        }
    }
}

fn set(bits: &[AtomicU64], v: usize) {
    bits[v / 64].fetch_or(1 << (v % 64), Relaxed);
}
fn bit(bits: &[AtomicU64], v: usize) -> bool {
    bits[v / 64].load(Relaxed) >> (v % 64) & 1 == 1
}

/// The vertices `p`'s captured triangles use (`kept` ones in a subset bake), and how many.
fn used_vertices(
    source: &StreamSource,
    p: &ScenePrimitive<'_>,
    kept: Option<&Ranks>,
    buffer_bytes: usize,
    workers: usize,
    poll: &mut dyn FnMut() -> Result<()>,
) -> Result<(Vec<AtomicU64>, u64)> {
    let used: Vec<AtomicU64> = (0..p.position.count.div_ceil(64))
        .map(|_| AtomicU64::new(0))
        .collect();
    let ran = run(
        p.triangles.div_ceil(WELD_TRIANGLES),
        workers,
        poll,
        |chunk, stop| {
            let start = chunk * WELD_TRIANGLES;
            let mut indices = p.indices.map(|v| source.reader(&v, buffer_bytes));
            for t in start..(start + WELD_TRIANGLES).min(p.triangles) {
                if t.is_multiple_of(1024) && stop.load(Relaxed) {
                    return Err(stopped());
                }
                if kept.is_some_and(|kept| !kept.contains(t)) {
                    continue;
                }
                for v in vertices(p, &mut indices, t)? {
                    set(
                        &used,
                        usize::try_from(v).map_err(|_| resource("vertex index"))?,
                    );
                }
            }
            Ok(())
        },
    )?;
    match ran {
        Ran::All(_) => {}
        Ran::Failed(failure) => return Err(failure),
    }
    let count = used
        .iter()
        .map(|w| u64::from(w.load(Relaxed).count_ones()))
        .sum();
    Ok((used, count))
}

/// Pairs every record of `sorted` (in [`Point`] order) with the window of earlier records
/// of its column within ε in x: each pair [`within`] ε and in one scope joins its two
/// vertices in `classes` and marks both in `paired`. `home` receives each vertex's own
/// record, once; `check` is called with the comparisons made so far at least every
/// [`CHECK_COMPARISONS`] comparisons and every 4,096 records.
///
/// A record at exactly the position and scope of one in the window joins it and is not
/// added: every later record pairs with both or neither, so the window holds distinct
/// positions and exact repeats cost one comparison each (ε = 0 never fills it). The work
/// is at most the window per record; more than [`MAX_COMPARISONS`] per record on average
/// (a source dense within ε) is refused as `resource_limit`, so the sweep is linear in
/// its records.
fn sweep(
    sorted: impl Iterator<Item = Result<Point>>,
    epsilon: f64,
    classes: &mut Classes,
    paired: &[AtomicU64],
    home: &mut dyn FnMut(Located) -> Result<()>,
    check: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<()> {
    let mut window: VecDeque<Point> = VecDeque::new();
    let (mut records, mut compared, mut checked) = (0u64, 0u64, 0u64);
    for point in sorted {
        let point = point?;
        records += 1;
        let id = point.id & !HOME;
        if point.id & HOME != 0 {
            home(Located {
                id,
                position: point.position,
            })?;
        }
        if window.front().is_some_and(|w| w.column != point.column) {
            window.clear();
        }
        let x = f64::from(f32::from_bits(point.position[0]));
        while window
            .front()
            .is_some_and(|w| x - f64::from(f32::from_bits(w.position[0])) > epsilon)
        {
            window.pop_front();
        }
        let mut repeat = false;
        for w in &window {
            if w.group != point.group {
                continue;
            }
            repeat |= w.position == point.position;
            let other = w.id & !HOME;
            if other != id && within(w.position, point.position, epsilon) {
                classes.union(other, id);
                set(paired, other as usize);
                set(paired, id as usize);
            }
        }
        compared += window.len() as u64;
        if compared - checked >= CHECK_COMPARISONS || records.is_multiple_of(4096) {
            check(compared)?;
            checked = compared;
        }
        if compared > MAX_COMPARISONS.saturating_mul(records) + COMPARISON_SLACK {
            return Err(resource(format!(
                "the source is too dense within the weld epsilon (more than \
                 {MAX_COMPARISONS} vertex comparisons per vertex); use a smaller \
                 --weld-epsilon"
            )));
        }
        if repeat {
            continue;
        }
        if window.len() == WINDOW_RECORDS {
            return Err(resource(format!(
                "more than {WINDOW_RECORDS} distinct source positions lie within the weld \
                 epsilon of each other along one column; use a smaller --weld-epsilon"
            )));
        }
        window.push_back(point);
    }
    check(compared)?;
    Ok(())
}

/// The weld's result: the merged vertices' new positions, read by the assembly's decoding.
pub(crate) struct Welded {
    moves: Sequence<Located>,
    original: OriginalPositions,
    /// Per primitive instance, the first move at or after each decoding chunk of `chunk`
    /// vertices, then the end of its moves.
    starts: Vec<Vec<u64>>,
    chunk: usize,
    pub report: MeshWeldReport,
}

/// A triangle retains its expanded primitive and authored vertex IDs through Morton
/// sorting. Positions differing from the capture are exactly the set bits in `moved`.
pub(crate) struct OriginalPositions {
    /// Expanded vertex offsets, including the end sentinel.
    offsets: Vec<u64>,
    moved: Ranks,
    xyz: ByteFile,
}

impl OriginalPositions {
    const POSITION_BYTES: u64 = 12;

    pub(crate) fn scratch_bytes(used: u64) -> Option<u64> {
        used.checked_mul(Self::POSITION_BYTES)
    }

    /// Rank bitmap and prefix counts, primitive offsets with sentinel, and file metadata.
    pub(crate) fn memory_bytes(vertices: u64, primitives: usize) -> Option<u64> {
        let vertices = usize::try_from(vertices).ok()?;
        (primitives as u64)
            .checked_add(1)?
            .checked_mul(8)?
            .checked_add(Ranks::bytes(vertices))?
            .checked_add(4096)
    }

    fn vertex(&self, expanded: u64, vertex: u64) -> Result<usize> {
        let i = usize::try_from(expanded).map_err(|_| invariant("weld primitive index"))?;
        let &[first, end] = self
            .offsets
            .get(i..)
            .and_then(|r| r.first_chunk())
            .ok_or_else(|| invariant("weld primitive outside the captured instances"))?;
        first
            .checked_add(vertex)
            .filter(|id| *id < end)
            .and_then(|id| usize::try_from(id).ok())
            .ok_or_else(|| invariant("weld vertex outside the captured primitive"))
    }

    pub(crate) fn face(&self, row: &super::attributes::FullTriangle) -> Result<[[f32; 3]; 3]> {
        let mut positions = row.triangle.positions;
        for (p, vertex) in positions.iter_mut().zip(row.triangle.vertices) {
            let id = self.vertex(row.expanded_ordinal, vertex)?;
            if self.moved.contains(id) {
                let mut bytes = [0; 12];
                self.xyz.read_at(
                    self.moved.rank(id) as u64 * Self::POSITION_BYTES,
                    &mut bytes,
                )?;
                *p = std::array::from_fn(|a| f32::from_bits(get_u32(&bytes, a * 4)));
            }
        }
        Ok(positions)
    }
}

/// The moves of one decoding range, read in vertex order.
pub(crate) struct Moves<'a> {
    reader: SequenceReader<'a, Located>,
    next: Option<Located>,
    offset: u64,
}
impl Moves<'_> {
    /// The position vertex `v` of the range's primitive is captured at: its class
    /// representative's, or `decoded` when it was not merged. Vertices are asked in
    /// increasing order.
    pub(crate) fn position(&mut self, v: usize, decoded: [f32; 3]) -> Result<[f32; 3]> {
        let id = self.offset + v as u64;
        while let Some(m) = self.next {
            if u64::from(m.id) > id {
                break;
            }
            self.next = self.reader.next().transpose()?;
            if u64::from(m.id) == id {
                return Ok(m.position.map(f32::from_bits));
            }
        }
        Ok(decoded)
    }
}

impl Welded {
    pub(crate) fn finish(self) -> (MeshWeldReport, Option<OriginalPositions>) {
        let original = (self.report.moved_vertices > 0).then_some(self.original);
        (self.report, original)
    }

    /// The moves of vertices `range` of primitive instance `primitive`; `range` starts at a
    /// decoding chunk and ends at the next or at the primitive's end.
    pub(crate) fn moves(
        &self,
        primitive: usize,
        range: std::ops::Range<usize>,
    ) -> Result<Moves<'_>> {
        let starts = &self.starts[primitive];
        if !range.start.is_multiple_of(self.chunk) {
            return Err(invariant("weld moves read from inside a decoding chunk"));
        }
        let first = range.start / self.chunk;
        let last = range.end.div_ceil(self.chunk).min(starts.len() - 1);
        let mut reader = self
            .moves
            .range(starts[first], starts[last.max(first)], MOVE_BUFFER)?;
        let next = reader.next().transpose()?;
        Ok(Moves {
            reader,
            next,
            offset: self.original.offsets[primitive],
        })
    }
}

/// Welds the vertices `source`'s captured triangles use (see the module notes), reporting
/// [`Step::Weld`]: every vertex decoded, then every vertex swept. `frame` is the capture's
/// resolved frame; the move index follows the assembly's decoding chunks.
pub(crate) fn weld(
    source: &StreamSource,
    frame: &mut SourceFrame,
    (weld, selection): (&MeshWeld, Option<&Selection>),
    scratch: &Scratch,
    (sorter, workers): (SortConfig, usize),
    progress: &StepProgress<'_>,
) -> Result<Welded> {
    let chunk = super::assembly::CHUNK_VERTICES;
    let buffer_bytes = sorter.buffer_bytes;
    let primitives: Vec<ScenePrimitive<'_>> = source.document.scene_primitives().collect();
    let mut offsets = Vec::with_capacity(primitives.len() + 1);
    let mut total = 0u64;
    for p in &primitives {
        offsets.push(total);
        total += p.position.count as u64;
    }
    offsets.push(total);
    if total >= u64::from(HOME) {
        return Err(resource(
            "the weld supports at most 2^31 - 1 source vertices",
        ));
    }
    progress.begin(Step::Weld, total.saturating_mul(2))?;
    // 1. Decode every used vertex and list it in its columns.
    let mut sort = ExternalSorter::<Point>::new(scratch, sorter)?.observed(progress);
    let mut considered = 0u64;
    let mut done = 0u64;
    for (i, p) in primitives.iter().enumerate() {
        let kept = selection.map(|s| &s.primitives[i].triangles);
        if selection.is_some_and(|s| s.primitives[i].vertices == 0) {
            done += p.position.count as u64;
            progress.advance(done)?;
            continue;
        }
        let (used, count) = used_vertices(source, p, kept, buffer_bytes, workers, &mut || {
            progress.check()
        })?;
        considered += count;
        let index = |n: u64| {
            u32::try_from(n)
                .map_err(|_| resource("the weld supports at most 2^31 - 1 source vertices"))
        };
        let group = if weld.same_material {
            p.material.unwrap_or(u32::MAX)
        } else {
            index(i as u64)?
        };
        let offset = offsets[i];
        let chunks = p.position.count.div_ceil(WELD_VERTICES);
        let task = |c: usize,
                    frame: &mut SourceFrame,
                    check: &mut dyn FnMut() -> Result<()>|
         -> Result<Vec<Point>> {
            let start = c * WELD_VERTICES;
            let end = (start + WELD_VERTICES).min(p.position.count);
            let mut positions = source.reader(&p.position, buffer_bytes);
            let mut out = Vec::new();
            for v in start..end {
                if v.is_multiple_of(4096) {
                    crate::test_hooks::pace("mesh.weld");
                    check()?;
                }
                if !bit(&used, v) {
                    continue;
                }
                let world = source_position(p, &mut positions, v as u64)?;
                let (position, _) = frame.vertex(world, false)?;
                let id = index(offset + v as u64)?;
                points(weld, group, id, canonical(position), &mut out)?;
            }
            Ok(out)
        };
        let wave = 2 * workers.max(1);
        for first in (0..chunks).step_by(wave) {
            let last = (first + wave).min(chunks);
            let found =
                match decode_chunks(frame, first..last, workers, &mut || progress.check(), task)? {
                    Ran::All(found) => found,
                    // The first failure in vertex order, for every worker count.
                    Ran::Failed(failure) => {
                        match decode_chunks(frame, first..last, 1, &mut || progress.check(), task)?
                        {
                            Ran::Failed(first) => return Err(first),
                            Ran::All(_) => return Err(failure),
                        }
                    }
                };
            for records in found {
                for point in records {
                    sort.push(point)?;
                }
            }
            done = offsets[i] + (last * WELD_VERTICES).min(p.position.count) as u64;
            progress.advance(done)?;
        }
    }
    done = total;
    progress.advance(done)?;
    // 2. Sweep each column in x order, pairing every record with the window's.
    let mut classes = Classes::new(
        usize::try_from(total).map_err(|_| resource("weld union-find size overflow"))?,
    )?;
    let paired: Vec<AtomicU64> = (0..total.div_ceil(64)).map(|_| AtomicU64::new(0)).collect();
    let mut located = SequenceWriter::<Located>::new(scratch, "weld-positions", buffer_bytes)?;
    let swept = std::cell::Cell::new(0u64);
    sweep(
        sort.finish()?.map(|p| p.map_err(JobError::from)),
        weld.epsilon,
        &mut classes,
        &paired,
        &mut |home| {
            located.push(&home)?;
            swept.set(swept.get() + 1);
            Ok(())
        },
        &mut |_| progress.advance(done + swept.get()),
    )?;
    let located = located.finish()?;
    if located.len() != considered {
        return Err(invariant("weld sweep does not hold every vertex once"));
    }
    progress.advance(total.saturating_mul(2))?;
    // 3. Each paired vertex under its representative, then the moves in vertex order.
    let mut by_class = ExternalSorter::<Class>::new(scratch, sorter)?.observed(progress);
    for (k, v) in located.reader()?.enumerate() {
        let v = v?;
        if k.is_multiple_of(4096) {
            progress.check()?;
        }
        if bit(&paired, v.id as usize) {
            by_class.push(Class {
                root: classes.find(v.id),
                id: v.id,
                position: v.position,
            })?;
        }
    }
    drop(located);
    drop(classes);
    drop(paired);
    let mut report = MeshWeldReport {
        epsilon_m: weld.epsilon,
        join_primitives: if weld.same_material {
            "same-material"
        } else {
            "none"
        }
        .to_owned(),
        vertices: considered,
        merged_vertices: 0,
        moved_vertices: 0,
        classes: 0,
        largest_class: 0,
        max_displacement_m: 0.,
        degenerate_triangles: 0,
        collapsed_triangles: 0,
        isolated_collapses: 0,
        collapse_bound_m: 0.,
        coincident_triangles: 0,
        geometric_error_term_m: 0.,
    };
    let mut unsorted = SequenceWriter::<Move>::new(scratch, "weld-moves-unsorted", buffer_bytes)?;
    let (mut root, mut rep, mut size) = (u32::MAX, [0u32; 3], 0u64);
    for (k, c) in by_class.finish()?.enumerate() {
        let c = c?;
        if k.is_multiple_of(4096) {
            progress.check()?;
        }
        if c.root != root {
            // The root is the lowest index of its class, so it comes first.
            if c.id != c.root {
                return Err(invariant("weld class does not start at its representative"));
            }
            (root, rep, size) = (c.root, c.position, 1);
            report.classes += 1;
            report.largest_class = report.largest_class.max(1);
            continue;
        }
        size += 1;
        report.largest_class = report.largest_class.max(size);
        report.merged_vertices += 1;
        if c.position != rep {
            report.moved_vertices += 1;
            let (a, b) = (value(c.position), value(rep));
            let d = (0..3)
                .map(|k| (a[k] - b[k]) * (a[k] - b[k]))
                .sum::<f64>()
                .sqrt();
            report.max_displacement_m = report.max_displacement_m.max(d);
            unsorted.push(&Move {
                target: Located {
                    id: c.id,
                    position: rep,
                },
                original: c.position,
            })?;
        }
    }
    let unsorted = unsorted.finish()?;
    let mut by_vertex = ExternalSorter::<Move>::new(scratch, sorter)?.observed(progress);
    for (k, m) in unsorted.reader()?.enumerate() {
        if k.is_multiple_of(4096) {
            progress.check()?;
        }
        by_vertex.push(m?)?;
    }
    drop(unsorted);
    // 4. The sorted moves and their index by decoding chunk.
    let mut moves = SequenceWriter::<Located>::new(scratch, "weld-moves", buffer_bytes)?;
    let mut xyz = ByteWriter::new(scratch, "weld-original-xyz", buffer_bytes)?;
    let mut starts: Vec<Vec<u64>> = primitives
        .iter()
        .map(|p| Vec::with_capacity(p.position.count.div_ceil(chunk) + 1))
        .collect();
    // Chunk boundaries in global order: (global index, primitive). Each takes the index of
    // the first move at or after it; a primitive's last is its end.
    let mut boundaries = primitives.iter().enumerate().flat_map(|(i, p)| {
        let (offset, count) = (offsets[i], p.position.count as u64);
        (0..=p.position.count.div_ceil(chunk))
            .map(move |c| (offset + count.min((c * chunk) as u64), i))
    });
    let mut pending = boundaries.next();
    let mut written = 0u64;
    let words = usize::try_from(total.div_ceil(64))
        .map_err(|_| resource("weld moved-vertex bitmap size overflow"))?;
    let mut moved = Vec::new();
    moved
        .try_reserve_exact(words)
        .map_err(|_| resource("weld moved-vertex bitmap allocation failed"))?;
    moved.resize(words, 0u64);
    for (k, m) in by_vertex.finish()?.enumerate() {
        let m = m?;
        if k.is_multiple_of(4096) {
            progress.check()?;
        }
        while let Some((_, i)) = pending.filter(|(at, _)| *at <= u64::from(m.target.id)) {
            starts[i].push(written);
            pending = boundaries.next();
        }
        moves.push(&m.target)?;
        let mut bytes = [0; 12];
        for (a, p) in m.original.iter().enumerate() {
            put_u32(&mut bytes, 4 * a, *p);
        }
        xyz.append(&bytes)?;
        moved[(m.target.id / 64) as usize] |= 1 << (m.target.id % 64);
        written += 1;
    }
    while let Some((_, i)) = pending {
        starts[i].push(written);
        pending = boundaries.next();
    }
    let moves = moves.finish()?;
    let moved = Ranks::new(moved);
    let xyz = xyz.finish()?;
    if moves.len() != report.moved_vertices
        || moved.ones() as u64 != moves.len()
        || xyz.len() != moves.len() * OriginalPositions::POSITION_BYTES
    {
        return Err(invariant("weld moves disagree with the classes"));
    }
    progress.end()?;
    Ok(Welded {
        moves,
        original: OriginalPositions {
            offsets,
            moved,
            xyz,
        },
        starts,
        chunk,
        report,
    })
}

/// A non-degenerate captured triangle's corner positions, sorted (either orientation), and
/// whether one of its corners moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Face {
    corners: [[u32; 3]; 3],
    moved: bool,
}
impl Record for Face {
    const SIZE: usize = 37;
    fn encode(&self, out: &mut [u8]) {
        for (c, corner) in self.corners.iter().enumerate() {
            for (a, v) in corner.iter().enumerate() {
                put_u32(out, 12 * c + 4 * a, *v);
            }
        }
        out[36] = u8::from(self.moved);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            corners: std::array::from_fn(|c| {
                std::array::from_fn(|a| get_u32(bytes, 12 * c + 4 * a))
            }),
            moved: bytes[36] != 0,
        }
    }
}

/// Collapsed triangles the audit holds at most; a weld collapsing more is refused.
pub(crate) const MAX_COLLAPSED: usize = 1 << 20;

/// The audit's resident bytes beside one external sort: the collapsed triangles and their
/// distinct corners with a mark each.
pub(crate) fn audit_memory_bytes() -> u64 {
    // Per collapsed triangle: its corners, up to three distinct corners and a segment,
    // each with a mark.
    (MAX_COLLAPSED as u64) * (36 + 3 * 13 + 25)
}

/// The audit's scratch for `triangles` captured triangles: one external sort of a record
/// per non-degenerate triangle (two run generations).
pub(crate) fn audit_scratch_bytes(triangles: u64) -> Option<u64> {
    triangles.checked_mul(2 * Face::SIZE as u64)
}

/// A collapsed triangle's segment (its two distinct corners, ordered), or `None` when it
/// collapsed to a point.
fn segment(c: &[[u32; 3]; 3]) -> Option<[[u32; 3]; 2]> {
    let (mut low, mut high) = (c[0], c[0]);
    for v in &c[1..] {
        low = low.min(*v);
        high = high.max(*v);
    }
    (low != high).then_some([low, high])
}

fn distance(a: [u32; 3], b: [u32; 3]) -> f64 {
    let (a, b) = (value(a), value(b));
    (0..3)
        .map(|k| (a[k] - b[k]) * (a[k] - b[k]))
        .sum::<f64>()
        .sqrt()
}

/// What the weld did to the captured faces, read from `triangles` (the assembly's
/// sequence) into the report, reporting [`Step::WeldCheck`] (two units per triangle):
///
/// - **Collapsed triangles** are degenerate after the weld with a moved corner. The weld
///   maps a source triangle's point at barycentric λ to its welded point at λ, at most `d`
///   (the largest displacement) away; for a collapsed triangle that point lies on its
///   collapsed segment or point, which HLOD does not keep as a face. When that segment is
///   an edge of a surviving face (or that point a corner of one), the point lies on the
///   surviving surface and the bound is `d`. Otherwise, if a corner `s` of the
///   collapsed triangle is a corner of a surviving (non-degenerate) captured face, every
///   point of the collapsed set is within `max_j |v′_j − s|` of `s`, so every source point
///   of the triangle is within `d + max_j |v′_j − s|` of a surviving face. The best such
///   corner supplies the reported collapse diagnostic. Direct certification retains the
///   pre-weld triangle in its owner's reference. A collapsed triangle with no such
///   shared corner is **isolated**. The topology policy refuses that weld.
/// - **Coincident triangles** are non-degenerate faces sharing their three corner
///   positions with another, in groups holding a face with a moved corner.
pub(crate) fn audit(
    welded: &mut Welded,
    triangles: &Sequence<super::attributes::FullTriangle>,
    scratch: &Scratch,
    sorter: SortConfig,
    progress: &StepProgress<'_>,
) -> Result<()> {
    let n = triangles.len();
    progress.begin(Step::WeldCheck, n.saturating_mul(2))?;
    let mut faces = ExternalSorter::<Face>::new(scratch, sorter)?.observed(progress);
    let mut collapsed: Vec<[[u32; 3]; 3]> = Vec::new();
    let (mut degenerate, mut seen) = (0u64, 0u64);
    for t in triangles.reader()? {
        let t = t?;
        seen += 1;
        if seen.is_multiple_of(4096) {
            progress.advance(seen)?;
        }
        let mut any = false;
        for v in t.triangle.vertices {
            any |= welded
                .original
                .moved
                .contains(welded.original.vertex(t.expanded_ordinal, v)?);
        }
        let corners = t.triangle.positions.map(bits);
        if t.triangle.degenerate {
            degenerate += 1;
            if any {
                if collapsed.len() == MAX_COLLAPSED {
                    return Err(resource(format!(
                        "the weld collapses more than {MAX_COLLAPSED} triangles; use a smaller \
                         --weld-epsilon"
                    )));
                }
                collapsed.push(corners);
            }
        } else {
            let mut sorted = corners;
            sorted.sort_unstable();
            faces.push(Face {
                corners: sorted,
                moved: any,
            })?;
        }
    }
    // Which collapsed corners, and which collapsed segments, lie on a surviving face.
    let mut distinct: Vec<[u32; 3]> = collapsed.iter().flatten().copied().collect();
    distinct.sort_unstable();
    distinct.dedup();
    let mut segments: Vec<[[u32; 3]; 2]> = collapsed.iter().filter_map(segment).collect();
    segments.sort_unstable();
    segments.dedup();
    let mut alive = vec![false; distinct.len()];
    let mut alive_segments = vec![false; segments.len()];
    if !distinct.is_empty() {
        for (k, t) in triangles.reader()?.enumerate() {
            let t = t?;
            if k.is_multiple_of(4096) {
                progress.advance(n + k as u64)?;
            }
            if t.triangle.degenerate {
                continue;
            }
            let corners = t.triangle.positions.map(bits);
            for p in corners {
                if let Ok(i) = distinct.binary_search(&p) {
                    alive[i] = true;
                }
            }
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let edge = if corners[a] < corners[b] {
                    [corners[a], corners[b]]
                } else {
                    [corners[b], corners[a]]
                };
                if let Ok(i) = segments.binary_search(&edge) {
                    alive_segments[i] = true;
                }
            }
        }
    }
    let d = welded.report.max_displacement_m;
    let (mut isolated, mut bound) = (0u64, 0f64);
    for c in &collapsed {
        // On a surviving face itself (a collapsed point that is a surviving corner, or a
        // segment that is a surviving edge): within d. Otherwise through a surviving corner.
        let on_face = match segment(c) {
            Some(edge) => segments
                .binary_search(&edge)
                .is_ok_and(|i| alive_segments[i]),
            None => distinct.binary_search(&c[0]).is_ok_and(|i| alive[i]),
        };
        let best = if on_face {
            0.
        } else {
            c.iter()
                .filter(|s| distinct.binary_search(s).is_ok_and(|i| alive[i]))
                .map(|s| c.iter().map(|v| distance(*v, *s)).fold(0., f64::max))
                .fold(f64::INFINITY, f64::min)
        };
        if best.is_finite() {
            bound = bound.max(d + best);
        } else {
            isolated += 1;
        }
    }
    // Coincident groups holding a moved face.
    let mut coincident = 0u64;
    let (mut group, mut size, mut any) = (None::<[[u32; 3]; 3]>, 0u64, false);
    for (k, f) in faces.finish()?.enumerate() {
        let f = f?;
        if k.is_multiple_of(4096) {
            progress.check()?;
        }
        if group != Some(f.corners) {
            if size > 1 && any {
                coincident += size;
            }
            (group, size, any) = (Some(f.corners), 0, false);
        }
        size += 1;
        any |= f.moved;
    }
    if size > 1 && any {
        coincident += size;
    }
    let report = &mut welded.report;
    report.degenerate_triangles = degenerate;
    report.collapsed_triangles = collapsed.len() as u64;
    report.isolated_collapses = isolated;
    report.collapse_bound_m = bound;
    report.coincident_triangles = coincident;
    progress.end()
}

#[cfg(test)]
#[path = "weld_tests.rs"]
mod tests;
