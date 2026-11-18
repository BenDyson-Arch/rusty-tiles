//! `--bbox` / `--polygon`: bake only the source triangles whose centroid lies in a region.
//!
//! **Region coordinates.** A local source's region is in the plane of its post-node glTF
//! coordinates: `x` is glTF X and `y` is glTF −Z (north for a Y-up scene), in source metres,
//! before any `--scene-offset`. With a source CRS it is WGS84 geodetic longitude (`x`) and
//! latitude (`y`) in degrees.
//!
//! **Centroid.** A triangle's centroid is `(a + b + c) / 3` of its three post-node source
//! positions, in f64. With a source CRS that point (plus the source offset) is mapped through
//! the capture's CRS operation to WGS84, so the test sees the mapped source-space centroid.
//!
//! **Boundary.** Inclusion is half-open: a bbox keeps `minx <= x < maxx` and
//! `miny <= y < maxy`. A polygon keeps a point when a ray towards +x crosses its rings an odd
//! number of times (even-odd over the polygon's rings, holes included, winding ignored),
//! where an edge counts when one endpoint has `y <= p.y` and the other `y > p.y` and its
//! crossing lies at `x > p.x`; the crossing is computed from the edge's lower endpoint, so an
//! edge shared by two polygons gives both the same answer and a centroid on it lands in
//! exactly one. A MultiPolygon (and a FeatureCollection) is the union of its polygons. A bbox
//! is exactly the polygon of its rectangle.
//!
//! **Placement.** Everything that places the output is taken from the whole source: the
//! source CRS origin and Jacobian lattice, the root cube of the cell tree and the position
//! lattice's zero origin and radius. Only the triangle set changes, so every kept vertex is
//! captured at the full bake's f32 position, in its frame. (Quantized profiles' per-depth
//! steps follow each bake's own tree, so they may differ by powers of two.)
//!
//! **Selection.** [`select`] runs once, after the preflight scan and before any scratch: one
//! bit per source triangle, the vertices kept triangles use, and the boxes capture checks. The
//! assembly then decodes and stores only those vertices ([`Ranks`]), so an excluded triangle
//! costs no scratch.
use super::{
    capture::{canonical, source_position, triangle_bytes, vertices},
    input::StreamSource,
    phases::{Step, StepProgress},
    records::Bounds,
    source_crs::SourceFrame,
    tasks::{run, Ran},
};
use crate::{gltf_source::source::ScenePrimitive, JobError, JobErrorKind};
use serde::Serialize;
use serde_json::Value;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering::Relaxed},
        Arc,
    },
};

type Result<T> = std::result::Result<T, JobError>;

fn request(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidRequest, message)
}
fn overflow() -> JobError {
    JobError::new(
        JobErrorKind::ResourceLimit,
        "mesh subset count or byte arithmetic overflow",
    )
}
fn changed() -> JobError {
    JobError::new(
        JobErrorKind::InvalidInput,
        "mesh source geometry changed between preflight and capture",
    )
}

/// GeoJSON files larger than this are refused before parsing.
pub(crate) const POLYGON_FILE_BYTES: u64 = 64 << 20;
/// Triangles one selection task tests (a multiple of 64: tasks own whole bitmap words).
const SELECT_TRIANGLES: usize = 1 << 16;
/// Vertices one marking or mapping task covers (a multiple of 64).
const SELECT_VERTICES: usize = 1 << 16;
/// The position window of the triangle-order reads, as the scan's.
const POSITION_WINDOW: usize = 64 << 10;
/// At most this many polygon bands.
const MAX_BANDS: usize = 1 << 16;

/// The region of a mesh subset: a bbox or polygons, in the coordinates the module notes
/// define.
#[derive(Clone)]
pub struct MeshSubset {
    region: Region,
}

#[derive(Clone)]
enum Region {
    Bbox([f64; 4]),
    Polygon(Arc<Polygons>),
}

impl std::fmt::Debug for MeshSubset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.region {
            Region::Bbox(bbox) => f.debug_tuple("Bbox").field(bbox).finish(),
            Region::Polygon(polygons) => f
                .debug_struct("Polygon")
                .field("polygons", &polygons.polygons)
                .field("rings", &polygons.rings)
                .field("bounds", &polygons.bounds)
                .finish(),
        }
    }
}

