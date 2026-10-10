//! Opt-in, per-layer count aggregates. Source metadata belongs to full-detail leaves.
use super::*;
use crate::point_sampling::VoxelGrid;
use rusqlite::{params, Connection};

pub(super) struct Aggregates {
    pub features: Vec<Feature>,
    pub summary: Value,
}

pub(super) enum Outcome {
    Unchanged,
    Rejected(&'static str),
    Ready(Aggregates),
}

pub(super) fn collect(
    db: &Connection,
    prefix: &str,
    tolerance: f64,
    options: &VectorOptions,
) -> Result<Outcome, Error> {
    let end = format!("{prefix}~");
    let mixed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM features WHERE path>=?1 AND path<?2 AND json_extract(data,'$.geometry.type') NOT IN ('Point','MultiPoint'))",
        params![prefix, end], |r| r.get(0)).map_err(sql)?;
    if mixed {
        return Ok(Outcome::Unchanged);
    }
    let (points, lo, hi): (i64, Point, Point) = db.query_row(
        "SELECT COALESCE(SUM(n),0),COALESCE(MIN(lx),0),COALESCE(MIN(ly),0),COALESCE(MIN(lz),0),COALESCE(MAX(hx),0),COALESCE(MAX(hy),0),COALESCE(MAX(hz),0) FROM features WHERE path>=?1 AND path<?2",
        params![prefix, end], |r| Ok((r.get(0)?,[r.get(1)?,r.get(2)?,r.get(3)?],[r.get(4)?,r.get(5)?,r.get(6)?]))).map_err(sql)?;
    if points < 2 {
        return Ok(Outcome::Unchanged);
    }
    let budget = options.max_parent_features.min(options.max_vertices);
    let grid = VoxelGrid::new(lo, sub(hi, lo), budget);
    let mut cells: BTreeMap<(String, usize), Feature> = BTreeMap::new();
    let mut maximum_distance: f64 = 0.;
    let mut estimate = 0usize;
    let mut rows = db
        .prepare("SELECT data FROM features WHERE path>=?1 AND path<?2 ORDER BY sortkey,id")
        .map_err(sql)?;
    let mut rows = rows.query(params![prefix, end]).map_err(sql)?;
    while let Some(row) = rows.next().map_err(sql)? {
        let source: Feature = serde_json::from_str(&row.get::<_, String>(0).map_err(sql)?)?;
        for point in source.geometry.points() {
            let key = (source.layer().to_owned(), grid.key(*point));
            if let Some(aggregate) = cells.get_mut(&key) {
                let count = aggregate.properties["pointCount"].as_i64().unwrap();
                aggregate
                    .properties
                    .insert("pointCount".into(), json!(count + 1));
                let Geometry::Point(representative) = aggregate.geometry else {
                    unreachable!()
                };
                maximum_distance = maximum_distance.max(norm(sub(*point, representative)));
                if maximum_distance > tolerance {
                    return Ok(Outcome::Rejected("pointAggregationTolerance"));
                }
            } else {
                if cells.len() == budget {
                    return Ok(Outcome::Rejected("pointAggregationBudget"));
                }
                let aggregate = Feature {
                    intrinsic: None,
                    geometry: Geometry::Point(*point),
                    properties: BTreeMap::from([
                        ("aggregation".into(), json!("voxel")),
                        ("pointCount".into(), json!(1)),
                        ("sourceLayer".into(), json!(source.layer())),
                    ]),
                    surface_fragment: false,
                    triangle_boundaries: Vec::new(),
                    fragment_path: String::new(),
                };
                estimate = estimate.saturating_add(aggregate.estimate() - 2048);
                if estimate > options.max_bytes.saturating_mul(2) {
                    return Ok(Outcome::Rejected("estimatedBytes"));
                }
                cells.insert(key, aggregate);
            }
        }
    }
    if cells.len() as i64 == points {
        return Ok(Outcome::Unchanged);
    }
    // Numerical cushion for the original-to-representative distance calculation.
    let distance_bound = if maximum_distance == 0. {
        0.
    } else {
        maximum_distance + 16. * f64::EPSILON * norm(lo).max(norm(hi)).max(1.)
    };
    if distance_bound > tolerance {
        return Ok(Outcome::Rejected("pointAggregationTolerance"));
    }
    let summary = json!({"method":"voxel","sourcePointCount":points,"aggregateCount":cells.len(),
        "maximumDistanceMetres":distance_bound,"cellDiagonalMetres":grid.error_bound(),
        "sourceProperties":"full-detail leaves only","grouping":"source layer and 3D voxel",
        "representative":"first source point in source-identity order"});
    Ok(Outcome::Ready(Aggregates {
        features: cells.into_values().collect(),
        summary,
    }))
}
