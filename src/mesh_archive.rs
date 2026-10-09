//! One bounded local static GLB mesh producer under the F0 file lifecycle.
//! Broader legacy mesh conversion is a separate, unreviewed operation.
use crate::{
    archive3tz::{self, CodecError, WriteFailure},
    runtime::{Attempt, Staging},
    CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl, RunEvent,
};
use serde::Serialize;
use serde_json::json;
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

mod encode;
mod partition;
mod source;

#[derive(Clone, Debug)]
pub struct MeshRequest {
    input: PathBuf,
    output: PathBuf,
    leaf_triangles: usize,
    policy: OutputPolicy,
}
impl MeshRequest {
    pub fn local_gltf(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        leaf_triangles: usize,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            leaf_triangles,
            policy: OutputPolicy::CreateNew,
        }
    }
    pub fn with_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct MeshReport {
    pub schema_version: u64,
    pub profile: &'static str,
    pub coordinates: &'static str,
    pub source_bytes: u64,
    pub triangles: u64,
    pub leaf_tiles: u64,
    pub leaf_triangles: u64,
    pub routing_geometric_error_metres: f64,
}
#[derive(Debug)]
pub struct MeshResult {
    pub output: PathBuf,
    pub report: MeshReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
struct PreparedMesh {
    request: MeshRequest,
    geometry: source::Geometry,
    leaves: Vec<partition::Leaf>,
    bounds: partition::Bounds,
}
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
struct CompletedMesh {
    workspace: Workspace,
    members: Vec<GeneratedMember>,
    report: MeshReport,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProducerStage {
    Decode,
    EncodeLeaf,
    WriteLeaf,
    WriteReport,
    Archive,
    Cleanup,
}
// Concrete mesh I/O seam for persistent writer failures in private tests.
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

fn invalid(kind: JobErrorKind, message: impl Into<String>) -> JobError {
    JobError::new(kind, message)
}
fn validate(request: &MeshRequest) -> Result<(), JobError> {
    if request.input.as_os_str().is_empty()
        || request.output.as_os_str().is_empty()
        || request.output.file_name().is_none()
        || request.leaf_triangles == 0
    {
        return Err(invalid(
            JobErrorKind::InvalidRequest,
            "mesh requires source/output paths and a positive explicit leaf-triangle limit",
        ));
    }
    if request.output.extension().is_none_or(|e| e != "3tz")
        && !request
            .output
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".3dtiles.zip"))
    {
        return Err(invalid(
            JobErrorKind::InvalidRequest,
            "mesh output must use .3tz or .3dtiles.zip",
        ));
    }
    Ok(())
}

