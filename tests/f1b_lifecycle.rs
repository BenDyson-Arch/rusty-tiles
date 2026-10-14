//! Resource ownership and precommit failure controls using an independent fixture.
use rusty_tiles::{
    mesh_to_archive, JobError, JobErrorKind, MeshRequest, Observer, OutputPolicy, RunControl,
    RunEvent,
};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

const SOURCE: &[u8] = include_bytes!("fixtures/f1b/basecolor.glb");
const IMAGE: &[u8] = include_bytes!("fixtures/f1b/source-rgba.png");

struct ChangeSource {
    input: PathBuf,
    output: PathBuf,
    changed: AtomicBool,
}
impl Observer for ChangeSource {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        assert!(
            !self.output.exists(),
            "observer must finish before installation"
        );
        if matches!(
            event,
            RunEvent::Progress {
                phase: "mesh_leaves",
                done: 0,
                ..
            }
        ) {
            fs::write(&self.input, b"source replaced after preparation").unwrap();
            self.changed.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[test]
fn prepared_image_snapshot_is_shared_and_never_reopened_after_callbacks() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.glb");
    let output = work.path().join("output.3tz");
    fs::write(&input, SOURCE).unwrap();
    let observer = Arc::new(ChangeSource {
        input: input.clone(),
        output: output.clone(),
        changed: AtomicBool::new(false),
    });
    let result = mesh_to_archive(
        MeshRequest::local_gltf(&input, &output, 1),
        &RunControl::new(Some(observer.clone())),
    )
    .unwrap();
    assert!(observer.changed.load(Ordering::SeqCst));
    assert_eq!(result.report.triangles, 8);
    assert_eq!(result.report.leaf_tiles, 8);
    assert_eq!(result.report.images, 1);
    assert_eq!(result.report.image_bytes, IMAGE.len() as u64);
    assert_eq!(result.report.image_pixels, 6);
    assert_eq!(result.report.source_bytes, SOURCE.len() as u64);
    let mut archive = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    let mut actual = Vec::new();
    archive
        .by_name("textures/0.png")
        .unwrap()
        .read_to_end(&mut actual)
        .unwrap();
    assert_eq!(actual, IMAGE);
    assert_eq!(archive.len(), 12); // eight leaves, one image, manifest, report, index
    assert!(
        archive.by_name("textures/1.png").is_err(),
        "unused image must not be published"
    );
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
}

struct StopAfterLeaf {
    output: PathBuf,
    fired: AtomicBool,
}
impl Observer for StopAfterLeaf {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        assert_eq!(fs::read(&self.output).unwrap(), b"previous");
        if matches!(
            event,
            RunEvent::Progress {
                phase: "mesh_leaves",
                done: 1,
                ..
            }
        ) {
            self.fired.store(true, Ordering::SeqCst);
            return Err(JobError::new(
                JobErrorKind::ObserverFailure,
                "stop after image and first leaf writes",
            ));
        }
        Ok(())
    }
}

#[test]
fn failure_after_shared_image_write_cleans_workspace_and_preserves_destination() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.glb");
    let output = work.path().join("output.3tz");
    fs::write(&input, SOURCE).unwrap();
    fs::write(&output, b"previous").unwrap();
    let observer = Arc::new(StopAfterLeaf {
        output: output.clone(),
        fired: AtomicBool::new(false),
    });
    let failure = mesh_to_archive(
        MeshRequest::local_gltf(&input, &output, 1).with_policy(OutputPolicy::Replace),
        &RunControl::new(Some(observer.clone())),
    )
    .unwrap_err();
    assert!(observer.fired.load(Ordering::SeqCst));
    assert_eq!(failure.error.kind(), JobErrorKind::ObserverFailure);
    assert!(failure.retained_paths.is_empty());
    assert_eq!(fs::read(&input).unwrap(), SOURCE);
    assert_eq!(fs::read(&output).unwrap(), b"previous");
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
}