impl MeshSubset {
    /// The shared adapter grammar: at most one of a bbox (`minx miny maxx maxy`) and a
    /// GeoJSON polygon file. Neither keeps the whole source.
    pub fn from_parameters(
        bbox: Option<&[f64]>,
        polygon_file: Option<&Path>,
    ) -> Result<Option<Self>> {
        match (bbox, polygon_file) {
            (Some(_), Some(_)) => Err(request("pass only one of a bbox and a polygon file")),
            (Some(bbox), None) => Self::bbox(bbox).map(Some),
            (None, Some(path)) => Self::polygon_file(path).map(Some),
            (None, None) => Ok(None),
        }
    }

    /// A bbox `[minx, miny, maxx, maxy]`: finite, with `minx < maxx` and `miny < maxy`.
    pub fn bbox(values: &[f64]) -> Result<Self> {
        let &[minx, miny, maxx, maxy] = values else {
            return Err(request(
                "bbox takes exactly four numbers: minx miny maxx maxy",
            ));
        };
        if !values.iter().all(|v| v.is_finite()) {
            return Err(request("bbox numbers must be finite"));
        }
        if !(minx < maxx && miny < maxy) {
            return Err(request("bbox needs minx < maxx and miny < maxy"));
        }
        Ok(Self {
            region: Region::Bbox([minx, miny, maxx, maxy]),
        })
    }

    /// A GeoJSON file holding a Polygon or MultiPolygon geometry, a Feature with one, or a
    /// FeatureCollection of such Features (at most 64 MiB).
    pub fn polygon_file(path: &Path) -> Result<Self> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|file| file.take(POLYGON_FILE_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|error| JobError::io("read polygon file", path, error))?;
        if bytes.len() as u64 > POLYGON_FILE_BYTES {
            return Err(request("polygon file exceeds 64 MiB"));
        }
        Self::polygon_geojson(&bytes)
    }

    /// [`Self::polygon_file`] from the file's bytes.
    pub fn polygon_geojson(bytes: &[u8]) -> Result<Self> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|error| request(format!("polygon file is not valid GeoJSON: {error}")))?;
        let mut polygons = Vec::new();
        collect(&value, Level::Top, &mut polygons)?;
        if polygons.is_empty() {
            return Err(request("polygon file holds no polygon"));
        }
        Ok(Self {
            region: Region::Polygon(Arc::new(Polygons::new(&polygons)?)),
        })
    }

    /// With a source CRS the region is geographic: every longitude in [-180, 180] and every
    /// latitude in [-90, 90].
    pub(crate) fn check_geographic(&self) -> Result<()> {
        let [minx, miny, maxx, maxy] = self.bounds();
        if minx < -180. || maxx > 180. || miny < -90. || maxy > 90. {
            return Err(request(
                "with a source CRS the subset region is WGS84 longitude in [-180, 180] and latitude in [-90, 90] (degrees)",
            ));
        }
        Ok(())
    }

    /// The region's bounding box `[minx, miny, maxx, maxy]`.
    fn bounds(&self) -> [f64; 4] {
        match &self.region {
            Region::Bbox(bbox) => *bbox,
            Region::Polygon(polygons) => polygons.bounds,
        }
    }

    /// Whether the region keeps a centroid at `point`.
    fn contains(&self, [x, y]: [f64; 2]) -> bool {
        match &self.region {
            Region::Bbox([minx, miny, maxx, maxy]) => {
                *minx <= x && x < *maxx && *miny <= y && y < *maxy
            }
            Region::Polygon(polygons) => polygons.contains([x, y]),
        }
    }

    /// Resident bytes of the selection for `source`: each expanded primitive's kept-triangle
    /// ranks (held through the assembly), its kept-vertex ranks (built by the assembly, all
    /// live while it distributes), two vertex bitmaps of the largest primitive while
    /// selecting, and the polygon index.
    pub(crate) fn memory_bytes(&self, source: &StreamSource) -> u64 {
        let mut bytes = match &self.region {
            Region::Bbox(_) => 0,
            Region::Polygon(polygons) => polygons.memory_bytes(),
        };
        let mut largest = 0u64;
        for p in source.document.scene_primitives() {
            let vertices = Ranks::bytes(p.position.count);
            bytes = bytes
                .saturating_add(Ranks::bytes(p.triangles))
                .saturating_add(vertices);
            largest = largest.max(vertices);
        }
        bytes.saturating_add(largest)
    }
}

