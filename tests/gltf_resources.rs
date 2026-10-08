//! Wrapping glTF preserves declared local resources and never publishes omissions.
use std::{fs, io::Read, path::Path, process::Command};

use serde_json::{json, Value};

fn model() -> (Value, Vec<u8>) {
    let bytes = rusty_tiles::fixtures::triangle_glb();
    let glb = gltf::binary::Glb::from_slice(&bytes).unwrap();
    (
        serde_json::from_slice(&glb.json).unwrap(),
        glb.bin.unwrap().into_owned(),
    )
}

fn wrap(input: &Path, output: &Path, force: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
    command
        .args(["glb-to-3tz", "--json", "-i"])
        .arg(input)
        .arg("-o")
        .arg(output)
        .env("PATH", "");
    if force {
        command.arg("--force");
    }
    command.output().unwrap()
}

fn read_member(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn wraps_external_buffers_and_images_without_changing_source_bytes() {
    let work = tempfile::tempdir().unwrap();
    let (mut doc, bin) = model();
    doc["buffers"][0]["uri"] = json!("./buffers/model.bin");
    doc["buffers"].as_array_mut().unwrap().push(json!({
        "byteLength":1, "uri":"data:application/octet-stream;base64,AA=="
    }));
    doc["images"] = json!([
        {"uri":"textures/color.png"},
        {"uri":"textures/../textures/color.png"}
    ]);
    fs::create_dir(work.path().join("buffers")).unwrap();
    fs::create_dir(work.path().join("textures")).unwrap();
    fs::write(work.path().join("buffers/model.bin"), &bin).unwrap();
    let texture_path = work.path().join("textures/color.png");
    image::RgbaImage::from_pixel(1, 1, image::Rgba([64, 128, 255, 128]))
        .save(&texture_path)
        .unwrap();
    let texture = fs::read(texture_path).unwrap();
    fs::write(work.path().join("unreferenced.txt"), b"leave out").unwrap();
    let input = work.path().join("model.gltf");
    let original = serde_json::to_vec_pretty(&doc).unwrap();
    fs::write(&input, &original).unwrap();
    let output = work.path().join("model.3tz");
    let result = wrap(&input, &output, false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    rusty_tiles::validate_3tz(&output).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    assert_eq!(zip.len(), 5); // Manifest, model, two unique resources, index.
    assert_eq!(read_member(&mut zip, "model.gltf"), original);
    assert_eq!(read_member(&mut zip, "buffers/model.bin"), bin);
    assert_eq!(read_member(&mut zip, "textures/color.png"), texture);
    assert_eq!(fs::read(&input).unwrap(), original);
    assert_eq!(
        fs::read(work.path().join("buffers/model.bin")).unwrap(),
        bin
    );
    assert_eq!(
        fs::read(work.path().join("textures/color.png")).unwrap(),
        texture
    );
    let before = fs::read(&output).unwrap();
    assert_eq!(wrap(&input, &output, false).status.code(), Some(5));
    assert_eq!(fs::read(&output).unwrap(), before);
}

#[test]
fn glb_can_also_reference_external_images() {
    let work = tempfile::tempdir().unwrap();
    let (mut doc, bin) = model();
    doc["images"] = json!([{"uri":"texture.png"}]);
    let original = gltf::binary::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: 0,
        },
        json: serde_json::to_vec(&doc).unwrap().into(),
        bin: Some(bin.into()),
    }
    .to_vec()
    .unwrap();
    let input = work.path().join("model.glb");
    fs::write(&input, &original).unwrap();
    let texture_path = work.path().join("texture.png");
    image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
        .save(&texture_path)
        .unwrap();
    let texture = fs::read(texture_path).unwrap();
    let output = work.path().join("result.3tz");
    let result = wrap(&input, &output, false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    rusty_tiles::validate_3tz(&output).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    assert_eq!(read_member(&mut zip, "model.glb"), original);
    assert_eq!(read_member(&mut zip, "texture.png"), texture);
}

#[test]
fn wraps_a_local_structural_metadata_schema() {
    let work = tempfile::tempdir().unwrap();
    let (mut doc, bin) = model();
    doc["buffers"][0]["uri"] = json!("model.bin");
    fs::write(work.path().join("model.bin"), bin).unwrap();
    doc["extensionsUsed"] = json!(["EXT_structural_metadata"]);
    doc["extensions"] = json!({"EXT_structural_metadata":{"schemaUri":"schema.json"}});
    let input = work.path().join("model.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let schema = br#"{"id":"invented-test-schema","classes":{}}"#;
    fs::write(work.path().join("schema.json"), schema).unwrap();
    let output = work.path().join("result.3tz");
    let result = wrap(&input, &output, false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    rusty_tiles::validate_3tz(&output).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    assert_eq!(read_member(&mut zip, "schema.json"), schema);
    fs::remove_file(work.path().join("schema.json")).unwrap();
    assert_eq!(wrap(&input, &output, true).status.code(), Some(3));
}

#[test]
fn unavailable_or_unsupported_resources_never_replace_an_archive() {
    for uri in [
        "missing.bin",
        "../outside.bin",
        "/absolute.bin",
        "https://example.com/model.bin",
        "model%20data.bin",
        "model.bin?query",
        "model.bin#fragment",
        "folder\\model.bin",
        "tileset.json",
        "@3dtilesIndex1@",
        "",
    ] {
        let work = tempfile::tempdir().unwrap();
        let (mut doc, _) = model();
        doc["buffers"][0]["uri"] = json!(uri);
        let input = work.path().join("model.gltf");
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let output = work.path().join("result.3tz");
        let failure = wrap(&input, &output, false);
        assert_eq!(
            failure.status.code(),
            Some(3),
            "{uri}: {}",
            String::from_utf8_lossy(&failure.stderr)
        );
        assert!(!output.exists());
        fs::write(&output, b"previous archive").unwrap();
        let failure = wrap(&input, &output, true);
        assert_eq!(failure.status.code(), Some(3), "{uri}");
        assert_eq!(fs::read(&output).unwrap(), b"previous archive");
        assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
    }
}

#[cfg(unix)]
#[test]
fn resources_cannot_escape_through_symlinks() {
    let work = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("model.bin"), b"outside").unwrap();
    std::os::unix::fs::symlink(external.path(), work.path().join("buffers")).unwrap();
    let (mut doc, _) = model();
    doc["buffers"][0]["uri"] = json!("buffers/model.bin");
    let input = work.path().join("model.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let output = work.path().join("result.3tz");
    assert_eq!(wrap(&input, &output, false).status.code(), Some(3));
    assert!(!output.exists());
}
