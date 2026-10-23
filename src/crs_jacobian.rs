//! Central-difference Jacobians of a declared horizontal source CRS at mesh
//! vertices, mapped into one local ENU frame (glTF Y-up). The `mesh` tiler's
//! source-CRS capture uses them for normals and tangents; the deprecated legacy
//! `mesh-to-3tz` general CRS bake shares them until it is removed. Source
//! projection, datum and offline-grid eligibility remain the shared resolver's.
use crate::{
    crs::Transform,
    georef::{EnuFrame, SourceAxes},
    Error,
};

/// Central-difference step along one post-node source axis: 0.1 m vertically,
/// one horizontal unit (1e-5 degree when geographic) otherwise.
pub(crate) fn probe_step(axes: SourceAxes, axis: usize, horizontal_step: f64) -> f64 {
    if (axes == SourceAxes::Xyz && axis == 2) || (axes == SourceAxes::YUp && axis == 1) {
        0.1
    } else {
        horizontal_step
    }
}

/// Horizontal probe step in source units for a resolved operation; `None`
/// when its units cannot be determined. Also returns the geographic degrees
/// factor used by [`samples`] for longitude wrapping and pole handling.
pub(crate) fn horizontal_step(operation: &Transform) -> Option<(f64, Option<f64>)> {
    let (geographic, factor) = operation.polygon_units()?;
    Some(if geographic {
        (1e-5 / factor, Some(factor))
    } else {
        (1.0 / factor, None)
    })
}

/// One vertex's Jacobian columns d(local Y-up ENU)/d(post-node source axis),
/// with the same probes, domain-edge one-sided rule and frame as batched [`derivatives`].
/// `base` is the vertex's validated ECEF position. A resolver promotion is the
/// caller's to detect; this function never mutates the vertex.
#[allow(clippy::too_many_arguments)]
pub(crate) fn vertex_jacobian(
    operation: &mut Transform,
    position: [f64; 3],
    base: [f64; 3],
    axes: SourceAxes,
    offset: crate::georef::SourceOffset,
    horizontal_step: f64,
    degrees_per_unit: Option<f64>,
    frame: &EnuFrame,
) -> Result<[[f64; 3]; 3], Error> {
    let mut probes = Vec::with_capacity(6);
    let mut deltas = Vec::with_capacity(3);
    for axis in 0..3 {
        let step = probe_step(axes, axis, horizontal_step);
        let (pair, span) = samples(position, axes, offset, axis, step, degrees_per_unit)?;
        probes.extend(pair);
        deltas.push(span);
    }
    let mut columns = derivatives(operation, &probes, &deltas, &[base], &[0], frame)?;
    Ok(columns.remove(0))
}

