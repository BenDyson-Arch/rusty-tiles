//! Regular-grid quantized-mesh 1.0 encoder. Heights use one float32-ended
//! interval for the whole pyramid, so adjacent edges decode identically.
use crate::Error;
const A: f64 = 6_378_137.;
const B: f64 = 6_356_752.314_245_179;

fn length(p: [f64; 3]) -> f64 {
    p.iter().map(|v| v * v).sum::<f64>().sqrt()
}
fn ecef(lon: f64, lat: f64, height: f64) -> [f64; 3] {
    let (lon, lat) = (lon.to_radians(), lat.to_radians());
    let e2 = 1. - (B / A).powi(2);
    let n = A / (1. - e2 * lat.sin().powi(2)).sqrt();
    [
        (n + height) * lat.cos() * lon.cos(),
        (n + height) * lat.cos() * lon.sin(),
        (n * (1. - e2) + height) * lat.sin(),
    ]
}

#[allow(clippy::too_many_arguments)] // Tile bounds, samples and encoding/error limits.
pub(super) fn encode(
    west: f64,
    south: f64,
    size: f64,
    heights: &[f64],
    grid: u16,
    floor: f64,
    ceiling: f64,
    max_error: f64,
) -> Result<Encoded, Error> {
    let count = usize::from(grid);
    if ![17, 33, 65, 129].contains(&grid)
        || heights.len() != count * count
        || !heights.iter().all(|h| h.is_finite())
    {
        return Err(Error::Data("invalid finite terrain sample grid".into()));
    }
    let uv: Vec<_> = (0..count)
        .map(|i| (i as f64 * 32767. / (count - 1) as f64).round_ties_even() as i32)
        .collect();
    let h: Vec<_> = heights
        .iter()
        .map(|height| {
            ((height - floor) / (ceiling - floor).max(1e-30) * 32767.)
                .round_ties_even()
                .clamp(0., 32767.) as i32
        })
        .collect();
    let xyz: Vec<_> = (0..count * count)
        .map(|i| {
            ecef(
                west + f64::from(uv[i % count]) / 32767. * size,
                south + f64::from(uv[i / count]) / 32767. * size,
                floor + f64::from(h[i]) / 32767. * (ceiling - floor),
            )
        })
        .collect();
    let center: [f64; 3] = std::array::from_fn(|axis| {
        let min = xyz.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
        let max = xyz
            .iter()
            .map(|p| p[axis])
            .fold(f64::NEG_INFINITY, f64::max);
        (min + max) / 2.
    });
    let radius = xyz
        .iter()
        .map(|p| length(std::array::from_fn(|axis| p[axis] - center[axis])))
        .fold(0., f64::max);
    let mut direction = [center[0] / A, center[1] / A, center[2] / B];
    let norm = length(direction);
    direction = if norm > 0. {
        direction.map(|v| v / norm)
    } else {
        [1., 0., 0.]
    };
    // Hemispheres/negative heights use a conservative point at infinity.
    // A zero horizon point would incorrectly cull the root at the Earth centre.
    let mut magnitude = 1e15;
    if size < 90. && floor >= 0. {
        let mut candidate: f64 = 0.;
        for p in &xyz {
            let scaled = [p[0] / A, p[1] / A, p[2] / B];
            let len = length(scaled);
            let unit = scaled.map(|v| v / len);
            let cosine: f64 = (0..3).map(|i| unit[i] * direction[i]).sum();
            let sine = length([
                unit[1] * direction[2] - unit[2] * direction[1],
                unit[2] * direction[0] - unit[0] * direction[2],
                unit[0] * direction[1] - unit[1] * direction[0],
            ]);
            let denom = (cosine - sine * (len * len - 1.).max(0.).sqrt()) / len.max(1.);
            if denom <= 0. || !denom.is_finite() {
                candidate = 1e15;
                break;
            }
            candidate = candidate.max(1. / denom);
        }
        magnitude = candidate;
    }
    let hop = direction.map(|v| v * magnitude);
    let mut triangles = Vec::with_capacity((count - 1).pow(2) * 6);
    for y in 0..count - 1 {
        for x in 0..count - 1 {
            let a = y * count + x;
            triangles.extend([a, a + 1, a + count, a + 1, a + count + 1, a + count]);
        }
    }
    let (triangles, mut statistics) =
        super::simplify::reduce(&xyz, &uv, &h, ceiling - floor, &triangles, max_error)?;
    let mut order = Vec::with_capacity(count * count);
    let mut remap = vec![usize::MAX; count * count];
    for &index in &triangles {
        if remap[index] == usize::MAX {
            remap[index] = order.len();
            order.push(index);
        }
    }
    let mut out = Vec::with_capacity(92 + count * count * 6 + triangles.len() * 2 + 20 + count * 8);
    for v in center {
        out.extend(v.to_le_bytes());
    }
    out.extend((floor as f32).to_le_bytes());
    out.extend((ceiling as f32).to_le_bytes());
    for v in center.into_iter().chain([radius]).chain(hop) {
        out.extend(v.to_le_bytes());
    }
    out.extend((order.len() as u32).to_le_bytes());
    for attribute in 0..3 {
        let mut previous = 0_i32;
        for &index in &order {
            let value = match attribute {
                0 => uv[index % count],
                1 => uv[index / count],
                _ => h[index],
            };
            let delta = value - previous;
            previous = value;
            out.extend((((delta << 1) ^ (delta >> 31)) as u16).to_le_bytes());
        }
    }
    out.extend(((triangles.len() / 3) as u32).to_le_bytes());
    let mut high = 0;
    for index in triangles {
        let index = remap[index];
        out.extend(((high - index) as u16).to_le_bytes());
        if index == high {
            high += 1;
        }
    }
    for edge in 0..4 {
        out.extend(u32::from(grid).to_le_bytes());
        for i in 0..count {
            let index = match edge {
                0 => i * count,
                1 => i,
                2 => i * count + count - 1,
                _ => i + count * (count - 1),
            };
            out.extend((remap[index] as u16).to_le_bytes());
        }
    }
    statistics.output_vertices = order.len() as u64;
    Ok(Encoded {
        bytes: out,
        statistics,
    })
}

