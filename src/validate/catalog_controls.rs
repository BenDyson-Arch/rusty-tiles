//! VS-A controls: the catalog and report rows on scratch (forced-spill profile) against the
//! resident profile, the closure message cap, and URI order of long shared prefixes.
use super::i1_fixtures::{package, point_glb, tileset, Files};
use super::*;
use crate::archive3tz::FORCED;

/// Runs `inspect` with the catalog profile forced on this thread. A successful report
/// states scratch exactly when the profile spills (`TMPDIR` is a disk in these runs).
fn inspect_as(path: &Path, forced: bool) -> Result<ValidationReport, Error> {
    FORCED.with(|f| f.set(Some(forced)));
    let result = inspect_selected(ValidationRequest::new(path), ValidationLimits::default());
    FORCED.with(|f| f.set(None));
    if let Ok(report) = &result {
        assert_eq!(report.scratch.is_some(), forced, "{:?}", report.scratch);
    }
    result
}

/// The comparable outcome: the CLI's JSON bytes without the profile's `scratch` member,
/// or the error category and message.
fn outcome(result: Result<ValidationReport, Error>) -> String {
    match result {
        Ok(report) => {
            let mut streamed = Vec::new();
            report.write_json(&mut streamed).unwrap();
            let mut materialized = serde_json::to_value(&report).unwrap();
            assert_eq!(
                String::from_utf8(streamed).unwrap(),
                materialized.to_string()
            );
            materialized.as_object_mut().unwrap().remove("scratch");
            materialized.to_string()
        }
        Err(error) => format!("{}: {error}", error.category().0),
    }
}

fn both(files: &Files) -> String {
    let (_work, path) = package(files);
    let resident = outcome(inspect_as(&path, false));
    let spilled = outcome(inspect_as(&path, true));
    assert_eq!(resident, spilled);
    resident
}

fn content_tile(uris: &[String]) -> Value {
    let half = 1.0;
    let children: Vec<Value> = uris
        .iter()
        .map(|uri| {
            json!({"boundingVolume":{"box":[0,0,0,half,0,0,0,half,0,0,0,half]},
                "geometricError":0,"content":{"uri":uri}})
        })
        .collect();
    json!({"boundingVolume":{"box":[0,0,0,half,0,0,0,half,0,0,0,half]},
        "geometricError":1,"refine":"ADD","children":children})
}

