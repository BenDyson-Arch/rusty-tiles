//! Regression fixtures retain the original Natural Earth source rings.
use super::*;
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn feature(mut geometry: Geometry, geographic: bool) -> Feature {
    let intrinsic = IntrinsicGeometry::capture(&geometry, Some((geographic, 1.))).unwrap();
    if geographic {
        geometry.map(|p| {
            crate::georef::geodetic_to_ecef(crate::georef::Cartographic::new(p[0], p[1], p[2]))
        });
    }
    Feature {
        properties: BTreeMap::from([
            ("_source_id".into(), json!("country")),
            ("_source_layer".into(), json!("countries")),
        ]),
        geometry,
        intrinsic,
        surface_fragment: false,
        triangle_boundaries: vec![],
        fragment_path: String::new(),
    }
}
fn countries() -> Vec<Feature> {
    let mut fixture: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/countries-source.geojson"
    ))
    .unwrap();
    fn xyz(value: &mut Value) {
        let a = value.as_array_mut().unwrap();
        if a.first().is_some_and(Value::is_number) {
            a.push(json!(0.));
        } else {
            for p in a {
                xyz(p);
            }
        }
    }
    fixture["features"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .map(|f| {
            xyz(&mut f["geometry"]["coordinates"]);
            feature(serde_json::from_value(f["geometry"].clone()).unwrap(), true)
        })
        .collect()
}
fn meshes(feature: &Feature) -> Vec<Polygon> {
    match &feature.geometry {
        Geometry::Polygon(rings) => vec![polygon_for(feature, rings, 0, [0.; 3], false).unwrap()],
        Geometry::MultiPolygon(parts) => parts
            .iter()
            .enumerate()
            .map(|(i, rings)| polygon_for(feature, rings, i, [0.; 3], false).unwrap())
            .collect(),
        _ => unreachable!(),
    }
}
fn radial_hits(meshes: &[Polygon], lon: f64, lat: f64) -> usize {
    let d = crate::georef::geodetic_to_ecef(crate::georef::Cartographic::new(lon, lat, 0.));
    let d = mul(d, 1. / norm(d));
    meshes
        .iter()
        .flat_map(|m| m.indices.as_chunks::<3>().0.iter().map(move |tri| (m, tri)))
        .filter(|(m, tri)| {
            let [a, b, c] = tri
                .iter()
                .map(|i| m.positions[*i as usize])
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            let u = sub(b, a);
            let v = sub(c, a);
            let n = cross(u, v);
            let den = dot(n, d);
            if den.abs() < 1e-12 {
                return false;
            }
            let t = dot(n, a) / den;
            if t <= 0. {
                return false;
            }
            let q = sub(mul(d, t), a);
            let uu = dot(u, u);
            let vv = dot(v, v);
            let uv = dot(u, v);
            let determinant = uu * vv - uv * uv;
            if determinant <= 0. {
                return false;
            }
            let x = (dot(q, u) * vv - dot(q, v) * uv) / determinant;
            let y = (dot(q, v) * uu - dot(q, u) * uv) / determinant;
            x >= -1e-9 && y >= -1e-9 && x + y <= 1. + 1e-9
        })
        .count()
}
#[test]
fn original_sudan_and_antarctica_fill_without_repair_or_source_vertex_changes() {
    let countries = countries();
    for country in &countries {
        let meshes = meshes(country);
        assert!(meshes
            .iter()
            .all(|m| m.report["topologyRepaired"] == false
                && m.report["addedIntersectionVertices"] == 0));
        for p in country.geometry.points() {
            assert!(meshes.iter().any(|m| m.positions.contains(p)));
        }
        for m in &meshes {
            for tri in m.indices.as_chunks::<3>().0.iter() {
                let a = m.positions[tri[0] as usize];
                let b = m.positions[tri[1] as usize];
                let c = m.positions[tri[2] as usize];
                assert!(dot(cross(sub(b, a), sub(c, a)), add(add(a, b), c)) >= -1e-6);
            }
        }
    }
    let sudan = meshes(&countries[0]);
    assert_eq!(sudan.iter().map(|m| m.indices.len() / 3).sum::<usize>(), 78);
    for (lon, lat) in [(30., 15.), (29., 16.), (31., 10.)] {
        assert_eq!(radial_hits(&sudan, lon, lat), 1);
    }
    let antarctica = meshes(&countries[1]);
    assert_eq!(
        antarctica
            .iter()
            .map(|m| m.report["collapsedPoleTriangles"].as_u64().unwrap())
            .sum::<u64>(),
        1
    );
    for (lon, lat) in [
        (0., -89.),
        (90., -89.),
        (-90., -89.),
        (179., -89.),
        (0., -80.),
        (90., -80.),
        (-90., -85.),
    ] {
        assert_eq!(radial_hits(&antarctica, lon, lat), 1, "{lon},{lat}");
    }
}
#[test]
fn varying_height_geographic_walls_keep_the_three_dimensional_policy() {
    let wall = feature(
        Geometry::Polygon(vec![vec![
            [30., 15., 0.],
            [30.00001, 15., 0.],
            [30.00001, 15., 10.],
            [30., 15., 10.],
            [30., 15., 0.],
        ]]),
        true,
    );
    assert!(wall.intrinsic.is_none());
    assert_eq!(meshes(&wall)[0].indices.len(), 6);
    let crossing = feature(
        Geometry::Polygon(vec![vec![
            [0., 0., 0.],
            [2., 2., 0.],
            [2., 0., 0.08],
            [0., 2., 0.08],
        ]]),
        false,
    );
    assert!(crossing.intrinsic.is_none());
    let Geometry::Polygon(rings) = &crossing.geometry else {
        unreachable!()
    };
    assert!(polygon_for(&crossing, rings, 0, [0.; 3], true)
        .err()
        .unwrap()
        .to_string()
        .contains("more than 2 cm"));
}
#[test]
fn distant_small_source_chart_keeps_outer_and_hole_winding() {
    let ring = |x: f64, y: f64, s: f64| {
        vec![
            [x, y, 0.],
            [x + s, y, 0.],
            [x + s, y + s, 0.],
            [x, y + s, 0.],
            [x, y, 0.],
        ]
    };
    let f = feature(
        Geometry::Polygon(vec![
            ring(10_000_000., 10_000_000., 0.01),
            ring(10_000_000.002, 10_000_000.002, 0.002),
        ]),
        false,
    );
    let m = &meshes(&f)[0];
    assert_eq!(m.indices.len() / 3, 8);
    assert_eq!(m.loops.iter().filter(|i| **i == u32::MAX).count(), 1);
    let areas: Vec<_> = m
        .loops
        .split(|i| *i == u32::MAX)
        .map(|ring| {
            let origin = m.positions[ring[0] as usize];
            (0..ring.len())
                .map(|i| {
                    let p = sub(m.positions[ring[i] as usize], origin);
                    let q = sub(m.positions[ring[(i + 1) % ring.len()] as usize], origin);
                    p[0] * q[1] - p[1] * q[0]
                })
                .sum::<f64>()
        })
        .collect();
    assert!(areas[0] > 0. && areas[1] < 0., "{areas:?}");
}

