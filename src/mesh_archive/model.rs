//! Exact-byte wrapping and one sibling manifest over the admitted static source.
//! Source/capture/placement own interpretation; F0 owns the single run/commit.
use super::{
    binding, cleanup_failure, codec_error, partition::Bounds, placement::ResolvedPlacement, source,
    write_member, GeneratedMember, MeshPlacement, MeshPlacementReport, ProducerOperations,
    ProducerStage, SystemProducer, Workspace,
};
use crate::{
    archive3tz::{self, WriteFailure},
    runtime::{Attempt, Staging},
    CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl, RunEvent,
};
use serde::Serialize;
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};

const MAX_EMITTED_PAYLOAD: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ModelWrapRequest {
    input: PathBuf,
    output: PathBuf,
    policy: OutputPolicy,
    placement: MeshPlacement,
}
impl ModelWrapRequest {
    pub fn local_gltf(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            policy: OutputPolicy::CreateNew,
            placement: MeshPlacement::Local,
        }
    }
    pub fn with_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
    pub fn with_placement(mut self, placement: MeshPlacement) -> Self {
        self.placement = placement;
        self
    }
}

#[derive(Clone, Debug)]
/// One reference manifest beside an admitted static local model.
///
/// The source document and resources must remain unchanged throughout the
/// entire call, including observer callbacks, and throughout subsequent use of
/// the manifest. This operation publishes only the completed manifest file;
/// it does not publish an immutable snapshot of its live resource directory.
pub struct ModelManifestRequest {
    input: PathBuf,
    policy: OutputPolicy,
    placement: MeshPlacement,
}
impl ModelManifestRequest {
    /// Publish tileset.json beside this admitted source; resources stay in place.
    pub fn local_gltf(input: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
            policy: OutputPolicy::CreateNew,
            placement: MeshPlacement::Local,
        }
    }
    pub fn with_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
    pub fn with_placement(mut self, placement: MeshPlacement) -> Self {
        self.placement = placement;
        self
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ModelReport {
    pub schema_version: u64,
    pub profile: &'static str,
    pub product: &'static str,
    pub source_coordinates: &'static str,
    pub coordinates: &'static str,
    pub placement: MeshPlacementReport,
    pub root_transform: [f64; 16],
    pub bounding_box: [f64; 12],
    /// Conservative omission error used to decide whether to visit the root.
    pub tileset_geometric_error_metres: f64,
    /// The unchanged root content already supplies the admitted full detail.
    pub root_geometric_error_metres: f64,
    pub source_bytes: u64,
    pub external_files: u64,
    pub external_bytes: u64,
    pub captured_bytes: u64,
    pub model_payload_files: u64,
    pub model_payload_bytes: u64,
    pub triangles: u64,
}
#[derive(Debug)]
pub struct ModelWrapResult {
    pub output: PathBuf,
    pub report: ModelReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
#[derive(Debug)]
pub struct ModelManifestResult {
    pub output: PathBuf,
    pub report: ModelReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}

fn invalid(kind: JobErrorKind, message: impl Into<String>) -> JobError {
    JobError::new(kind, message)
}
fn require_input(input: &std::path::Path) -> Result<(), JobError> {
    if input.as_os_str().is_empty() || input.file_name().is_none() {
        return Err(invalid(
            JobErrorKind::InvalidRequest,
            "model source path is required",
        ));
    }
    Ok(())
}
fn require_archive(output: &std::path::Path) -> Result<(), JobError> {
    if output.as_os_str().is_empty()
        || output.file_name().is_none()
        || (output
            .extension()
            .is_none_or(|extension| extension != "3tz")
            && !output
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".3dtiles.zip")))
    {
        return Err(invalid(
            JobErrorKind::InvalidRequest,
            "model archive output must use .3tz or .3dtiles.zip",
        ));
    }
    Ok(())
}

