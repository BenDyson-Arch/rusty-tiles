#![cfg(feature = "native-geospatial")]

use rusty_tiles::georef::{geodetic_to_ecef, Cartographic};
use rusty_tiles::geospatial::{versions, Crs, EcefTransform, StrictTransform};

fn assert_position(actual: [f64; 3], expected: [f64; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
}

#[test]
fn projected_xyz_uses_explicit_metre_heights_and_traditional_axes() {
    let mut transform =
        EcefTransform::new(Crs::from_definition("EPSG:32632").unwrap(), Some(7.)).unwrap();
    // The UTM zone's central meridian/equator is known independently of PROJ.
    let result = transform.transform(&[[500_000., 0., 123.]]).unwrap();
    assert_position(result[0], geodetic_to_ecef(Cartographic::new(9., 0., 130.)));
    let source = Crs::from_definition("EPSG:4978").unwrap();
    let target = Crs::from_definition("EPSG:4979").unwrap();
    assert_position(
        StrictTransform::new(&source, &target)
            .unwrap()
            .transform(&result)
            .unwrap()[0],
        [9., 0., 130.],
    );
}

#[test]
fn horizontal_linear_units_preserve_offsets_and_explicit_metre_heights() {
    for (units, factor) in [
        ("m", 1.),
        ("ft", 0.3048),
        ("us-ft", 1200. / 3937.),
        ("km", 1000.),
    ] {
        let source = Crs::from_definition(&format!(
            "+proj=utm +zone=32 +datum=WGS84 +units={units} +type=crs"
        ))
        .unwrap();
        let mut transform = EcefTransform::new(source, Some(7.)).unwrap();
        assert_position(
            transform
                .transform(&[[500_000. / factor, 0., 123.]])
                .unwrap()[0],
            geodetic_to_ecef(Cartographic::new(9., 0., 130.)),
        );
        // The false origin is known independently of projection inverses. Its
        // linear parameters must follow XY's unit conversion, while Z stays m.
        let easting = 5000. / factor;
        let northing = 6000. / factor;
        for definition in [
            format!("+proj=aea +lat_1=33 +lat_2=45 +lat_0=10 +lon_0=20 +x_0=5000 +y_0=6000 +datum=WGS84 +units={units}"),
            format!(r#"PROJCS["Invented Albers",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Albers_Conic_Equal_Area"],PARAMETER["latitude_of_center",10],PARAMETER["longitude_of_center",20],PARAMETER["standard_parallel_1",33],PARAMETER["standard_parallel_2",45],PARAMETER["false_easting",{easting}],PARAMETER["false_northing",{northing}],UNIT["{units}",{factor}]]"#),
            format!(r#"PROJCRS["Invented Albers",BASEGEOGCRS["WGS 84",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563,LENGTHUNIT["metre",1]]],PRIMEM["Greenwich",0,ANGLEUNIT["degree",0.0174532925199433]]],CONVERSION["Invented conversion",METHOD["Albers Equal Area",ID["EPSG",9822]],PARAMETER["Latitude of false origin",10,ANGLEUNIT["degree",0.0174532925199433]],PARAMETER["Longitude of false origin",20,ANGLEUNIT["degree",0.0174532925199433]],PARAMETER["Latitude of 1st standard parallel",33,ANGLEUNIT["degree",0.0174532925199433]],PARAMETER["Latitude of 2nd standard parallel",45,ANGLEUNIT["degree",0.0174532925199433]],PARAMETER["Easting at false origin",{easting},LENGTHUNIT["{units}",{factor}]],PARAMETER["Northing at false origin",{northing},LENGTHUNIT["{units}",{factor}]]],CS[Cartesian,2],AXIS["Easting",east,ORDER[1],LENGTHUNIT["{units}",{factor}]],AXIS["Northing",north,ORDER[2],LENGTHUNIT["{units}",{factor}]]]"#),
        ] {
            let source = Crs::from_definition(&definition).unwrap();
            let mut transform = EcefTransform::new(source, Some(7.)).unwrap();
            assert_position(transform.transform(&[[easting, northing, 123.]]).unwrap()[0],
                geodetic_to_ecef(Cartographic::new(20., 10., 130.)));
        }
    }
}

#[test]
fn native_three_axis_crs_keeps_height_and_transform_outlives_source() {
    let mut transform =
        EcefTransform::new(Crs::from_definition("EPSG:4979").unwrap(), None).unwrap();
    // EcefTransform owns its operation; source/target SRS handles have been dropped.
    assert_position(
        transform.transform(&[[153., -27., 120.]]).unwrap()[0],
        geodetic_to_ecef(Cartographic::new(153., -27., 120.)),
    );
    assert!(transform.transform(&[]).unwrap().is_empty());
    let source = Crs::from_definition("+proj=geocent +datum=WGS84 +units=ft +type=crs").unwrap();
    assert_position(
        EcefTransform::new(source, None)
            .unwrap()
            .transform(&[[6378137. / 0.3048, 0., 0.]])
            .unwrap()[0],
        [6378137., 0., 0.],
    );
}

#[test]
fn crs_definitions_cannot_load_urls_or_files() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/definition.wkt", listener.local_addr().unwrap());
    assert_eq!(
        Crs::from_definition(&url).unwrap_err().category(),
        ("data", 3)
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let wkt = "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]";
    assert!(Crs::from_definition(wkt).is_ok());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("definition.wkt");
    std::fs::write(&path, wkt).unwrap();
    assert_eq!(
        Crs::from_definition(path.to_str().unwrap())
            .unwrap_err()
            .category(),
        ("data", 3)
    );
}

#[test]
fn compound_height_uses_a_local_synthetic_geoid_grid() {
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("invented-geoid.gtx");
    let mut grid = Vec::new();
    // GTX header: latitude/longitude origin, steps, rows/columns, then float32
    // corrections, all big-endian. A constant grid provides an independent
    // five-metre vertical reference without relying on installed datum grids.
    for value in [-2_f64, -2., 2., 2.] {
        grid.extend(value.to_be_bytes());
    }
    for value in [3_i32, 3] {
        grid.extend(value.to_be_bytes());
    }
    for _ in 0..9 {
        grid.extend(5_f32.to_be_bytes());
    }
    std::fs::write(&path, grid).unwrap();
    let source = Crs::from_definition(&format!(
        "+proj=longlat +datum=WGS84 +geoidgrids={} +type=crs",
        path.display()
    ))
    .unwrap();
    assert!(source.has_native_height());
    let result = EcefTransform::new(source, None)
        .unwrap()
        .transform(&[[0., 0., 100.]])
        .unwrap();
    assert_position(result[0], [6378137. + 105., 0., 0.]);
    assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 1);
}

#[test]
fn invalid_height_policy_crs_and_epoch_are_data_errors() {
    for offset in [None, Some(f64::NAN), Some(f64::INFINITY)] {
        let error =
            EcefTransform::new(Crs::from_definition("EPSG:4326").unwrap(), offset).unwrap_err();
        assert_eq!(error.category(), ("data", 3));
    }
    let error =
        EcefTransform::new(Crs::from_definition("EPSG:4979").unwrap(), Some(0.)).unwrap_err();
    assert_eq!(error.category(), ("data", 3));
    assert_eq!(
        Crs::from_definition("invalid CRS").unwrap_err().category(),
        ("data", 3)
    );
    assert_eq!(
        Crs::from_definition("EPSG:4326\0").unwrap_err().category(),
        ("data", 3)
    );
    let mut source = Crs::from_definition("EPSG:4326").unwrap();
    for epoch in [0., -1., f64::NAN, f64::INFINITY] {
        assert_eq!(
            source.set_coordinate_epoch(epoch).unwrap_err().category(),
            ("data", 3)
        );
    }
    assert_eq!(source.coordinate_epoch(), None);
}

#[test]
fn invalid_batch_fails_without_mutating_input_and_operation_recovers() {
    let source = Crs::from_definition("EPSG:4979").unwrap();
    let target = Crs::from_definition("EPSG:4978").unwrap();
    let mut transform = StrictTransform::new(&source, &target).unwrap();
    for point in [[0., 91., 0.], [181., 0., 0.], [0., 0., f64::INFINITY]] {
        let input = [[0., 0., 10.], point];
        assert_eq!(
            transform.transform(&input).unwrap_err().category(),
            ("data", 3)
        );
        assert_eq!(input[0], [0., 0., 10.]);
        assert_eq!(input[1], point);
    }
    assert_position(
        transform.transform(&[[0., 0., 10.]]).unwrap()[0],
        [6378147., 0., 0.],
    );
}

#[test]
fn unknown_datum_cannot_silently_use_ballpark_transformation() {
    let source = Crs::from_definition("+proj=longlat +a=6378200 +b=6356818 +type=crs").unwrap();
    let target = Crs::from_definition("EPSG:4978").unwrap();
    assert_eq!(
        StrictTransform::new(&source, &target)
            .unwrap_err()
            .category(),
        ("environment", 4)
    );
}

#[test]
fn declared_height_grid_cannot_fall_back_when_missing() {
    let source = Crs::from_definition(
        "+proj=longlat +datum=WGS84 +geoidgrids=rusty-tiles-deliberately-missing-grid.gtx +type=crs",
    )
    .unwrap();
    let error = match EcefTransform::new(source, None) {
        Err(error) => error,
        Ok(mut transform) => transform.transform(&[[0., 0., 100.]]).unwrap_err(),
    };
    assert_eq!(error.category(), ("environment", 4));
}

#[test]
fn missing_proj_database_is_an_environment_error() {
    const CHILD: &str = "RUSTY_TILES_TEST_MISSING_PROJ_DATABASE";
    if let Some(directory) = std::env::var_os(CHILD) {
        let directory = std::ffi::CString::new(directory.as_encoded_bytes()).unwrap();
        let paths = [directory.as_ptr(), std::ptr::null()];
        // SAFETY: GDAL copies this null-terminated path list. This isolated
        // subprocess has no other CRS handles or workers using its global policy.
        unsafe { gdal_sys::OSRSetPROJSearchPaths(paths.as_ptr()) };
        let error = Crs::from_definition("EPSG:4326").unwrap_err();
        assert_eq!(error.category(), ("environment", 4));
        assert!(error.to_string().contains("PROJ database unavailable"));
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "missing_proj_database_is_an_environment_error"])
        .env(CHILD, directory.path())
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn each_worker_creates_independent_handles() {
    let workers: Vec<_> = (0..8)
        .map(|index| {
            std::thread::spawn(move || {
                let mut transform =
                    EcefTransform::new(Crs::from_definition("EPSG:4979").unwrap(), None).unwrap();
                transform.transform(&[[0., 0., index as f64]]).unwrap()[0]
            })
        })
        .collect();
    for (index, worker) in workers.into_iter().enumerate() {
        assert_position(worker.join().unwrap(), [6378137. + index as f64, 0., 0.]);
    }
    let linked = versions().unwrap();
    assert!(linked.proj >= [9, 2, 0]);
    assert!(!linked.gdal.is_empty());
}
