//! Opt-in exact-byte check on an explicitly supplied admitted static GLB.
//! Historical large-input/3d-tiles-tools manifest and Euler comparisons were
//! deliberately retired with that product; W1 has its own finite contract.
use rusty_tiles::{model_to_archive, ModelWrapRequest, RunControl};
use std::{fs, io::Read, path::PathBuf};

#[test]
#[ignore = "requires explicitly supplied admitted RUSTY_TILES_DEMO_GLB"]
fn user_static_model_wrap_preserves_source_bytes() {
    let input =
        PathBuf::from(std::env::var_os("RUSTY_TILES_DEMO_GLB").expect("set RUSTY_TILES_DEMO_GLB"));
    let original = fs::read(&input).unwrap();
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("model.3tz");
    model_to_archive(
        ModelWrapRequest::local_gltf(&input, &output),
        &RunControl::default(),
    )
    .unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    let mut captured = Vec::new();
    archive
        .by_name("model/source.glb")
        .unwrap()
        .read_to_end(&mut captured)
        .unwrap();
    assert_eq!(captured, original);
    assert_eq!(fs::read(input).unwrap(), original);
    rusty_tiles::validate::inspect(rusty_tiles::validate::ValidationRequest::new(output)).unwrap();
}
