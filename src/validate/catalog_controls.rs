//! VS-A controls: the catalog and report rows on scratch (forced-spill profile) against the
//! resident profile, the closure message cap, and URI order of long shared prefixes.
use super::i1_fixtures::{package, point_glb, tileset, Files};
use super::*;
use crate::archive3tz::FORCED;

/// Runs `inspect` with the catalog profile forced on this thread.
fn inspect_as(path: &Path, forced: bool) -> Result<ValidationReport, Error> {
    FORCED.with(|f| f.set(Some(forced)));
    let result = inspect_selected(ValidationRequest::new(path), ValidationLimits::default());
    FORCED.with(|f| f.set(None));
    result
}

/// The comparable outcome: the CLI's JSON bytes, or the error category and message.
fn outcome(result: Result<ValidationReport, Error>) -> String {
    match result {
        Ok(report) => {
            let mut streamed = Vec::new();
            report.write_json(&mut streamed).unwrap();
            let materialized = serde_json::to_value(&report).unwrap().to_string();
            assert_eq!(String::from_utf8(streamed).unwrap(), materialized);
            materialized
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
        format!("{long}/A.glb"),
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
