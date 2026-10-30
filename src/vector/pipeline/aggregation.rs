//! Opt-in, per-layer count aggregates. Source metadata belongs to full-detail leaves.
use super::*;
use crate::point_sampling::VoxelGrid;
use rusqlite::{params, Connection};

pub(super) struct Aggregates {
    pub features: Vec<Feature>,
    pub summary: Value,
    /// Bounds every source point's distance to its aggregate.
    pub distance: f64,
}

pub(super) enum Outcome {
    Unchanged,
    Rejected(&'static str),
    Ready(Aggregates),
}

pub(super) fn collect(
    db: &Connection,
    prefix: &str,
    options: &VectorOptions,
) -> Result<Outcome, Error> {
    let end = format!("{prefix}~");
    let (points, lo, hi): (i64, Point, Point) = db.query_row(
        "SELECT COALESCE(SUM(n),0),COALESCE(MIN(lx),0),COALESCE(MIN(ly),0),COALESCE(MIN(lz),0),COALESCE(MAX(hx),0),COALESCE(MAX(hy),0),COALESCE(MAX(hz),0) FROM features WHERE path>=?1 AND path<?2",
        params![prefix, end], |r| Ok((r.get(0)?,[r.get(1)?,r.get(2)?,r.get(3)?],[r.get(4)?,r.get(5)?,r.get(6)?]))).map_err(sql)?;
    if points < 2 {
        return Ok(Outcome::Unchanged);
    }
    let budget = options.max_parent_features.min(options.max_vertices);
    let mut side = VoxelGrid::new(lo, sub(hi, lo), budget).side();
    // Coarser grids until the occupied (layer, voxel) cells and their bytes fit:
    // at most log2(side) + 1 passes; one cell per layer is the coarsest.
    let (grid, cells, maximum_distance) = loop {
        let grid = VoxelGrid::new(lo, sub(hi, lo), side.pow(3));
        match pass(db, prefix, &end, &grid, budget, options)? {
            Pass::Mixed => return Ok(Outcome::Unchanged),
            Pass::Done(cells, distance) => break (grid, cells, distance),
            Pass::Over(_) if side > 1 => side /= 2,
            Pass::Over(reason) => return Ok(Outcome::Rejected(reason)),
        }
    };
    if cells.len() as i64 == points {
        return Ok(Outcome::Unchanged);
    }
    // Numerical cushion for the original-to-representative distance calculation.
    let distance_bound = if maximum_distance == 0. {
        0.
    } else {
        maximum_distance + 16. * f64::EPSILON * norm(lo).max(norm(hi)).max(1.)
    };
    let summary = json!({"method":"voxel","sourcePointCount":points,"aggregateCount":cells.len(),
        "maximumDistanceMetres":distance_bound,"cellDiagonalMetres":grid.error_bound(),
        "sourceProperties":"full-detail leaves only","grouping":"source layer and 3D voxel",
        "representative":"first source point in source-identity order"});
    Ok(Outcome::Ready(Aggregates {
        features: cells.into_values().collect(),
        summary,
        distance: distance_bound,
    }))
}

enum Pass {
    Mixed,
    Over(&'static str),
    Done(BTreeMap<(String, usize), Feature>, f64),
}

/// One pass over the tile's rows in source-identity order. A Point row's box is its
/// point and `sortkey` starts with its layer, so only multi-point rows are decoded.
fn pass(
    db: &Connection,
    prefix: &str,
    end: &str,
    grid: &VoxelGrid,
    budget: usize,
    options: &VectorOptions,
) -> Result<Pass, Error> {
    let mut cells: BTreeMap<(String, usize), Feature> = BTreeMap::new();
    let mut maximum_distance: f64 = 0.;
    let mut estimate = 0usize;
    let mut rows = db
        .prepare("SELECT n,lx,ly,lz,sortkey,id FROM features WHERE path>=?1 AND path<?2 ORDER BY sortkey,id")
        .map_err(sql)?;
    // Sorted without `data`: only rows that are not plain points read it, by id.
    let mut data = db
        .prepare_cached("SELECT data FROM features WHERE id=?1")
        .map_err(sql)?;
    let mut rows = rows.query(params![prefix, end]).map_err(sql)?;
    while let Some(row) = rows.next().map_err(sql)? {
        let sortkey: String = row.get(4).map_err(sql)?;
        let parts: Vec<&str> = sortkey.split('\0').collect();
        let (layer, multi) = if row.get::<_, i64>(0).map_err(sql)? == 1 && parts.len() == 3 {
            let point = [
                row.get(1).map_err(sql)?,
                row.get(2).map_err(sql)?,
                row.get(3).map_err(sql)?,
            ];
            (parts[0].to_owned(), vec![point])
        } else {
            let id: i64 = row.get(5).map_err(sql)?;
            let text: String = data.query_row([id], |r| r.get(0)).map_err(sql)?;
            let source: Feature = serde_json::from_str(&text)?;
            let (Geometry::Point(_) | Geometry::MultiPoint(_)) = source.geometry else {
                return Ok(Pass::Mixed);
            };
            (
                source.layer().to_owned(),
                source.geometry.points().copied().collect(),
            )
        };
        for point in multi {
            let key = (layer.clone(), grid.key(point));
            if let Some(aggregate) = cells.get_mut(&key) {
                let count = aggregate.properties["pointCount"].as_i64().unwrap();
                aggregate
                    .properties
                    .insert("pointCount".into(), json!(count + 1));
                let Geometry::Point(representative) = aggregate.geometry else {
                    unreachable!()
                };
                maximum_distance = maximum_distance.max(norm(sub(point, representative)));
            } else {
                if cells.len() == budget {
                    return Ok(Pass::Over("pointAggregationBudget"));
                }
                let aggregate = Feature {
                    intrinsic: None,
                    geometry: Geometry::Point(point),
                    properties: BTreeMap::from([
                        ("aggregation".into(), json!("voxel")),
                        ("pointCount".into(), json!(1)),
                        ("sourceLayer".into(), json!(layer)),
                    ]),
                    surface_fragment: false,
                    fragment_path: String::new(),
                };
                estimate = estimate.saturating_add(aggregate.estimate() - 2048);
                if estimate > options.max_bytes.saturating_mul(2) {
                    return Ok(Pass::Over("estimatedBytes"));
                }
                cells.insert(key, aggregate);
            }
        }
    }
    Ok(Pass::Done(cells, maximum_distance))
}
