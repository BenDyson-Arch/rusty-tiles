//! One bounded local glTF mesh producer with explicit rigid placement under F0.
//! Broader legacy mesh conversion is a separate, unreviewed operation.
use crate::{
    archive3tz::{self, CodecError, WriteFailure},
    runtime::{Attempt, Staging},
    CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl, RunEvent,
};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::Write,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

mod approximation;
mod binding;
mod encode;
mod model;
mod partition;
mod placement;
mod source;

pub use model::{
    model_to_archive, model_to_manifest, ModelManifestRequest, ModelManifestResult, ModelReport,
    ModelWrapRequest, ModelWrapResult,
};
use placement::ResolvedPlacement;
pub use placement::{MeshPlacement, MeshPlacementReport};

/// Full-detail delivery or one bounded coarse root over unchanged leaves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MeshApproximation {
    #[default]
    FullDetail,
    RootProxy {
        triangle_limit: usize,
        max_error_metres: f64,
    },
}

/// Conservative local surface bound and performed adaptive proof work.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct MeshCertificateReport {
    pub error_metres: f64,
    pub patch_face_tests: u64,
    pub accepted_patches: u64,
    /// Deepest accepted proof patch across both directions and all regions.
    pub max_depth: u8,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeshApproximationReport {
    FullDetail,
    RootProxy {
        triangle_limit: u64,
        triangles: u64,
        regions: u64,
        certificate: MeshCertificateReport,
        geometric_error_metres: f64,
        appearance: &'static str,
    },
}