#[test]
fn intrinsic_repair_keeps_the_physical_intersection_ambiguity_guard() {
    let f = feature(
        Geometry::Polygon(vec![vec![
            [0., 0., 0.],
            [2., 2., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
        ]]),
        true,
    );
    let Geometry::Polygon(rings) = &f.geometry else {
        unreachable!()
    };
    let error = polygon_for(&f, rings, 0, [0.; 3], true).err().unwrap();
    assert!(error.to_string().contains("more than 2 cm"), "{error}");
}
#[test]
fn small_constant_height_polygons_still_simplify_with_source_correspondence() {
    let mut ring: Vec<_> = (0..200)
        .map(|i| {
            let a = i as f64 * std::f64::consts::TAU / 200.;
            [100. + 1e-5 * a.cos(), 40. + 1e-5 * a.sin(), 0.]
        })
        .collect();
    ring.push(ring[0]);
    let f = feature(Geometry::Polygon(vec![ring]), true);
    let (coarse, error) = simplify(&f, 0.05, &BTreeSet::new(), &mut vec![], false).unwrap();
    assert!(coarse.geometry.size() < f.geometry.size() / 4);
    assert!(error <= 0.05);
    assert_eq!(
        coarse.geometry.size(),
        coarse.intrinsic.as_ref().unwrap().geometry.size()
    );
    assert!(!meshes(&coarse)[0].indices.is_empty());
}