/// Names sharing 64- and 128-byte prefixes, prefixes of one another, and names that end
/// exactly at a chunk boundary all come out in URI byte order under both profiles.
#[test]
fn payload_rows_are_in_uri_order_across_shared_long_prefixes() {
    let long = "p".repeat(70);
    let longer = format!("{long}/{}", "q".repeat(60));
    let mut uris = vec![
        format!("{long}/a.glb"),
        format!("{long}/b.glb"),
        // Distinct ignoring case too: Windows and macOS file systems fold case.
        format!("{long}/C.glb"),
        format!("{longer}/z.glb"),
        format!("{longer}/y.glb"),
        format!("{longer}/yy.glb"),
        format!("{longer}0.glb"),
        format!("{}.glb", "p".repeat(60)),
        "a.glb".into(),
        "b/c.glb".into(),
        "b.glb".into(),
    ];
    let mut files: Files = uris.iter().map(|uri| (uri.clone(), point_glb())).collect();
    files.insert("tileset.json".into(), tileset(content_tile(&uris)));
    let report = both(&files);
    let report: Value = serde_json::from_str(&report).unwrap();
    let listed: Vec<String> = report["payloads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["uri"].as_str().unwrap().to_owned())
        .collect();
    uris.sort();
    assert_eq!(listed, uris);
}

/// More than 16 unreferenced members: the first 16 names in byte order and a count. Up to
/// 16 keeps the full list.
#[test]
fn unreferenced_entries_list_sixteen_names_and_count_the_rest() {
    let mut files = Files::new();
    files.insert(
        "tileset.json".into(),
        tileset(content_tile(&["used.glb".into()])),
    );
    files.insert("used.glb".into(), point_glb());
    for i in (0..16).rev() {
        files.insert(format!("extra/{i:02}.bin"), vec![i as u8]);
    }
    let names: Vec<String> = (0..16).map(|i| format!("extra/{i:02}.bin")).collect();
    assert_eq!(
        both(&files),
        format!(
            "invalid_input: unreferenced archive entries: {}",
            names.join(", ")
        )
    );
    files.insert("extra/16.bin".into(), vec![16]);
    files.insert("a-first.bin".into(), vec![17]);
    files.insert("z-last.bin".into(), vec![18]);
    let mut shown = vec!["a-first.bin".to_owned()];
    shown.extend(names[..15].iter().cloned());
    assert_eq!(
        both(&files),
        format!(
            "invalid_input: unreferenced archive entries: {} and 3 more",
            shown.join(", ")
        )
    );
}

/// The I1 and explicit fixtures of this module give the same report, byte for byte,
/// whichever profile holds the catalog; a missing member keeps its message.
#[test]
fn profiles_agree_on_reports_and_failures() {
    let mut files = Files::new();
    let uris: Vec<String> = (0..12).map(|i| format!("t/{i}.glb")).collect();
    files.insert("tileset.json".into(), tileset(content_tile(&uris)));
    for uri in &uris {
        files.insert(uri.clone(), point_glb());
    }
    let report = both(&files);
    assert!(report.contains("\"ok\":true"), "{report}");
    files.remove("t/7.glb");
    assert_eq!(
        both(&files),
        "invalid_input: missing archive entry: t/7.glb"
    );
}

/// Only the `vector-build.json` digest survives the hash pass, and `conversion.json` is
/// marked used by its presence alone: both checks keep their verdicts.
#[test]
fn vector_build_digest_and_conversion_marker_survive_the_hash_pass() {
    use sha2::Digest;
    let state = b"{\"state\":1}".to_vec();
    let digest = format!("{:x}", sha2::Sha256::digest(&state));
    let manifest = |expected: &str| {
        let mut doc: Value =
            serde_json::from_slice(&tileset(content_tile(&["a.glb".into()]))).unwrap();
        doc["asset"]["extras"] = json!({"vectorBuildStateSha256": expected});
        // A rusty-tiles vector archive's empty root refines until it is about a pixel.
        (doc["geometricError"], doc["root"]["geometricError"]) = (json!(64), json!(64));
        serde_json::to_vec(&doc).unwrap()
    };
    let mut files = Files::new();
    files.insert("a.glb".into(), point_glb());
    files.insert("vector-build.json".into(), state);
    files.insert("conversion.json".into(), b"{}".to_vec());
    files.insert("tileset.json".into(), manifest(&digest));
    assert!(both(&files).contains("\"ok\":true"));
    files.insert("tileset.json".into(), manifest(&"0".repeat(64)));
    assert_eq!(
        both(&files),
        "invalid_input: vector build-state checksum mismatch"
    );
}

/// A central directory in reverse URI order (the log order the row passes break ties by),
/// with payload URIs tied past 64 and 128 bytes: rows still come out in URI order.
#[test]
fn payload_rows_follow_uri_order_when_the_directory_does_not() {
    let one = "d".repeat(100);
    let two = format!("{}/{}", "e".repeat(64), "f".repeat(70));
    let mut uris: Vec<String> = (0..12)
        .flat_map(|i| [format!("{one}/{i:02}.glb"), format!("{two}/{i}.glb")])
        .collect();
    uris.sort();
    uris.reverse();
    let glb = point_glb();
    let manifest = tileset(content_tile(&uris));
    let mut records: Vec<(&[u8], &[u8])> = vec![(b"tileset.json", &manifest)];
    records.extend(uris.iter().map(|uri| (uri.as_bytes(), glb.as_slice())));
    let bytes = crate::archive3tz::read_tests::indexed_zip(&records);
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("reversed.3tz");
    std::fs::write(&path, bytes).unwrap();
    let resident = outcome(inspect_as(&path, false));
    assert_eq!(resident, outcome(inspect_as(&path, true)));
    let report: Value = serde_json::from_str(&resident).unwrap();
    let listed: Vec<&str> = report["payloads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["uri"].as_str().unwrap())
        .collect();
    uris.sort();
    assert_eq!(listed, uris);
}

/// CAT-6: the validator's own scratch failures map to typed categories with the
/// `validation scratch:` prefix, never to a verdict on the archive.
#[test]
fn scratch_failures_map_to_io_resource_limit_and_invalid_state() {
    use crate::spill::{InsufficientSpace, PreflightStage, SpaceRole, SpillError};
    let path = std::path::PathBuf::from("/scratch");
    let cases = [
        (
            SpillError::io(
                "write spill run",
                &path,
                std::io::Error::from(std::io::ErrorKind::StorageFull),
            ),
            "io",
        ),
        (
            SpillError::InsufficientSpace(InsufficientSpace {
                path: path.clone(),
                needed: 2,
                available: 1,
                filesystem: SpaceRole::Scratch,
                stage: PreflightStage::Geometry,
            }),
            "io",
        ),
        (
            SpillError::MemoryBackedOnly {
                candidates: vec![path.clone()],
            },
            "environment",
        ),
        (SpillError::Overflow, "resource_limit"),
        (
            SpillError::CorruptRun {
                path: path.clone(),
                detail: "short".into(),
            },
            "invalid_state",
        ),
        (SpillError::Poisoned, "invalid_state"),
        (SpillError::InvalidConfig("bad"), "invalid_state"),
    ];
    for (error, category) in cases {
        let failure = spill_failure(error);
        assert_eq!(failure.category().0, category, "{failure}");
        assert!(
            failure.to_string().starts_with("validation scratch: "),
            "{failure}"
        );
    }
    let kind = match spill_failure(SpillError::io(
        "write spill run",
        &path,
        std::io::Error::from(std::io::ErrorKind::StorageFull),
    )) {
        Error::Scratch(ScratchRefusal::Io(error)) => error.kind(),
        other => panic!("{other:?}"),
    };
    assert_eq!(kind, std::io::ErrorKind::StorageFull);
    // Through the archive owner too.
    assert_eq!(
        archive_admission_error(crate::archive3tz::ReadError::Scratch(SpillError::Poisoned))
            .category()
            .0,
        "invalid_state"
    );
}
