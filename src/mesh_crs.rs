//! General mesh axes and horizontal CRS placement. Source projection, datum and
//! offline-grid eligibility remain the shared resolver's responsibility.
use crate::{
    crs::Transform,
    georef::{ecef_to_cartographic, Cartographic, EnuFrame, SourceAxes},
    mesh::Scene,
    tile::MeshTo3tzOptions,
    Error,
};

pub(crate) fn bake(
    scene: &mut Scene,
    definition: &str,
    options: &MeshTo3tzOptions,
) -> Result<Cartographic, Error> {
    let axes = options.source_axes.ok_or_else(|| {
        Error::Data(
            "general mesh CRS requires --source-axes xyz or y-up; axes are not inferred".into(),
        )
    })?;
    let height_offset = options.height_offset.ok_or_else(|| Error::Data(
        "general mesh CRS requires --height-offset (metres to ellipsoidal height, including explicit 0)".into(),
    ))?;
    if options.cartographic.is_some() || options.rotation.is_some() {
        return Err(Error::Data(
            "general mesh CRS cannot be combined with manual cartographic placement or rotation"
                .into(),
        ));
    }
    let offset = options.source_offset.unwrap_or_default();
    if ![offset.easting, offset.northing, offset.height]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(Error::Data(
            "source offset must contain finite values".into(),
        ));
    }
    let mut operation = Transform::new(definition, height_offset)?;
    let (geographic, factor) = operation
        .polygon_units()
        .ok_or_else(|| Error::Data("cannot determine horizontal mesh CRS units".into()))?;
    let horizontal_step = if geographic {
        1e-5 / factor
    } else {
        1.0 / factor
    };
    let (min, max) = crate::mesh::position_aabb_yup(scene);
    let centre = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
    let origin =
        ecef_to_cartographic(operation.transform(&[axes.coordinates(centre, offset)])?[0])?;
    let frame = EnuFrame::new(origin);
    // GDAL operations stay on this thread. Bound the extra coordinate/normal
    // working set to a batch, including retries at a projection-domain edge.
    for vertices in scene.vertices.chunks_mut(4096) {
        let positions: Vec<_> = vertices
            .iter()
            .map(|v| axes.coordinates(v.pos.map(f64::from), offset))
            .collect();
        // Validate actual input before wrapping or replacing synthetic probes.
        // A normal must never make an invalid source position acceptable.
        let mut output = operation.transform(&positions)?;
        let mut probes = Vec::with_capacity(vertices.len() * 6);
        let mut deltas = Vec::with_capacity(vertices.len() * 3);
        let mut normal_indices = Vec::with_capacity(vertices.len());
        for (index, vertex) in vertices.iter().enumerate() {
            if vertex.nrm != [0.; 3] {
                normal_indices.push(index);
                for axis in 0..3 {
                    // Differentiate before adding a large offset. The CRS
                    // resolver receives f64 coordinates for both samples.
                    let step = if (axes == SourceAxes::Xyz && axis == 2)
                        || (axes == SourceAxes::YUp && axis == 1)
                    {
                        0.1
                    } else {
                        horizontal_step
                    };
                    let (pair, span) = samples(
                        vertex.pos.map(f64::from),
                        axes,
                        offset,
                        axis,
                        step,
                        geographic.then_some(factor),
                    )?;
                    probes.extend(pair);
                    deltas.push(span);
                }
            }
        }
        let columns = loop {
            let operation_kind = std::mem::discriminant(&operation);
            let result = derivatives(
                &mut operation,
                &probes,
                &deltas,
                &output,
                &normal_indices,
                &frame,
            );
            if std::mem::discriminant(&operation) != operation_kind {
                // Either bulk sampling or an individual retry may promote the
                // resolver. Discard every derivative, including failures, and
                // restart this complete batch before mutating any vertex.
                // The resolver can promote Pure -> Native at most once.
                output = operation.transform(&positions)?;
                continue;
            }
            break result?;
        };
        let mut columns = columns.into_iter();
        for (index, vertex) in vertices.iter_mut().enumerate() {
            let point = to_y_up(frame.to_enu(output[index]));
            if vertex.nrm != [0.; 3] {
                vertex.nrm = normal(columns.next().unwrap(), vertex.nrm)?;
            }
            if point
                .iter()
                .any(|v| !v.is_finite() || v.abs() > f64::from(f32::MAX))
            {
                return Err(Error::Data(
                    "mesh CRS produced nonfinite/out-of-range local coordinates".into(),
                ));
            }
            vertex.pos = point.map(|v| v as f32);
        }
    }
    Ok(origin)
}

