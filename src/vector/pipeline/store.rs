//! Disk-backed feature/lock indexing, deterministic spatial partitions and LOD.
use super::*;
use crate::runtime::{Attempt, RunEvent};
use crate::tileset_node::{
    top_level_error, GeometricError, NodePlacement, Refine, TileBox, TileRecord, TileTree,
    TilesetDocument, TilesetExtension, EMPTY_PARENT_SCALE,
};
use encoding::Candidate;
use rayon::prelude::*;
use records::{VectorRecord, VectorTileExtras, VectorTree};
use reuse::{Cut, Reuse};
use rusqlite::{params, Connection};
use source::Reader;
use std::{
    cell::{Cell, RefCell},
    io::Write,
    path::PathBuf,
    time::Instant,
};

fn cleanup_failure(original: FeatureFailure, cleanup: Error) -> Error {
    // A rejection is provisional, not a job abort cause. If it cannot be
    // rolled back safely, infrastructure failure is the first fatal cause.
    let (primary, secondary) = match original {
        FeatureFailure::Rejected(rejection) => (cleanup, Error::Data(rejection.message)),
        FeatureFailure::Fatal(error) => (error, cleanup),
    };
    let mut failure = match primary {
        Error::Job(failure) => failure,
        other => crate::JobFailure {
            error: super::super::job_error(other),
            secondary: Vec::new(),
            retained_paths: Vec::new(),
            recovery: None,
        },
    };
    failure.secondary.push(super::super::job_error(secondary));
    Error::Job(failure)
}

/// Open the private scratch spool. Feature admission rolls back savepoints, so
/// the journal must support ROLLBACK; an in-memory journal does, without the
/// journal-file writes. The spool is never reopened after a failed attempt, so
/// it needs neither crash durability (`synchronous=OFF`) nor an on-disk journal.
/// The in-memory savepoint sub-journal holds only pages that one source
/// feature modifies, and RELEASE frees it.
fn open_spool(path: &std::path::Path) -> Result<Connection, Error> {
    let db = Connection::open(path).map_err(sql)?;
    db.execute_batch("PRAGMA journal_mode=MEMORY; PRAGMA synchronous=OFF; PRAGMA temp_store=FILE; PRAGMA cache_size=-32768;
        CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,n INTEGER,estimate INTEGER,x REAL,y REAL,z REAL,lx REAL,ly REAL,lz REAL,hx REAL,hy REAL,hz REAL,fingerprint TEXT,sortkey TEXT,omissible INTEGER);
        CREATE INDEX paths ON features(path);
        CREATE TABLE vertices(x REAL,y REAL,z REAL,owner INTEGER,shared INTEGER DEFAULT 0,PRIMARY KEY(x,y,z)) WITHOUT ROWID;").map_err(sql)?;
    Ok(db)
}

/// After admission nothing rolls back: every later SQL failure is fatal and
/// discards the spool. An in-memory journal would then hold an image of every
/// page the fingerprint and partition updates touch, so drop the journal.
fn end_admission(db: &Connection) -> Result<(), Error> {
    let mode: String = db
        .query_row("PRAGMA journal_mode=OFF", [], |r| r.get(0))
        .map_err(sql)?;
    if mode != "off" {
        return Err(Error::Environment(format!(
            "vector scratch spool kept journal mode {mode} after admission"
        )));
    }
    Ok(())
}

fn emit(attempt: &Attempt, event: RunEvent<'_>) -> Result<(), Error> {
    attempt
        .emit(&event)
        .map_err(|error| Error::Job(attempt.fail(error)))
}

fn final_members(output: &Path) -> Result<Vec<VectorMember>, Error> {
    // The workspace contains only finalized producer resources; scratch stays
    // in a separately owned temporary directory.
    fn visit(root: &Path, directory: &Path, out: &mut Vec<VectorMember>) -> Result<(), Error> {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                visit(root, &path, out)?;
            } else {
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(VectorMember { name, path });
            }
        }
        Ok(())
    }
    let mut members = Vec::new();
    visit(output, output, &mut members)?;
    members.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(members)
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Counters {
    pub features: usize,
    pub fragments: usize,
    pub leaf_tiles: usize,
    pub tiles: usize,
    pub routing_tiles: usize,
    pub maximum_tile_vertices: usize,
    pub maximum_tile_bytes: usize,
    pub fragmented_polygons: usize,
    pub skipped_features: usize,
    pub features_without_geometry: usize,
}
/// A built subtree of H, with the z-up centre of its original points, the largest content
/// padding within it and the source fragments its full-detail leaves hold.
pub(super) struct Node {
    pub tree: VectorTree,
    pub center: Point,
    pub padding: f64,
    pub fragments: usize,
}
struct Reports {
    file: Box<dyn Write>,
    first: Vec<Value>,
    count: usize,
}
impl Reports {
    fn finish(&mut self) -> Result<(), Error> {
        self.file.flush()?;
        Ok(())
    }
    fn write(&mut self, value: Value) -> Result<(), Error> {
        let bytes = serde_json::to_vec(&value)?;
        self.file.write_all(&bytes)?;
        self.file.write_all(b"\n")?;
        self.count += 1;
        if self.first.len() < 100 {
            self.first.push(value);
        }
        Ok(())
    }
}

pub(super) fn split(
    feature: &Feature,
    repair: bool,
    counters: &mut Counters,
    reports: &mut Vec<Value>,
) -> FeatureResult<Vec<Feature>> {
    let mut geometries = Vec::new();
    match &feature.geometry {
        Geometry::LineString(p) if p.len() > 2 => {
            let middle = p.len() / 2;
            geometries.extend([
                Geometry::LineString(p[..=middle].to_vec()),
                Geometry::LineString(p[middle..].to_vec()),
            ]);
        }
        Geometry::MultiPoint(p) if p.len() > 1 => {
            let middle = p.len() / 2;
            geometries.extend([
                Geometry::MultiPoint(p[..middle].to_vec()),
                Geometry::MultiPoint(p[middle..].to_vec()),
            ]);
        }
        Geometry::MultiLineString(p) if p.len() > 1 => {
            let middle = p.len() / 2;
            geometries.extend([
                Geometry::MultiLineString(p[..middle].to_vec()),
                Geometry::MultiLineString(p[middle..].to_vec()),
            ]);
        }
        Geometry::MultiPolygon(p) if p.len() > 1 => {
            let middle = p.len() / 2;
            geometries.extend([
                Geometry::MultiPolygon(p[..middle].to_vec()),
                Geometry::MultiPolygon(p[middle..].to_vec()),
            ]);
        }
        Geometry::MultiLineString(p) if p.len() == 1 => {
            let mut f = feature.clone();
            f.geometry = Geometry::LineString(p[0].clone());
            return split(&f, repair, counters, reports);
        }
        Geometry::MultiPolygon(p) if p.len() == 1 => {
            if feature.surface_fragment {
                return Err(FeatureFailure::reject(
                    "indivisible polygon fill/boundary or metadata exceeds tile budget",
                ));
            }
            let mut f = feature.clone();
            f.geometry = Geometry::Polygon(p[0].clone());
            if let Some(intrinsic) = &mut f.intrinsic {
                let Geometry::MultiPolygon(source) = &intrinsic.geometry else {
                    return Err(FeatureFailure::reject(
                        "intrinsic polygon correspondence was lost",
                    ));
                };
                intrinsic.geometry = Geometry::Polygon(source[0].clone());
            }
            return split(&f, repair, counters, reports);
        }
        Geometry::Polygon(rings) => {
            let polygon = geometry::polygon_for(feature, rings, 0, [0.; 3], repair)?;
            let triangles = polygon.indices.as_chunks::<3>().0.to_vec();
            if triangles.len() <= 1 {
                return Err(FeatureFailure::reject(
                    "one triangle or its metadata exceeds tile budget",
                ));
            }
            let mut parts = Vec::new();
            let mut source_parts = Vec::new();
            for tri in triangles {
                let points: Vec<_> = tri.iter().map(|i| polygon.positions[*i as usize]).collect();
                let mut ring = points.clone();
                ring.push(points[0]);
                parts.push(vec![ring]);
                if let Some(source) = &polygon.source_positions {
                    let mut source_ring: Vec<_> = tri.iter().map(|i| source[*i as usize]).collect();
                    source_ring.push(source_ring[0]);
                    source_parts.push(vec![source_ring]);
                }
            }
            // The outline comes from the source loops, not triangle
            // incidence: dropping a zero-area seam face must not expose its
            // interior edges or lose its original (collapsed) boundary edge.
            // It is its own locked line feature of closed rings, so the line
            // splitter cuts an oversized ring into contiguous runs placed
            // where they lie, and tiles chain them back (see `outline`).
            let outline = outline::source_rings(&polygon.positions, &polygon.loops);
            counters.fragmented_polygons += 1;
            let mut report = polygon.report;
            report["sourceId"] = feature.source_id().clone();
            report["sourceLayer"] = json!(feature.layer());
            report["reason"]=json!("oversized polygon surfaces partitioned; original boundary rendered separately without internal edges");
            reports.push(report);
            let middle = parts.len() / 2;
            let mut fragments: Vec<_> = [(0, middle), (middle, parts.len())]
                .into_iter()
                .map(|(a, b)| {
                    let mut f = feature.clone();
                    f.geometry = Geometry::MultiPolygon(parts[a..b].to_vec());
                    if let Some(intrinsic) = &mut f.intrinsic {
                        intrinsic.geometry = Geometry::MultiPolygon(source_parts[a..b].to_vec());
                    }
                    f.surface_fragment = true;
                    f
                })
                .collect();
            if !outline.is_empty() {
                let mut f = feature.clone();
                f.geometry = Geometry::MultiLineString(outline);
                f.intrinsic = None;
                f.surface_fragment = true;
                fragments.push(f);
            }
            return Ok(fragments);
        }
        _ => return Err(FeatureFailure::reject(
            "indivisible geometry or its metadata exceeds tile budget; raise maxVertices/maxBytes",
        )),
    }
    let parts: Vec<_> = geometries
        .into_iter()
        .enumerate()
        .map(|(index, geometry)| {
            let mut f = feature.clone();
            f.geometry = geometry;
            if let Some(intrinsic) = &mut f.intrinsic {
                if let Geometry::MultiPolygon(source) = &intrinsic.geometry {
                    let middle = source.len() / 2;
                    intrinsic.geometry = Geometry::MultiPolygon(if index == 0 {
                        source[..middle].to_vec()
                    } else {
                        source[middle..].to_vec()
                    });
                }
            }
            f
        })
        .collect();
    Ok(parts)
}
/// Spool writer: features enter the disk index, oversized ones split first.
#[derive(Clone, Copy)]
struct Spool<'a> {
    db: &'a Connection,
    repair: bool,
    options: &'a VectorOptions,
    policy: encoding::ProducerPolicy,
    attempt: &'a Attempt,
}
impl Spool<'_> {
    fn insert(
        self,
        feature: Feature,
        path: &str,
        hint: bool,
        counters: &mut Counters,
        reports: &mut Vec<Value>,
    ) -> FeatureResult<()> {
        let Self {
            db,
            repair,
            options,
            policy,
            attempt,
        } = self;
        attempt.check().map_err(|cause| {
            encoding::codec_failure(
                crate::vector_encoding::CodecError::Checkpoint(cause),
                attempt,
            )
        })?;
        if hint
            && (feature.geometry.size() > options.max_vertices
                || feature.estimate() > options.max_bytes
                || !encoding::fits_feature(&feature, repair, options, policy, attempt)?)
        {
            for (index, mut part) in split(&feature, repair, counters, reports)?
                .into_iter()
                .enumerate()
            {
                part.fragment_path = format!("{}{index}", feature.fragment_path);
                self.insert(part, path, true, counters, reports)?;
            }
            return Ok(());
        }
        insert_row(db, &feature, path)?;
        Ok(())
    }
}
// This transaction owns SQL and delta reports while validating one feature;
// codec policy and the original Attempt arrive from the conversion boundary.
#[allow(clippy::too_many_arguments)]
fn accept_feature(
    db: &Connection,
    mut feature: Feature,
    repair: bool,
    ambiguous: bool,
    options: &VectorOptions,
    policy: encoding::ProducerPolicy,
    attempt: &Attempt,
    counters: &mut Counters,
    reports: &mut Reports,
) -> FeatureResult<()> {
    attempt.check().map_err(|cause| {
        encoding::codec_failure(
            crate::vector_encoding::CodecError::Checkpoint(cause),
            attempt,
        )
    })?;
    db.execute_batch("SAVEPOINT feature;").map_err(sql)?;
    let mut delta_counters = Counters::default();
    let mut delta_reports = Vec::new();
    let result = (|| -> FeatureResult<()> {
        delta_reports.extend(geometry::validate(&mut feature, repair, ambiguous)?);
        Spool {
            db,
            repair,
            options,
            policy,
            attempt,
        }
        .insert(feature, "", true, &mut delta_counters, &mut delta_reports)?;
        Ok(())
    })();
    if let Err(original) = result {
        if let Err(cleanup) = db.execute_batch("ROLLBACK TO feature; RELEASE feature;") {
            return Err(FeatureFailure::Fatal(cleanup_failure(
                original,
                sql(cleanup),
            )));
        }
        return Err(original);
    }
    // Required diagnostics are committed only after feature work succeeds.
    // Any failure here aborts the entire private workspace.
    for report in delta_reports {
        if let Err(error) = reports.write(report) {
            let original = FeatureFailure::Fatal(error);
            if let Err(cleanup) = db.execute_batch("ROLLBACK TO feature; RELEASE feature;") {
                return Err(FeatureFailure::Fatal(cleanup_failure(
                    original,
                    sql(cleanup),
                )));
            }
            return Err(original);
        }
    }
    db.execute_batch("RELEASE feature;").map_err(sql)?;
    counters.features += 1;
    counters.fragmented_polygons += delta_counters.fragmented_polygons;
    Ok(())
}