#[derive(Clone, Debug)]
pub struct MeshRequest {
    input: PathBuf,
    output: PathBuf,
    leaf_triangles: usize,
    policy: OutputPolicy,
    placement: MeshPlacement,
    approximation: MeshApproximation,
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
            placement: MeshPlacement::Local,
            approximation: MeshApproximation::FullDetail,
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
    pub fn with_approximation(mut self, approximation: MeshApproximation) -> Self {
        self.approximation = approximation;
        self
    }
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct MeshReport {
    pub schema_version: u64,
    pub profile: &'static str,
    pub source_coordinates: &'static str,
    pub coordinates: &'static str,
    pub placement: MeshPlacementReport,
    pub root_transform: [f64; 16],
    pub source_bytes: u64,
    pub external_files: u64,
    pub external_bytes: u64,
    pub triangles: u64,
    pub leaf_tiles: u64,
    pub leaf_triangles: u64,
    pub images: u64,
    pub image_bytes: u64,
    pub image_pixels: u64,
    pub routing_geometric_error_metres: f64,
    pub approximation: MeshApproximationReport,
}
#[derive(Debug)]
pub struct MeshResult {
    pub output: PathBuf,
    pub report: MeshReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
// Constructed once by core request validation. Lower stages consume these
// limits without reinterpreting raw request policy.
#[derive(Clone, Copy)]
struct RootProxyLimits {
    triangle_limit: NonZeroUsize,
    max_error_metres: f64,
}
impl RootProxyLimits {
    fn new(triangle_limit: usize, max_error_metres: f64) -> Result<Self, JobError> {
        let triangle_limit = NonZeroUsize::new(triangle_limit).ok_or_else(|| {
            invalid(
                JobErrorKind::InvalidRequest,
                "root proxy requires a positive triangle limit",
            )
        })?;
        if !max_error_metres.is_finite() || max_error_metres <= 0.0 {
            return Err(invalid(
                JobErrorKind::InvalidRequest,
                "root proxy requires finite positive maximum error metres",
            ));
        }
        Ok(Self {
            triangle_limit,
            max_error_metres,
        })
    }
    fn triangle_limit(self) -> usize {
        self.triangle_limit.get()
    }
}
enum ValidatedApproximation {
    FullDetail,
    RootProxy(RootProxyLimits),
}

struct PreparedMesh {
    output: PathBuf,
    policy: OutputPolicy,
    leaf_triangles: usize,
    placement: ResolvedPlacement,
    geometry: source::Geometry,
    source_bytes: u64,
    external_files: u64,
    external_bytes: u64,
    leaves: Vec<partition::Leaf>,
    bounds: partition::Bounds,
    images: Vec<usize>,
    root: PreparedRoot,
}
enum PreparedRoot {
    FullDetail,
    Proxy {
        geometry: approximation::ProxyGeometry,
        limits: RootProxyLimits,
    },
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
    Approximate,
    EncodeProxy,
    WriteProxy,
    EncodeLeaf,
    WriteImage,
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
fn validate(request: &MeshRequest) -> Result<ValidatedApproximation, JobError> {
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
    match request.approximation {
        MeshApproximation::FullDetail => Ok(ValidatedApproximation::FullDetail),
        MeshApproximation::RootProxy {
            triangle_limit,
            max_error_metres,
        } => RootProxyLimits::new(triangle_limit, max_error_metres)
            .map(ValidatedApproximation::RootProxy),
    }
}

fn prepare(
    request: MeshRequest,
    attempt: &Attempt,
    operations: &mut impl ProducerOperations,
) -> Result<PreparedMesh, JobError> {
    attempt.check()?;
    let approximation = validate(&request)?;
    let MeshRequest {
        input,
        output,
        leaf_triangles,
        policy,
        placement,
        approximation: _,
    } = request;
    let placement = ResolvedPlacement::resolve(&placement)?;
    crate::runtime::file_publication_supported()?;
    let snapshot = binding::load(&input, &output, || attempt.check())?;
    let output = snapshot.output.clone();
    operations.stage(ProducerStage::Decode)?;
    let geometry = source::decode(
        &snapshot.document,
        &snapshot.buffers(),
        &snapshot.images(),
        || attempt.check(),
    )?;
    let source_bytes = snapshot.source_bytes;
    let external_files = snapshot.external_files;
    let external_bytes = snapshot.external_bytes;
    drop(snapshot);
    let (leaves, bounds) = partition::plan(
        &geometry.triangles,
        leaf_triangles,
        source::MAX_LEAVES,
        || attempt.check(),
    )?;
    let root = match approximation {
        ValidatedApproximation::FullDetail => PreparedRoot::FullDetail,
        ValidatedApproximation::RootProxy(limits) => {
            operations.stage(ProducerStage::Approximate)?;
            attempt.emit(&RunEvent::Progress {
                phase: "mesh_approximation",
                done: 0,
                total: Some(1),
            })?;
            let proxy = approximation::prepare(&geometry, limits, || attempt.check())?;
            attempt.emit(&RunEvent::Progress {
                phase: "mesh_approximation",
                done: 1,
                total: Some(1),
            })?;
            PreparedRoot::Proxy {
                geometry: proxy,
                limits,
            }
        }
    };
    let proxy = match &root {
        PreparedRoot::FullDetail => None,
        PreparedRoot::Proxy { geometry, .. } => Some(geometry),
    };
    encode::validate_identity_budget(&geometry, &leaves, proxy, || attempt.check())?;
    let images = encode::used_images(&geometry).map_err(|e| {
        invalid(
            JobErrorKind::InvalidState,
            format!("prepare mesh image closure: {e}"),
        )
    })?;
    Ok(PreparedMesh {
        output,
        policy,
        leaf_triangles,
        placement,
        geometry,
        source_bytes,
        external_files,
        external_bytes,
        leaves,
        bounds,
        images,
        root,
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
    let mut members = Vec::with_capacity(prepared.leaves.len() + prepared.images.len() + 2);
    if !prepared.images.is_empty() {
        let directory = workspace.path().join("textures");
        fs::create_dir(&directory)
            .map_err(|e| JobError::io("create mesh texture directory", &directory, e))?;
        for &id in &prepared.images {
            attempt.check()?;
            let image = &prepared.geometry.images[id];
            operations.stage(ProducerStage::WriteImage)?;
            members.push(write_member(
                workspace,
                &encode::image_name(id, image),
                &image.bytes,
                attempt,
                operations,
            )?);
        }
    }
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
        let name = encode::leaf_name(id);
        operations.stage(ProducerStage::WriteLeaf)?;
        members.push(write_member(workspace, &name, &bytes, attempt, operations)?);
        drop(bytes);
        attempt.emit(&RunEvent::Progress {
            phase: "mesh_leaves",
            done: (id + 1) as u64,
            total: Some(prepared.leaves.len() as u64),
        })?;
    }
    let omission_error = prepared.bounds.diagonal().max(1.0);
    let (root_content, approximation, routing_error) = match &prepared.root {
        PreparedRoot::FullDetail => (
            encode::RootContent::Routing {
                omission_error_metres: omission_error,
            },
            MeshApproximationReport::FullDetail,
            omission_error,
        ),
        PreparedRoot::Proxy { geometry, limits } => {
            operations.stage(ProducerStage::EncodeProxy)?;
            let bytes = encode::proxy(&prepared.geometry, geometry, || attempt.check())?;
            operations.stage(ProducerStage::WriteProxy)?;
            members.push(write_member(
                workspace,
                encode::ROOT_PROXY_NAME,
                &bytes,
                attempt,
                operations,
            )?);
            (
                encode::RootContent::Proxy {
                    geometric_error_metres: limits.max_error_metres,
                },
                MeshApproximationReport::RootProxy {
                    triangle_limit: limits.triangle_limit() as u64,
                    triangles: geometry.triangles.len() as u64,
                    regions: geometry.regions.len() as u64,
                    certificate: MeshCertificateReport {
                        error_metres: geometry.certificate.error_metres,
                        patch_face_tests: geometry.certificate.patch_face_tests,
                        accepted_patches: geometry.certificate.accepted_patches,
                        max_depth: geometry.certificate.max_depth,
                    },
                    geometric_error_metres: limits.max_error_metres,
                    appearance: "opaque-untextured-factors",
                },
                omission_error.max(limits.max_error_metres),
            )
        }
    };
    let manifest = encode::tileset(
        &prepared.leaves,
        prepared.bounds,
        prepared.placement.transform(),
        root_content,
        routing_error,
        || attempt.check(),
    )
    .map_err(|e| match e {
        encode::EncodeError::Checkpoint(error) => error,
        other => invalid(
            JobErrorKind::InvalidState,
            format!("encode validated mesh manifest: {other}"),
        ),
    })?;
    members.push(write_member(
        workspace,
        "tileset.json",
        &manifest,
        attempt,
        operations,
    )?);
    let report = MeshReport {
        schema_version: 7,
        profile: "f1d2-adaptive-root-proxy-gltf-v1",
        source_coordinates: "local-gltf",
        coordinates: prepared.placement.coordinates(),
        placement: prepared.placement.report(),
        root_transform: prepared.placement.transform(),
        source_bytes: prepared.source_bytes,
        external_files: prepared.external_files,
        external_bytes: prepared.external_bytes,
        triangles: prepared.geometry.triangles.len() as u64,
        leaf_tiles: prepared.leaves.len() as u64,
        leaf_triangles: prepared.leaf_triangles as u64,
        images: prepared.images.len() as u64,
        image_bytes: prepared
            .images
            .iter()
            .map(|&id| prepared.geometry.images[id].bytes.len() as u64)
            .sum(),
        image_pixels: prepared
            .images
            .iter()
            .map(|&id| {
                let image = &prepared.geometry.images[id];
                u64::from(image.width) * u64::from(image.height)
            })
            .sum(),
        routing_geometric_error_metres: routing_error,
        approximation,
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
    let workspace = Workspace::create(&prepared.output).map_err(|e| attempt.fail(e))?;
    let (members, report) = match produce(&prepared, &workspace, &attempt, operations) {
        Ok(value) => value,
        Err(e) => return Err(cleanup_failure(attempt.fail(e), workspace)),
    };
    let completed = CompletedMesh {
        workspace,
        members,
        report,
    };
    let mut staging = match Staging::create(&prepared.output, &attempt) {
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
            WriteFailure::Codec(e) => codec_error(e, &prepared.output),
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
    let published = staging.seal()?.publish(&prepared.output, prepared.policy)?;
    Ok(MeshResult {
        output: published.output,
        report: completed.report,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
        let position_bytes = bin.len();
        for _ in 0..2 {
            for uv in [[0_f32, 0.], [1., 0.], [0., 1.]] {
                for value in uv {
                    bin.extend(value.to_le_bytes());
                }
            }
        }
        let image_offset = bin.len();
        // This fixture exercises writer/lifecycle faults, not image fidelity.
        // The independently authored PNG/texel oracle owns that evidence.
        let mut png = Vec::new();
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(&mut png),
            &[255, 0, 0, 255],
            1,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        bin.extend(&png);
        let document = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"material":0}]}],"buffers":[{"byteLength":bin.len()}],
            "bufferViews":[{"buffer":0,"byteLength":position_bytes},{"buffer":0,"byteOffset":position_bytes,"byteLength":image_offset-position_bytes},
                {"buffer":0,"byteOffset":image_offset,"byteLength":png.len()}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":6,"type":"VEC3","min":[0,0,0],"max":[3,1,0]},
                {"bufferView":1,"componentType":5126,"count":6,"type":"VEC2"}],
            "images":[{"bufferView":2,"mimeType":"image/png"}],"textures":[{"source":0}],
            "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}]});
        fixture_glb(document, bin)
    }
    fn proxy_fixture() -> Vec<u8> {
        let mut bin = Vec::new();
        for y in 0..=4 {
            for x in 0..=4 {
                for v in [x as f32, y as f32, 0.0] {
                    bin.extend(v.to_le_bytes());
                }
            }
        }
        let position_bytes = bin.len();
        for y in 0..4 {
            for x in 0..4 {
                let a = (y * 5 + x) as u32;
                for i in [a, a + 1, a + 5, a + 1, a + 6, a + 5] {
                    bin.extend(i.to_le_bytes());
                }
            }
        }
        let document = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],"buffers":[{"byteLength":bin.len()}],
            "bufferViews":[{"buffer":0,"byteLength":position_bytes},{"buffer":0,"byteOffset":position_bytes,"byteLength":bin.len()-position_bytes}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":25,"type":"VEC3","min":[0,0,0],"max":[4,4,0]},
                {"bufferView":1,"componentType":5125,"count":96,"type":"SCALAR"}]});
        fixture_glb(document, bin)
    }
    fn fixture_glb(document: serde_json::Value, mut bin: Vec<u8>) -> Vec<u8> {
        let mut json = serde_json::to_vec(&document).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
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
            ProducerStage::WriteImage,
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
        for target in [
            ProducerStage::WriteImage,
            ProducerStage::WriteLeaf,
            ProducerStage::WriteReport,
        ] {
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
    #[test]
    fn proxy_failures_preserve_replace_target_and_cleanup_owned_candidates() {
        for target in [
            ProducerStage::Approximate,
            ProducerStage::EncodeProxy,
            ProducerStage::WriteProxy,
        ] {
            let (work, input, output) = setup();
            fs::write(&input, proxy_fixture()).unwrap();
            let mut operations = FailStage {
                target,
                reached: false,
            };
            let control = RunControl::default();
            let failure = mesh_to_archive_with_operations(
                MeshRequest::local_gltf(&input, &output, 8)
                    .with_policy(OutputPolicy::Replace)
                    .with_approximation(MeshApproximation::RootProxy {
                        triangle_limit: 8,
                        max_error_metres: 10.0,
                    }),
                &control,
                &mut operations,
            )
            .unwrap_err();
            assert!(operations.reached);
            assert_eq!(failure.error.kind(), JobErrorKind::Io);
            assert_eq!(fs::read(&output).unwrap(), b"previous");
            assert!(failure.retained_paths.is_empty());
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
            assert!(!control.cancellation_handle().cancel());
        }
        let (work, input, output) = setup();
        fs::write(&input, proxy_fixture()).unwrap();
        let mut operations = PartialWrite {
            target: ProducerStage::WriteProxy,
            current: ProducerStage::Decode,
            reached: false,
        };
        let control = RunControl::default();
        let failure = mesh_to_archive_with_operations(
            MeshRequest::local_gltf(&input, &output, 8)
                .with_policy(OutputPolicy::Replace)
                .with_approximation(MeshApproximation::RootProxy {
                    triangle_limit: 8,
                    max_error_metres: 10.0,
                }),
            &control,
            &mut operations,
        )
        .unwrap_err();
        assert!(operations.reached);
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
        assert_eq!(fs::read(&output).unwrap(), b"previous");
        assert!(failure.retained_paths.is_empty());
        assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
    }
}