pub(super) struct Encoded {
    pub bytes: Vec<u8>,
    pub statistics: Statistics,
}
#[derive(Default)]
pub(super) struct Statistics {
    pub input_vertices: u64,
    pub output_vertices: u64,
    pub input_triangles: u64,
    pub output_triangles: u64,
    pub max_added_height_error: f64,
    pub max_added_surface_error: f64,
    pub max_meshopt_estimate: f64,
}
impl Statistics {
    pub fn add(&mut self, other: &Self) {
        self.input_vertices += other.input_vertices;
        self.output_vertices += other.output_vertices;
        self.input_triangles += other.input_triangles;
        self.output_triangles += other.output_triangles;
        self.max_added_height_error = self
            .max_added_height_error
            .max(other.max_added_height_error);
        self.max_added_surface_error = self
            .max_added_surface_error
            .max(other.max_added_surface_error);
        self.max_meshopt_estimate = self.max_meshopt_estimate.max(other.max_meshopt_estimate);
    }
    pub fn report(&self, max_error: f64) -> serde_json::Value {
        serde_json::json!({"maxErrorMetres":max_error,"inputVertices":self.input_vertices,"outputVertices":self.output_vertices,
            "inputTriangles":self.input_triangles,"outputTriangles":self.output_triangles,
            "maxAddedHeightErrorMetres":self.max_added_height_error,"maxAddedSurfaceErrorMetres":self.max_added_surface_error,"maxMeshoptEstimateMetres":self.max_meshopt_estimate,
            "boundaryPolicy":"retain every original tile-edge vertex", "reference":"quantized regular grid; source sampling and height quantization errors are separate"})
    }
}