fn insert_row(db: &Connection, feature: &Feature, path: &str) -> Result<(), Error> {
    let (lo, hi) = bounds(feature.geometry.points())?;
    let center = mul(add(lo, hi), 0.5);
    let bytes = canonical(feature)?;
    let data = String::from_utf8(bytes).unwrap();
    // Coarse tiles may omit whole unsplit lines and polygons (`encoding::encode`).
    let omissible = feature.fragment_path.is_empty()
        && !feature.surface_fragment
        && !matches!(
            feature.geometry,
            Geometry::Point(_) | Geometry::MultiPoint(_)
        );
    db.execute("INSERT INTO features(path,data,n,estimate,x,y,z,lx,ly,lz,hx,hy,hz,sortkey,omissible) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![path,data,feature.geometry.size() as i64,feature.estimate() as i64,center[0],center[1],center[2],lo[0],lo[1],lo[2],hi[0],hi[1],hi[2],format!("{}\0{}\0{}",feature.layer(),feature.source_id().as_str().unwrap(),feature.fragment_path),omissible]).map_err(sql)?;
    let owner = db.last_insert_rowid();
    let mut vertex=db.prepare_cached("INSERT INTO vertices(x,y,z,owner) VALUES(?1,?2,?3,?4) ON CONFLICT(x,y,z) DO UPDATE SET shared=shared OR owner!=excluded.owner").map_err(sql)?;
    for point in feature.geometry.points() {
        vertex
            .execute(params![point[0], point[1], point[2], owner])
            .map_err(sql)?;
    }
    Ok(())
}
/// A row's reuse fingerprint: its data, which vertices are locked and, once
/// a quantized build has surveyed them, its shared vertices' exponents.
fn fingerprint(db: &Connection, id: i64, value: &str, grid: bool) -> Result<(), Error> {
    let feature: Feature = serde_json::from_str(value)?;
    let points: Vec<_> = feature.geometry.points().collect();
    let mut mask = vec![0; points.len().div_ceil(8)];
    let mut query = db
        .prepare_cached("SELECT shared FROM vertices WHERE x=?1 AND y=?2 AND z=?3")
        .map_err(sql)?;
    for (i, p) in points.iter().enumerate() {
        if feature.surface_fragment
            || query
                .query_row(params![p[0], p[1], p[2]], |r| r.get::<_, i32>(0))
                .map_err(sql)?
                != 0
        {
            mask[i / 8] |= 1 << (7 - i % 8);
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher.update(mask);
    if grid {
        let mut exponents = db
            .prepare_cached("SELECT kind,prefix,level,ex,ey,ez,fl,al FROM grid WHERE cx=?1 AND cy=?2 AND cz=?3 ORDER BY kind,prefix,level")
            .map_err(sql)?;
        for p in &points {
            let p = encoding::cell(**p);
            let held: Vec<(i64, String, u32, [i32; 3], bool, bool)> = exponents
                .query_map(params![p[0], p[1], p[2]], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        [r.get(3)?, r.get(4)?, r.get(5)?],
                        r.get(6)?,
                        r.get(7)?,
                    ))
                })
                .map_err(sql)?
                .collect::<Result<_, _>>()
                .map_err(sql)?;
            hasher.update(serde_json::to_vec(&held)?);
        }
    }
    db.execute(
        "UPDATE features SET fingerprint=?1 WHERE id=?2",
        params![format!("{:x}", hasher.finalize()), id],
    )
    .map_err(sql)?;
    Ok(())
}
/// Whether tile `a` (prefix, level) is an ancestor of tile `b`: a partition
/// node is above every tile under its prefix, and an LOD chain's coarser
/// levels are above its finer ones at the same prefix.
fn ancestor(a: &(String, u32), b: &(String, u32)) -> bool {
    b.0.starts_with(&a.0) && (b.0.len() > a.0.len() || b.1 < a.1)
}

/// The survey's record of shared vertices (`holders`: per vertex cell and
/// kind, each tile holding it and that tile's own u16 and float32
/// exponents) and the resolved grid (`grid`: each holder's snapping
/// exponents, whether the tile stores float32, and whether no other tile
/// that can render with it holds the vertex).
const GRID_TABLES: &str = "CREATE TABLE holders(cx INTEGER,cy INTEGER,cz INTEGER,kind INTEGER,prefix TEXT,level INTEGER,ex INTEGER,ey INTEGER,ez INTEGER,fx INTEGER,fy INTEGER,fz INTEGER,PRIMARY KEY(cx,cy,cz,kind,prefix,level)) WITHOUT ROWID; CREATE TABLE grid(cx INTEGER,cy INTEGER,cz INTEGER,kind INTEGER,prefix TEXT,level INTEGER,ex INTEGER,ey INTEGER,ez INTEGER,fl INTEGER,al INTEGER,PRIMARY KEY(cx,cy,cz,kind,prefix,level)) WITHOUT ROWID;";

type Tile = (String, u32);

/// Calls `visit` with each shared vertex (cell and kind) and its holders'
/// co-rendering groups: holders that can render together (neither is the
/// other's ancestor) are joined, transitively.
fn vertex_groups(
    db: &Connection,
    mut visit: impl FnMut([i64; 4], Vec<Vec<Tile>>) -> Result<(), Error>,
) -> Result<(), Error> {
    fn groups(held: &[Tile]) -> Vec<Vec<Tile>> {
        let n = held.len();
        let mut group: Vec<usize> = (0..n).collect();
        fn root(group: &mut [usize], mut i: usize) -> usize {
            while group[i] != i {
                group[i] = group[group[i]];
                i = group[i];
            }
            i
        }
        for a in 0..n {
            for b in a + 1..n {
                if !ancestor(&held[a], &held[b]) && !ancestor(&held[b], &held[a]) {
                    let (ra, rb) = (root(&mut group, a), root(&mut group, b));
                    group[ra] = rb;
                }
            }
        }
        let mut out: BTreeMap<usize, Vec<Tile>> = BTreeMap::new();
        for (i, tile) in held.iter().enumerate() {
            out.entry(root(&mut group, i))
                .or_default()
                .push(tile.clone());
        }
        out.into_values().collect()
    }
    let mut stmt = db
        .prepare(
            "SELECT cx,cy,cz,kind,prefix,level FROM holders ORDER BY cx,cy,cz,kind,prefix,level",
        )
        .map_err(sql)?;
    let mut rows = stmt.query([]).map_err(sql)?;
    let mut current: Option<[i64; 4]> = None;
    let mut held: Vec<Tile> = Vec::new();
    while let Some(r) = rows.next().map_err(sql)? {
        let p: [i64; 4] = [
            r.get(0).map_err(sql)?,
            r.get(1).map_err(sql)?,
            r.get(2).map_err(sql)?,
            r.get(3).map_err(sql)?,
        ];
        if let Some(c) = current.filter(|c| *c != p) {
            visit(c, groups(&held))?;
            held.clear();
        }
        current = Some(p);
        held.push((r.get(4).map_err(sql)?, r.get(5).map_err(sql)?));
    }
    if let Some(c) = current {
        visit(c, groups(&held))?;
    }
    Ok(())
}