pub(crate) fn derivatives(
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

pub(crate) fn samples(
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

pub(crate) fn to_y_up([east, north, up]: [f64; 3]) -> [f64; 3] {
    [east, up, -north]
}

/// `det(J)·J^-T·n` scaled by the determinant's sign, for Jacobian columns `J`.
/// The result is unnormalized; a singular or nonfinite Jacobian is refused.
pub(crate) fn inverse_transpose(
    columns: [[f64; 3]; 3],
    normal: [f64; 3],
) -> Result<[f64; 3], Error> {
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
    if !determinant.is_finite() || determinant == 0. {
        return Err(Error::Data(
            "mesh CRS normal Jacobian is singular/nonfinite".into(),
        ));
    }
    Ok(std::array::from_fn(|i| {
        (0..3).map(|j| cofactors[j][i] * normal[j]).sum::<f64>() * determinant.signum()
    }))
}

#[cfg(test)]
mod tests {
    //! Analytic checks of the shared helpers through [`vertex_jacobian`], the
    //! path the tiler's capture uses. They cover the same projections, domain
    //! edges and units as the legacy bake's tests, so those can retire with it.
    use super::*;
    use crate::georef::{ecef_to_cartographic, root_transform, SourceOffset};

    /// The unit ECEF image of a post-node source normal at `position`, and the
    /// position's ECEF coordinates, with the local frame at the vertex.
    fn world_normal(
        definition: &str,
        axes: SourceAxes,
        offset: SourceOffset,
        height_offset: f64,
        position: [f64; 3],
        normal: [f64; 3],
    ) -> Result<([f64; 3], [f64; 3]), Error> {
        let mut operation = Transform::new(definition, height_offset)?;
        let (step, degrees_per_unit) = horizontal_step(&operation).expect("horizontal units");
        let base = operation.transform(&[axes.coordinates(position, offset)])?[0];
        let origin = ecef_to_cartographic(base)?;
        let frame = EnuFrame::new(origin);
        let columns = vertex_jacobian(
            &mut operation,
            position,
            base,
            axes,
            offset,
            step,
            degrees_per_unit,
            &frame,
        )?;
        let local = inverse_transpose(columns, normal)?;
        // Local glTF Y-up (east, up, -north) back to ENU, then to ECEF.
        let enu = [local[0], -local[2], local[1]];
        let m = root_transform(origin, None);
        let world: [f64; 3] =
            std::array::from_fn(|i| m[i] * enu[0] + m[4 + i] * enu[1] + m[8 + i] * enu[2]);
        let length = world.iter().map(|v| v * v).sum::<f64>().sqrt();
        Ok((world.map(|v| v / length), base))
    }

    fn assert_close(actual: [f64; 3], expected: [f64; 3], tolerance: f64) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!((a - e).abs() < tolerance, "{actual:?} != {expected:?}");
        }
    }

    fn unit(v: [f64; 3]) -> [f64; 3] {
        let length = v.iter().map(|c| c * c).sum::<f64>().sqrt();
        v.map(|c| c / length)
    }

    #[test]
    fn mercator_normals_follow_the_analytic_inverse_transpose() {
        let a = 6378137_f64;
        let f = 1. / 298.257223563;
        let e2 = f * (2. - f);
        let lat = 60_f64.to_radians();
        let height = 100.;
        let offset = SourceOffset {
            easting: 0.,
            northing: a * lat.tan().asinh(),
            height,
        };
        let normal = |n| world_normal("EPSG:3857", SourceAxes::Xyz, offset, 0., [0.; 3], n);
        // Analytic Mercator horizontal scales on the WGS84 ellipsoid.
        let n = a / (1. - e2 * lat.sin().powi(2)).sqrt();
        let m = a * (1. - e2) / (1. - e2 * lat.sin().powi(2)).powf(1.5);
        let east_scale = (n + height) * lat.cos() / a;
        let north_scale = (m + height) * lat.cos() / a;
        let expected = unit([
            lat.cos() - lat.sin() / north_scale,
            1. / east_scale,
            lat.sin() + lat.cos() / north_scale,
        ]);
        assert_close(normal([1.; 3]).unwrap().0, expected, 2e-7);
        assert_close(
            normal([0., 0., 1.]).unwrap().0,
            [lat.cos(), 0., lat.sin()],
            2e-7,
        );
    }

    #[test]
    fn projected_domain_edge_uses_a_valid_one_sided_difference() {
        // Independent ellipsoidal Albers forward equations put the vertex just
        // inside the resolver's verified 80-degree source-latitude domain, so
        // the northward probe falls outside it.
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
        let offset = SourceOffset {
            northing: a * c.sqrt() / n - rho,
            ..Default::default()
        };
        let definition = "+proj=aea +lat_1=20 +lat_2=40 +lat_0=0 +lon_0=0 +datum=WGS84 +units=m";
        let normal = |v| world_normal(definition, SourceAxes::Xyz, offset, 0., [0.; 3], v);
        let radius = a / (1. - e2 * lat.sin().powi(2)).sqrt();
        let meridian = a * (1. - e2) / (1. - e2 * lat.sin().powi(2)).powf(1.5);
        let q_prime = 2. * (1. - e2) * lat.cos() / (1. - e2 * lat.sin().powi(2)).powi(2);
        let east_scale = radius * lat.cos() / (n * rho);
        let north_scale = meridian * 2. * n * rho / (a * a * q_prime);
        let expected = unit([
            lat.cos() - lat.sin() / north_scale,
            1. / east_scale,
            lat.sin() + lat.cos() / north_scale,
        ]);
        assert_close(normal([1.; 3]).unwrap().0, expected, 2e-6);
        assert_close(
            normal([0., 0., 1.]).unwrap().0,
            [lat.cos(), 0., lat.sin()],
            2e-6,
        );
    }

    #[test]
    fn geographic_pole_jacobian_is_refused() {
        let error = world_normal(
            "EPSG:4326",
            SourceAxes::Xyz,
            SourceOffset::default(),
            0.,
            [0., 90., 0.],
            [0., 0., 1.],
        )
        .unwrap_err();
        assert!(error.to_string().contains("geographic pole"), "{error}");
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn native_geographic_epoch_wraps_only_synthetic_longitudes() {
        // An explicit coordinate epoch deliberately requires strict native
        // resolution, retaining the known WGS84 datum and its eligible operation.
        let definition = "EPSG:4326@2020";
        let offset = |easting| SourceOffset {
            easting,
            ..Default::default()
        };
        let normal = |easting| {
            world_normal(
                definition,
                SourceAxes::YUp,
                offset(easting),
                0.,
                [0.; 3],
                [0., 1., 0.],
            )
        };
        let lon = 180_f64.to_radians();
        assert_close(normal(180.).unwrap().0, [lon.cos(), lon.sin(), 0.], 2e-7);
        assert!(normal(180.01).is_err());
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn strict_native_units_match_analytic_azimuthal_origin_with_horizontal_feet() {
        for (units, factor) in [("m", 1.), ("ft", 0.3048), ("us-ft", 1200. / 3937.)] {
            let definition =
                format!("+proj=aeqd +lat_0=0 +lon_0=9 +datum=WGS84 +units={units} +type=crs");
            let (normal, world) = world_normal(
                &definition,
                SourceAxes::Xyz,
                SourceOffset::default(),
                7.,
                [0.; 3],
                [1.; 3],
            )
            .unwrap();
            let lon = 9_f64.to_radians();
            let expected = [(6378137. + 7.) * lon.cos(), (6378137. + 7.) * lon.sin(), 0.];
            assert_close(world, expected, 2e-6);
            // Unit scaling affects X/Y while Z remains metres.
            let a = 6378137_f64;
            let flattening = 1. / 298.257223563;
            let meridian_radius = a * (1. - flattening * (2. - flattening));
            let east = a / (factor * (a + 7.));
            let north = meridian_radius / (factor * (meridian_radius + 7.));
            let expected = unit([
                lon.cos() - east * lon.sin(),
                lon.sin() + east * lon.cos(),
                north,
            ]);
            assert_close(normal, expected, 2e-7);
        }
    }
}