/// Where a GeoJSON object sits: the file's top level, a Feature's geometry, or a
/// FeatureCollection's feature.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Top,
    Geometry,
    Feature,
}

/// A polygon's rings, each a closed ring of `[x, y]` positions.
type Rings = Vec<Vec<[f64; 2]>>;

fn collect(value: &Value, level: Level, out: &mut Vec<Rings>) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| request("GeoJSON polygon file: expected an object"))?;
    if object.contains_key("crs") {
        return Err(request(
            "GeoJSON \"crs\" members are not supported: coordinates are local x/-z, or WGS84 longitude/latitude with a source CRS",
        ));
    }
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| request("GeoJSON object has no \"type\""))?;
    let coordinates = || {
        object
            .get("coordinates")
            .ok_or_else(|| request(format!("GeoJSON {kind} has no \"coordinates\"")))
    };
    match (kind, level) {
        ("Polygon", _) => out.push(polygon(coordinates()?)?),
        ("MultiPolygon", _) => {
            let members = coordinates()?
                .as_array()
                .ok_or_else(|| request("GeoJSON MultiPolygon coordinates must be an array"))?;
            if members.is_empty() {
                return Err(request("GeoJSON MultiPolygon holds no polygon"));
            }
            for member in members {
                out.push(polygon(member)?);
            }
        }
        ("Feature", Level::Top | Level::Feature) => match object.get("geometry") {
            Some(geometry) if !geometry.is_null() => collect(geometry, Level::Geometry, out)?,
            _ => return Err(request("GeoJSON Feature has no geometry")),
        },
        ("FeatureCollection", Level::Top) => {
            let features = object
                .get("features")
                .and_then(Value::as_array)
                .ok_or_else(|| request("GeoJSON FeatureCollection has no \"features\" array"))?;
            for feature in features {
                if feature.get("type").and_then(Value::as_str) != Some("Feature") {
                    return Err(request(
                        "GeoJSON FeatureCollection members must be Features",
                    ));
                }
                collect(feature, Level::Feature, out)?;
            }
        }
        (other, _) => {
            return Err(request(format!(
                "GeoJSON type {other} is not supported here: use a Polygon or MultiPolygon geometry, a Feature with one, or a FeatureCollection of them"
            )))
        }
    }
    Ok(())
}

fn polygon(value: &Value) -> Result<Rings> {
    let rings = value
        .as_array()
        .ok_or_else(|| request("GeoJSON polygon coordinates must be an array of rings"))?;
    if rings.is_empty() {
        return Err(request("GeoJSON polygon has no ring"));
    }
    rings.iter().map(ring).collect()
}

fn ring(value: &Value) -> Result<Vec<[f64; 2]>> {
    let positions = value
        .as_array()
        .ok_or_else(|| request("GeoJSON ring must be an array of positions"))?;
    let ring = positions
        .iter()
        .map(|position| {
            let numbers = position
                .as_array()
                .filter(|n| (2..=3).contains(&n.len()))
                .ok_or_else(|| request("GeoJSON position must hold two or three numbers"))?;
            let mut xy = [0.; 2];
            for (out, number) in xy.iter_mut().zip(numbers) {
                *out = number
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| request("GeoJSON coordinates must be finite numbers"))?;
            }
            if numbers.get(2).is_some_and(|z| z.as_f64().is_none()) {
                return Err(request("GeoJSON coordinates must be finite numbers"));
            }
            Ok(xy)
        })
        .collect::<Result<Vec<_>>>()?;
    if ring.len() < 4 || ring.first() != ring.last() {
        return Err(request(
            "GeoJSON ring must be closed (first position equal to the last) with at least four positions",
        ));
    }
    Ok(ring)
}

/// A non-horizontal ring edge with `lo.y < hi.y`, and its polygon.
#[derive(Clone, Copy)]
struct Edge {
    polygon: u32,
    lo: [f64; 2],
    hi: [f64; 2],
}

impl Edge {
    /// Whether a ray from `p` towards +x crosses the edge (the module notes' rule).
    fn crosses(&self, [x, y]: [f64; 2]) -> bool {
        self.lo[1] <= y
            && y < self.hi[1]
            && self.lo[0] + (y - self.lo[1]) * (self.hi[0] - self.lo[0]) / (self.hi[1] - self.lo[1])
                > x
    }
}