/// Resolves the survey into `grid`. Each co-rendering group of a shared
/// vertex must decode it to one point, a multiple of every member's step.
/// A tile whose own u16 step is coarser, on any axis, than another member's
/// in any of its groups stores float32 (2^8 times finer); the u16 members of
/// a group then share one step. Each group snaps at the largest step its
/// members can hold, per axis: the u16 members' own, or a float32 member's.
fn resolve_grid(db: &Connection) -> Result<(), Error> {
    let mut own: BTreeMap<Tile, encoding::OwnExponents> = BTreeMap::new();
    {
        let mut stmt = db
            .prepare("SELECT DISTINCT prefix,level,ex,ey,ez,fx,fy,fz FROM holders")
            .map_err(sql)?;
        let mut rows = stmt.query([]).map_err(sql)?;
        while let Some(r) = rows.next().map_err(sql)? {
            let get = |i: usize| r.get::<_, i32>(i).map_err(sql);
            let exponents = encoding::OwnExponents {
                u16: [get(2)?, get(3)?, get(4)?],
                float: [get(5)?, get(6)?, get(7)?],
            };
            // One encoding per tile: its rows agree on its exponents.
            if own
                .insert((r.get(0).map_err(sql)?, r.get(1).map_err(sql)?), exponents)
                .is_some_and(|other| other != exponents)
            {
                return Err(data(
                    "quantization survey recorded two exponents for one tile",
                ));
            }
        }
    }
    let mut float: BTreeSet<Tile> = BTreeSet::new();
    vertex_groups(db, |_, groups| {
        for group in groups {
            for tile in &group {
                let e = own[tile].u16;
                if group
                    .iter()
                    .any(|other| (0..3).any(|i| own[other].u16[i] < e[i]))
                {
                    float.insert(tile.clone());
                }
            }
        }
        Ok(())
    })?;
    db.execute_batch("BEGIN;").map_err(sql)?;
    {
        let mut insert = db
            .prepare_cached(
                "INSERT INTO grid(cx,cy,cz,kind,prefix,level,ex,ey,ez,fl,al) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            )
            .map_err(sql)?;
        vertex_groups(db, |p, groups| {
            for group in groups {
                let finest = |tile: &Tile| match float.contains(tile) {
                    true => own[tile].float,
                    false => own[tile].u16,
                };
                let e: [i32; 3] = std::array::from_fn(|i| {
                    group
                        .iter()
                        .map(|t| finest(t)[i])
                        .max()
                        .expect("a group has a member")
                });
                for tile in &group {
                    insert
                        .execute(params![
                            p[0],
                            p[1],
                            p[2],
                            p[3],
                            tile.0,
                            tile.1,
                            e[0],
                            e[1],
                            e[2],
                            float.contains(tile),
                            group.len() == 1
                        ])
                        .map_err(sql)?;
                }
            }
            Ok(())
        })?;
    }
    db.execute_batch("COMMIT;").map_err(sql)?;
    Ok(())
}

/// The refusals the final pass replays for `levels` of one tile, from the
/// survey's: a refused level stays refused, and a refused full-detail leaf
/// (level 0) refuses its coarse levels too, as they never joined the
/// hierarchy.
/// A parent's tolerance is at least its data diagonal over this ratio: under CesiumJS's
/// default maximum screen-space error of 16 it refines when about 16 x this many pixels.
const PARENT_EXTENT_PER_TOLERANCE: f64 = 64.;

/// The coarsest tolerance among the content tiles that render first below `tree`.
fn top_tolerance(tree: &VectorTree) -> f64 {
    match tree
        .record()
        .extras()
        .and_then(VectorTileExtras::statistics)
    {
        Some(statistics) if !tree.record().contents().is_empty() => statistics.tolerance_metres,
        _ => tree.children().iter().map(top_tolerance).fold(0., f64::max),
    }
}

fn replayed(levels: &[u32], survey: &[Option<&'static str>]) -> Vec<Option<&'static str>> {
    let leaf = levels.iter().position(|l| *l == 0).and_then(|i| survey[i]);
    survey.iter().map(|refusal| refusal.or(leaf)).collect()
}

struct Stats {
    count: usize,
    vertices: usize,
    estimate: usize,
    lo: Point,
    hi: Point,
}
struct Build<'a> {
    attempt: &'a Attempt,
    db: &'a Connection,
    spool: &'a Path,
    output: &'a Path,
    options: &'a VectorOptions,
    policy: encoding::ProducerPolicy,
    schemas: &'a BTreeMap<String, String>,
    max_features: usize,
    repair: bool,
    pool: rayon::ThreadPool,
    counters: Counters,
    reports: Reports,
    reuse: Reuse,
    workers: BTreeSet<usize>,
    partition_seconds: f64,
    /// The pass of a quantized build (see `encoding::lattice`).
    pass: encoding::GridPass,
    /// Survey outcome of every candidate, by (prefix, level): accepted, or
    /// its reason. The final pass replays it, so the hierarchy is the same.
    outcomes: BTreeMap<(String, u32), Option<&'static str>>,
}
impl Build<'_> {
    fn stats(&self, prefix: &str) -> Result<Stats, Error> {
        self.db.query_row("SELECT COUNT(*),COALESCE(SUM(n),0),COALESCE(SUM(estimate),0),COALESCE(MIN(lx),0),COALESCE(MIN(ly),0),COALESCE(MIN(lz),0),COALESCE(MAX(hx),0),COALESCE(MAX(hy),0),COALESCE(MAX(hz),0) FROM features WHERE path>=?1 AND path<?2",params![prefix,format!("{prefix}~")],|r|Ok(Stats{count:r.get::<_,i64>(0)? as usize,vertices:r.get::<_,i64>(1)? as usize,estimate:r.get::<_,i64>(2)? as usize,lo:[r.get(3)?,r.get(4)?,r.get(5)?],hi:[r.get(6)?,r.get(7)?,r.get(8)?]})).map_err(sql)
    }
    fn candidates(
        &mut self,
        prefix: &str,
        center: Point,
        levels: &[(u32, f64)],
    ) -> Result<Vec<Candidate>, Error> {
        let tolerance: BTreeMap<u32, f64> = levels.iter().copied().collect();
        let levels: Vec<u32> = levels.iter().map(|(level, _)| *level).collect();
        let levels = levels.as_slice();
        let encoder = encoding::Encoder {
            spool: self.spool,
            max_features: self.max_features,
            repair: self.repair,
            options: self.options,
            schemas: self.schemas,
            output: self.output,
            policy: self.policy,
            attempt: self.attempt,
            pass: self.pass,
        };
        // The final pass does not re-encode what the survey refused.
        let refusals: Vec<Option<&'static str>> = levels
            .iter()
            .map(|level| match self.pass {
                encoding::GridPass::Final => self
                    .outcomes
                    .get(&(prefix.to_string(), *level))
                    .copied()
                    .flatten(),
                _ => None,
            })
            .collect();
        let refusals = replayed(levels, &refusals);
        let refused = |level: &u32| refusals[levels.iter().position(|l| l == level).unwrap()];
        let todo: Vec<u32> = levels
            .iter()
            .filter(|level| refused(level).is_none())
            .copied()
            .collect();
        let values: Vec<Result<Candidate, Error>> = self.pool.install(|| {
            todo.par_iter()
                .map(|level| encoding::encode(encoder, prefix, center, *level, tolerance[level]))
                .collect()
        });
        let mut encoded = values
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .into_iter();
        self.workers
            .extend(encoded.as_slice().iter().map(|candidate| candidate.worker));
        let mut values = Vec::with_capacity(levels.len());
        for level in levels {
            let candidate = match refused(level) {
                Some(reason) => Candidate {
                    node: None,
                    reports: Vec::new(),
                    reason: Some(reason),
                    worker: 0,
                    survey: None,
                },
                None => encoded.next().expect("one encoding per unrefused level"),
            };
            if self.pass == encoding::GridPass::Survey {
                let outcome = if candidate.node.is_some() {
                    None
                } else {
                    Some(candidate.reason.unwrap_or("refused"))
                };
                self.outcomes.insert((prefix.to_string(), *level), outcome);
            }
            values.push(candidate);
        }
        Ok(values)
    }
    fn consume(
        &mut self,
        candidate: Candidate,
    ) -> Result<(Option<VectorRecord>, Option<&'static str>), Error> {
        if candidate.node.is_some() {
            if self.pass == encoding::GridPass::Survey {
                // Each shared vertex records the tiles that hold it and their
                // exponents; reports are written by the final pass.
                if let Some(survey) = &candidate.survey {
                    self.db.execute_batch("SAVEPOINT holders;").map_err(sql)?;
                    let mut insert = self.db.prepare_cached("INSERT OR REPLACE INTO holders(cx,cy,cz,kind,prefix,level,ex,ey,ez,fx,fy,fz) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)").map_err(sql)?;
                    let (e, f) = (survey.exponent.u16, survey.exponent.float);
                    for (p, kind) in &survey.shared {
                        insert
                            .execute(params![
                                p[0],
                                p[1],
                                p[2],
                                *kind as i64,
                                survey.prefix,
                                survey.level,
                                e[0],
                                e[1],
                                e[2],
                                f[0],
                                f[1],
                                f[2]
                            ])
                            .map_err(sql)?;
                    }
                    drop(insert);
                    self.db.execute_batch("RELEASE holders;").map_err(sql)?;
                }
            } else {
                for report in candidate.reports {
                    self.reports.write(report)?;
                }
            }
        }
        Ok((candidate.node, candidate.reason))
    }
    /// Place `children` under `record`, whose own content covers `stats` around `center`:
    /// each child at its translation from this tile's centre, the box padded and grown to
    /// enclose every child box, and the error the largest of its own and its children's.
    fn attach(
        &mut self,
        mut record: VectorRecord,
        stats: &Stats,
        center: Point,
        mut children: Vec<Node>,
    ) -> Result<Node, Error> {
        let zcenter = zup(center);
        let dimensions = sub(stats.hi, stats.lo);
        let half = [
            dimensions[0] * 0.5,
            dimensions[2] * 0.5,
            dimensions[1] * 0.5,
        ]
        .map(|v| v.max(0.001));
        let mut padding = record.extras().map_or(0., VectorTileExtras::padding);
        for child in &children {
            padding = padding.max(child.padding);
        }
        let mut bounds = TileBox::centred(half.map(|v| v + padding))?;
        let mut error = record.geometric_error();
        for child in &mut children {
            let delta = sub(child.center, zcenter);
            let child = child.tree.record_mut();
            child.set_placement(NodePlacement::translation(delta)?);
            bounds.grow_to_enclose(delta, child.bounds())?;
            error = error.max(child.geometric_error());
        }
        if record.contents().is_empty() && !children.is_empty() {
            // CesiumJS draws nothing at an empty REPLACE tile within its error.
            error = error.max(GeometricError::new(EMPTY_PARENT_SCALE * bounds.diagonal())?);
        }
        record.set_bounds(bounds);
        record.set_refine(Refine::Replace);
        record.set_geometric_error(error);
        self.counters.tiles += 1;
        emit(
            self.attempt,
            RunEvent::Progress {
                phase: "encoding",
                done: self.counters.tiles as u64,
                total: None,
            },
        )?;
        if self.counters.tiles > self.options.max_tiles {
            return Err(data(
                "hierarchy exceeds maxTiles; raise budgets or maxTiles explicitly",
            ));
        }
        if let (false, Some(statistics)) = (
            record.contents().is_empty(),
            record.extras().and_then(VectorTileExtras::statistics),
        ) {
            self.counters.maximum_tile_vertices =
                self.counters.maximum_tile_vertices.max(statistics.vertices);
            self.counters.maximum_tile_bytes = self
                .counters
                .maximum_tile_bytes
                .max(statistics.encoded_bytes);
        }
        let fragments = if children.is_empty() {
            records::own_fragments(&record)?
        } else {
            children.iter().map(|n| n.fragments).sum()
        };
        Ok(Node {
            tree: TileTree::new(record, children.into_iter().map(|n| n.tree).collect()),
            center: zcenter,
            padding,
            fragments,
        })
    }
    fn build(&mut self, prefix: &str, depth: usize) -> Result<Node, Error> {
        self.attempt
            .check()
            .map_err(|error| Error::Job(self.attempt.fail(error)))?;
        if self.pass == encoding::GridPass::Survey {
            // The survey encodes everything: reused payloads report no exponents.
            return self.build_uncached(prefix, depth);
        }
        let signature = self.reuse.signature(self.db, prefix)?;
        let n = self.stats(prefix)?.count;
        if let Some(node) = self.reuse.restore(
            prefix,
            &signature,
            &mut self.counters,
            n,
            self.options.max_tiles,
        )? {
            return Ok(node);
        }
        let mut node = self.build_uncached(prefix, depth)?;
        // The subtree's own leaf total, not the spool rows under `prefix`: rows inside a
        // restored descendant keep their unsplit form.
        self.reuse.remember(prefix, signature, &mut node)?;
        Ok(node)
    }
    fn build_uncached(&mut self, prefix: &str, depth: usize) -> Result<Node, Error> {
        if depth > 64 {
            return Err(data("partition depth exceeds 64"));
        }
        let stats = self.stats(prefix)?;
        let center = mul(add(stats.lo, stats.hi), 0.5);
        if stats.count <= self.max_features
            && stats.vertices <= self.options.max_vertices
            && stats.estimate <= self.options.max_bytes.saturating_mul(2)
        {
            // Indexed parallel collection preserves the same level/report order
            // across worker counts and bounds live candidates by LOD levels/jobs.
            let lod = &self.options.lod;
            let levels: Vec<_> = (0..=u32::from(lod.levels))
                .map(|level| (level, lod.tolerance_metres * 2f64.powi(level as i32 - 1)))
                .collect();
            let mut candidates = self.candidates(prefix, center, &levels)?.into_iter();
            let (leaf, _) = self.consume(candidates.next().unwrap())?;
            if let Some(leaf) = leaf {
                self.counters.leaf_tiles += 1;
                let mut node = self.attach(leaf, &stats, center, Vec::new())?;
                let vertices = |record: &VectorRecord| {
                    record
                        .extras()
                        .and_then(VectorTileExtras::statistics)
                        .map(|statistics| statistics.vertices)
                        .expect("a content candidate records its statistics")
                };
                for candidate in candidates {
                    if candidate
                        .node
                        .as_ref()
                        .is_some_and(|coarse| vertices(coarse) >= vertices(node.tree.record()))
                    {
                        continue;
                    }
                    let (coarse, _) = self.consume(candidate)?;
                    if let Some(coarse) = coarse {
                        node = self.attach(coarse, &stats, center, vec![node])?;
                    }
                }
                return Ok(node);
            }
            // These speculative levels do not belong to the resulting hierarchy.
        }
        if stats.count == 1 {
            let (id, value): (i64, String) = self
                .db
                .query_row(
                    "SELECT id,data FROM features WHERE path>=?1 AND path<?2",
                    params![prefix, format!("{prefix}~")],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(sql)?;
            let feature: Feature = serde_json::from_str(&value)?;
            let mut staged_reports = Vec::new();
            let parts = split(
                &feature,
                self.repair,
                &mut self.counters,
                &mut staged_reports,
            )
            .map_err(FeatureFailure::into_error)?;
            self.db
                .execute("DELETE FROM features WHERE id=?1", params![id])
                .map_err(sql)?;
            let n = parts.len();
            for (index, mut part) in parts.into_iter().enumerate() {
                part.fragment_path = format!("{}{index}", feature.fragment_path);
                Spool {
                    db: self.db,
                    repair: self.repair,
                    options: self.options,
                    policy: self.policy,
                    attempt: self.attempt,
                }
                .insert(part, prefix, false, &mut self.counters, &mut staged_reports)
                .map_err(FeatureFailure::into_error)?;
            }
            for report in staged_reports {
                self.reports.write(report)?;
            }
            self.counters.fragments += n - 1;
            let mut stmt = self
                .db
                .prepare("SELECT id,data FROM features WHERE path>=?1 AND path<?2")
                .map_err(sql)?;
            let rows = stmt
                .query_map(params![prefix, format!("{prefix}~")], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(sql)?;
            for row in rows {
                let (id, value) = row.map_err(sql)?;
                fingerprint(self.db, id, &value, false)?;
            }
            return self.build_uncached(prefix, depth + 1);
        }
        // The final pass of a quantized build reuses the survey's partition.
        if self.pass != encoding::GridPass::Final {
            let started = Instant::now();
            let median = |db: &Connection| -> Result<Cut, Error> {
                let span = sub(stats.hi, stats.lo);
                let mut index = 0;
                for i in 1..3 {
                    if span[i] > span[index] {
                        index = i;
                    }
                }
                let axis = ["x", "y", "z"][index];
                db.query_row(&format!("SELECT {axis},sortkey,id FROM features WHERE path>=?1 AND path<?2 ORDER BY {axis},sortkey,id LIMIT 1 OFFSET ?3"),params![prefix,format!("{prefix}~"),(stats.count/2-1) as i64],|r|Ok(Cut{axis:axis.into(),value:r.get(0)?,key:r.get(1)?,id:r.get(2)?})).map_err(sql)
            };
            let mut cut = self
                .reuse
                .cuts
                .get(prefix)
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| median(self.db))?;
            if !matches!(cut.axis.as_str(), "x" | "y" | "z") || !cut.value.is_finite() {
                return Err(data("invalid previous spatial partition axis"));
            }
            let condition = |cut: &Cut| {
                format!(
                    "({}< ?2 OR ({}= ?2 AND (sortkey< ?3 OR (sortkey= ?3 AND id<= ?4))))",
                    cut.axis, cut.axis
                )
            };
            let left: usize = self
                .db
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM features WHERE path=?1 AND {}",
                        condition(&cut)
                    ),
                    params![prefix, cut.value, cut.key, cut.id],
                    |r| Ok(r.get::<_, i64>(0)? as usize),
                )
                .map_err(sql)?;
            if left == 0 || left == stats.count {
                cut = median(self.db)?;
            }
            self.db
                .execute(
                    &format!(
                        "UPDATE features SET path=?5 WHERE path=?1 AND {}",
                        condition(&cut)
                    ),
                    params![prefix, cut.value, cut.key, cut.id, format!("{prefix}0")],
                )
                .map_err(sql)?;
            self.db
                .execute(
                    "UPDATE features SET path=?1 WHERE path=?2",
                    params![format!("{prefix}1"), prefix],
                )
                .map_err(sql)?;
            self.reuse.cuts.insert(prefix.into(), cut);
            self.partition_seconds += started.elapsed().as_secs_f64();
        }
        let mut children = Vec::new();
        for suffix in ["0", "1"] {
            let path = format!("{prefix}{suffix}");
            if self.stats(&path)?.count > 0 {
                children.push(self.build(&path, depth + 1)?);
            }
        }
        let level = u32::from(self.options.lod.levels)
            + (stats.count as f64 / self.max_features as f64)
                .log2()
                .ceil() as u32;
        // The parent's tolerance scales with its extent and is coarser than a child's,
        // or the top of the levels above a leaf, by at least sqrt 2 (a binary split).
        let lod = &self.options.lod;
        let chain = lod.tolerance_metres * 2f64.powi(i32::from(lod.levels) - 1);
        let tolerance = children
            .iter()
            .map(|child| std::f64::consts::SQRT_2 * top_tolerance(&child.tree).max(chain))
            .fold(
                norm(sub(stats.hi, stats.lo)) / PARENT_EXTENT_PER_TOLERANCE,
                f64::max,
            );
        let candidate = self
            .candidates(prefix, center, &[(level.max(1), tolerance)])?
            .pop()
            .unwrap();
        let (mut coarse, mut reason) = self.consume(candidate)?;
        let diagonal = norm(sub(stats.hi, stats.lo));
        if coarse.is_none() && tolerance < diagonal {
            // At its own diagonal every unsplit line or polygon is a stand-in or omitted:
            // content there beats a routing tile, whose error would bound every ancestor.
            let level = level.max(1) + 1;
            let retry = self.candidates(prefix, center, &[(level, diagonal)])?.pop();
            (coarse, reason) = self.consume(retry.unwrap())?;
        }
        let coarse = match coarse {
            Some(coarse) => coarse,
            None => {
                self.counters.routing_tiles += 1;
                TileRecord::new(
                    NodePlacement::INHERIT,
                    TileBox::centred([0.; 3])?,
                    GeometricError::new(0.)?,
                    Refine::Replace,
                    Vec::new(),
                    Some(VectorTileExtras::routing(reason)),
                )?
            }
        };
        self.attach(coarse, &stats, center, children)
    }
}