fn derivatives(
    operation: &mut Transform,
    probes: &[[f64; 3]],
    deltas: &[[f64; 2]],
    positions: &[[f64; 3]],
    normal_indices: &[usize],
    frame: &EnuFrame,
) -> Result<Vec<[[f64; 3]; 3]>, Error> {
    let output = match operation.transform(probes) {
        Ok(points) => Some(points),
        // Actual vertices already passed the same strict operation. Only a
        // coordinate-domain failure may use a valid one-sided sample.
        Err(Error::Data(_)) => None,
        Err(error) => return Err(error),
    };
    let mut result = Vec::with_capacity(normal_indices.len());
    let mut cursor = 0;
    for &index in normal_indices {
        let mut columns = [[0.; 3]; 3];
        for column in &mut columns {
            let span = deltas[cursor / 2];
            let (a, b, denominator) = if let Some(points) = &output {
                (points[cursor], points[cursor + 1], span[1] - span[0])
            } else {
                one_sided(
                    operation,
                    &probes[cursor..cursor + 2],
                    span,
                    positions[index],
                )?
            };
            let a = to_y_up(frame.to_enu(a));
            let b = to_y_up(frame.to_enu(b));
            *column = std::array::from_fn(|i| (b[i] - a[i]) / denominator);
            cursor += 2;
        }
        result.push(columns);
    }
    Ok(result)
}

fn samples(
    position: [f64; 3],
    axes: SourceAxes,
    offset: crate::georef::SourceOffset,
    axis: usize,
    step: f64,
    degrees_per_unit: Option<f64>,
) -> Result<([[f64; 3]; 2], [f64; 2]), Error> {
    let base = axes.coordinates(position, offset);
    let mut deltas = [-step, step];
    let mut pair = deltas.map(|delta| {
        let mut sample = position;
        sample[axis] += delta;
        axes.coordinates(sample, offset)
    });
    if let Some(factor) = degrees_per_unit {
        if ((base[1] * factor).abs() - 90.).abs() < 1e-12 {
            return Err(Error::Data(
                "mesh CRS normal Jacobian is singular at a geographic pole".into(),
            ));
        }
        for (sample, delta) in pair.iter_mut().zip(&mut deltas) {
            // Longitude is periodic in ECEF; retain the original derivative
            // span rather than differentiating the wrapped coordinate jump.
            let longitude_limit = 180. / factor;
            if sample[0] > longitude_limit {
                sample[0] -= 2. * longitude_limit;
            } else if sample[0] < -longitude_limit {
                sample[0] += 2. * longitude_limit;
            }
            if (sample[1] * factor).abs() > 90. {
                // Latitude is not periodic. Replace only the artificial point
                // with the validated base, yielding an inward one-sided slope.
                *sample = base;
                *delta = 0.;
            }
        }
    }
    Ok((pair, deltas))
}