/// Polygons as non-horizontal edges, indexed by horizontal bands: each band lists the edges
/// whose y range meets it, by polygon.
struct Polygons {
    polygons: u64,
    rings: u64,
    bounds: [f64; 4],
    edges: Vec<Edge>,
    /// Lower y of each band; the last band ends at `bounds[3]`.
    starts: Vec<f64>,
    /// Edge indices of band `k` at `members[offsets[k]..offsets[k + 1]]`, by polygon.
    members: Vec<u32>,
    offsets: Vec<usize>,
}

impl Polygons {
    fn new(polygons: &[Rings]) -> Result<Self> {
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut edges = Vec::new();
        let mut rings = 0u64;
        for (index, polygon) in polygons.iter().enumerate() {
            let id = u32::try_from(index).map_err(|_| request("too many polygons"))?;
            for ring in polygon {
                rings += 1;
                for pair in ring.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    for p in [a, b] {
                        bounds = [
                            bounds[0].min(p[0]),
                            bounds[1].min(p[1]),
                            bounds[2].max(p[0]),
                            bounds[3].max(p[1]),
                        ];
                    }
                    // A horizontal edge never has one endpoint on each side of a ray.
                    if a[1] != b[1] {
                        let (lo, hi) = if a[1] < b[1] { (a, b) } else { (b, a) };
                        edges.push(Edge {
                            polygon: id,
                            lo,
                            hi,
                        });
                    }
                }
            }
        }
        if u32::try_from(edges.len()).is_err() {
            return Err(request("polygon file has too many edges"));
        }
        let mut index = Self {
            polygons: polygons.len() as u64,
            rings,
            bounds,
            edges,
            starts: vec![bounds[1]],
            members: Vec::new(),
            offsets: Vec::new(),
        };
        // Fewer, wider bands until long edges are listed at most about eight times each.
        let mut bands = index.edges.len().clamp(1, MAX_BANDS);
        loop {
            index.starts = (0..bands)
                .map(|k| bounds[1] + (bounds[3] - bounds[1]) * (k as f64 / bands as f64))
                .collect();
            let listed: usize = index
                .edges
                .iter()
                .map(|e| index.band(e.hi[1]) - index.band(e.lo[1]) + 1)
                .sum();
            if bands == 1 || listed <= 8 * index.edges.len() + bands {
                break;
            }
            bands /= 2;
        }
        let mut counts = vec![0usize; bands + 1];
        for e in &index.edges {
            for k in index.band(e.lo[1])..=index.band(e.hi[1]) {
                counts[k + 1] += 1;
            }
        }
        for k in 0..bands {
            counts[k + 1] += counts[k];
        }
        let mut members = vec![0u32; counts[bands]];
        let mut next = counts.clone();
        // Edges are in polygon order, so each band lists its edges by polygon.
        for (i, e) in (0u32..).zip(&index.edges) {
            for k in index.band(e.lo[1])..=index.band(e.hi[1]) {
                members[next[k]] = i;
                next[k] += 1;
            }
        }
        index.members = members;
        index.offsets = counts;
        Ok(index)
    }

    /// The band holding `y` (the first for `y` below the first start, the last above).
    fn band(&self, y: f64) -> usize {
        self.starts
            .partition_point(|start| *start <= y)
            .saturating_sub(1)
    }

    fn contains(&self, p: [f64; 2]) -> bool {
        // No edge crosses a ray outside [min y, max y): outside (and NaN).
        if !(self.bounds[1] <= p[1] && p[1] < self.bounds[3]) {
            return false;
        }
        let k = self.band(p[1]);
        let mut current = None;
        let mut odd = false;
        for &i in &self.members[self.offsets[k]..self.offsets[k + 1]] {
            let edge = &self.edges[i as usize];
            if current != Some(edge.polygon) {
                if odd {
                    return true;
                }
                (current, odd) = (Some(edge.polygon), false);
            }
            if edge.crosses(p) {
                odd = !odd;
            }
        }
        odd
    }

    fn memory_bytes(&self) -> u64 {
        (self.edges.len() * std::mem::size_of::<Edge>()
            + self.members.len() * 4
            + (self.starts.len() + self.offsets.len()) * 8) as u64
    }
}

