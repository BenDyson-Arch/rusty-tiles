//! Conservative bounds for unchanged authored node chains, before the f32 bake.
//! Directed intervals enclose raw and normalized admitted quaternion conventions.
use super::*;

#[derive(Clone, Copy)]
struct Interval {
    low: f64,
    high: f64,
}
impl Interval {
    const ZERO: Self = Self::point(0.);
    const ONE: Self = Self::point(1.);
    const fn point(value: f64) -> Self {
        Self {
            low: value,
            high: value,
        }
    }
    fn outward(low: f64, high: f64) -> Self {
        if !low.is_finite() || !high.is_finite() {
            return Self {
                low: f64::NEG_INFINITY,
                high: f64::INFINITY,
            };
        }
        Self {
            low: low.next_down(),
            high: high.next_up(),
        }
    }
    fn add(self, rhs: Self) -> Self {
        Self::outward(self.low + rhs.low, self.high + rhs.high)
    }
    fn neg(self) -> Self {
        Self {
            low: -self.high,
            high: -self.low,
        }
    }
    fn sub(self, rhs: Self) -> Self {
        self.add(rhs.neg())
    }
    fn mul(self, rhs: Self) -> Self {
        let products = [
            self.low * rhs.low,
            self.low * rhs.high,
            self.high * rhs.low,
            self.high * rhs.high,
        ];
        if products.iter().any(|v| !v.is_finite()) {
            return Self {
                low: f64::NEG_INFINITY,
                high: f64::INFINITY,
            };
        }
        Self::outward(
            products.into_iter().fold(f64::INFINITY, f64::min),
            products.into_iter().fold(f64::NEG_INFINITY, f64::max),
        )
    }
    fn sqrt(self) -> Self {
        // Used only for a sum of quaternion squares, whose exact value is >= 0.
        Self::outward(self.low.max(0.).sqrt(), self.high.sqrt())
    }
    fn reciprocal(self) -> Self {
        if self.low <= 0. || !self.high.is_finite() {
            return Self {
                low: f64::NEG_INFINITY,
                high: f64::INFINITY,
            };
        }
        Self::outward(1. / self.high, 1. / self.low)
    }
    fn hull(self, rhs: Self) -> Self {
        Self {
            low: self.low.min(rhs.low),
            high: self.high.max(rhs.high),
        }
    }
}
type IntervalMatrix = [[Interval; 4]; 4];
fn identity() -> IntervalMatrix {
    IDENTITY.map(|row| row.map(Interval::point))
}
fn rotation([x, y, z, w]: [Interval; 4]) -> [[Interval; 3]; 3] {
    let two = Interval::point(2.);
    let one = Interval::ONE;
    [
        [
            one.sub(two.mul(y.mul(y).add(z.mul(z)))),
            two.mul(x.mul(y).sub(z.mul(w))),
            two.mul(x.mul(z).add(y.mul(w))),
        ],
        [
            two.mul(x.mul(y).add(z.mul(w))),
            one.sub(two.mul(x.mul(x).add(z.mul(z)))),
            two.mul(y.mul(z).sub(x.mul(w))),
        ],
        [
            two.mul(x.mul(z).sub(y.mul(w))),
            two.mul(y.mul(z).add(x.mul(w))),
            one.sub(two.mul(x.mul(x).add(y.mul(y)))),
        ],
    ]
}
fn local(node: &Value) -> Result<IntervalMatrix> {
    if let Some(value) = node.get("matrix") {
        let values = vector::<16>(value)?;
        return Ok(std::array::from_fn(|r| {
            std::array::from_fn(|c| Interval::point(values[c * 4 + r]))
        }));
    }
    let t = node
        .get("translation")
        .map(vector::<3>)
        .transpose()?
        .unwrap_or([0.; 3]);
    let s = node
        .get("scale")
        .map(vector::<3>)
        .transpose()?
        .unwrap_or([1.; 3]);
    let q = node
        .get("rotation")
        .map(vector::<4>)
        .transpose()?
        .unwrap_or([0., 0., 0., 1.]);
    let raw = q.map(Interval::point);
    let norm = raw
        .into_iter()
        .fold(Interval::ZERO, |sum, v| sum.add(v.mul(v)))
        .sqrt();
    let normalized = raw.map(|v| v.mul(norm.reciprocal()));
    let a = rotation(raw);
    let b = rotation(normalized);
    let mut result = identity();
    for r in 0..3 {
        for (c, scale) in s.into_iter().enumerate() {
            result[r][c] = a[r][c].hull(b[r][c]).mul(Interval::point(scale));
        }
        result[r][3] = Interval::point(t[r]);
    }
    Ok(result)
}
fn multiply(a: IntervalMatrix, b: IntervalMatrix) -> IntervalMatrix {
    let mut result = identity();
    for r in 0..3 {
        for c in 0..4 {
            let mut value = Interval::ZERO;
            for (k, row) in b.iter().enumerate().take(3) {
                value = value.add(a[r][k].mul(row[c]));
            }
            result[r][c] = if c == 3 { value.add(a[r][3]) } else { value };
        }
    }
    result
}

