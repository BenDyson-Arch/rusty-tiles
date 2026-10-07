//! Disk-backed feature/lock indexing, deterministic spatial partitions and LOD.
use super::*;
use encoding::Candidate;
use rayon::prelude::*;
use reuse::{Cut, Reuse};
use rusqlite::{params, Connection};
use source::{Frame, Reader};
use std::{
    cell::{Cell, RefCell},
    io::Write,
    path::PathBuf,
    time::Instant,
};

#[derive(Default, Serialize)]
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
pub(super) struct Node {
    pub value: Value,
    pub center: Point,
    pub padding: f64,
}
struct Reports {
    file: std::fs::File,
    first: Vec<Value>,
    count: usize,
}
impl Reports {
    fn write(&mut self, value: Value) -> Result<(), Error> {
        self.count += 1;
        serde_json::to_writer(&mut self.file, &value)?;
        self.file.write_all(b"\n")?;
        if self.first.len() < 100 {
            self.first.push(value);
        }
        Ok(())
    }
}
fn progress(phase: &str, done: usize, total: Option<usize>) {
    if std::env::var("RUSTY_TILES_PROGRESS_JSON").as_deref() == Ok("1") {
        eprintln!(
            "{}",
            json!({"event":"progress","phase":phase,"done":done,"total":total})
        );
    }
}