/// A bitmap with ones counted before each word: membership and rank in O(1).
pub(crate) struct Ranks {
    words: Vec<u64>,
    before: Vec<usize>,
}

impl Ranks {
    pub(crate) fn new(words: Vec<u64>) -> Self {
        let mut before = Vec::with_capacity(words.len());
        let mut ones = 0usize;
        for w in &words {
            before.push(ones);
            ones += w.count_ones() as usize;
        }
        before.push(ones);
        Self { words, before }
    }
    /// Resident bytes for `len` bits.
    pub(crate) fn bytes(len: usize) -> u64 {
        (len.div_ceil(64) as u64 + 1).saturating_mul(16)
    }
    pub(crate) fn contains(&self, i: usize) -> bool {
        self.words
            .get(i / 64)
            .is_some_and(|w| w >> (i % 64) & 1 == 1)
    }
    /// Ones before bit `i` (every one for `i` at or past the end).
    pub(crate) fn rank(&self, i: usize) -> usize {
        let word = i / 64;
        match self.words.get(word) {
            Some(w) => self.before[word] + (w & ((1u64 << (i % 64)) - 1)).count_ones() as usize,
            None => self.ones(),
        }
    }
    pub(crate) fn ones(&self) -> usize {
        self.before.last().copied().unwrap_or(0)
    }
}

/// One expanded primitive's selection.
pub(crate) struct SelectedPrimitive {
    /// Its kept triangles.
    pub triangles: Ranks,
    /// Vertices its kept triangles use.
    pub vertices: u64,
}

/// What [`select`] found: the kept triangles and the boxes capture checks them against.
pub(crate) struct Selection {
    /// Per expanded primitive, in scene order.
    pub primitives: Vec<SelectedPrimitive>,
    pub triangles: u64,
    pub geometry_bytes: u64,
    /// Post-node f64 source box of the kept triangles' corners.
    pub source_bounds: [[f64; 3]; 2],
    /// Their mapped, canonical f32 box: what the assembly decodes.
    pub bounds: Bounds,
    /// With a source CRS, the mapped box of every used vertex: the root cube a full bake
    /// derives. A local source's is the scan's own box.
    pub full_bounds: Option<Bounds>,
    pub report: MeshSubsetReport,
}

/// The published subset statement (`tilerReport.subset`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MeshSubsetReport {
    /// `bbox` or `polygon`.
    pub region: &'static str,
    /// `local` (post-node glTF x and -z, source metres) or `wgs84` (degrees).
    pub coordinates: &'static str,
    pub axes: [&'static str; 2],
    /// The bbox, or the polygons' bounding box: `[minx, miny, maxx, maxy]`.
    pub bounds: [f64; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polygons: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rings: Option<u64>,
    /// A triangle is kept when its centroid is inside (half-open; see the guide).
    pub rule: &'static str,
    pub source_triangles: u64,
    pub kept_triangles: u64,
    /// The kept triangles' corners in the region's coordinates: `[minx, miny, maxx, maxy]`.
    /// Triangles are not split, so this may reach past the region.
    pub extent: [f64; 4],
    /// The kept triangles' box in the tile frame (local metres, Y up): `[min, max]`.
    pub local_bounds: [[f32; 3]; 2],
}

/// A 2D box being grown.
#[derive(Clone, Copy)]
struct Extent([f64; 4]);
impl Extent {
    const EMPTY: Self = Self([
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ]);
    fn include(&mut self, [x, y]: [f64; 2]) {
        self.0 = [
            self.0[0].min(x),
            self.0[1].min(y),
            self.0[2].max(x),
            self.0[3].max(y),
        ];
    }
    fn union(&mut self, other: &Self) {
        self.0 = [
            self.0[0].min(other.0[0]),
            self.0[1].min(other.0[1]),
            self.0[2].max(other.0[2]),
            self.0[3].max(other.0[3]),
        ];
    }
}

fn include_source(bounds: &mut [[f64; 3]; 2], world: [f64; 3]) {
    for axis in 0..3 {
        bounds[0][axis] = bounds[0][axis].min(world[axis]);
        bounds[1][axis] = bounds[1][axis].max(world[axis]);
    }
}
fn union_source(bounds: &mut [[f64; 3]; 2], other: &[[f64; 3]; 2]) {
    for axis in 0..3 {
        bounds[0][axis] = bounds[0][axis].min(other[0][axis]);
        bounds[1][axis] = bounds[1][axis].max(other[1][axis]);
    }
}
const SOURCE_EMPTY: [[f64; 3]; 2] = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];

