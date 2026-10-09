//! Public lifecycle probe isolated in its own process because CWD is process-global.
use rusty_tiles::{
    vector::{vector_to_archive, VectorOptions, VectorRequest},
    JobError, Observer, RunControl, RunEvent,
};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct ChangeDirectory {
    destination: PathBuf,
    calls: AtomicUsize,
}
impl Observer for ChangeDirectory {
    fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            std::env::set_current_dir(&self.destination).unwrap();
        }
        Ok(())
    }
}

#[test]
fn relative_paths_survive_observer_cwd_change() {
    if std::env::var_os("RT_VECTOR_CWD_CHILD").is_none() {
        let root = tempfile::tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "relative_paths_survive_observer_cwd_change",
                "--nocapture",
            ])
            .env("RT_VECTOR_CWD_CHILD", "1")
            .current_dir(root.path())
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let original = std::env::current_dir().unwrap();
    fs::create_dir("elsewhere").unwrap();
    fs::write("source.geojson", r#"{"type":"FeatureCollection","features":[{"type":"Feature","id":"accepted","properties":{},"geometry":{"type":"Point","coordinates":[20,20,0]}}]}"#).unwrap();
    let observer = Arc::new(ChangeDirectory {
        destination: original.join("elsewhere"),
        calls: AtomicUsize::new(0),
    });
    let control = RunControl::new(Some(observer.clone()));
    let options = VectorOptions {
        source_crs: Some("local".into()),
        explicit: true,
        reproducible: true,
        ..VectorOptions::default()
    };
    let result = vector_to_archive(
        VectorRequest::new("source.geojson", "output.3tz", options),
        &control,
    )
    .unwrap();
    assert!(observer.calls.load(Ordering::SeqCst) > 0);
    assert_eq!(result.output, original.join("output.3tz"));
    assert!(result.output.is_file());
    assert!(!original.join("elsewhere/output.3tz").exists());
    let mut archive = zip::ZipArchive::new(fs::File::open(result.output).unwrap()).unwrap();
    let archived: serde_json::Value =
        serde_json::from_reader(archive.by_name("conversion.json").unwrap()).unwrap();
    assert_eq!(archived["features"], 1);
    assert_eq!(archived, result.report);
}