fn prepare(
    mut request: MeshRequest,
    attempt: &Attempt,
    operations: &mut impl ProducerOperations,
) -> Result<PreparedMesh, JobError> {
    attempt.check()?;
    validate(&request)?;
    let source_metadata = fs::symlink_metadata(&request.input)
        .map_err(|e| JobError::io("inspect mesh source", &request.input, e))?;
    if !source_metadata.is_file() {
        return Err(invalid(
            JobErrorKind::InvalidInput,
            "mesh source must be a regular file without a symlink leaf",
        ));
    }
    if source_metadata.len() > source::MAX_SOURCE_BYTES as u64 {
        return Err(invalid(
            JobErrorKind::Unsupported,
            "mesh source exceeds F1a source admission limit",
        ));
    }
    let input = fs::canonicalize(&request.input)
        .map_err(|e| JobError::io("resolve mesh source", &request.input, e))?;
    let output = crate::output_path::resolve(&request.output)?;
    let output_metadata = match fs::symlink_metadata(&request.output) {
        Ok(m) => Some(m),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(JobError::io("inspect mesh output", &request.output, e)),
    };
    if output == input
        || output.starts_with(&input)
        || (output_metadata.is_some()
            && same_file::is_same_file(&request.output, &input).map_err(|e| {
                JobError::io("compare mesh source/output identity", &request.output, e)
            })?)
    {
        return Err(invalid(
            JobErrorKind::InvalidRequest,
            "mesh output overlaps or aliases source",
        ));
    }
    if output_metadata.is_some_and(|m| !m.is_file()) {
        return Err(invalid(
            JobErrorKind::InvalidInput,
            "mesh output must be a regular file without symlinks",
        ));
    }
    request.input = input;
    request.output = output;
    let mut file = File::open(&request.input)
        .map_err(|e| JobError::io("open mesh source", &request.input, e))?;
    let before = file
        .metadata()
        .map_err(|e| JobError::io("inspect open mesh source", &request.input, e))?;
    if !before.is_file()
        || before.len() != source_metadata.len()
        || before.modified().ok() != source_metadata.modified().ok()
    {
        return Err(invalid(
            JobErrorKind::InvalidInput,
            "mesh source changed during preparation",
        ));
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&mut file)
        .take(source::MAX_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| JobError::io("read mesh source", &request.input, e))?;
    let after = file
        .metadata()
        .map_err(|e| JobError::io("inspect mesh source after reading", &request.input, e))?;
    if bytes.len() > source::MAX_SOURCE_BYTES {
        return Err(invalid(
            JobErrorKind::Unsupported,
            "mesh source exceeds F1a source admission limit",
        ));
    }
    if bytes.len() as u64 != before.len()
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
    {
        return Err(invalid(
            JobErrorKind::InvalidInput,
            "mesh source changed during bounded reading",
        ));
    }
    drop(file);
    operations.stage(ProducerStage::Decode)?;
    let geometry = source::decode(&bytes, || attempt.check())?;
    drop(bytes);
    let (leaves, bounds) = partition::plan(
        &geometry.triangles,
        request.leaf_triangles,
        source::MAX_LEAVES,
        || attempt.check(),
    )?;
    Ok(PreparedMesh {
        request,
        geometry,
        leaves,
        bounds,
    })
}
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
fn produce(
    prepared: &PreparedMesh,
    workspace: &Workspace,
    attempt: &Attempt,
    operations: &mut impl ProducerOperations,
) -> Result<(Vec<GeneratedMember>, MeshReport), JobError> {
    let leaf_path = workspace.path().join("t");
    fs::create_dir(&leaf_path)
        .map_err(|e| JobError::io("create mesh content directory", &leaf_path, e))?;
    let mut members = Vec::with_capacity(prepared.leaves.len() + 2);
    let mut children = Vec::with_capacity(prepared.leaves.len());
    for (id, leaf) in prepared.leaves.iter().enumerate() {
        attempt.check()?;
        operations.stage(ProducerStage::EncodeLeaf)?;
        let bytes =
            encode::leaf(&prepared.geometry, leaf, || attempt.check()).map_err(|e| match e {
                encode::EncodeError::Checkpoint(error) => error,
                other => invalid(
                    JobErrorKind::InvalidState,
                    format!("encode validated mesh leaf: {other}"),
                ),
            })?;
        let name = format!("t/{id}.glb");
        operations.stage(ProducerStage::WriteLeaf)?;
        members.push(write_member(workspace, &name, &bytes, attempt, operations)?);
        drop(bytes);
        children.push(json!({"boundingVolume":{"box":leaf.bounds.box_values()},"geometricError":0.0,"content":{"uri":name}}));
        attempt.emit(&RunEvent::Progress {
            phase: "mesh_leaves",
            done: (id + 1) as u64,
            total: Some(prepared.leaves.len() as u64),
        })?;
    }
    let routing_error = prepared.bounds.diagonal().max(1.0);
    let tileset = json!({"asset":{"version":"1.1"},"geometricError":routing_error,
        "root":{"boundingVolume":{"box":prepared.bounds.box_values()},"geometricError":routing_error,"refine":"REPLACE","children":children}});
    let manifest = serde_json::to_vec(&tileset).map_err(|e| {
        invalid(
            JobErrorKind::InvalidState,
            format!("serialize mesh manifest: {e}"),
        )
    })?;
    members.push(write_member(
        workspace,
        "tileset.json",
        &manifest,
        attempt,
        operations,
    )?);
    let report = MeshReport {
        schema_version: 1,
        profile: "f1a-local-static-glb-v1",
        coordinates: "local-gltf",
        source_bytes: prepared.geometry.source_bytes,
        triangles: prepared.geometry.triangles.len() as u64,
        leaf_tiles: prepared.leaves.len() as u64,
        leaf_triangles: prepared.request.leaf_triangles as u64,
        routing_geometric_error_metres: routing_error,
    };
    let bytes = serde_json::to_vec(&report).map_err(|e| {
        invalid(
            JobErrorKind::InvalidState,
            format!("serialize mesh report: {e}"),
        )
    })?;
    operations.stage(ProducerStage::WriteReport)?;
    members.push(write_member(
        workspace,
        "conversion.json",
        &bytes,
        attempt,
        operations,
    )?);
    Ok((members, report))
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
        other => invalid(
            JobErrorKind::InvalidState,
            format!("encode generated mesh inventory: {other}"),
        ),
    }
}
pub fn mesh_to_archive(
    request: MeshRequest,
    control: &RunControl,
) -> Result<MeshResult, JobFailure> {
    mesh_to_archive_with_operations(request, control, &mut SystemProducer)
}
// Private typed stage seam, solely for deterministic producer-failure tests.
// It carries neither frontend configuration nor runtime converter dispatch.
fn mesh_to_archive_with_operations(
    request: MeshRequest,
    control: &RunControl,
    operations: &mut impl ProducerOperations,
) -> Result<MeshResult, JobFailure> {
    let attempt = control.begin()?;
    let prepared = prepare(request, &attempt, operations).map_err(|e| attempt.fail(e))?;
    attempt
        .emit(&RunEvent::Progress {
            phase: "mesh_leaves",
            done: 0,
            total: Some(prepared.leaves.len() as u64),
        })
        .map_err(|e| attempt.fail(e))?;
    let workspace = Workspace::create(&prepared.request.output).map_err(|e| attempt.fail(e))?;
    let (members, report) = match produce(&prepared, &workspace, &attempt, operations) {
        Ok(value) => value,
        Err(e) => return Err(cleanup_failure(attempt.fail(e), workspace)),
    };
    let completed = CompletedMesh {
        workspace,
        members,
        report,
    };
    let mut staging = match Staging::create(&prepared.request.output, &attempt) {
        Ok(value) => value,
        Err(failure) => return Err(cleanup_failure(failure, completed.workspace)),
    };
    let selected: Vec<_> = completed
        .members
        .iter()
        .map(|m| archive3tz::Member {
            name: &m.name,
            source: &m.path,
            size: m.size,
            modified: m.modified,
        })
        .collect();
    let total = completed.members.iter().map(|m| m.size).sum();
    if let Err(e) = operations.stage(ProducerStage::Archive) {
        return Err(cleanup_failure(staging.fail(e), completed.workspace));
    }
    let encoded = operations.archive(&selected, staging.writer(), &mut |done| {
        attempt.check()?;
        attempt.emit(&RunEvent::Progress {
            phase: "mesh_archive",
            done,
            total: Some(total),
        })
    });
    if let Err(e) = encoded {
        let e = match e {
            WriteFailure::Checkpoint(e) => e,
            WriteFailure::Codec(e) => codec_error(e, &prepared.request.output),
        };
        return Err(cleanup_failure(staging.fail(e), completed.workspace));
    }
    if let Err(error) = operations.stage(ProducerStage::Cleanup) {
        let path = completed.workspace.0.keep();
        let mut failure = staging.fail(error);
        failure.retained_paths.push(path);
        return Err(failure);
    }
    if let Err(diagnostic) = completed.workspace.cleanup() {
        let mut failure = staging.fail(diagnostic.error);
        failure.retained_paths.push(diagnostic.path);
        return Err(failure);
    }
    if let Err(e) = attempt
        .emit(&RunEvent::Progress {
            phase: "ready_to_publish",
            done: total,
            total: Some(total),
        })
        .and_then(|_| attempt.close_events())
    {
        return Err(staging.fail(e));
    }
    let published = staging
        .seal()?
        .publish(&prepared.request.output, prepared.request.policy)?;
    Ok(MeshResult {
        output: published.output,
        report: completed.report,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let positions = [
            [0_f32, 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [2., 0., 0.],
            [3., 0., 0.],
            [2., 1., 0.],
        ];
        let mut bin = Vec::new();
        for point in positions {
            for value in point {
                bin.extend(value.to_le_bytes());
            }
        }
        let document = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"buffers":[{"byteLength":bin.len()}],
            "bufferViews":[{"buffer":0,"byteLength":bin.len()}],"accessors":[{"bufferView":0,"componentType":5126,
                "count":6,"type":"VEC3","min":[0,0,0],"max":[3,1,0]}]});
        let mut json = serde_json::to_vec(&document).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bytes = b"glTF".to_vec();
        for value in [
            2,
            (28 + json.len() + bin.len()) as u32,
            json.len() as u32,
            0x4e4f534a,
        ] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(json);
        bytes.extend((bin.len() as u32).to_le_bytes());
        bytes.extend(b"BIN\0");
        bytes.extend(bin);
        bytes
    }
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("source.glb");
        let output = work.path().join("out.3tz");
        fs::write(&input, fixture()).unwrap();
        fs::write(&output, b"previous").unwrap();
        (work, input, output)
    }
    struct FailStage {
        target: ProducerStage,
        reached: bool,
    }
    impl ProducerOperations for FailStage {
        fn stage(&mut self, stage: ProducerStage) -> Result<(), JobError> {
            if stage == self.target {
                self.reached = true;
                Err(JobError::new(
                    JobErrorKind::Io,
                    "injected mesh producer-stage failure",
                ))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn injected_producer_stage_failures_abort_preserve_and_account_for_workspace() {
        for target in [
            ProducerStage::Decode,
            ProducerStage::EncodeLeaf,
            ProducerStage::WriteLeaf,
            ProducerStage::WriteReport,
            ProducerStage::Archive,
            ProducerStage::Cleanup,
        ] {
            let (work, input, output) = setup();
            let control = RunControl::default();
            let mut operations = FailStage {
                target,
                reached: false,
            };
            let failure = mesh_to_archive_with_operations(
                MeshRequest::local_gltf(input, &output, 1).with_policy(OutputPolicy::Replace),
                &control,
                &mut operations,
            )
            .unwrap_err();
            assert!(operations.reached, "{target:?}");
            assert_eq!(failure.error.kind(), JobErrorKind::Io, "{target:?}");
            assert_eq!(fs::read(&output).unwrap(), b"previous");
            assert!(!control.cancellation_handle().cancel());
            if target == ProducerStage::Cleanup {
                assert_eq!(failure.retained_paths.len(), 1);
                assert!(failure.retained_paths[0].is_dir());
                fs::remove_dir_all(&failure.retained_paths[0]).unwrap();
            } else {
                assert!(failure.retained_paths.is_empty(), "{target:?}: {failure:?}");
            }
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2, "{target:?}");
        }
    }
    struct PartialWrite {
        target: ProducerStage,
        current: ProducerStage,
        reached: bool,
    }
    impl ProducerOperations for PartialWrite {
        fn stage(&mut self, stage: ProducerStage) -> Result<(), JobError> {
            self.current = stage;
            Ok(())
        }
        fn write(&mut self, file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
            if self.current != self.target {
                return file.write_all(bytes);
            }
            file.write_all(&bytes[..bytes.len() / 2])?;
            self.reached = true;
            Err(std::io::Error::other(
                "injected persistent generated-file write fault",
            ))
        }
    }
    #[test]
    fn actual_partial_leaf_and_report_write_faults_remove_partial_inventory_and_preserve_output() {
        for target in [ProducerStage::WriteLeaf, ProducerStage::WriteReport] {
            let (work, input, output) = setup();
            let control = RunControl::default();
            let mut operations = PartialWrite {
                target,
                current: ProducerStage::Decode,
                reached: false,
            };
            let failure = mesh_to_archive_with_operations(
                MeshRequest::local_gltf(input, &output, 1).with_policy(OutputPolicy::Replace),
                &control,
                &mut operations,
            )
            .unwrap_err();
            assert!(operations.reached);
            assert_eq!(failure.error.kind(), JobErrorKind::Io);
            assert_eq!(fs::read(output).unwrap(), b"previous");
            assert!(failure.retained_paths.is_empty());
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
            assert!(!control.cancellation_handle().cancel());
        }
    }
    struct ArchiveFault {
        fault: archive3tz::test_faults::Fault,
        reached: bool,
    }
    impl ProducerOperations for ArchiveFault {
        fn archive(
            &mut self,
            members: &[archive3tz::Member<'_>],
            writer: &mut File,
            checkpoint: &mut dyn FnMut(u64) -> Result<(), JobError>,
        ) -> Result<(), WriteFailure<JobError>> {
            let mut writer = archive3tz::test_faults::FaultWriter::new(writer, self.fault);
            let result = archive3tz::serialize(members, &mut writer, checkpoint);
            self.reached = writer.fired;
            result
        }
    }
    #[test]
    fn actual_archive_finalization_and_flush_faults_abort_and_clean_both_candidates() {
        use archive3tz::test_faults::Fault;
        for fault in [Fault::Finalize, Fault::Flush] {
            let (work, input, output) = setup();
            let control = RunControl::default();
            let mut operations = ArchiveFault {
                fault,
                reached: false,
            };
            let failure = mesh_to_archive_with_operations(
                MeshRequest::local_gltf(input, &output, 1).with_policy(OutputPolicy::Replace),
                &control,
                &mut operations,
            )
            .unwrap_err();
            assert!(operations.reached);
            assert_eq!(failure.error.kind(), JobErrorKind::Io);
            assert_eq!(fs::read(output).unwrap(), b"previous");
            assert!(failure.retained_paths.is_empty());
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
            assert!(!control.cancellation_handle().cancel());
        }
    }
    struct PartialArchive;
    struct PayloadWriter<'a> {
        file: &'a mut File,
        fired: bool,
    }
    impl Write for PayloadWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.fired {
                return Err(std::io::Error::other("persistent archive payload fault"));
            }
            if bytes.len() >= 64 {
                self.file.write_all(&bytes[..bytes.len() / 2])?;
                self.fired = true;
                return Err(std::io::Error::other(
                    "injected partial archive payload write",
                ));
            }
            self.file.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }
    impl std::io::Seek for PayloadWriter<'_> {
        fn seek(&mut self, at: std::io::SeekFrom) -> std::io::Result<u64> {
            std::io::Seek::seek(self.file, at)
        }
    }
    impl ProducerOperations for PartialArchive {
        fn archive(
            &mut self,
            members: &[archive3tz::Member<'_>],
            writer: &mut File,
            checkpoint: &mut dyn FnMut(u64) -> Result<(), JobError>,
        ) -> Result<(), WriteFailure<JobError>> {
            let mut writer = PayloadWriter {
                file: writer,
                fired: false,
            };
            let result = archive3tz::serialize(members, &mut writer, checkpoint);
            assert!(
                writer.fired,
                "physical archive payload injection not reached"
            );
            result
        }
    }
    #[test]
    fn actual_archive_payload_write_fault_cleans_workspace_and_partial_archive() {
        let (work, input, output) = setup();
        let control = RunControl::default();
        let failure = mesh_to_archive_with_operations(
            MeshRequest::local_gltf(input, &output, 1).with_policy(OutputPolicy::Replace),
            &control,
            &mut PartialArchive,
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
        assert_eq!(fs::read(output).unwrap(), b"previous");
        assert!(failure.retained_paths.is_empty());
        assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
        assert!(!control.cancellation_handle().cancel());
    }
}