/// One triangle-test task's result.
struct Tested {
    words: Vec<u64>,
    kept: u64,
    source_bounds: [[f64; 3]; 2],
    /// Every tested centroid, for the empty-selection message.
    centroids: Extent,
}

/// One mapping task's result.
struct Mapped {
    full: Bounds,
    kept: Bounds,
    extent: Extent,
}

fn index(v: u64) -> Result<usize> {
    usize::try_from(v).map_err(|_| overflow())
}

fn set(bits: &[AtomicU64], vertex: usize) {
    bits[vertex / 64].fetch_or(1 << (vertex % 64), Relaxed);
}
fn bit(bits: &[AtomicU64], vertex: usize) -> bool {
    bits[vertex / 64].load(Relaxed) >> (vertex % 64) & 1 == 1
}

/// Runs `task` over `chunks` with a source frame ([`super::assembly::decode_chunks`]). A
/// failure is rerun in order on the calling thread, so the error is the first in input order
/// for every worker count.
fn chunks_in_order<T: Send>(
    frame: &mut SourceFrame,
    chunks: usize,
    workers: usize,
    poll: &mut dyn FnMut() -> Result<()>,
    task: impl Fn(usize, &mut SourceFrame, &mut dyn FnMut() -> Result<()>) -> Result<T> + Sync,
) -> Result<Vec<T>> {
    match super::assembly::decode_chunks(frame, 0..chunks, workers, poll, &task)? {
        Ran::All(found) => Ok(found),
        Ran::Failed(failure) => {
            match super::assembly::decode_chunks(frame, 0..chunks, 1, poll, &task)? {
                Ran::Failed(first) => Err(first),
                // A transient failure that does not recur still stands.
                Ran::All(_) => Err(failure),
            }
        }
    }
}

/// The region coordinates of a post-node source position.
fn region_point(frame: &mut SourceFrame, world: [f64; 3]) -> Result<[f64; 2]> {
    match frame {
        SourceFrame::Local => Ok([world[0], -world[2]]),
        SourceFrame::Crs(_) => frame.lon_lat(world),
    }
}

