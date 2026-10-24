//! Shared F0 publication plumbing for the W1 model producers (`glb-to-3tz`,
//! `createTilesetJson`) and the rigid placement the `mesh` tiler also uses.
use crate::{
    archive3tz::{self, CodecError, WriteFailure},
    runtime::Attempt,
    CleanupDiagnostic, JobError, JobErrorKind, JobFailure,
};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) use crate::gltf_source::binding;
mod model;
mod partition;
mod placement;
pub(crate) use crate::gltf_source::source;

pub use model::{
    model_to_archive, model_to_manifest, ModelManifestRequest, ModelManifestResult, ModelReport,
    ModelWrapRequest, ModelWrapResult,
};
pub(crate) use placement::ResolvedPlacement;
pub use placement::{MeshPlacement, MeshPlacementReport};
struct Workspace(tempfile::TempDir);
impl Workspace {
    fn create(output: &Path) -> Result<Self, JobError> {
        let parent = output
            .parent()
            .expect("resolved absolute output has parent");
        fs::create_dir_all(parent)
            .map_err(|e| JobError::io("create mesh output parent", parent, e))?;
        tempfile::Builder::new()
            .prefix(".mesh-work-")
            .tempdir_in(parent)
            .map(Self)
            .map_err(|e| JobError::io("create mesh workspace", parent, e))
    }
    fn path(&self) -> &Path {
        self.0.path()
    }
    fn cleanup(self) -> Result<(), CleanupDiagnostic> {
        // Explicit ownership transfer prevents a hidden Drop retry after a failed
        // removal has been reported as a retained path.
        let path = self.0.keep();
        fs::remove_dir_all(&path).map_err(|e| CleanupDiagnostic {
            error: JobError::io("remove mesh workspace", &path, e),
            path,
        })
    }
}
struct GeneratedMember {
    name: String,
    path: PathBuf,
    size: u64,
    modified: Option<std::time::SystemTime>,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProducerStage {
    WriteReport,
    Archive,
    Cleanup,
}
// Concrete model I/O seam for persistent writer failures in private tests.
// No producer registry, public configuration or runtime dispatch is introduced.
trait ProducerOperations {
    fn stage(&mut self, _stage: ProducerStage) -> Result<(), JobError> {
        Ok(())
    }
    fn write(&mut self, file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
        file.write_all(bytes)
    }
    fn archive(
        &mut self,
        members: &[archive3tz::Member<'_>],
        writer: &mut File,
        checkpoint: &mut dyn FnMut(u64) -> Result<(), JobError>,
    ) -> Result<(), WriteFailure<JobError>> {
        archive3tz::serialize(members, writer, checkpoint)
    }
}
struct SystemProducer;
impl ProducerOperations for SystemProducer {}

fn write_member(
    workspace: &Workspace,
    name: &str,
    bytes: &[u8],
    attempt: &Attempt,
    operations: &mut impl ProducerOperations,
) -> Result<GeneratedMember, JobError> {
    let path = workspace.path().join(name);
    let mut file =
        File::create(&path).map_err(|e| JobError::io("create generated mesh member", &path, e))?;
    for chunk in bytes.chunks(64 * 1024) {
        attempt.check()?;
        operations
            .write(&mut file, chunk)
            .map_err(|e| JobError::io("write generated mesh member", &path, e))?;
    }
    drop(file);
    let metadata =
        fs::metadata(&path).map_err(|e| JobError::io("inspect generated mesh member", &path, e))?;
    Ok(GeneratedMember {
        name: name.into(),
        path,
        size: metadata.len(),
        modified: metadata.modified().ok(),
    })
}
fn cleanup_failure(mut failure: JobFailure, workspace: Workspace) -> JobFailure {
    if let Err(diagnostic) = workspace.cleanup() {
        failure.retained_paths.push(diagnostic.path);
        failure.secondary.push(diagnostic.error);
    }
    failure
}
fn codec_error(error: CodecError, output: &Path) -> JobError {
    match error {
        CodecError::Io(e) => JobError::io("serialize mesh archive", output, e),
        CodecError::SourceIo { path, source } => {
            JobError::io("read generated mesh member", &path, source)
        }
        CodecError::Zip(e) => {
            JobError::io("serialize mesh archive", output, std::io::Error::other(e))
        }
        other => JobError::new(
            JobErrorKind::InvalidState,
            format!("encode generated mesh inventory: {other}"),
        ),
    }
}