fn captured_inventory(
    snapshot: &binding::Snapshot,
) -> Result<BTreeMap<String, Arc<Vec<u8>>>, JobError> {
    let source_name = if snapshot.root_bytes().starts_with(b"glTF") {
        "model/source.glb"
    } else {
        "model/source.gltf"
    };
    let mut members = BTreeMap::from([(source_name.to_owned(), snapshot.root_owner())]);
    let mut total = snapshot.source_bytes;
    for member in snapshot.captured_members() {
        let parts = member
            .relative
            .iter()
            .map(|part| {
                part.to_str().ok_or_else(|| {
                    invalid(
                        JobErrorKind::InvalidState,
                        "admitted resource path is not UTF-8",
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let name = format!("model/{}", parts.join("/"));
        if name == source_name
            || name
                .strip_prefix(source_name)
                .is_some_and(|suffix| suffix.starts_with('/'))
        {
            return Err(invalid(
                JobErrorKind::Unsupported,
                "model dependency collides with generated source member",
            ));
        }
        if let Some(existing) = members.get(&name) {
            // Equal normalized paths are admitted once. Their immutable owner
            // must agree; equality by bytes is not identity for distinct paths.
            if !Arc::ptr_eq(existing, &member.bytes) {
                return Err(invalid(
                    JobErrorKind::InvalidInput,
                    "model resource paths collide with different captures",
                ));
            }
        } else {
            total = total
                .checked_add(member.bytes.len() as u64)
                .filter(|total| *total <= MAX_EMITTED_PAYLOAD)
                .ok_or_else(|| {
                    invalid(
                        JobErrorKind::Unsupported,
                        "model emitted alias payload exceeds 64 MiB",
                    )
                })?;
            members.insert(name, member.bytes.clone());
        }
    }
    Ok(members)
}

fn tileset_geometric_error_metres(bounding_box: &[f64; 12]) -> f64 {
    // The tileset error describes omitting all content, independently of the
    // full-detail root's zero error. Bound it by the conservative box diagonal.
    // Round each operation outward; a final next_up alone would not account
    // for earlier rounding. The admitted local bounds preclude overflow.
    let squared_radius = [bounding_box[3], bounding_box[7], bounding_box[11]]
        .into_iter()
        .fold(0.0_f64, |sum, half| {
            (sum + (half * half).next_up()).next_up()
        });
    // A one-metre floor gives even degenerate and sub-metre content a positive
    // omission error; it is a conservative visibility policy, not source LOD.
    (2.0 * squared_radius.sqrt().next_up()).next_up().max(1.0)
}

fn prepare_report(
    snapshot: &binding::Snapshot,
    placement: &ResolvedPlacement,
    product: &'static str,
    files: usize,
    bytes: u64,
    attempt: &Attempt,
) -> Result<ModelReport, JobError> {
    // Full admitted payload/image checks belong to the existing source owner.
    let geometry = source::decode(
        &snapshot.document,
        &snapshot.buffers(),
        &snapshot.images(),
        || attempt.check(),
    )?;
    let triangles = geometry.triangles.len() as u64;
    drop(geometry);
    let bounds =
        source::original_bounds(&snapshot.document, &snapshot.buffers(), || attempt.check())?;
    if bounds
        .min
        .iter()
        .chain(&bounds.max)
        .any(|value| !value.is_finite() || value.abs() > 1_000_000.)
    {
        return Err(invalid(
            JobErrorKind::Unsupported,
            "unchanged model conservative bounds exceed the local 1e6 metre component profile",
        ));
    }
    let bounds = Bounds {
        min: bounds.min,
        max: bounds.max,
    };
    let bounding_box = bounds.box_values();
    if bounding_box.iter().any(|value| !value.is_finite()) {
        return Err(invalid(
            JobErrorKind::Unsupported,
            "original model bounds are not finite",
        ));
    }
    Ok(ModelReport {
        schema_version: 1,
        profile: "w1-static-model-v1",
        product,
        source_coordinates: "local-gltf",
        coordinates: placement.coordinates(),
        placement: placement.report(),
        root_transform: placement.transform(),
        bounding_box,
        tileset_geometric_error_metres: tileset_geometric_error_metres(&bounding_box),
        root_geometric_error_metres: 0.,
        source_bytes: snapshot.source_bytes,
        external_files: snapshot.external_files,
        external_bytes: snapshot.external_bytes,
        captured_bytes: snapshot.source_bytes + snapshot.external_bytes,
        model_payload_files: files as u64,
        model_payload_bytes: bytes,
        triangles,
    })
}

fn manifest(report: &ModelReport, content_uri: &str) -> Result<Vec<u8>, JobError> {
    serde_json::to_vec(&json!({
        "asset": {"version": "1.1"},
        "geometricError": report.tileset_geometric_error_metres,
        "root": {
            "boundingVolume": {"box": report.bounding_box},
            "geometricError": report.root_geometric_error_metres,
            "refine": "REPLACE",
            "transform": report.root_transform,
            "content": {"uri": content_uri}
        }
    }))
    .map_err(|error| {
        invalid(
            JobErrorKind::InvalidState,
            format!("serialize prepared model manifest: {error}"),
        )
    })
}

pub fn model_to_archive(
    request: ModelWrapRequest,
    control: &RunControl,
) -> Result<ModelWrapResult, JobFailure> {
    model_to_archive_with_operations(request, control, &mut SystemProducer)
}
fn model_to_archive_with_operations(
    request: ModelWrapRequest,
    control: &RunControl,
    operations: &mut impl ProducerOperations,
) -> Result<ModelWrapResult, JobFailure> {
    let attempt = control.begin()?;
    let prepare = || -> Result<_, JobError> {
        require_input(&request.input)?;
        require_archive(&request.output)?;
        let placement = ResolvedPlacement::resolve(&request.placement)?;
        crate::runtime::file_publication_supported()?;
        let snapshot = binding::load(&request.input, &request.output, || attempt.check())?;
        let inventory = captured_inventory(&snapshot)?;
        let report = prepare_report(
            &snapshot,
            &placement,
            "archive",
            inventory.len(),
            inventory.values().map(|bytes| bytes.len() as u64).sum(),
            &attempt,
        )?;
        // Required serialization completes before side effects.
        let content = if snapshot.root_bytes().starts_with(b"glTF") {
            "model/source.glb"
        } else {
            "model/source.gltf"
        };
        let manifest = manifest(&report, content)?;
        let report_bytes = serde_json::to_vec(&report).map_err(|error| {
            invalid(
                JobErrorKind::InvalidState,
                format!("serialize model report: {error}"),
            )
        })?;
        Ok((snapshot.output, report, manifest, report_bytes, inventory))
    };
    let (output, report, manifest, report_bytes, inventory) =
        prepare().map_err(|error| attempt.fail(error))?;
    // Capture is complete before the first observer. Later source changes do
    // not alter published model/resource bytes.
    attempt
        .emit(&RunEvent::Progress {
            phase: "model_capture",
            done: report.captured_bytes,
            total: Some(report.captured_bytes),
        })
        .map_err(|error| attempt.fail(error))?;
    let workspace = Workspace::create(&output).map_err(|error| attempt.fail(error))?;
    let produce = || -> Result<Vec<GeneratedMember>, JobError> {
        let mut members = Vec::new();
        for (name, bytes) in inventory {
            attempt.check()?;
            let parent = workspace
                .path()
                .join(&name)
                .parent()
                .expect("member parent")
                .to_owned();
            fs::create_dir_all(&parent).map_err(|error| {
                JobError::io("create captured model member parent", &parent, error)
            })?;
            members.push(write_member(
                &workspace, &name, &bytes, &attempt, operations,
            )?);
        }
        operations.stage(ProducerStage::WriteReport)?;
        members.push(write_member(
            &workspace,
            "tileset.json",
            &manifest,
            &attempt,
            operations,
        )?);
        members.push(write_member(
            &workspace,
            "conversion.json",
            &report_bytes,
            &attempt,
            operations,
        )?);
        members.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(members)
    };
    let members = match produce() {
        Ok(members) => members,
        Err(error) => return Err(cleanup_failure(attempt.fail(error), workspace)),
    };
    let mut staging = match Staging::create(&output, &attempt) {
        Ok(staging) => staging,
        Err(failure) => return Err(cleanup_failure(failure, workspace)),
    };
    let selected = members
        .iter()
        .map(|member| archive3tz::Member {
            name: &member.name,
            source: &member.path,
            size: member.size,
            modified: member.modified,
        })
        .collect::<Vec<_>>();
    let total = members.iter().map(|member| member.size).sum::<u64>();
    let encoded = operations
        .stage(ProducerStage::Archive)
        .map_err(WriteFailure::Checkpoint)
        .and_then(|()| {
            operations.archive(&selected, staging.writer(), &mut |done| {
                attempt.check()?;
                attempt.emit(&RunEvent::Progress {
                    phase: "model_archive",
                    done,
                    total: Some(total),
                })
            })
        });
    if let Err(error) = encoded {
        let error = match error {
            WriteFailure::Checkpoint(error) => error,
            WriteFailure::Codec(error) => codec_error(error, &output),
        };
        return Err(cleanup_failure(staging.fail(error), workspace));
    }
    if let Err(error) = operations.stage(ProducerStage::Cleanup) {
        let path = workspace.0.keep();
        let mut failure = staging.fail(error);
        failure.retained_paths.push(path);
        return Err(failure);
    }
    if let Err(diagnostic) = workspace.cleanup() {
        let mut failure = staging.fail(diagnostic.error);
        failure.retained_paths.push(diagnostic.path);
        return Err(failure);
    }
    if let Err(error) = attempt
        .emit(&RunEvent::Progress {
            phase: "ready_to_publish",
            done: total,
            total: Some(total),
        })
        .and_then(|()| attempt.close_events())
    {
        return Err(staging.fail(error));
    }
    let published = staging.seal()?.publish(&output, request.policy)?;
    Ok(ModelWrapResult {
        output: published.output,
        report,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}

/// Publish a full-detail tileset.json beside the source through one F0 run.
/// The source tree must stay unchanged during the entire call (including
/// observers) and continued manifest use. Use model_to_archive for captured
/// immutable model/resource bytes in a self-contained result.
pub fn model_to_manifest(
    request: ModelManifestRequest,
    control: &RunControl,
) -> Result<ModelManifestResult, JobFailure> {
    model_to_manifest_with_operations(request, control, &mut SystemProducer)
}

fn model_to_manifest_with_operations(
    request: ModelManifestRequest,
    control: &RunControl,
    operations: &mut impl ProducerOperations,
) -> Result<ModelManifestResult, JobFailure> {
    let attempt = control.begin()?;
    let prepare = || -> Result<_, JobError> {
        require_input(&request.input)?;
        let placement = ResolvedPlacement::resolve(&request.placement)?;
        let content_uri = binding::filename_uri(&request.input)?;
        let absolute_input = std::path::absolute(&request.input).map_err(|error| {
            JobError::io("resolve manifest source parent", &request.input, error)
        })?;
        let output = absolute_input
            .parent()
            .expect("absolute source parent")
            .join("tileset.json");
        crate::runtime::file_publication_supported()?;
        let snapshot = binding::load(&request.input, &output, || attempt.check())?;
        // Manifest output does not need a synthetic model member or alias-copy
        // byte ceiling; it references the source's admitted directory in place.
        let report = prepare_report(&snapshot, &placement, "manifest", 0, 0, &attempt)?;
        let bytes = manifest(&report, &content_uri)?;
        serde_json::to_vec(&report).map_err(|error| {
            invalid(
                JobErrorKind::InvalidState,
                format!("serialize manifest report: {error}"),
            )
        })?;
        Ok((snapshot.output, report, bytes))
    };
    let (output, report, bytes) = prepare().map_err(|error| attempt.fail(error))?;
    attempt
        .emit(&RunEvent::Progress {
            phase: "model_capture",
            done: report.captured_bytes,
            total: Some(report.captured_bytes),
        })
        .map_err(|error| attempt.fail(error))?;
    let mut staging = Staging::create(&output, &attempt)?;
    for chunk in bytes.chunks(64 * 1024) {
        if let Err(error) = attempt.check() {
            return Err(staging.fail(error));
        }
        if let Err(error) = operations.write(staging.writer(), chunk) {
            return Err(staging.fail(JobError::io(
                "write prepared model manifest",
                &output,
                error,
            )));
        }
    }
    if let Err(error) = attempt
        .emit(&RunEvent::Progress {
            phase: "ready_to_publish",
            done: bytes.len() as u64,
            total: Some(bytes.len() as u64),
        })
        .and_then(|()| attempt.close_events())
    {
        return Err(staging.fail(error));
    }
    let published = staging.seal()?.publish(&output, request.policy)?;
    Ok(ModelManifestResult {
        output: published.output,
        report,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct PartialWrite;
    impl ProducerOperations for PartialWrite {
        fn write(&mut self, file: &mut fs::File, bytes: &[u8]) -> std::io::Result<()> {
            file.write_all(&bytes[..bytes.len().min(7)])?;
            Err(std::io::Error::other(
                "persistent partial model writer failure",
            ))
        }
    }
    #[test]
    fn persistent_partial_writes_never_replace_either_completed_product() {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("source.glb");
        fs::write(&input, crate::fixtures::triangle_glb()).unwrap();
        let output = work.path().join("model.3tz");
        fs::write(&output, b"KEEP").unwrap();
        let failure = model_to_archive_with_operations(
            ModelWrapRequest::local_gltf(&input, &output).with_policy(OutputPolicy::Replace),
            &RunControl::default(),
            &mut PartialWrite,
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
        assert_eq!(fs::read(&output).unwrap(), b"KEEP");
        let manifest = work.path().join("tileset.json");
        fs::write(&manifest, b"KEEP").unwrap();
        let failure = model_to_manifest_with_operations(
            ModelManifestRequest::local_gltf(&input).with_policy(OutputPolicy::Replace),
            &RunControl::default(),
            &mut PartialWrite,
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
        assert_eq!(fs::read(manifest).unwrap(), b"KEEP");
        assert_eq!(fs::read_dir(work.path()).unwrap().count(), 3);
    }

    struct StageFailure(ProducerStage);
    impl ProducerOperations for StageFailure {
        fn stage(&mut self, stage: ProducerStage) -> Result<(), JobError> {
            if stage == self.0 {
                Err(JobError::new(
                    JobErrorKind::Io,
                    "injected required model stage failure",
                ))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn required_report_archive_and_cleanup_failures_have_truthful_owned_residuals() {
        for stage in [
            ProducerStage::WriteReport,
            ProducerStage::Archive,
            ProducerStage::Cleanup,
        ] {
            let work = tempfile::tempdir().unwrap();
            let input = work.path().join("source.glb");
            fs::write(&input, crate::fixtures::triangle_glb()).unwrap();
            let output = work.path().join("model.3tz");
            fs::write(&output, b"KEEP").unwrap();
            let failure = model_to_archive_with_operations(
                ModelWrapRequest::local_gltf(&input, &output).with_policy(OutputPolicy::Replace),
                &RunControl::default(),
                &mut StageFailure(stage),
            )
            .unwrap_err();
            assert_eq!(failure.error.kind(), JobErrorKind::Io);
            assert_eq!(fs::read(output).unwrap(), b"KEEP");
            assert_eq!(
                failure.retained_paths.len(),
                usize::from(stage == ProducerStage::Cleanup)
            );
            for path in failure.retained_paths {
                assert!(path.is_dir());
                fs::remove_dir_all(path).unwrap();
            }
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
        }
    }
}