fn split(
    feature: &Feature,
    repair: bool,
    counters: &mut Counters,
    reports: &mut Reports,
) -> Result<Vec<Feature>, Error> {
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
                return Err(data(
                    "indivisible polygon fill/boundary or metadata exceeds tile budget",
                ));
            }
            let mut f = feature.clone();
            f.geometry = Geometry::Polygon(p[0].clone());
            return split(&f, repair, counters, reports);
        }
        Geometry::Polygon(rings) => {
            let polygon = geometry::polygon(rings, repair)?;
            let triangles: Vec<_> = polygon
                .indices
                .chunks_exact(3)
                .map(|v| <[u32; 3]>::try_from(v).unwrap())
                .collect();
            if triangles.len() <= 1 {
                return Err(data("one triangle or its metadata exceeds tile budget"));
            }
            let mut edges = BTreeMap::new();
            for tri in &triangles {
                for i in 0..3 {
                    let mut pair = [tri[i], tri[(i + 1) % 3]];
                    pair.sort();
                    *edges.entry(pair).or_insert(0) += 1;
                }
            }
            let mut parts = Vec::new();
            let mut boundaries = Vec::new();
            for tri in triangles {
                let points: Vec<_> = tri.iter().map(|i| polygon.positions[*i as usize]).collect();
                let mut ring = points.clone();
                ring.push(points[0]);
                parts.push(vec![ring]);
                boundaries.push(
                    (0..3)
                        .filter_map(|i| {
                            let mut pair = [tri[i], tri[(i + 1) % 3]];
                            pair.sort();
                            (edges[&pair] == 1).then(|| vec![points[i], points[(i + 1) % 3]])
                        })
                        .collect::<Vec<_>>(),
                );
            }
            counters.fragmented_polygons += 1;
            let mut report = polygon.report;
            report["sourceId"] = feature.source_id().clone();
            report["sourceLayer"] = json!(feature.layer());
            report["reason"]=json!("oversized polygon surfaces partitioned; original boundary rendered separately without internal edges");
            reports.write(report)?;
            let middle = parts.len() / 2;
            return Ok([(0, middle), (middle, parts.len())]
                .into_iter()
                .map(|(a, b)| {
                    let mut f = feature.clone();
                    f.geometry = Geometry::MultiPolygon(parts[a..b].to_vec());
                    f.surface_fragment = true;
                    f.triangle_boundaries = boundaries[a..b].to_vec();
                    f
                })
                .collect());
        }
        _ => return Err(data(
            "indivisible geometry or its metadata exceeds tile budget; raise maxVertices/maxBytes",
        )),
    }
    let mut parts: Vec<_> = geometries
        .into_iter()
        .map(|geometry| {
            let mut f = feature.clone();
            f.geometry = geometry;
            f
        })
        .collect();
    if feature.surface_fragment && matches!(feature.geometry, Geometry::MultiPolygon(_)) {
        let middle = feature.triangle_boundaries.len() / 2;
        parts[0].triangle_boundaries = feature.triangle_boundaries[..middle].to_vec();
        parts[1].triangle_boundaries = feature.triangle_boundaries[middle..].to_vec();
    }
    Ok(parts)
}
fn insert(
    db: &Connection,
    feature: Feature,
    path: &str,
    hint: bool,
    repair: bool,
    options: &VectorOptions,
    counters: &mut Counters,
    reports: &mut Reports,
) -> Result<(), Error> {
    if hint
        && (feature.geometry.size() > options.max_vertices
            || feature.estimate() > options.max_bytes)
    {
        for (index, mut part) in split(&feature, repair, counters, reports)?
            .into_iter()
            .enumerate()
        {
            part.fragment_path = format!("{}{index}", feature.fragment_path);
            insert(db, part, path, true, repair, options, counters, reports)?;
        }
        return Ok(());
    }
    let (lo, hi) = bounds(feature.geometry.points())?;
    let center = mul(add(lo, hi), 0.5);
    let bytes = canonical(&feature)?;
    let data = String::from_utf8(bytes).unwrap();
    db.execute("INSERT INTO features(path,data,n,estimate,x,y,z,lx,ly,lz,hx,hy,hz,sortkey) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![path,data,feature.geometry.size() as i64,feature.estimate() as i64,center[0],center[1],center[2],lo[0],lo[1],lo[2],hi[0],hi[1],hi[2],format!("{}\0{}\0{}",feature.layer(),feature.source_id().as_str().unwrap(),feature.fragment_path)]).map_err(sql)?;
    let owner = db.last_insert_rowid();
    let mut vertex=db.prepare_cached("INSERT INTO vertices(x,y,z,owner) VALUES(?1,?2,?3,?4) ON CONFLICT(x,y,z) DO UPDATE SET shared=shared OR owner!=excluded.owner").map_err(sql)?;
    for point in feature.geometry.points() {
        vertex
            .execute(params![point[0], point[1], point[2], owner])
            .map_err(sql)?;
    }
    Ok(())
}
fn fingerprint(db: &Connection, id: i64, value: &str) -> Result<(), Error> {
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
    db.execute(
        "UPDATE features SET fingerprint=?1 WHERE id=?2",
        params![format!("{:x}", hasher.finalize()), id],
    )
    .map_err(sql)?;
    Ok(())
}
struct Stats {
    count: usize,
    vertices: usize,
    estimate: usize,
    lo: Point,
    hi: Point,
}
struct Build<'a> {
    db: &'a Connection,
    spool: &'a Path,
    output: &'a Path,
    options: &'a VectorOptions,
    schemas: &'a BTreeMap<String, String>,
    max_features: usize,
    repair: bool,
    pool: rayon::ThreadPool,
    counters: Counters,
    reports: Reports,
    reuse: Reuse,
    workers: BTreeSet<usize>,
    partition_seconds: f64,
}
impl Build<'_> {
    fn stats(&self, prefix: &str) -> Result<Stats, Error> {
        self.db.query_row("SELECT COUNT(*),COALESCE(SUM(n),0),COALESCE(SUM(estimate),0),COALESCE(MIN(lx),0),COALESCE(MIN(ly),0),COALESCE(MIN(lz),0),COALESCE(MAX(hx),0),COALESCE(MAX(hy),0),COALESCE(MAX(hz),0) FROM features WHERE path>=?1 AND path<?2",params![prefix,format!("{prefix}~")],|r|Ok(Stats{count:r.get::<_,i64>(0)? as usize,vertices:r.get::<_,i64>(1)? as usize,estimate:r.get::<_,i64>(2)? as usize,lo:[r.get(3)?,r.get(4)?,r.get(5)?],hi:[r.get(6)?,r.get(7)?,r.get(8)?]})).map_err(sql)
    }
    fn candidates(
        &mut self,
        prefix: &str,
        center: Point,
        levels: &[u32],
    ) -> Result<Vec<Candidate>, Error> {
        let (spool, max_features, repair, options, schemas, output) = (
            self.spool,
            self.max_features,
            self.repair,
            self.options,
            self.schemas,
            self.output,
        );
        let values: Vec<Result<Candidate, Error>> = self.pool.install(|| {
            levels
                .par_iter()
                .map(|level| {
                    encoding::encode(
                        spool,
                        prefix,
                        center,
                        *level,
                        max_features,
                        repair,
                        options,
                        schemas,
                        output,
                    )
                })
                .collect()
        });
        values.into_iter().collect()
    }
    fn consume(
        &mut self,
        candidate: Candidate,
    ) -> Result<(Option<Value>, Option<&'static str>), Error> {
        self.workers.insert(candidate.worker);
        for report in candidate.reports {
            self.reports.write(report)?;
        }
        Ok((candidate.node, candidate.reason))
    }
    fn attach(
        &mut self,
        mut value: Value,
        stats: &Stats,
        center: Point,
        mut children: Vec<Node>,
    ) -> Result<Node, Error> {
        let zcenter = zup(center);
        let dimensions = sub(stats.hi, stats.lo);
        let mut half = [
            dimensions[0] * 0.5,
            dimensions[2] * 0.5,
            dimensions[1] * 0.5,
        ]
        .map(|v| v.max(0.001));
        let mut padding = value["extras"]["positionRoundingMetres"]
            .as_f64()
            .unwrap_or(0.)
            + value["extras"]["quantizationErrorMetres"]
                .as_f64()
                .unwrap_or(0.);
        for child in &children {
            padding = padding.max(child.padding);
        }
        half = half.map(|v| v + padding);
        let mut error = value["geometricError"].as_f64().unwrap_or(0.);
        for child in &mut children {
            let delta = sub(child.center, zcenter);
            child.value["transform"] =
                json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, delta[0], delta[1], delta[2], 1]);
            let box_ = child.value["boundingVolume"]["box"]
                .as_array()
                .ok_or_else(|| data("invalid child bounds"))?;
            for i in 0..3 {
                half[i] = half[i].max(
                    delta[i].abs()
                        + box_[3 + i * 4]
                            .as_f64()
                            .ok_or_else(|| data("invalid child bounds"))?,
                );
            }
            error = error.max(child.value["geometricError"].as_f64().unwrap_or(0.));
        }
        value["boundingVolume"] = json!({"box":[0,0,0,half[0],0,0,0,half[1],0,0,0,half[2]]});
        value["refine"] = json!("REPLACE");
        value["geometricError"] = json!(error);
        if !children.is_empty() {
            value["children"] = json!(children.into_iter().map(|n| n.value).collect::<Vec<_>>());
        }
        self.counters.tiles += 1;
        progress("encoding", self.counters.tiles, None);
        if self.counters.tiles > self.options.max_tiles {
            return Err(data(
                "hierarchy exceeds maxTiles; raise budgets or maxTiles explicitly",
            ));
        }
        if !reuse::contents(&value).is_empty() {
            self.counters.maximum_tile_vertices = self
                .counters
                .maximum_tile_vertices
                .max(value["extras"]["vertices"].as_u64().unwrap_or(0) as usize);
            self.counters.maximum_tile_bytes = self
                .counters
                .maximum_tile_bytes
                .max(value["extras"]["encodedBytes"].as_u64().unwrap_or(0) as usize);
        }
        Ok(Node {
            value,
            center: zcenter,
            padding,
        })
    }
    fn build(&mut self, prefix: &str, depth: usize) -> Result<Node, Error> {
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
        let count = self.stats(prefix)?.count;
        self.reuse.remember(prefix, signature, &mut node, count);
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
            let levels: Vec<_> = (0..=u32::from(self.options.lod.levels)).collect();
            let mut candidates = self.candidates(prefix, center, &levels)?.into_iter();
            let (leaf, _) = self.consume(candidates.next().unwrap())?;
            if let Some(leaf) = leaf {
                self.counters.leaf_tiles += 1;
                let mut node = self.attach(leaf, &stats, center, Vec::new())?;
                for candidate in candidates {
                    let (coarse, _) = self.consume(candidate)?;
                    if let Some(coarse) = coarse {
                        if coarse["extras"]["vertices"].as_u64().unwrap()
                            >= node.value["extras"]["vertices"].as_u64().unwrap()
                        {
                            continue;
                        }
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
            let parts = split(&feature, self.repair, &mut self.counters, &mut self.reports)?;
            self.db
                .execute("DELETE FROM features WHERE id=?1", params![id])
                .map_err(sql)?;
            let n = parts.len();
            for (index, mut part) in parts.into_iter().enumerate() {
                part.fragment_path = format!("{}{index}", feature.fragment_path);
                insert(
                    self.db,
                    part,
                    prefix,
                    false,
                    self.repair,
                    self.options,
                    &mut self.counters,
                    &mut self.reports,
                )?;
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
                fingerprint(self.db, id, &value)?;
            }
            return self.build_uncached(prefix, depth + 1);
        }
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
        let candidate = self
            .candidates(prefix, center, &[level.max(1)])?
            .pop()
            .unwrap();
        let (coarse, reason) = self.consume(candidate)?;
        let coarse=coarse.unwrap_or_else(||{self.counters.routing_tiles+=1;json!({"geometricError":norm(sub(stats.hi,stats.lo)),"extras":{"routing":true,"routingReason":reason}})});
        self.attach(coarse, &stats, center, children)
    }
}

pub(super) fn convert(
    input: &Path,
    output: &Path,
    max_features: usize,
    repair: bool,
    ambiguous: bool,
    options: &VectorOptions,
) -> Result<(), Error> {
    super::available()?;
    let started = Instant::now();
    progress("ingestion", 0, None);
    std::fs::create_dir_all(output.join("t"))?;
    let reuse = Reuse::new(output, options)?;
    let mut reader = Reader::new(input, options, reuse.frame.clone())?;
    let scratch = tempfile::tempdir_in(output.parent().unwrap_or(Path::new(".")))?;
    let spool: PathBuf = scratch.path().join("features.sqlite");
    let db = Connection::open(&spool).map_err(sql)?;
    db.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=FILE; PRAGMA cache_size=-32768;
        CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,n INTEGER,estimate INTEGER,x REAL,y REAL,z REAL,lx REAL,ly REAL,lz REAL,hx REAL,hy REAL,hz REAL,fingerprint TEXT,sortkey TEXT);
        CREATE INDEX paths ON features(path);
        CREATE TABLE vertices(x REAL,y REAL,z REAL,owner INTEGER,shared INTEGER DEFAULT 0,PRIMARY KEY(x,y,z)) WITHOUT ROWID;
        BEGIN;").map_err(sql)?;
    let counters = RefCell::new(Counters::default());
    let reports = RefCell::new(Reports {
        file: std::fs::File::create(output.join("geometry-reports.jsonl"))?,
        first: Vec::new(),
        count: 0,
    });
    let failures = Cell::new(0usize);
    let first_failure = RefCell::new(None);
    reader.read(
        options,
        |mut feature| {
            db.execute_batch("SAVEPOINT feature;").map_err(sql)?;
            let result = (|| {
                let geometry_reports = geometry::validate(&mut feature, repair, ambiguous)?;
                insert(
                    &db,
                    feature,
                    "",
                    true,
                    repair,
                    options,
                    &mut counters.borrow_mut(),
                    &mut reports.borrow_mut(),
                )?;
                for report in geometry_reports {
                    reports.borrow_mut().write(report)?;
                }
                counters.borrow_mut().features += 1;
                Ok(())
            })();
            if result.is_err() {
                db.execute_batch("ROLLBACK TO feature;").map_err(sql)?;
            }
            db.execute_batch("RELEASE feature;").map_err(sql)?;
            result
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
                if std::env::var("RUSTY_TILES_PROGRESS_JSON").as_deref() == Ok("1") {
                    let mut warning = value.clone();
                    warning["event"] = json!("warning");
                    eprintln!("{warning}");
                } else {
                    eprintln!(
                        "layer '{}', feature {}: {}",
                        value["sourceLayer"].as_str().unwrap_or(""),
                        value["sourceId"].as_str().unwrap_or(""),
                        value["reason"].as_str().unwrap_or("")
                    );
                }
            }
            reports.borrow_mut().write(value)
        },
    )?;
    if failures.get() > 0 && !options.skip_invalid {
        return Err(data(format!("{} unconvertible feature(s); first failure: {}. No tileset published. Fix the reported features or explicitly use --skipInvalid.",failures.get(),first_failure.borrow().as_deref().unwrap())));
    }
    let ingestion_seconds = started.elapsed().as_secs_f64();
    progress(
        "ingestion",
        counters.borrow().features,
        Some(counters.borrow().features),
    );
    db.execute_batch("COMMIT;").map_err(sql)?;
    let mut counters = counters.into_inner();
    counters.skipped_features = failures.get();
    counters.features_without_geometry = reader.without_geometry;
    if counters.features == 0 && !reuse.has_previous() {
        if reader.without_geometry == 0 && options.where_clause.is_none() {
            return Err(data("empty selected layers"));
        }
        reader.frame = Some(Frame::local([0.; 3]));
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
        fingerprint(&db, id, &value)?;
    }
    drop(stmt);
    db.execute_batch("COMMIT;").map_err(sql)?;
    let partition_seconds = started.elapsed().as_secs_f64() - ingestion_seconds;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .build()
        .map_err(|e| Error::Environment(format!("cannot start vector workers: {e}")))?;
    let mut build = Build {
        db: &db,
        spool: &spool,
        output,
        options,
        schemas: &reader.schemas,
        max_features,
        repair,
        pool,
        counters,
        reports: reports.into_inner(),
        reuse,
        workers: BTreeSet::new(),
        partition_seconds,
    };
    let encoding_started = Instant::now();
    let before_partition = build.partition_seconds;
    let root = if build.counters.features > 0 {
        build.build("", 0)?
    } else {
        let mut node = Node {
            value: json!({"boundingVolume":{"box":[0,0,0,0.001,0,0,0,0.001,0,0,0,0.001]},"geometricError":0,"refine":"REPLACE"}),
            center: [0.; 3],
            padding: 0.,
        };
        build.counters.tiles = 1;
        build
            .reuse
            .remember("", build.reuse.signature(&db, "")?, &mut node, 0);
        node
    };
    let encoding_seconds = (encoding_started.elapsed().as_secs_f64()
        - (build.partition_seconds - before_partition))
        .max(0.);
    let publication_started = Instant::now();
    let frame = reader
        .frame
        .as_ref()
        .ok_or_else(|| data("missing local vector frame"))?;
    let anchor = (0..3).fold(frame.anchor, |p, i| {
        add(p, mul(frame.axes[i], root.center[i]))
    });
    let mut root = root.value;
    root["transform"] = json!([
        frame.axes[0][0],
        frame.axes[0][1],
        frame.axes[0][2],
        0,
        frame.axes[1][0],
        frame.axes[1][1],
        frame.axes[1][2],
        0,
        frame.axes[2][0],
        frame.axes[2][1],
        frame.axes[2][2],
        0,
        anchor[0],
        anchor[1],
        anchor[2],
        1
    ]);
    let box_ = root["boundingVolume"]["box"].as_array().unwrap();
    let diagonal = norm([
        box_[3].as_f64().unwrap(),
        box_[7].as_f64().unwrap(),
        box_[11].as_f64().unwrap(),
    ]) * 2.;
    // Leave an SSE interval in which the root itself can render. Cesium skips
    // the whole tileset below its top-level error, so equality with the root
    // error made large-tolerance/coincident aggregate roots unreachable.
    let top_error = 1f64
        .max(root["geometricError"].as_f64().unwrap() * 2.)
        .max(diagonal);
    if !top_error.is_finite() {
        return Err(data("vector LOD error overflows; reduce lodTolerance"));
    }
    let mut manifest = json!({"asset":{"version":"1.1"},"extensionsUsed":["3DTILES_content_gltf_vector"],"geometricError":top_error,"root":root});
    let reuse_report = build.reuse.publish(&mut manifest, frame)?;
    std::fs::write(output.join("tileset.json"), serde_json::to_vec(&manifest)?)?;
    build.reports.file.flush()?;
    let shared: usize = db
        .query_row("SELECT COUNT(*) FROM vertices WHERE shared=1", [], |r| {
            Ok(r.get::<_, i64>(0)? as usize)
        })
        .map_err(sql)?;
    fn nodes<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
        out.push(node);
        if let Some(children) = node["children"].as_array() {
            for child in children {
                nodes(child, out);
            }
        }
    }
    let mut list = Vec::new();
    nodes(&manifest["root"], &mut list);
    let sum = |key: &str| {
        list.iter()
            .map(|n| n["extras"][key].as_u64().unwrap_or(0))
            .sum::<u64>()
    };
    let max = |key: &str| {
        list.iter()
            .map(|n| n["extras"][key].as_f64().unwrap_or(0.))
            .fold(0., f64::max)
    };
    let mut report = serde_json::to_value(&build.counters)?;
    let values = json!({"inputDriver":reader.driver,"layers":reader.layer_reports,
        "performance":{"jobs":options.jobs,"workersUsed":build.workers.len(),"phaseSeconds":{"ingestion":ingestion_seconds,"partitioning":build.partition_seconds,"encoding":encoding_seconds,"publication":publication_started.elapsed().as_secs_f64()}},
        "budgets":{"features":max_features,"parentFeatures":options.max_parent_features,"vertices":options.max_vertices,"bytes":options.max_bytes,"tiles":options.max_tiles},
        "attributeFilter":options.where_clause,"metadata":{"listFields":options.list_fields,"fields":options.fields,"dropFields":options.drop_fields},
        "encoding":{"quantize":options.quantize,"meshopt":options.meshopt,"primitiveReferences":sum("primitives"),"maximumTilePrimitives":list.iter().map(|n| n["extras"]["primitives"].as_u64().unwrap_or(0)).max().unwrap_or(0),"maximumQuantizationErrorMetres":max("quantizationErrorMetres"),"uncompressedTileBytes":sum("uncompressedBytes"),"encodedTileBytes":sum("encodedBytes")},
        "parentRepairEnabled":options.parent_repair,"skipInvalidEnabled":options.skip_invalid,"repairEnabled":repair,"lodToleranceMetres":options.lod.tolerance_metres,"lodLevels":options.lod.levels,"reuse":reuse_report,
        "lodFallbacks":build.reports.first,"geometryReportCount":build.reports.count,"geometryReports":"geometry-reports.jsonl","geometryReportsScope":"current ingestion and newly encoded geometry; previous content reports remain in the prior archive",
        "lockedSharedVertices":shared,"pointPolicy":if options.aggregate_points {"opt-in per-layer voxel count aggregates in point-only parents; original identities and properties in full-detail leaves"} else {"retain every semantic point feature; oversized parents route without content"},
        "pointAggregation":{"enabled":options.aggregate_points,"contentTiles":list.iter().filter(|n| n["extras"].get("pointAggregation").is_some()).count()},
        "polygonFragmentPolicy":"standard glTF fills plus vector source boundaries; no internal fragment outlines",
        "errorPolicy":"direct original-to-parent distance plus float32 rounding; all source bounds retained"});
    report
        .as_object_mut()
        .unwrap()
        .extend(values.as_object().unwrap().clone());
    if options.reproducible {
        report.as_object_mut().unwrap().remove("performance");
    }
    std::fs::write(
        output.join("conversion.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