pub(super) fn convert(
    input: &Path,
    output: &Path,
    options: &VectorOptions,
    attempt: &Attempt,
) -> Result<CompletedVector, Error> {
    let policy = encoding::ProducerPolicy::new(options.meshopt).map_err(|error| {
        encoding::codec_failure(crate::vector_encoding::CodecError::Format(error), attempt)
    })?;
    let max_features = options.max_features;
    let repair = options.repair;
    let ambiguous = options.ambiguous_outlines;
    super::available()?;
    let started = Instant::now();
    emit(
        attempt,
        RunEvent::Progress {
            phase: "ingestion",
            done: 0,
            total: None,
        },
    )?;
    std::fs::create_dir_all(output.join("t"))?;
    let reuse = Reuse::new(output, options)?;
    let mut reader = Reader::new(input, options, reuse.frame.clone(), output)?;
    let scratch = tempfile::tempdir_in(output)?;
    let spool: PathBuf = scratch.path().join("features.sqlite");
    let db = open_spool(&spool)?;
    db.execute_batch("BEGIN;").map_err(sql)?;
    let counters = RefCell::new(Counters::default());
    let reports = RefCell::new(Reports {
        file: Box::new(std::fs::File::create(
            output.join("geometry-reports.jsonl"),
        )?),
        first: Vec::new(),
        count: 0,
    });
    let failures = Cell::new(0usize);
    let first_failure = RefCell::new(None);
    reader.read(
        options,
        |feature| -> FeatureResult<()> {
            attempt
                .check()
                .map_err(|error| Error::Job(attempt.fail(error)))?;
            accept_feature(
                &db,
                feature,
                repair,
                ambiguous,
                options,
                policy,
                attempt,
                &mut counters.borrow_mut(),
                &mut reports.borrow_mut(),
            )
        },
        |mut value, invalid| {
            if invalid {
                failures.set(failures.get() + 1);
                if first_failure.borrow().is_none() {
                    *first_failure.borrow_mut() = Some(
                        value["reason"]
                            .as_str()
                            .unwrap_or("invalid feature")
                            .to_string(),
                    );
                }
                value["outcome"] = json!(if options.skip_invalid {
                    "skipped"
                } else {
                    "invalid"
                });
                let message = format!(
                    "layer '{}', feature {}: {}",
                    value["sourceLayer"].as_str().unwrap_or(""),
                    value["sourceId"].as_str().unwrap_or(""),
                    value["reason"].as_str().unwrap_or("")
                );
                emit(
                    attempt,
                    RunEvent::Warning {
                        code: "vector-feature-rejected",
                        message: &message,
                    },
                )?;
            }
            reports.borrow_mut().write(value)
        },
    )?;
    if failures.get() > 0 && !options.skip_invalid {
        return Err(data(format!("{} unconvertible feature(s); first failure: {}. No tileset published. Fix the reported features or explicitly use --skipInvalid.",failures.get(),first_failure.borrow().as_deref().unwrap())));
    }
    let ingestion_seconds = started.elapsed().as_secs_f64();
    let ingested = counters.borrow().features as u64;
    emit(
        attempt,
        RunEvent::Progress {
            phase: "ingestion",
            done: ingested,
            total: Some(ingested),
        },
    )?;
    db.execute_batch("COMMIT;").map_err(sql)?;
    end_admission(&db)?;
    let mut counters = counters.into_inner();
    counters.skipped_features = failures.get();
    counters.features_without_geometry = reader.without_geometry();
    if counters.features == 0 && !reuse.has_previous() {
        if reader.without_geometry() == 0 && options.where_clause.is_none() {
            return Err(data("empty selected layers"));
        }
        *reader.frame_mut() = Some(Frame::local([0.; 3]));
    }
    let mut reuse = reuse;
    reuse.configure(max_features, repair, ambiguous, options, &reader)?;
    counters.fragments = db
        .query_row("SELECT COUNT(*) FROM features", [], |r| {
            Ok(r.get::<_, i64>(0)? as usize)
        })
        .map_err(sql)?;
    db.execute_batch("BEGIN;").map_err(sql)?;
    let mut stmt = db
        .prepare("SELECT id,data FROM features ORDER BY id")
        .map_err(sql)?;
    for row in stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(sql)?
    {
        let (id, value) = row.map_err(sql)?;
        fingerprint(&db, id, &value, false)?;
    }
    drop(stmt);
    db.execute_batch("COMMIT;").map_err(sql)?;
    let partition_seconds = started.elapsed().as_secs_f64() - ingestion_seconds;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .build()
        .map_err(|e| Error::Environment(format!("cannot start vector workers: {e}")))?;
    // The survey pass of a quantized build publishes into scratch.
    let survey_output = scratch.path().join("survey");
    let mut build = Build {
        attempt,
        db: &db,
        spool: &spool,
        output,
        options,
        policy,
        schemas: reader.schemas(),
        max_features,
        repair,
        pool,
        counters,
        reports: reports.into_inner(),
        reuse,
        workers: BTreeSet::new(),
        partition_seconds,
        pass: encoding::GridPass::Single,
        outcomes: BTreeMap::new(),
    };
    let encoding_started = Instant::now();
    let before_partition = build.partition_seconds;
    // A quantized build also keys its whole input by the rows' fingerprints
    // before the survey: unchanged input restores the previous build's
    // hierarchy, grid and contents as they are, without surveying or
    // encoding. (Changed input surveys everything; the final pass still
    // reuses unchanged subtrees.)
    let mut surveyed = None;
    let mut restored = None;
    if options.quantize && build.counters.features > 0 {
        let unchanged = build.reuse.signature(&db, "")?;
        let rows = build.stats("")?.count;
        restored = build.reuse.restore_surveyed(
            "",
            &unchanged,
            &mut build.counters,
            rows,
            options.max_tiles,
        )?;
        surveyed = Some(unchanged);
    }
    if restored.is_none() && options.quantize && build.counters.features > 0 {
        // Survey: build the hierarchy with every content on its own range and
        // record, per shared vertex, the largest exponent of the tiles holding
        // it. Splits and partitions it makes are kept; tiles are rebuilt.
        db.execute_batch(GRID_TABLES).map_err(sql)?;
        std::fs::create_dir_all(survey_output.join("t"))?;
        let before = build.counters.clone();
        build.pass = encoding::GridPass::Survey;
        build.output = &survey_output;
        drop(build.build("", 0)?);
        for (counter, value) in [
            (&mut build.counters.tiles, before.tiles),
            (&mut build.counters.leaf_tiles, before.leaf_tiles),
            (&mut build.counters.routing_tiles, before.routing_tiles),
            (
                &mut build.counters.maximum_tile_vertices,
                before.maximum_tile_vertices,
            ),
            (
                &mut build.counters.maximum_tile_bytes,
                before.maximum_tile_bytes,
            ),
        ] {
            *counter = value;
        }
        std::fs::remove_dir_all(&survey_output)?;
        resolve_grid(&db)?;
        // Reuse keys a subtree by its rows' shared-vertex exponents too.
        db.execute_batch("BEGIN;").map_err(sql)?;
        let mut stmt = db
            .prepare("SELECT id,data FROM features ORDER BY id")
            .map_err(sql)?;
        for row in stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(sql)?
        {
            let (id, value) = row.map_err(sql)?;
            fingerprint(&db, id, &value, true)?;
        }
        drop(stmt);
        db.execute_batch("COMMIT;").map_err(sql)?;
        build.pass = encoding::GridPass::Final;
        build.output = output;
    }
    let root = if let Some(node) = restored {
        node
    } else if build.counters.features > 0 {
        build.build("", 0)?
    } else {
        let mut node = Node {
            tree: TileTree::new(TileRecord::empty_root(None)?, Vec::new()),
            center: [0.; 3],
            padding: 0.,
            fragments: 0,
        };
        build.counters.tiles = 1;
        build
            .reuse
            .remember("", build.reuse.signature(&db, "")?, &mut node)?;
        node
    };
    if let Some(surveyed) = surveyed {
        build.reuse.remember_survey("", surveyed);
    }
    // Identity accounting: every source fragment is in exactly one full-detail leaf.
    let held = records::leaf_fragments(&root.tree)?;
    if held != build.counters.fragments {
        return Err(data(format!(
            "vector hierarchy accounting: full-detail leaves hold {held} source fragments; \
             the build has {}",
            build.counters.fragments
        )));
    }
    let encoding_seconds = (encoding_started.elapsed().as_secs_f64()
        - (build.partition_seconds - before_partition))
        .max(0.);
    let publication_started = Instant::now();
    let frame = reader
        .frame()
        .ok_or_else(|| data("missing local vector frame"))?;
    let anchor = (0..3).fold(frame.anchor, |p, i| {
        add(p, mul(frame.axes[i], root.center[i]))
    });
    // The local frame's rigid ENU→ECEF placement, at the root's centre.
    let mut tree = root.tree;
    tree.record_mut()
        .set_placement(NodePlacement::rigid(frame.axes, anchor)?);
    let diagonal = tree.record().bounds().diagonal();
    let root_error = tree.record().geometric_error().metres();
    // Leave an SSE interval in which the root itself can render (margin 2).
    // Cesium skips the whole tileset below its top-level error, so equality
    // with the root error made large-tolerance/coincident aggregate roots unreachable.
    let top_error = top_level_error(diagonal, root_error, 2.);
    if !top_error.is_finite() {
        return Err(data("vector LOD error overflows; reduce lodTolerance"));
    }
    let document = TilesetDocument::root(
        GeometricError::new(top_error)?,
        [TilesetExtension::ContentGltfVector],
    );
    // The forest holds each payload once; reuse recovers rebased payloads from the
    // published members through `vector-build.json` (no retained raw sources).
    // The producer's hierarchy H is the forest's input and is kept in the build
    // state as `explicitRoot`. Every content is checked against its staged payload.
    let (mut manifest, written) = crate::implicit::write_forest_records(
        &tree,
        &document,
        output,
        crate::implicit::SubdivisionScheme::Quadtree,
        &mut || {
            attempt
                .check()
                .map_err(|error| Error::Job(attempt.fail(error)))
        },
    )?;
    let (build_state, reuse_report) = build.reuse.publish(&manifest, frame, &tree, &written)?;
    // Re-emit the head with the build state digest, which covers the written forest.
    let root = match manifest.as_object_mut().and_then(|m| m.remove("root")) {
        Some(Value::Object(root)) => root,
        _ => return Err(data("the implicit forest wrote no root tile")),
    };
    let manifest = document
        .with_schema(true)
        .with_build_state(build_state)?
        .to_value(root);
    std::fs::write(output.join("tileset.json"), serde_json::to_vec(&manifest)?)?;
    build.reports.finish()?;
    let shared: usize = db
        .query_row("SELECT COUNT(*) FROM vertices WHERE shared=1", [], |r| {
            Ok(r.get::<_, i64>(0)? as usize)
        })
        .map_err(sql)?;
    // Tile statistics: one extras record per tile of the hierarchy, in preorder.
    // Implicit tiles carry the written (rebased) encodedBytes (design §4.5).
    let list: Vec<&Value> = written.tiles.iter().map(|tile| &tile.extras).collect();
    build.counters.maximum_tile_bytes = list
        .iter()
        .map(|extras| extras["encodedBytes"].as_u64().unwrap_or(0) as usize)
        .max()
        .unwrap_or(0);
    let sum = |key: &str| {
        list.iter()
            .map(|extras| extras[key].as_u64().unwrap_or(0))
            .sum::<u64>()
    };
    let max = |key: &str| {
        list.iter()
            .map(|extras| extras[key].as_f64().unwrap_or(0.))
            .fold(0., f64::max)
    };
    let mut report = serde_json::to_value(&build.counters)?;
    let values = json!({"inputDriver":reader.driver(),"layers":reader.layer_reports(),
        "performance":{"jobs":options.jobs,"workersUsed":build.workers.len(),"phaseSeconds":{"ingestion":ingestion_seconds,"partitioning":build.partition_seconds,"encoding":encoding_seconds,"publication":publication_started.elapsed().as_secs_f64()}},
        "budgets":{"features":max_features,"parentFeatures":options.max_parent_features,"vertices":options.max_vertices,"bytes":options.max_bytes,"tiles":options.max_tiles},
        "attributeFilter":options.where_clause,"metadata":{"listFields":options.list_fields,"fields":options.fields,"dropFields":options.drop_fields},
        "encoding":{"quantize":options.quantize,"meshopt":options.meshopt,"primitiveReferences":sum("primitives"),"maximumTilePrimitives":list.iter().map(|extras| extras["primitives"].as_u64().unwrap_or(0)).max().unwrap_or(0),"maximumQuantizationErrorMetres":max("quantizationErrorMetres"),"uncompressedTileBytes":sum("uncompressedBytes"),"uncompressedTileBytesScope":"selected-framed raw bridge after requested quantization","encodedTileBytes":sum("encodedBytes")},
        "parentRepairEnabled":options.parent_repair,"skipInvalidEnabled":options.skip_invalid,"repairEnabled":repair,"lodToleranceMetres":options.lod.tolerance_metres,"lodLevels":options.lod.levels,"reuse":reuse_report,
        "lodFallbacks":build.reports.first,"geometryReportCount":build.reports.count,"geometryReports":"geometry-reports.jsonl","geometryReportsScope":"current ingestion and newly encoded geometry; previous content reports remain in the prior archive",
        "lockedSharedVertices":shared,"pointPolicy":if options.aggregate_points {"opt-in per-layer voxel count aggregates in point-only parents; original identities and properties in full-detail leaves"} else {"retain every semantic point feature; oversized parents route without content"},
        "pointAggregation":{"enabled":options.aggregate_points,"contentTiles":list.iter().filter(|extras| extras.get("pointAggregation").is_some()).count()},
        "polygonFragmentPolicy":"standard glTF fills plus vector source boundaries; no internal fragment outlines",
        "errorPolicy":"direct original-to-parent distance plus float32 rounding; all source bounds retained"});
    if let (Value::Object(report), Value::Object(values)) = (&mut report, values) {
        report.extend(values);
    }
    if options.reproducible {
        report.as_object_mut().unwrap().remove("performance");
    }
    report["tiling"] = json!("implicit");
    let report = crate::output::write_report(output, report, true)?;
    // Close resources explicitly before inventory admission. Earlier failures
    // leave scratch beneath the facade-owned workspace for required cleanup.
    drop(build);
    db.close().map_err(|(_, error)| sql(error))?;
    scratch.close()?;
    let members = final_members(output)?;
    Ok(CompletedVector { report, members })
}

