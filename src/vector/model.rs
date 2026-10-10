//! Owned vector values shared by readers and the tiling pipeline.
//!
//! These types contain no native handles. Keep their serialized representation
//! stable: feature spools and reuse state persist these values.
use crate::vec3::{dot, sub};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) type Point = [f64; 3];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "coordinates")]
pub(super) enum Geometry {
    Point(Point),
    MultiPoint(Vec<Point>),
    LineString(Vec<Point>),
    MultiLineString(Vec<Vec<Point>>),
    Polygon(Vec<Vec<Point>>),
    MultiPolygon(Vec<Vec<Vec<Point>>>),
}
impl Geometry {
    pub(super) fn paths(&self) -> Vec<&[Point]> {
        match self {
            Self::Point(p) => vec![std::slice::from_ref(p)],
            Self::MultiPoint(p) | Self::LineString(p) => vec![p],
            Self::MultiLineString(p) | Self::Polygon(p) => p.iter().map(Vec::as_slice).collect(),
            Self::MultiPolygon(p) => p.iter().flatten().map(Vec::as_slice).collect(),
        }
    }
    pub(super) fn points(&self) -> impl Iterator<Item = &Point> {
        self.paths().into_iter().flatten()
    }
    pub(super) fn size(&self) -> usize {
        self.points().count()
    }
    pub(super) fn map(&mut self, mut f: impl FnMut(Point) -> Point) {
        match self {
            Self::Point(p) => *p = f(*p),
            Self::MultiPoint(p) | Self::LineString(p) => p.iter_mut().for_each(|p| *p = f(*p)),
            Self::MultiLineString(p) | Self::Polygon(p) => {
                p.iter_mut().flatten().for_each(|p| *p = f(*p))
            }
            Self::MultiPolygon(p) => p.iter_mut().flatten().flatten().for_each(|p| *p = f(*p)),
        }
    }
}
/// Constant-height geospatial surface chart, retained before ECEF placement.
/// Geographic XY is in degrees and projected XY in metres. The chart's
/// distinct seam/pole vertices may share a physical ECEF location.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct IntrinsicGeometry {
    pub(super) geometry: Geometry,
    pub(super) geographic: bool,
    pub(super) earth_center: Point,
}
impl IntrinsicGeometry {
    pub(super) fn capture(
        geometry: &Geometry,
        units: Option<(bool, f64)>,
    ) -> Result<Option<Self>, crate::Error> {
        if !matches!(geometry, Geometry::Polygon(_) | Geometry::MultiPolygon(_)) {
            return Ok(None);
        }
        let Some((geographic, factor)) = units else {
            return Ok(None);
        };
        if !factor.is_finite() || factor <= 0. {
            return Err(crate::Error::Data(
                "polygon chart unit factor must be finite and positive".into(),
            ));
        }
        let Some(first) = geometry.points().next() else {
            return Ok(None);
        };
        let height = first[2];
        // Varying-height surfaces (walls, overhangs, roofs) keep their existing
        // 3D best-fit policy rather than being flattened onto source XY.
        if geometry.points().any(|p| p[2] != height) {
            return Ok(None);
        }
        let mut geometry = geometry.clone();
        geometry.map(|p| [p[0] * factor, p[1] * factor, p[2]]);
        if geometry.points().flatten().any(|v| !v.is_finite()) {
            return Err(crate::Error::Data(
                "intrinsic polygon coordinates must be finite XYZ".into(),
            ));
        }
        Ok(Some(Self {
            geometry,
            geographic,
            earth_center: [0.; 3],
        }))
    }
    pub(super) fn rings(&self, index: usize) -> Option<&[Vec<Point>]> {
        match &self.geometry {
            Geometry::Polygon(rings) if index == 0 => Some(rings),
            Geometry::MultiPolygon(polygons) => polygons.get(index).map(Vec::as_slice),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Feature {
    pub(super) properties: BTreeMap<String, Value>,
    pub(super) geometry: Geometry,
    #[serde(
        default,
        rename = "_intrinsic_geometry",
        skip_serializing_if = "Option::is_none"
    )]
    pub(super) intrinsic: Option<IntrinsicGeometry>,
    #[serde(
        default,
        rename = "_surface_fragment",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub(super) surface_fragment: bool,
    #[serde(
        default,
        rename = "_triangle_boundaries",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub(super) triangle_boundaries: Vec<Vec<Vec<Point>>>,
    #[serde(
        default,
        rename = "_fragment_path",
        skip_serializing_if = "String::is_empty"
    )]
    pub(super) fragment_path: String,
}
impl Feature {
    pub(super) fn rendered_points(&self) -> impl Iterator<Item = &Point> {
        self.geometry
            .points()
            .chain(self.triangle_boundaries.iter().flatten().flatten())
    }
    pub(super) fn estimate(&self) -> usize {
        (self.rendered_points().count()
            + self
                .intrinsic
                .as_ref()
                .map_or(0, |source| source.geometry.size()))
            * 32
            + serde_json::to_vec(&self.properties).map_or(0, |p| p.len())
            + 2048
    }
    pub(super) fn source_id(&self) -> &Value {
        &self.properties["_source_id"]
    }
    pub(super) fn layer(&self) -> &str {
        self.properties["_source_layer"].as_str().unwrap()
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Frame {
    pub(super) anchor: Point,
    pub(super) axes: [Point; 3],
}
impl Frame {
    /// Orient the local frame at a WGS84 ECEF anchor. Keep the exact anchor;
    /// the inverse operation establishes the surface axes only.
    pub(super) fn georeferenced(anchor: Point) -> Result<Self, crate::Error> {
        let geographic = crate::georef::ecef_to_cartographic(anchor)?;
        let matrix = crate::georef::root_transform(geographic, None);
        Ok(Self {
            anchor,
            axes: [
                [matrix[0], matrix[1], matrix[2]],
                [matrix[4], matrix[5], matrix[6]],
                [matrix[8], matrix[9], matrix[10]],
            ],
        })
    }

    pub(super) fn local(anchor: Point) -> Self {
        Self {
            anchor,
            axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        }
    }
    pub(super) fn project(&self, p: Point) -> Point {
        let d = sub(p, self.anchor);
        [
            dot(d, self.axes[0]),
            dot(d, self.axes[2]),
            -dot(d, self.axes[1]),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn geometry_mapping_preserves_all_paths_and_serialized_variants() {
        let cases = [
            json!({"type":"Point","coordinates":[1,2,3]}),
            json!({"type":"MultiPoint","coordinates":[[1,2,3],[4,5,6]]}),
            json!({"type":"LineString","coordinates":[[1,2,3],[4,5,6]]}),
            json!({"type":"MultiLineString","coordinates":[[[1,2,3]],[[4,5,6]]]}),
            json!({"type":"Polygon","coordinates":[[[1,2,3]],[[4,5,6]]]}),
            json!({"type":"MultiPolygon","coordinates":[[[[1,2,3]]],[[[4,5,6]]]]}),
        ];
        for source in cases {
            let mut geometry: Geometry = serde_json::from_value(source.clone()).unwrap();
            let before: Vec<_> = geometry.points().copied().collect();
            assert_eq!(geometry.size(), before.len());
            assert_eq!(
                serde_json::to_value(&geometry).unwrap()["type"],
                source["type"]
            );
            geometry.map(|p| [p[0] + 10., p[1] + 20., p[2] + 30.]);
            assert_eq!(
                geometry.points().copied().collect::<Vec<_>>(),
                before
                    .iter()
                    .map(|p| [p[0] + 10., p[1] + 20., p[2] + 30.])
                    .collect::<Vec<_>>()
            );
            let serialized = serde_json::to_vec(&geometry).unwrap();
            assert_eq!(
                serde_json::from_slice::<Geometry>(&serialized).unwrap(),
                geometry
            );
        }
    }

    #[test]
    fn spool_features_retain_default_and_fragment_serialization() {
        let source = json!({"properties":{"_source_id":"\"id\"","_source_layer":"layer","large":u64::MAX},"geometry":{"type":"Point","coordinates":[1.,2.,3.]}});
        let mut feature: Feature = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(feature.source_id(), &json!("\"id\""));
        assert_eq!(feature.layer(), "layer");
        assert_eq!(serde_json::to_value(&feature).unwrap(), source);
        feature.surface_fragment = true;
        feature.triangle_boundaries = vec![vec![vec![[1., 2., 3.], [4., 5., 6.]]]];
        feature.fragment_path = "01".into();
        let persisted = serde_json::to_value(&feature).unwrap();
        assert_eq!(persisted["_surface_fragment"], true);
        assert_eq!(persisted["_fragment_path"], "01");
        let restored: Feature = serde_json::from_value(persisted.clone()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), persisted);
    }

    #[test]
    fn reused_frames_preserve_anchor_axes_and_y_up_projection() {
        let local = Frame::local([10., 20., 30.]);
        assert_eq!(local.project([11., 22., 33.]), [1., 3., -2.]);
        let source = json!({"anchor":[10.,20.,30.],"axes":[[0.,1.,0.],[0.,0.,1.],[1.,0.,0.]]});
        let frame: Frame = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(frame.project([11., 22., 33.]), [2., 1., -3.]);
        assert_eq!(serde_json::to_value(frame).unwrap(), source);
    }
}
