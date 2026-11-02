//! Optional fixed city sweep; persist rows so develop/candidate G is compared by cell.
use super::*;

#[test]
#[ignore = "200-layout source/validator acceptance; set RT_HLOD_CITY_EVIDENCE"]
fn descendant_city_sweep() {
    use crate::{mesh_large::delivery, validate::ExpectedProfile};
    let destination = PathBuf::from(std::env::var_os("RT_HLOD_CITY_EVIDENCE").unwrap());
    fs::create_dir_all(&destination).unwrap();
    let count = std::env::var("RT_HLOD_CITY_COUNT").map_or(200, |v| v.parse().unwrap());
    for layout in 0..count {
        let work = tempfile::tempdir().unwrap();
        let input = write_gltf(&work.path().join("src"), &city_block(layout));
        let dir = stage(
            &input,
            &work.path().join("hlod"),
            &Run {
                cap: 300,
                ..Run::default()
            },
        )
        .unwrap();
        fs::copy(
            dir.join("proxies.jsonl"),
            destination.join(format!("{layout}.jsonl")),
        )
        .unwrap();
        let output = work.path().join("out.3tz");
        let mut file = fs::File::create(&output).unwrap();
        delivery::publish_with_limits(
            &MeshTilesetRequest::new(&dir, &output),
            delivery::plan::LIMITS,
            &mut file,
            work.path(),
        )
        .unwrap();
        drop(file);
        let result =
            inspect(ValidationRequest::new(&output).expect_profile(ExpectedProfile::Hlod)).unwrap();
        fs::write(
            destination.join(format!("{layout}.validation.json")),
            serde_json::to_vec(&result).unwrap(),
        )
        .unwrap();
        eprintln!("city {layout}: validated");
    }
}