#[cfg(test)]
mod countries_fragment_tests {
    use super::*;

    fn accept_feature(
        db: &Connection,
        feature: Feature,
        repair: bool,
        ambiguous: bool,
        options: &VectorOptions,
        counters: &mut Counters,
        reports: &mut Reports,
    ) -> FeatureResult<()> {
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        super::accept_feature(
            db,
            feature,
            repair,
            ambiguous,
            options,
            encoding::ProducerPolicy::new(options.meshopt).unwrap(),
            &attempt,
            counters,
            reports,
        )
    }

    fn fixture_db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA journal_mode=MEMORY; CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,n INTEGER,estimate INTEGER,x REAL,y REAL,z REAL,lx REAL,ly REAL,lz REAL,hx REAL,hy REAL,hz REAL,sortkey TEXT,omissible INTEGER); CREATE TABLE vertices(x REAL,y REAL,z REAL,owner INTEGER,shared INTEGER DEFAULT 0,UNIQUE(x,y,z));").unwrap();
        db
    }
    fn fixture_feature(geometry: Geometry) -> Feature {
        Feature {
            properties: BTreeMap::from([
                ("_source_id".into(), json!("fixture")),
                ("_source_layer".into(), json!("test")),
            ]),
            geometry,
            intrinsic: None,
            surface_fragment: false,
            fragment_path: String::new(),
        }
    }
    fn fixture_reports() -> Reports {
        Reports {
            file: Box::new(Vec::<u8>::new()),
            first: Vec::new(),
            count: 0,
        }
    }
    #[test]
    fn fitting_resource_limit_rolls_back_instead_of_skip_or_split() {
        for meshopt in [false, true] {
            let db = fixture_db();
            let run = crate::RunControl::default();
            let attempt = run.begin().unwrap();
            let options = VectorOptions {
                meshopt,
                skip_invalid: true,
                max_bytes: 1_000_000,
                ..Default::default()
            };
            let limits = crate::vector_encoding::CompressionLimits {
                source_bytes: 1,
                ..Default::default()
            };
            let mut policy = encoding::ProducerPolicy::new(meshopt).unwrap();
            policy.limits = limits.validate().unwrap();
            let mut counters = Counters::default();
            let mut reports = fixture_reports();
            let error = super::accept_feature(
                &db,
                fixture_feature(Geometry::Point([0.; 3])),
                false,
                false,
                &options,
                policy,
                &attempt,
                &mut counters,
                &mut reports,
            )
            .unwrap_err();
            let FeatureFailure::Fatal(Error::Job(failure)) = error else {
                panic!("actual bridge limit is infrastructure failure, never skippable")
            };
            assert_eq!(failure.error.kind(), crate::JobErrorKind::ResourceLimit);
            assert!(failure.error.same_cause(&attempt.check().unwrap_err()));
            assert_eq!(
                (
                    counters.features,
                    counters.fragments,
                    counters.fragmented_polygons,
                    reports.count
                ),
                (0, 0, 0, 0)
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM features", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert!(reports.first.is_empty());
            // Feature savepoint is released after rollback, so a fresh feature
            // transaction can begin; no accepted row was retained or split.
            db.execute_batch("SAVEPOINT feature; RELEASE feature;")
                .unwrap();
        }
    }

    #[test]
    fn actual_native_fitting_refusal_rolls_back_without_split_or_skip() {
        let db = fixture_db();
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let options = VectorOptions {
            meshopt: true,
            skip_invalid: true,
            max_bytes: 1_000_000,
            ..Default::default()
        };
        let mut counters = Counters::default();
        let mut reports = fixture_reports();
        let (result, reached) = crate::vector_encoding::with_capacity_one(|| {
            super::accept_feature(
                &db,
                fixture_feature(Geometry::Point([0.; 3])),
                false,
                false,
                &options,
                encoding::ProducerPolicy::new(true).unwrap(),
                &attempt,
                &mut counters,
                &mut reports,
            )
        });
        assert_eq!(reached, 1);
        let FeatureFailure::Fatal(Error::Job(failure)) = result.unwrap_err() else {
            panic!("actual native refusal must abort admission")
        };
        assert_eq!(failure.error.kind(), crate::JobErrorKind::InvalidState);
        assert!(failure.error.same_cause(&attempt.check().unwrap_err()));
        assert!(matches!(
            std::error::Error::source(&failure.error)
                .unwrap()
                .downcast_ref::<crate::vector_encoding::CodecError<crate::JobError>>()
                .unwrap(),
            crate::vector_encoding::CodecError::EncodingFailure("ATTRIBUTES encoder")
        ));
        assert_eq!(
            (
                counters.features,
                counters.fragments,
                counters.fragmented_polygons,
                reports.count
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM features", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch("SAVEPOINT feature; RELEASE feature;")
            .unwrap();
        assert!(run.begin().is_err());
    }
    #[test]
    fn sql_admission_faults_are_fatal_and_do_not_commit_delta() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
        for fault in ["savepoint", "insert", "release", "rollback"] {
            let db = fixture_db();
            db.authorizer(Some(move |context: AuthContext<'_>| {
                let deny = match context.action {
                    AuthAction::Savepoint {
                        operation: TransactionOperation::Begin,
                        ..
                    } => fault == "savepoint",
                    AuthAction::Savepoint {
                        operation: TransactionOperation::Release,
                        ..
                    } => fault == "release",
                    AuthAction::Savepoint {
                        operation: TransactionOperation::Rollback,
                        ..
                    } => fault == "rollback",
                    AuthAction::Insert { table_name } => {
                        fault == "insert" && table_name == "features"
                    }
                    _ => false,
                };
                if deny {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }))
            .unwrap();
            let feature = if fault == "rollback" {
                fixture_feature(Geometry::LineString(Vec::new()))
            } else {
                fixture_feature(Geometry::Point([0.; 3]))
            };
            let mut counters = Counters::default();
            let mut reports = fixture_reports();
            let options = VectorOptions {
                skip_invalid: true,
                ..VectorOptions::default()
            };
            let error = accept_feature(
                &db,
                feature,
                false,
                false,
                &options,
                &mut counters,
                &mut reports,
            )
            .unwrap_err();
            assert!(
                matches!(error, FeatureFailure::Fatal(_)),
                "{fault}: {error}"
            );
            assert_eq!(
                (
                    counters.features,
                    counters.fragmented_polygons,
                    reports.count
                ),
                (0, 0, 0)
            );
            assert!(reports.first.is_empty());
            if fault == "rollback" {
                let FeatureFailure::Fatal(Error::Job(failure)) = error else {
                    panic!("cleanup must preserve both causes")
                };
                assert_eq!(failure.error.kind(), crate::JobErrorKind::Io);
                assert!(failure.error.to_string().contains("not authorized"));
                assert_eq!(failure.secondary.len(), 1);
                assert!(failure.secondary[0]
                    .to_string()
                    .contains("empty/degenerate feature"));
            }
        }
    }
    struct ReportFault {
        flush: bool,
    }
    impl Write for ReportFault {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.flush {
                Ok(bytes.len())
            } else {
                Err(std::io::Error::from_raw_os_error(28))
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::from_raw_os_error(28))
        }
    }
    #[test]
    fn required_report_write_and_finalization_failures_remain_io_fatal() {
        let db = fixture_db();
        let ring: Vec<_> = (0..=20)
            .map(|i| {
                let angle = (i % 20) as f64 * std::f64::consts::TAU / 20.;
                [angle.cos(), angle.sin(), 0.]
            })
            .collect();
        let options = VectorOptions {
            max_vertices: 16,
            max_bytes: 100_000,
            skip_invalid: true,
            ..VectorOptions::default()
        };
        let mut counters = Counters::default();
        let mut reports = Reports {
            file: Box::new(ReportFault { flush: false }),
            first: Vec::new(),
            count: 0,
        };
        let error = accept_feature(
            &db,
            fixture_feature(Geometry::Polygon(vec![ring.clone()])),
            false,
            false,
            &options,
            &mut counters,
            &mut reports,
        )
        .unwrap_err();
        let FeatureFailure::Fatal(Error::Io(error)) = error else {
            panic!("report ENOSPC must be fatal I/O")
        };
        assert_eq!(error.raw_os_error(), Some(28));
        assert_eq!(
            (
                counters.features,
                counters.fragmented_polygons,
                reports.count
            ),
            (0, 0, 0)
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM features", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        // A later rollback failure must not replace the first fatal ENOSPC.
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
        db.authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Savepoint {
                    operation: TransactionOperation::Rollback,
                    ..
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
        let combined = accept_feature(
            &db,
            fixture_feature(Geometry::Polygon(vec![ring])),
            false,
            false,
            &options,
            &mut counters,
            &mut reports,
        )
        .unwrap_err();
        let FeatureFailure::Fatal(Error::Job(failure)) = combined else {
            panic!("both fatal causes must survive")
        };
        let source = std::error::Error::source(&failure.error)
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(source.raw_os_error(), Some(28));
        assert_eq!(failure.secondary.len(), 1);
        assert!(failure.secondary[0].to_string().contains("not authorized"));
        reports.file = Box::new(ReportFault { flush: true });
        let Error::Io(error) = reports.finish().unwrap_err() else {
            panic!("flush ENOSPC must remain I/O")
        };
        assert_eq!(error.raw_os_error(), Some(28));
    }
    #[test]
    fn ingestion_commit_fault_is_fatal() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        let db = fixture_db();
        db.execute_batch("BEGIN;").unwrap();
        db.authorizer(Some(|context: AuthContext<'_>| {
            if matches!(context.action, AuthAction::Transaction { .. }) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
        let error = db.execute_batch("COMMIT;").map_err(sql).unwrap_err();
        assert!(matches!(error, Error::Io(_)));
        assert!(error.to_string().contains("not authorized"));
    }

    #[test]
    fn production_spool_journal_rolls_back_feature_savepoints() {
        // The scratch spool keeps a rollback-capable journal without a journal
        // file: a rejected feature's rows and vertex locks must disappear.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("features.sqlite");
        let db = open_spool(&path).unwrap();
        let mode: String = db
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "memory");
        db.execute_batch("BEGIN;").unwrap();
        let feature = fixture_feature(Geometry::LineString(vec![[0., 0., 0.], [1., 0., 0.]]));
        insert_row(&db, &feature, "").unwrap();
        db.execute_batch("SAVEPOINT feature;").unwrap();
        let rejected = fixture_feature(Geometry::LineString(vec![[1., 0., 0.], [2., 0., 0.]]));
        insert_row(&db, &rejected, "").unwrap();
        db.execute_batch("ROLLBACK TO feature; RELEASE feature; COMMIT;")
            .unwrap();
        let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(count("SELECT COUNT(*) FROM features"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM vertices"), 2);
        assert_eq!(count("SELECT COUNT(*) FROM vertices WHERE shared"), 0);
        assert!(!dir.path().join("features.sqlite-journal").exists());
        end_admission(&db).unwrap();
        let mode: String = db
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "off");
        // The switch is refused inside a transaction; that must not pass silently.
        let db = open_spool(&dir.path().join("open.sqlite")).unwrap();
        db.execute_batch("BEGIN; INSERT INTO vertices(x,y,z,owner) VALUES(0,0,0,1);")
            .unwrap();
        assert!(end_admission(&db).is_err());
    }

    #[test]
    fn rejected_late_fragment_restores_exact_sql_inventory() {
        // Fixture expectation is independent of Spool traversal: only the
        // pre-existing accepted source row and its two vertices may survive.
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA journal_mode=MEMORY; CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,n INTEGER,estimate INTEGER,x REAL,y REAL,z REAL,lx REAL,ly REAL,lz REAL,hx REAL,hy REAL,hz REAL,sortkey TEXT,omissible INTEGER); CREATE TABLE vertices(x REAL,y REAL,z REAL,owner INTEGER,shared INTEGER DEFAULT 0,UNIQUE(x,y,z));").unwrap();
        let make = |id: &str, geometry: Geometry| Feature {
            properties: BTreeMap::from([
                ("_source_id".into(), json!(id)),
                ("_source_layer".into(), json!("fixture")),
            ]),
            geometry,
            intrinsic: None,
            surface_fragment: false,
            fragment_path: String::new(),
        };
        insert_row(
            &db,
            &make(
                "accepted",
                Geometry::LineString(vec![[10., 0., 0.], [11., 0., 0.]]),
            ),
            "",
        )
        .unwrap();
        let options = VectorOptions {
            max_vertices: 2,
            max_bytes: 1_000_000,
            ..VectorOptions::default()
        };
        let rejected = make(
            "rejected",
            Geometry::MultiLineString(vec![
                vec![[0., 0., 0.], [1., 0., 0.]],
                vec![[-1e39, 0., 0.], [1e39, 0., 0.]],
            ]),
        );
        db.execute_batch("SAVEPOINT feature;").unwrap();
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let outcome = Spool {
            db: &db,
            repair: false,
            options: &options,
            policy: encoding::ProducerPolicy::new(options.meshopt).unwrap(),
            attempt: &attempt,
        }
        .insert(
            rejected,
            "",
            true,
            &mut Counters::default(),
            &mut Vec::new(),
        );
        assert!(matches!(outcome, Err(FeatureFailure::Rejected(_))));
        let provisional: i64 = db
            .query_row("SELECT COUNT(*) FROM features", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            provisional, 2,
            "fixture must exercise insertion before rejection"
        );
        db.execute_batch("ROLLBACK TO feature; RELEASE feature;")
            .unwrap();
        let mut query = db.prepare("SELECT data FROM features ORDER BY id").unwrap();
        let inventory: Vec<(String, String, String)> = query
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|row| {
                let feature: Feature = serde_json::from_str(&row.unwrap()).unwrap();
                (
                    feature.layer().to_string(),
                    feature.source_id().as_str().unwrap().to_string(),
                    feature.fragment_path,
                )
            })
            .collect();
        assert_eq!(
            inventory,
            vec![("fixture".into(), "accepted".into(), "".into())]
        );
        let vertices: Vec<(f64, f64, f64, i64)> = db
            .prepare("SELECT x,y,z,shared FROM vertices ORDER BY x,y,z")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(vertices, vec![(10., 0., 0., 0), (11., 0., 0., 0)]);
    }

    #[test]
    fn collapsed_polar_face_keeps_source_boundaries_as_closed_outline_rings() {
        let source = Geometry::Polygon(vec![vec![
            [-180., -90., 0.],
            [-180., -80., 0.],
            [0., -70., 0.],
            [180., -80., 0.],
            [180., -90., 0.],
            [-180., -90., 0.],
        ]]);
        let intrinsic = IntrinsicGeometry::capture(&source, Some((true, 1.))).unwrap();
        let mut geometry = source;
        geometry.map(|p| {
            crate::georef::geodetic_to_ecef(crate::georef::Cartographic::new(p[0], p[1], p[2]))
        });
        let feature = Feature {
            properties: BTreeMap::from([
                ("_source_id".into(), json!("polar")),
                ("_source_layer".into(), json!("test")),
            ]),
            geometry,
            intrinsic,
            surface_fragment: false,
            fragment_path: String::new(),
        };
        let Geometry::Polygon(rings) = &feature.geometry else {
            unreachable!()
        };
        let mesh = geometry::polygon_for(&feature, rings, 0, [0.; 3], false).unwrap();
        assert_eq!(mesh.report["collapsedPoleTriangles"], 1);
        let edge = |a: Point, b: Point| {
            let mut pair = [key(a), key(b)];
            pair.sort();
            pair
        };
        let expected: BTreeSet<_> = mesh
            .loops
            .split(|i| *i == u32::MAX)
            .flat_map(|ring| {
                (0..ring.len()).map(|i| {
                    edge(
                        mesh.positions[ring[i] as usize],
                        mesh.positions[ring[(i + 1) % ring.len()] as usize],
                    )
                })
            })
            .collect();
        let parts = split(&feature, false, &mut Counters::default(), &mut Vec::new()).unwrap();
        // Two fill halves plus one locked outline feature of closed rings.
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|f| f.surface_fragment));
        let Geometry::MultiLineString(rings) = &parts[2].geometry else {
            panic!("outline geometry {:?}", parts[2].geometry)
        };
        assert!(parts[2].intrinsic.is_none());
        assert!(rings.iter().all(|r| r.len() >= 4 && r.first() == r.last()));
        // Each ring opens and closes at the midpoint of one source edge;
        // without that seam vertex it is the source loop.
        let actual: Vec<_> = rings
            .iter()
            .flat_map(|r| {
                let cycle = &r[1..r.len() - 1];
                let n = cycle.len();
                assert_eq!(r[0], mul(add(cycle[n - 1], cycle[0]), 0.5), "seam");
                (0..n).map(move |i| edge(cycle[i], cycle[(i + 1) % n]))
            })
            .collect();
        assert_eq!(actual.len(), expected.len(), "each source edge once");
        assert_eq!(
            actual.into_iter().collect::<BTreeSet<_>>(),
            expected,
            "fragmentation must not introduce internal meridian outlines"
        );
    }
}