fn one_sided(
    operation: &mut Transform,
    probes: &[[f64; 3]],
    span: [f64; 2],
    base: [f64; 3],
) -> Result<([f64; 3], [f64; 3], f64), Error> {
    // Resource, datum-operation and unavailable-grid errors must still fail.
    let a = match operation.transform(&probes[..1]) {
        Err(error) if !matches!(error, Error::Data(_)) => return Err(error),
        result => result,
    };
    let b = match operation.transform(&probes[1..]) {
        Err(error) if !matches!(error, Error::Data(_)) => return Err(error),
        result => result,
    };
    match (a, b) {
        (Ok(a), Ok(b)) => Ok((a[0], b[0], span[1] - span[0])),
        (Ok(a), Err(Error::Data(_))) if span[0] != 0. => Ok((a[0], base, -span[0])),
        (Err(Error::Data(_)), Ok(b)) if span[1] != 0. => Ok((base, b[0], span[1])),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn to_y_up([east, north, up]: [f64; 3]) -> [f64; 3] {
    [east, up, -north]
}

fn normal(columns: [[f64; 3]; 3], normal: [f32; 3]) -> Result<[f32; 3], Error> {
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let cofactors = [
        cross(columns[1], columns[2]),
        cross(columns[2], columns[0]),
        cross(columns[0], columns[1]),
    ];
    let determinant = (0..3).map(|i| columns[0][i] * cofactors[0][i]).sum::<f64>();
    let transformed: [f64; 3] = std::array::from_fn(|i| {
        (0..3)
            .map(|j| cofactors[j][i] * f64::from(normal[j]))
            .sum::<f64>()
            * determinant.signum()
    });
    let length = transformed.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !determinant.is_finite() || determinant == 0. || !length.is_finite() || length == 0. {
        return Err(Error::Data(
            "mesh CRS normal Jacobian is singular/nonfinite".into(),
        ));
    }
    Ok(transformed.map(|v| (v / length) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::georef::SourceOffset;
    use crate::mesh::{Triangle, Vertex};

    fn scene() -> Scene {
        Scene {
            // Coincident vertices have distinct authored hard-edge normals.
            vertices: vec![
                Vertex {
                    pos: [0.; 3],
                    nrm: [1.; 3],
                    uv: [0.25, 0.5],
                },
                Vertex {
                    pos: [0.; 3],
                    nrm: [0., 0., 1.],
                    uv: [0.75, 0.5],
                },
                Vertex {
                    pos: [1., 0., 0.],
                    nrm: [0.; 3],
                    uv: [0., 0.],
                },
            ],
            triangles: vec![Triangle {
                verts: [0, 1, 2],
                image: None,
                material: None,
            }],
            images: vec![],
            source_bytes: 100,
            materials: vec![],
            triangle_nodes: vec![17],
            node_names: [(17, "Source".into())].into(),
        }
    }
    fn world_normal(origin: Cartographic, normal: [f32; 3]) -> [f64; 3] {
        let m = crate::georef::root_transform(origin, None);
        let n = [
            f64::from(normal[0]),
            -f64::from(normal[2]),
            f64::from(normal[1]),
        ];
        std::array::from_fn(|i| m[i] * n[0] + m[4 + i] * n[1] + m[8 + i] * n[2])
    }
    fn assert_close(actual: [f64; 3], expected: [f64; 3], tolerance: f64) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!((a - e).abs() < tolerance, "{actual:?} != {expected:?}");
        }
    }
    #[test]
    fn authored_normals_follow_analytic_mercator_inverse_transpose_and_keep_hard_edges() {
        let a = 6378137_f64;
        let f = 1. / 298.257223563;
        let e2 = f * (2. - f);
        let lat = 60_f64.to_radians();
        let height = 100.;
        let mut options = MeshTo3tzOptions {
            source_axes: Some(SourceAxes::Xyz),
            height_offset: Some(0.),
            source_offset: Some(SourceOffset {
                easting: 0.,
                northing: a * lat.tan().asinh(),
                height,
            }),
            ..Default::default()
        };
        options.set_source_crs("EPSG:3857").unwrap();
        let mut mesh = scene();
        let origin = bake(&mut mesh, "EPSG:3857", &options).unwrap();
        // Analytic Mercator horizontal scales on the WGS84 ellipsoid.
        let n = a / (1. - e2 * lat.sin().powi(2)).sqrt();
        let m = a * (1. - e2) / (1. - e2 * lat.sin().powi(2)).powf(1.5);
        let east_scale = (n + height) * lat.cos() / a;
        let north_scale = (m + height) * lat.cos() / a;
        let normal = [
            lat.cos() - lat.sin() / north_scale,
            1. / east_scale,
            lat.sin() + lat.cos() / north_scale,
        ];
        let length = normal.iter().map(|v| v * v).sum::<f64>().sqrt();
        assert_close(
            world_normal(origin, mesh.vertices[0].nrm),
            normal.map(|v| v / length),
            2e-7,
        );
        assert_close(
            world_normal(origin, mesh.vertices[1].nrm),
            [lat.cos(), 0., lat.sin()],
            2e-7,
        );
        assert_eq!(mesh.vertices[2].nrm, [0.; 3]);
        assert_eq!(mesh.vertices[0].uv, [0.25, 0.5]);
        assert_eq!(mesh.vertices[1].uv, [0.75, 0.5]);
        assert_eq!(mesh.triangles[0].verts, [0, 1, 2]);
        assert_eq!(mesh.triangle_nodes, [17]);
        assert_eq!(mesh.node_names[&17], "Source");
    }

    #[test]
    fn projected_accuracy_domain_uses_valid_one_sided_normals() {
        // Independent ellipsoidal Albers forward equations put the origin just
        // inside the resolver's verified 80-degree source-latitude domain.
        let a = 6378137_f64;
        let f = 1. / 298.257223563;
        let e2: f64 = f * (2. - f);
        let e = e2.sqrt();
        let q = |lat: f64| {
            let s = lat.sin();
            (1. - e2) * (s / (1. - e2 * s * s) - ((1. - e * s) / (1. + e * s)).ln() / (2. * e))
        };
        let m2 = |lat: f64| lat.cos().powi(2) / (1. - e2 * lat.sin().powi(2));
        let first = 20_f64.to_radians();
        let second = 40_f64.to_radians();
        let n = (m2(first) - m2(second)) / (q(second) - q(first));
        let c = m2(first) + n * q(first);
        let lat = 79.99999999_f64.to_radians();
        let rho = a * (c - n * q(lat)).sqrt() / n;
        let northing = a * c.sqrt() / n - rho;
        let mut options = MeshTo3tzOptions {
            source_axes: Some(SourceAxes::Xyz),
            height_offset: Some(0.),
            source_offset: Some(SourceOffset {
                northing,
                ..Default::default()
            }),
            ..Default::default()
        };
        let definition = "+proj=aea +lat_1=20 +lat_2=40 +lat_0=0 +lon_0=0 +datum=WGS84 +units=m";
        options.set_source_crs(definition).unwrap();
        let mut mesh = scene();
        let origin = bake(&mut mesh, definition, &options).unwrap();
        let radius = a / (1. - e2 * lat.sin().powi(2)).sqrt();
        let meridian = a * (1. - e2) / (1. - e2 * lat.sin().powi(2)).powf(1.5);
        let q_prime = 2. * (1. - e2) * lat.cos() / (1. - e2 * lat.sin().powi(2)).powi(2);
        let east_scale = radius * lat.cos() / (n * rho);
        let north_scale = meridian * 2. * n * rho / (a * a * q_prime);
        let expected = [
            lat.cos() - lat.sin() / north_scale,
            1. / east_scale,
            lat.sin() + lat.cos() / north_scale,
        ];
        let length = expected.iter().map(|v| v * v).sum::<f64>().sqrt();
        assert_close(
            world_normal(origin, mesh.vertices[0].nrm),
            expected.map(|v| v / length),
            2e-6,
        );
        assert_close(
            world_normal(origin, mesh.vertices[1].nrm),
            [lat.cos(), 0., lat.sin()],
            2e-6,
        );
        assert_eq!(mesh.vertices[2].nrm, [0.; 3]);
    }

    #[test]
    fn pole_positions_without_normals_do_not_need_a_jacobian() {
        let mut mesh = scene();
        for vertex in &mut mesh.vertices {
            vertex.pos[1] = 90.;
            vertex.nrm = [0.; 3];
        }
        let mut options = MeshTo3tzOptions {
            source_axes: Some(SourceAxes::Xyz),
            height_offset: Some(0.),
            ..Default::default()
        };
        options.set_source_crs("EPSG:4326").unwrap();
        bake(&mut mesh, "EPSG:4326", &options).unwrap();
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.pos.iter().all(|n| n.is_finite()) && v.nrm == [0.; 3]));
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn native_geographic_epoch_wraps_only_synthetic_longitudes() {
        // An explicit coordinate epoch deliberately requires strict native
        // resolution, retaining the known WGS84 datum and its eligible operation.
        let definition = "EPSG:4326@2020";
        let mut mesh = scene();
        for vertex in &mut mesh.vertices {
            vertex.pos = [0.; 3];
            vertex.nrm = [0., 1., 0.];
        }
        let mut options = MeshTo3tzOptions {
            source_axes: Some(SourceAxes::YUp),
            height_offset: Some(0.),
            source_offset: Some(SourceOffset {
                easting: 180.,
                ..Default::default()
            }),
            ..Default::default()
        };
        options.set_source_crs(definition).unwrap();
        let origin = bake(&mut mesh, definition, &options).unwrap();
        let lon = 180_f64.to_radians();
        assert_close(
            world_normal(origin, mesh.vertices[0].nrm),
            [lon.cos(), lon.sin(), 0.],
            2e-7,
        );
        options.source_offset.as_mut().unwrap().easting = 180.01;
        assert!(bake(&mut mesh, definition, &options).is_err());
    }
    #[cfg(feature = "native-geospatial")]
    #[test]
    fn strict_native_units_match_analytic_azimuthal_origin_with_horizontal_feet() {
        for (units, factor) in [("m", 1.), ("ft", 0.3048), ("us-ft", 1200. / 3937.)] {
            let definition =
                format!("+proj=aeqd +lat_0=0 +lon_0=9 +datum=WGS84 +units={units} +type=crs");
            let mut mesh = scene();
            let mut options = MeshTo3tzOptions {
                source_axes: Some(SourceAxes::Xyz),
                height_offset: Some(7.),
                ..Default::default()
            };
            options.set_source_crs(&definition).unwrap();
            let origin = bake(&mut mesh, &definition, &options).unwrap();
            let m = crate::georef::root_transform(origin, None);
            let p = mesh.vertices[0].pos;
            let xyz = [f64::from(p[0]), -f64::from(p[2]), f64::from(p[1])];
            let world = std::array::from_fn(|i| {
                m[i] * xyz[0] + m[4 + i] * xyz[1] + m[8 + i] * xyz[2] + m[12 + i]
            });
            let lon = 9_f64.to_radians();
            let expected = [(6378137. + 7.) * lon.cos(), (6378137. + 7.) * lon.sin(), 0.];
            assert_close(world, expected, 2e-6);
            // Unit scaling affects X/Y while Z remains metres.
            let a = 6378137_f64;
            let flattening = 1. / 298.257223563;
            let meridian_radius = a * (1. - flattening * (2. - flattening));
            let east = a / (factor * (a + 7.));
            let north = meridian_radius / (factor * (meridian_radius + 7.));
            let expected = [
                lon.cos() - east * lon.sin(),
                lon.sin() + east * lon.cos(),
                north,
            ];
            let length = expected.iter().map(|v| v * v).sum::<f64>().sqrt();
            assert_close(
                world_normal(origin, mesh.vertices[0].nrm),
                expected.map(|v| v / length),
                2e-7,
            );
        }
    }
}
