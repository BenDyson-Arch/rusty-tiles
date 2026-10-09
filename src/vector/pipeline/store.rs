//! Disk-backed feature/lock indexing, deterministic spatial partitions and LOD.
use super::*;
use crate::runtime::{Attempt, RunEvent};
use encoding::Candidate;
use rayon::prelude::*;
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
pub(super) struct Node {
    pub value: Value,
    pub center: Point,
    pub padding: f64,
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

fn split(
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
            // Boundary ownership comes from the source loops, not triangle
            // incidence. Dropping a zero-area seam face must not expose its
            // interior edges or lose its original (collapsed) boundary edge.
            let mut edges = BTreeSet::new();
            for ring in polygon.loops.split(|i| *i == u32::MAX) {
                for i in 0..ring.len() {
                    let mut pair = [ring[i], ring[(i + 1) % ring.len()]];
                    pair.sort();
                    edges.insert(pair);
                }
            }
            let mut represented = BTreeSet::new();
            let mut parts = Vec::new();
            let mut boundaries = Vec::new();
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
                boundaries.push(
                    (0..3)
                        .filter_map(|i| {
                            let mut pair = [tri[i], tri[(i + 1) % 3]];
                            pair.sort();
                            edges.contains(&pair).then(|| {
                                represented.insert(pair);
                                vec![points[i], points[(i + 1) % 3]]
                            })
                        })
                        .collect::<Vec<_>>(),
                );
            }
            for pair in edges.difference(&represented) {
                boundaries[0].push(vec![
                    polygon.positions[pair[0] as usize],
                    polygon.positions[pair[1] as usize],
                ]);
            }
            counters.fragmented_polygons += 1;
            let mut report = polygon.report;
            report["sourceId"] = feature.source_id().clone();
            report["sourceLayer"] = json!(feature.layer());
            report["reason"]=json!("oversized polygon surfaces partitioned; original boundary rendered separately without internal edges");
            reports.push(report);
            let middle = parts.len() / 2;
            return Ok([(0, middle), (middle, parts.len())]
                .into_iter()
                .map(|(a, b)| {
                    let mut f = feature.clone();
                    f.geometry = Geometry::MultiPolygon(parts[a..b].to_vec());
                    if let Some(intrinsic) = &mut f.intrinsic {
                        intrinsic.geometry = Geometry::MultiPolygon(source_parts[a..b].to_vec());
                    }
                    f.surface_fragment = true;
                    f.triangle_boundaries = boundaries[a..b].to_vec();
                    f
                })
                .collect());
        }
        _ => return Err(FeatureFailure::reject(
            "indivisible geometry or its metadata exceeds tile budget; raise maxVertices/maxBytes",
        )),
    }
    let mut parts: Vec<_> = geometries
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
    if feature.surface_fragment && matches!(feature.geometry, Geometry::MultiPolygon(_)) {
        let middle = feature.triangle_boundaries.len() / 2;
        parts[0].triangle_boundaries = feature.triangle_boundaries[..middle].to_vec();
        parts[1].triangle_boundaries = feature.triangle_boundaries[middle..].to_vec();
    }
    Ok(parts)
}
/// Spool writer: features enter the disk index, oversized ones split first.
#[derive(Clone, Copy)]
struct Spool<'a> {
    db: &'a Connection,
    repair: bool,
    options: &'a VectorOptions,
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
        } = self;
        if hint
            && (feature.geometry.size() > options.max_vertices
                || feature.estimate() > options.max_bytes
                || !encoding::fits_feature(&feature, repair, options)?)
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
fn accept_feature(
    db: &Connection,
    mut feature: Feature,
    repair: bool,
    ambiguous: bool,
    options: &VectorOptions,
    counters: &mut Counters,
    reports: &mut Reports,
) -> FeatureResult<()> {
    db.execute_batch("SAVEPOINT feature;").map_err(sql)?;
    let mut delta_counters = Counters::default();
    let mut delta_reports = Vec::new();
    let result = (|| -> FeatureResult<()> {
        delta_reports.extend(geometry::validate(&mut feature, repair, ambiguous)?);
        Spool {
            db,
            repair,
            options,
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
    let (lo, hi) = bounds(feature.rendered_points())?;
    let center = mul(add(lo, hi), 0.5);
    let bytes = canonical(feature)?;
    let data = String::from_utf8(bytes).unwrap();
    db.execute("INSERT INTO features(path,data,n,estimate,x,y,z,lx,ly,lz,hx,hy,hz,sortkey) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![path,data,feature.geometry.size() as i64,feature.estimate() as i64,center[0],center[1],center[2],lo[0],lo[1],lo[2],hi[0],hi[1],hi[2],format!("{}\0{}\0{}",feature.layer(),feature.source_id().as_str().unwrap(),feature.fragment_path)]).map_err(sql)?;
    let owner = db.last_insert_rowid();
    let mut vertex=db.prepare_cached("INSERT INTO vertices(x,y,z,owner) VALUES(?1,?2,?3,?4) ON CONFLICT(x,y,z) DO UPDATE SET shared=shared OR owner!=excluded.owner").map_err(sql)?;
    for point in feature.rendered_points() {
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
    attempt: &'a Attempt,
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
        let encoder = encoding::Encoder {
            spool: self.spool,
            max_features: self.max_features,
            repair: self.repair,
            options: self.options,
            schemas: self.schemas,
            output: self.output,
        };
        let values: Vec<Result<Candidate, Error>> = self.pool.install(|| {
            levels
                .par_iter()
                .map(|level| encoding::encode(encoder, prefix, center, *level))
                .collect()
        });
        values.into_iter().collect()
    }
    fn consume(
        &mut self,
        candidate: Candidate,
    ) -> Result<(Option<Value>, Option<&'static str>), Error> {
        self.workers.insert(candidate.worker);
        if candidate.node.is_some() {
            for report in candidate.reports {
                self.reports.write(report)?;
            }
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
        let mut deltas = Vec::with_capacity(children.len());
        for child in &mut children {
            let delta = sub(child.center, zcenter);
            child.value["transform"] = crate::tileset_node::translation(delta);
            deltas.push(delta);
        }
        let error = crate::tileset_node::enclose_children(
            &mut half,
            value["geometricError"].as_f64().unwrap_or(0.),
            deltas.into_iter().zip(children.iter().map(|n| &n.value)),
        )?;
        value["boundingVolume"] = json!({"box":crate::tileset_node::box_json(0, half)});
        value["refine"] = json!("REPLACE");
        value["geometricError"] = json!(error);
        if !children.is_empty() {
            // Move, rather than re-serialise, each finished subtree into its parent.
            value["children"] = Value::Array(children.into_iter().map(|n| n.value).collect());
        }
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
        self.attempt
            .check()
            .map_err(|error| Error::Job(self.attempt.fail(error)))?;
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
                    if candidate.node.as_ref().is_some_and(|coarse| {
                        coarse["extras"]["vertices"].as_u64().unwrap()
                            >= node.value["extras"]["vertices"].as_u64().unwrap()
                    }) {
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
    options: &VectorOptions,
    attempt: &Attempt,
) -> Result<CompletedVector, Error> {
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
    let db = Connection::open(&spool).map_err(sql)?;
    db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=OFF; PRAGMA temp_store=FILE; PRAGMA cache_size=-32768;
        CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,n INTEGER,estimate INTEGER,x REAL,y REAL,z REAL,lx REAL,ly REAL,lz REAL,hx REAL,hy REAL,hz REAL,fingerprint TEXT,sortkey TEXT);
        CREATE INDEX paths ON features(path);
        CREATE TABLE vertices(x REAL,y REAL,z REAL,owner INTEGER,shared INTEGER DEFAULT 0,PRIMARY KEY(x,y,z)) WITHOUT ROWID;
        BEGIN;").map_err(sql)?;
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
        attempt,
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
    let diagonal = norm(crate::tileset_node::box_half(
        &root["boundingVolume"]["box"],
    )?) * 2.;
    let root_error = root["geometricError"]
        .as_f64()
        .ok_or_else(|| data("invalid root geometric error"))?;
    // Leave an SSE interval in which the root itself can render (margin 2).
    // Cesium skips the whole tileset below its top-level error, so equality
    // with the root error made large-tolerance/coincident aggregate roots unreachable.
    let top_error = crate::tileset_node::top_level_error(diagonal, root_error, 2.);
    if !top_error.is_finite() {
        return Err(data("vector LOD error overflows; reduce lodTolerance"));
    }
    let mut manifest = json!({"asset":{"version":"1.1"},"extensionsUsed":["3DTILES_content_gltf_vector"],"geometricError":top_error,"root":root});
    let explicit_root = if options.explicit {
        None
    } else {
        Some(manifest["root"].clone())
    };
    if !options.explicit {
        crate::implicit::write_tileset(
            &mut manifest,
            output,
            crate::implicit::SubdivisionScheme::Quadtree,
            true,
        )?;
    }
    let reuse_report = build
        .reuse
        .publish(&mut manifest, frame, explicit_root.as_ref())?;
    std::fs::write(output.join("tileset.json"), serde_json::to_vec(&manifest)?)?;
    build.reports.finish()?;
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
    let display_manifest = if options.explicit {
        manifest.clone()
    } else {
        crate::implicit::expand_tileset(&manifest, |name| Ok(std::fs::read(output.join(name))?))?
    };
    nodes(&display_manifest["root"], &mut list);
    if !options.explicit {
        build.counters.maximum_tile_bytes = list
            .iter()
            .map(|node| node["extras"]["encodedBytes"].as_u64().unwrap_or(0) as usize)
            .max()
            .unwrap_or(0);
    }
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
    if let (Value::Object(report), Value::Object(values)) = (&mut report, values) {
        report.extend(values);
    }
    if options.reproducible {
        report.as_object_mut().unwrap().remove("performance");
    }
    if !options.explicit {
        report["tiling"] = json!("implicit");
    }
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

    fn fixture_db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA journal_mode=MEMORY; CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,n INTEGER,estimate INTEGER,x REAL,y REAL,z REAL,lx REAL,ly REAL,lz REAL,hx REAL,hy REAL,hz REAL,sortkey TEXT); CREATE TABLE vertices(x REAL,y REAL,z REAL,owner INTEGER,shared INTEGER DEFAULT 0,UNIQUE(x,y,z));").unwrap();
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
            triangle_boundaries: Vec::new(),
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
            explicit: true,
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
    fn rejected_late_fragment_restores_exact_sql_inventory() {
        // Fixture expectation is independent of Spool traversal: only the
        // pre-existing accepted source row and its two vertices may survive.
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA journal_mode=MEMORY; CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,n INTEGER,estimate INTEGER,x REAL,y REAL,z REAL,lx REAL,ly REAL,lz REAL,hx REAL,hy REAL,hz REAL,sortkey TEXT); CREATE TABLE vertices(x REAL,y REAL,z REAL,owner INTEGER,shared INTEGER DEFAULT 0,UNIQUE(x,y,z));").unwrap();
        let make = |id: &str, geometry: Geometry| Feature {
            properties: BTreeMap::from([
                ("_source_id".into(), json!(id)),
                ("_source_layer".into(), json!("fixture")),
            ]),
            geometry,
            intrinsic: None,
            surface_fragment: false,
            triangle_boundaries: Vec::new(),
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
            explicit: true,
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
        let outcome = Spool {
            db: &db,
            repair: false,
            options: &options,
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
    fn source_boundaries_outside_a_fill_triangle_enter_spool_bounds() {
        let mut feature: Feature = serde_json::from_value(json!({"properties":{"_source_id":"1","_source_layer":"polar"},"geometry":{"type":"Polygon","coordinates":[[[0.,0.,0.],[1.,0.,0.],[0.,1.,0.],[0.,0.,0.]]]}})).unwrap();
        let before = feature.estimate();
        feature.triangle_boundaries = vec![vec![vec![[10., 20., 30.], [11., 21., 31.]]]];
        assert_eq!(feature.estimate(), before + 64);
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,n INTEGER,estimate INTEGER,x REAL,y REAL,z REAL,lx REAL,ly REAL,lz REAL,hx REAL,hy REAL,hz REAL,sortkey TEXT); CREATE TABLE vertices(x REAL,y REAL,z REAL,owner INTEGER,shared INTEGER DEFAULT 0,UNIQUE(x,y,z));").unwrap();
        insert_row(&db, &feature, "").unwrap();
        let high: Point = db
            .query_row("SELECT hx,hy,hz FROM features", [], |r| {
                Ok([r.get(0)?, r.get(1)?, r.get(2)?])
            })
            .unwrap();
        assert_eq!(high, [11., 21., 31.]);
    }

    #[test]
    fn collapsed_polar_face_keeps_source_boundaries_and_expands_fragment_bounds() {
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
            triangle_boundaries: vec![],
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
        let actual: BTreeSet<_> = parts
            .iter()
            .flat_map(|f| f.triangle_boundaries.iter().flatten())
            .map(|line| edge(line[0], line[1]))
            .collect();
        assert_eq!(
            actual, expected,
            "fragmentation must not introduce internal meridian outlines"
        );
        for part in parts {
            let (lo, hi) = bounds(part.rendered_points()).unwrap();
            for point in part.triangle_boundaries.iter().flatten().flatten() {
                for axis in 0..3 {
                    assert!(point[axis] >= lo[axis] && point[axis] <= hi[axis]);
                }
            }
            assert!(part.estimate() >= part.rendered_points().count() * 32);
        }
    }
}