#[cfg(test)]
mod grid_tests {
    use super::*;

    /// Four tiles hold one fill vertex: two sibling leaves, a coarse leaf
    /// elsewhere and an LOD ancestor of the siblings. All are joined (the
    /// coarse leaf renders with each), yet the vertex snaps at the finest
    /// leaf's own step, not the coarse leaf's: every tile coarser than it
    /// stores float32. An ancestor and its descendant are never joined, and
    /// line copies group apart from fill copies.
    #[test]
    fn grid_groups_keep_the_finest_tile_on_its_own_step() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(GRID_TABLES).unwrap();
        let tiles = [
            ("00", 0, [3, 3, 3]),
            ("01", 0, [4, 3, 3]),
            ("1", 0, [6, 6, 6]),
            ("0", 2, [5, 5, 5]),
        ];
        let hold = |cell: i64, kind: i64, tile: usize| {
            let (prefix, level, e) = tiles[tile];
            db.execute(
                "INSERT INTO holders VALUES(?1,0,0,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    cell,
                    kind,
                    prefix,
                    level,
                    e[0],
                    e[1],
                    e[2],
                    e[0] - 8,
                    e[1] - 8,
                    e[2] - 8
                ],
            )
            .unwrap();
        };
        for tile in 0..4 {
            hold(1, 0, tile);
        }
        hold(2, 0, 0);
        hold(2, 0, 3);
        hold(3, 1, 0);
        hold(3, 1, 1);
        resolve_grid(&db).unwrap();
        let row = |cell: i64, kind: i64, prefix: &str, level: u32| -> ([i32; 3], bool) {
            db.query_row(
                "SELECT ex,ey,ez,fl FROM grid WHERE cx=?1 AND kind=?2 AND prefix=?3 AND level=?4",
                params![cell, kind, prefix, level],
                |r| Ok(([r.get(0)?, r.get(1)?, r.get(2)?], r.get(3)?)),
            )
            .unwrap()
        };
        for (prefix, level, float) in [
            ("00", 0, false),
            ("01", 0, true),
            ("1", 0, true),
            ("0", 2, true),
        ] {
            assert_eq!(row(1, 0, prefix, level), ([3, 3, 3], float), "{prefix}");
        }
        assert_eq!(row(2, 0, "00", 0), ([3, 3, 3], false));
        // Only an ancestor shares vertex 2: no copy it must agree with.
        let alone = |cell: i64| -> Vec<bool> {
            db.prepare("SELECT al FROM grid WHERE cx=?1 ORDER BY prefix")
                .unwrap()
                .query_map([cell], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(alone(2), [true, true]);
        assert_eq!(alone(1), [false; 4]);
        assert_eq!(row(2, 0, "0", 2), ([-3, -3, -3], true));
        assert_eq!(row(3, 1, "01", 0), ([3, 3, 3], true));
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM grid", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            8
        );
    }

    /// The survey's g > E case: a float32 member's finest step can be
    /// coarser than a u16 member's own. The group snaps there, and the u16
    /// tile stays u16.
    #[test]
    fn grid_group_snaps_at_a_coarser_float32_members_step() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(GRID_TABLES).unwrap();
        for (prefix, u16, float) in [
            ("0", [0, 0, 0], [-8, -8, -8]),
            ("1", [12, 1, 1], [4, -7, -7]),
        ] {
            db.execute(
                "INSERT INTO holders VALUES(1,0,0,0,?1,0,?2,?3,?4,?5,?6,?7)",
                params![prefix, u16[0], u16[1], u16[2], float[0], float[1], float[2]],
            )
            .unwrap();
        }
        resolve_grid(&db).unwrap();
        let rows: Vec<(String, [i32; 3], bool)> = db
            .prepare("SELECT prefix,ex,ey,ez,fl FROM grid ORDER BY prefix")
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, [r.get(1)?, r.get(2)?, r.get(3)?], r.get(4)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            [
                ("0".into(), [4, 0, 0], false),
                ("1".into(), [4, 0, 0], true)
            ]
        );
    }

    /// A tile recorded with two different exponents is a broken survey.
    #[test]
    fn conflicting_survey_exponents_are_an_error() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(GRID_TABLES).unwrap();
        for (cell, e) in [(1, 3), (2, 4)] {
            db.execute(
                "INSERT INTO holders VALUES(?1,0,0,0,'0',0,?2,?2,?2,-5,-5,-5)",
                params![cell, e],
            )
            .unwrap();
        }
        assert!(resolve_grid(&db).is_err());
    }

    /// The final pass replays the survey's refusals; a refused leaf takes
    /// its coarse levels with it.
    #[test]
    fn refused_leaves_discard_their_coarse_levels_on_replay() {
        assert_eq!(
            replayed(&[0, 1, 2], &[Some("vertices"), None, None]),
            [Some("vertices"); 3]
        );
        assert_eq!(
            replayed(&[0, 1, 2], &[None, Some("parentFeatures"), None]),
            [None, Some("parentFeatures"), None]
        );
        assert_eq!(replayed(&[4], &[None]), [None]);
        assert_eq!(replayed(&[4], &[Some("bytes")]), [Some("bytes")]);
    }

    /// Reuse fingerprints change when a row's shared vertex changes storage
    /// or exponents, so a reused subtree is re-encoded.
    #[test]
    fn storage_and_exponent_changes_invalidate_reuse_fingerprints() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE features(id INTEGER PRIMARY KEY,data TEXT,fingerprint TEXT); CREATE TABLE vertices(x REAL,y REAL,z REAL,shared INTEGER);",
        )
        .unwrap();
        db.execute_batch(GRID_TABLES).unwrap();
        let ring = vec![[0., 0., 0.], [10.5, 0., 0.], [10.5, 0., 7.25], [0., 0., 0.]];
        let feature = Feature {
            properties: BTreeMap::from([("_source_id".into(), json!("f"))]),
            geometry: Geometry::Polygon(vec![ring.clone()]),
            intrinsic: None,
            surface_fragment: false,
            fragment_path: String::new(),
        };
        let value = serde_json::to_string(&feature).unwrap();
        db.execute("INSERT INTO features(id,data) VALUES(1,?1)", [&value])
            .unwrap();
        for p in &ring[..3] {
            db.execute(
                "INSERT INTO vertices VALUES(?1,?2,?3,1)",
                params![p[0], p[1], p[2]],
            )
            .unwrap();
        }
        let c = encoding::cell(ring[1]);
        db.execute(
            "INSERT INTO grid VALUES(?1,?2,?3,0,'0',0,2,2,2,0,0)",
            params![c[0], c[1], c[2]],
        )
        .unwrap();
        let print = |db: &Connection| -> String {
            fingerprint(db, 1, &value, true).unwrap();
            db.query_row("SELECT fingerprint FROM features WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        let u16 = print(&db);
        db.execute("UPDATE grid SET fl=1", []).unwrap();
        let float = print(&db);
        assert_ne!(u16, float, "storage flip");
        db.execute("UPDATE grid SET ex=3", []).unwrap();
        let exponent = print(&db);
        assert_ne!(float, exponent, "exponent change");
        db.execute("UPDATE grid SET al=1", []).unwrap();
        assert_ne!(exponent, print(&db), "partners gone");
    }
}