pub(super) fn evaluate(
    document: &Document,
    supplied_buffers: &[&[u8]],
    mut check: impl FnMut() -> Result<()>,
) -> Result<OriginalBounds> {
    check()?;
    if supplied_buffers.len() != document.buffer_lengths.len() {
        return Err(invalid("original bounds buffer snapshot count mismatch"));
    }
    let buffers = supplied_buffers
        .iter()
        .zip(&document.buffer_lengths)
        .map(|(buffer, &length)| {
            buffer
                .get(..length)
                .ok_or_else(|| invalid("original bounds buffer shorter than declared length"))
        })
        .collect::<Result<Vec<_>>>()?;
    let data = decode_accessors(&document.accessors, &buffers, &mut check)?;
    validate_primitive_payload(&document.meshes, &data, &mut check)?;
    let nodes = list(&document.value, "nodes")?;
    let mut locals = Vec::with_capacity(nodes.len());
    let mut parents = vec![None; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        check()?;
        locals.push(local(node)?);
        for child in list(node, "children")? {
            parents[uint(child)?] = Some(i);
        }
    }
    let mut result = OriginalBounds {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };
    // Document admission established the finite forest/depth and selected instances.
    for instance in &document.instances {
        check()?;
        let mut chain = Vec::new();
        let mut node = Some(instance.node);
        while let Some(i) = node {
            chain.push(i);
            node = parents[i];
        }
        let mut world = identity();
        for i in chain.into_iter().rev() {
            world = multiply(world, locals[i]);
        }
        for primitive in &document.meshes[instance.mesh] {
            for start in (0..primitive.count).step_by(3) {
                if start.is_multiple_of(3072) {
                    check()?;
                }
                for index in primitive.vertices(&data, start) {
                    let point = data[primitive.position].vec3(index).map(Interval::point);
                    let yup: [Interval; 3] = std::array::from_fn(|r| {
                        let mut value = world[r][3];
                        for (k, coordinate) in point.into_iter().enumerate() {
                            value = value.add(world[r][k].mul(coordinate));
                        }
                        value
                    });
                    for (axis, value) in [yup[0], yup[2].neg(), yup[1]].into_iter().enumerate() {
                        if !value.low.is_finite() || !value.high.is_finite() {
                            return Err(unsupported(
                                "authored node bounds exceed finite interval domain",
                            ));
                        }
                        result.min[axis] = result.min[axis].min(value.low);
                        result.max[axis] = result.max[axis].max(value.high);
                    }
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(nodes: Value, roots: Value) -> Document {
        let doc = json!({"asset":{"version":"2.0"},
            "buffers":[{"uri":"source.bin","byteLength":36}],
            "bufferViews":[{"buffer":0,"byteLength":36,"target":34962}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
                "min":[800000,300000,-600000],"max":[800001,300001,-600000]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
            "nodes":nodes,"scenes":[{"nodes":roots}],"scene":0});
        Document::parse(&serde_json::to_vec(&doc).unwrap(), || Ok(())).unwrap()
    }
    fn buffer() -> Vec<u8> {
        [
            [800000f32, 300000., -600000.],
            [800000., 300001., -600000.],
            [800001., 300000., -600000.],
        ]
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect()
    }
    #[test]
    fn raw_near_unit_authored_coordinates_are_not_normalized_away() {
        let document = fixture(
            json!([{"mesh":0,"rotation":[0,0,0.7071069,0.7071069],
            "translation":[100000,-200000,300000]}]),
            json!([0]),
        );
        let binary = buffer();
        let bounds = evaluate(&document, &[&binary], || Ok(())).unwrap();
        // Frozen independent 90-digit Decimal results in original-bounds-feasibility.json.
        // Node-baked normalization alone would put x=-200000,y=600000 instead.
        for point in [
            [-200000.36966074195, 300000., 600000.16802761],
            [-200000., 300000., 600000.],
        ] {
            for (axis, value) in point.into_iter().enumerate() {
                assert!(bounds.min[axis] <= value && value <= bounds.max[axis]);
            }
        }
        assert!(bounds.min[0] < -200000.3);
        assert!(bounds.max[2] > 600000.1);
    }
    #[test]
    fn selected_scene_roots_with_global_parents_are_refused() {
        let value = json!({"asset":{"version":"2.0"},
            "buffers":[{"uri":"unused.bin","byteLength":36}],
            "bufferViews":[{"buffer":0,"byteLength":36,"target":34962}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
            "nodes":[{"children":[1],"translation":[1000,0,0]},{"mesh":0}],
            "scenes":[{"nodes":[0]},{"nodes":[1]}],"scene":1});
        let error = Document::parse(&serde_json::to_vec(&value).unwrap(), || Ok(()))
            .err()
            .unwrap();
        assert_eq!(error.kind(), JobErrorKind::InvalidInput);
    }
    #[test]
    fn original_bound_cancellation_is_observed_before_position_bake() {
        let document = fixture(
            json!([
                {"children":[1],"translation":[1e15,0,0]},
                {"mesh":0,"translation":[-1e15,0,0]}
            ]),
            json!([0]),
        );
        let binary = buffer();
        let bounds = evaluate(&document, &[&binary], || Ok(())).unwrap();
        assert!(bounds.min[0] < 800000. && bounds.max[0] > 800001.);
        assert!(bounds.min[1] <= 600000. && bounds.max[1] >= 600000.);
    }
}