/// Tests every triangle of `p` against `subset`.
fn test_primitive(
    source: &StreamSource,
    p: &ScenePrimitive<'_>,
    subset: &MeshSubset,
    frame: &mut SourceFrame,
    buffer_bytes: usize,
    workers: usize,
    poll: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<Vec<Tested>> {
    let done = AtomicU64::new(0);
    let chunks = p.triangles.div_ceil(SELECT_TRIANGLES);
    chunks_in_order(
        frame,
        chunks,
        workers,
        &mut || poll(done.load(Relaxed)),
        |chunk, frame, check| {
            let start = chunk * SELECT_TRIANGLES;
            let end = (start + SELECT_TRIANGLES).min(p.triangles);
            let mut indices = p.indices.map(|v| source.reader(&v, buffer_bytes));
            let mut positions = source.reader(&p.position, buffer_bytes.min(POSITION_WINDOW));
            let mut out = Tested {
                words: vec![0; (end - start).div_ceil(64)],
                kept: 0,
                source_bounds: SOURCE_EMPTY,
                centroids: Extent::EMPTY,
            };
            for t in start..end {
                if t.is_multiple_of(1024) {
                    crate::test_hooks::pace("mesh.select");
                    check()?;
                }
                let mut corners = [[0.; 3]; 3];
                for (corner, v) in corners.iter_mut().zip(vertices(p, &mut indices, t)?) {
                    *corner = source_position(p, &mut positions, v)?;
                }
                let [a, b, c] = corners;
                let centroid = std::array::from_fn(|axis| (a[axis] + b[axis] + c[axis]) / 3.);
                let point = region_point(frame, centroid)?;
                out.centroids.include(point);
                if subset.contains(point) {
                    out.words[(t - start) / 64] |= 1 << ((t - start) % 64);
                    out.kept += 1;
                    for corner in corners {
                        include_source(&mut out.source_bounds, corner);
                    }
                }
            }
            done.fetch_add((end - start) as u64, Relaxed);
            Ok(out)
        },
    )
}

/// Marks the vertices `p`'s triangles use: those of kept triangles in `kept`, and with
/// `all`, every used one.
fn mark_vertices(
    source: &StreamSource,
    p: &ScenePrimitive<'_>,
    triangles: &Ranks,
    all: bool,
    buffer_bytes: usize,
    workers: usize,
    poll: &mut dyn FnMut() -> Result<()>,
) -> Result<(Vec<AtomicU64>, Option<Vec<AtomicU64>>)> {
    let words = p.position.count.div_ceil(64);
    let kept: Vec<AtomicU64> = (0..words).map(|_| AtomicU64::new(0)).collect();
    let used: Option<Vec<AtomicU64>> = all.then(|| (0..words).map(|_| AtomicU64::new(0)).collect());
    let ran = run(
        p.triangles.div_ceil(SELECT_TRIANGLES),
        workers,
        poll,
        |chunk, stop| {
            let start = chunk * SELECT_TRIANGLES;
            let end = (start + SELECT_TRIANGLES).min(p.triangles);
            let mut indices = p.indices.map(|v| source.reader(&v, buffer_bytes));
            for t in start..end {
                if t.is_multiple_of(1024) && stop.load(Relaxed) {
                    return Err(JobError::new(JobErrorKind::Cancelled, "selection stopped"));
                }
                let keep = triangles.contains(t);
                if !keep && used.is_none() {
                    continue;
                }
                for v in vertices(p, &mut indices, t)? {
                    let v = index(v)?;
                    if keep {
                        set(&kept, v);
                    }
                    if let Some(used) = &used {
                        set(used, v);
                    }
                }
            }
            Ok(())
        },
    )?;
    match ran {
        Ran::All(_) => Ok((kept, used)),
        Ran::Failed(failure) => Err(failure),
    }
}

/// Maps the marked vertices of `p`: the `kept` ones into the kept box (and, with a CRS, their
/// geographic extent), and with `used`, every used one into the full box.
fn map_vertices(
    source: &StreamSource,
    p: &ScenePrimitive<'_>,
    frame: &mut SourceFrame,
    (kept, used): (&[AtomicU64], Option<&[AtomicU64]>),
    buffer_bytes: usize,
    workers: usize,
    poll: &mut dyn FnMut() -> Result<()>,
) -> Result<Vec<Mapped>> {
    let count = p.position.count;
    chunks_in_order(
        frame,
        count.div_ceil(SELECT_VERTICES),
        workers,
        poll,
        |chunk, frame, check| {
            let start = chunk * SELECT_VERTICES;
            let end = (start + SELECT_VERTICES).min(count);
            let mut positions = source.reader(&p.position, buffer_bytes.min(POSITION_WINDOW));
            let mut out = Mapped {
                full: Bounds::EMPTY,
                kept: Bounds::EMPTY,
                extent: Extent::EMPTY,
            };
            for v in start..end {
                if v.is_multiple_of(4096) {
                    check()?;
                }
                let keep = bit(kept, v);
                if !keep && !used.is_some_and(|used| bit(used, v)) {
                    continue;
                }
                let world = source_position(p, &mut positions, v as u64)?;
                let (position, lon_lat) = frame.locate(world)?;
                let position = canonical(position);
                out.full.include(position);
                if keep {
                    out.kept.include(position);
                    if let Some(point) = lon_lat {
                        out.extent.include(point);
                    }
                }
            }
            Ok(out)
        },
    )
}

/// Selects the triangles of `source` whose centroid lies in `subset`; see the module notes.
/// `frame` is the capture's resolved frame. Reports [`Step::Select`]: every triangle tested,
/// then every POSITION element of each expanded primitive marked or mapped.
pub(crate) fn select(
    source: &StreamSource,
    frame: &mut SourceFrame,
    subset: &MeshSubset,
    buffer_bytes: usize,
    workers: usize,
    progress: &StepProgress<'_>,
) -> Result<Selection> {
    let primitives: Vec<ScenePrimitive<'_>> = source.document.scene_primitives().collect();
    let crs = matches!(frame, SourceFrame::Crs(_));
    let mut total = 0u64;
    let mut source_triangles = 0u64;
    for p in &primitives {
        let (t, v) = (p.triangles as u64, p.position.count as u64);
        source_triangles = source_triangles.checked_add(t).ok_or_else(overflow)?;
        total = total
            .checked_add(t)
            .and_then(|n| n.checked_add(v))
            .ok_or_else(overflow)?;
    }
    progress.begin(Step::Select, total)?;
    let mut done = 0u64;
    let mut selected = Vec::with_capacity(primitives.len());
    let (mut triangles, mut geometry_bytes) = (0u64, 0u64);
    let mut source_bounds = SOURCE_EMPTY;
    let mut centroids = Extent::EMPTY;
    let mut bounds = Bounds::EMPTY;
    let mut full = Bounds::EMPTY;
    let mut extent = Extent::EMPTY;
    for p in &primitives {
        let tested = test_primitive(source, p, subset, frame, buffer_bytes, workers, &mut |n| {
            progress.advance(done + n)
        })?;
        let mut words = Vec::with_capacity(p.triangles.div_ceil(64));
        let mut kept = 0u64;
        for task in tested {
            words.extend_from_slice(&task.words);
            kept += task.kept;
            union_source(&mut source_bounds, &task.source_bounds);
            centroids.union(&task.centroids);
        }
        let ranks = Ranks::new(words);
        done += p.triangles as u64;
        progress.advance(done)?;
        triangles = triangles.checked_add(kept).ok_or_else(overflow)?;
        geometry_bytes = kept
            .checked_mul(u64::from(triangle_bytes(p)))
            .and_then(|b| b.checked_add(geometry_bytes))
            .ok_or_else(overflow)?;
        let mut used_vertices = 0u64;
        if kept > 0 || crs {
            let (kept_vertices, used) =
                mark_vertices(source, p, &ranks, crs, buffer_bytes, workers, &mut || {
                    progress.check()
                })?;
            used_vertices = kept_vertices
                .iter()
                .map(|w| u64::from(w.load(Relaxed).count_ones()))
                .sum();
            for task in map_vertices(
                source,
                p,
                frame,
                (&kept_vertices, used.as_deref()),
                buffer_bytes,
                workers,
                &mut || progress.check(),
            )? {
                full.union(&task.full);
                bounds.union(&task.kept);
                extent.union(&task.extent);
            }
        }
        done += p.position.count as u64;
        progress.advance(done)?;
        selected.push(SelectedPrimitive {
            triangles: ranks,
            vertices: used_vertices,
        });
    }
    progress.end()?;
    let region = subset.bounds();
    if triangles == 0 {
        let [x0, y0, x1, y1] = centroids.0;
        return Err(request(format!(
            "the subset region [{}, {}, {}, {}] keeps no source triangle: the {source_triangles} triangle centroids span x {x0} to {x1} and y {y0} to {y1} in the region's coordinates ({})",
            region[0],
            region[1],
            region[2],
            region[3],
            if crs {
                "WGS84 longitude and latitude, degrees"
            } else {
                "post-node glTF x and -z, source metres"
            },
        )));
    }
    if !crs {
        // A local source's region axes are post-node x and -z.
        let [min, max] = source_bounds;
        extent = Extent([min[0], -max[2], max[0], -min[2]]);
    }
    let (polygons, rings) = match &subset.region {
        Region::Bbox(_) => (None, None),
        Region::Polygon(polygons) => (Some(polygons.polygons), Some(polygons.rings)),
    };
    let report = MeshSubsetReport {
        region: match subset.region {
            Region::Bbox(_) => "bbox",
            Region::Polygon(_) => "polygon",
        },
        coordinates: if crs { "wgs84" } else { "local" },
        axes: if crs {
            ["longitude", "latitude"]
        } else {
            ["x", "-z"]
        },
        bounds: region,
        polygons,
        rings,
        rule: "centroid",
        source_triangles,
        kept_triangles: triangles,
        extent: extent.0,
        local_bounds: [bounds.min, bounds.max],
    };
    Ok(Selection {
        primitives: selected,
        triangles,
        geometry_bytes,
        source_bounds,
        bounds,
        full_bounds: crs.then_some(full),
        report,
    })
}

impl Selection {
    /// Whether the assembly's decoded boxes are the selection's.
    pub(crate) fn check_decoded(&self, source: [[f64; 3]; 2], mapped: Bounds) -> Result<()> {
        if source != self.source_bounds || mapped != self.bounds {
            return Err(changed());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "subset_tests.rs"]
mod tests;
