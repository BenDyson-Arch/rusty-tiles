//! Bounded LAS/LAZ producer under the F0 file lifecycle.
use crate::archive3tz::{self, CodecError, WriteFailure};
use crate::runtime::{Attempt, Staging};
use crate::{crs, Error};
use crate::{
    CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl, RunEvent,
};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};
mod source;
mod tiles;
use source::{header_crs, las_error, read_source, Layout, RAW};

#[derive(Clone, Debug)]
pub enum PointCloudCrs {
    Header,
    Definition(String),
}
#[derive(Clone, Debug)]
pub enum PointCloudCoordinates {
    LocalMetres,
    Horizontal {
        source: PointCloudCrs,
        height_offset_metres: f64,
    },
}
#[derive(Clone, Debug)]
pub struct PointCloudOptions {
    pub explicit: bool,
    pub metadata_attributes: bool,
    pub max_points: usize,
    pub chunk_points: usize,
}
impl Default for PointCloudOptions {
    fn default() -> Self {
        Self {
            explicit: false,
            metadata_attributes: false,
            max_points: 50_000,
            chunk_points: 100_000,
        }
    }
}
#[derive(Clone, Debug)]
pub struct PointCloudRequest {
    input: PathBuf,
    output: PathBuf,
    coordinates: PointCloudCoordinates,
    options: PointCloudOptions,
    policy: OutputPolicy,
}
impl PointCloudRequest {
    pub fn new(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        coordinates: PointCloudCoordinates,
        options: PointCloudOptions,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            coordinates,
            options,
            policy: OutputPolicy::CreateNew,
        }
    }
    pub fn with_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointCloudReport {
    pub points: u64,
    pub tiles: usize,
    pub source_crs: String,
    pub resolved_crs: Option<String>,
    pub source_scales: [f64; 3],
    pub source_offsets: [f64; 3],
    pub height_offset: Option<f64>,
    pub properties: Vec<String>,
    pub max_position_rounding_metres: f64,
    pub sampling: String,
    pub max_points: usize,
    pub chunk_points: usize,
    pub encoder: String,
    pub tiling: String,
    pub coordinate_profile: String,
    pub scaled_extra_bytes_tolerance: String,
}
#[derive(Debug)]
pub struct PointCloudResult {
    pub output: PathBuf,
    pub report: PointCloudReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
struct PreparedPointCloud {
    request: PointCloudRequest,
    reader: las::Reader,
    header: las::Header,
    layout: Layout,
    coordinates: Coordinates,
    resolved: Option<String>,
    scales: [f64; 3],
    offsets: [f64; 3],
    expected: u64,
}
struct CompletedPointCloud {
    members: Vec<String>,
    report: PointCloudReport,
}
fn checkpoint(attempt: &Attempt) -> Result<(), Error> {
    attempt.check().map_err(|e| Error::Job(attempt.fail(e)))
}
fn progress(attempt: &Attempt, phase: &str, done: u64, total: u64) -> Result<(), Error> {
    attempt
        .emit(&RunEvent::Progress {
            phase,
            done,
            total: Some(total),
        })
        .map_err(|e| Error::Job(attempt.fail(e)))
}
fn job_error(error: Error) -> JobError {
    match error {
        Error::Job(failure) => failure.error,
        Error::Io(e) => JobError::io("point-cloud producer", Path::new("."), e),
        Error::Environment(v) => JobError::new(JobErrorKind::Unsupported, v),
        Error::Message(v) => JobError::new(JobErrorKind::InvalidRequest, v),
        Error::Json(e) if e.is_io() => JobError::io(
            "point-cloud JSON transport",
            Path::new("."),
            std::io::Error::other(e),
        ),
        other => JobError::new(JobErrorKind::InvalidInput, other.to_string()),
    }
}
fn prepare(
    mut request: PointCloudRequest,
    attempt: &Attempt,
) -> Result<PreparedPointCloud, JobError> {
    attempt.check()?;
    if request.options.max_points == 0 || request.options.chunk_points == 0 {
        return Err(JobError::new(
            JobErrorKind::InvalidRequest,
            "maxPoints and chunkPoints must be at least 1",
        ));
    }
    if request.output.extension().is_none_or(|e| e != "3tz")
        && !request
            .output
            .file_name()
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.ends_with(".3dtiles.zip"))
    {
        return Err(JobError::new(
            JobErrorKind::InvalidRequest,
            "point-cloud output must use .3tz or .3dtiles.zip",
        ));
    }
    let output = crate::output_path::resolve(&request.output)?;
    let existing = match fs::symlink_metadata(&output) {
        Ok(m) => Some(m),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(JobError::io("inspect point-cloud output", &output, e)),
    };
    if existing.as_ref().is_some_and(|m| !m.is_file()) {
        return Err(JobError::new(
            JobErrorKind::InvalidRequest,
            "point-cloud output must be a regular file without a symlink leaf",
        ));
    }
    let metadata = fs::symlink_metadata(&request.input)
        .map_err(|e| JobError::io("inspect point-cloud source", &request.input, e))?;
    if !metadata.is_file() {
        return Err(JobError::new(
            JobErrorKind::InvalidInput,
            "point-cloud source must be a regular file without a symlink leaf",
        ));
    }
    request.input = fs::canonicalize(&request.input)
        .map_err(|e| JobError::io("resolve point-cloud source", &request.input, e))?;
    if request.input == output
        || (existing.is_some()
            && same_file::is_same_file(&request.input, &output).map_err(|e| {
                JobError::io("compare point-cloud source/output identity", &output, e)
            })?)
    {
        return Err(JobError::new(
            JobErrorKind::InvalidRequest,
            "point-cloud output aliases source",
        ));
    }
    if existing.is_some() && request.policy == OutputPolicy::CreateNew {
        return Err(JobError::new(
            JobErrorKind::Conflict,
            "point-cloud output already exists",
        ));
    }
    request.output = output;
    let admitted = same_file::Handle::from_path(&request.input)
        .map_err(|e| JobError::io("capture point-cloud source identity", &request.input, e))?;
    let (reader, header) = read_source(&request.input, &admitted).map_err(job_error)?;
    attempt.check()?;
    let layout = Layout::new(&header).map_err(job_error)?;
    if request.options.metadata_attributes {
        for name in [
            "vertex_classification",
            "vertex_intensity",
            "vertex_return_number",
        ] {
            if layout.dimensions.iter().any(|d| d.name == name) {
                return Err(JobError::new(
                    JobErrorKind::InvalidInput,
                    format!("LAS dimension {name:?} conflicts with metadata attribute property"),
                ));
            }
        }
    }
    let (coordinates, resolved) =
        Coordinates::new(&header, &request.coordinates).map_err(job_error)?;
    let transforms = header.transforms();
    let scales = [transforms.x.scale, transforms.y.scale, transforms.z.scale];
    let offsets = [
        transforms.x.offset,
        transforms.y.offset,
        transforms.z.offset,
    ];
    if !scales.iter().all(|v| v.is_finite() && *v > 0.) || !offsets.iter().all(|v| v.is_finite()) {
        return Err(JobError::new(
            JobErrorKind::InvalidInput,
            "LAS scales must be positive finite and offsets finite",
        ));
    }
    let expected = header.number_of_points();
    if expected == 0 {
        return Err(JobError::new(
            JobErrorKind::InvalidInput,
            "empty point cloud",
        ));
    }
    for budget in [request.options.chunk_points, request.options.max_points] {
        let count = (budget as u64).min(expected);
        if usize::try_from(count)
            .ok()
            .and_then(|n| n.checked_mul(layout.record_len))
            .is_none_or(|n| n > isize::MAX as usize)
        {
            return Err(JobError::new(
                JobErrorKind::InvalidRequest,
                "point budget exceeds platform limits",
            ));
        }
    }
    attempt.check()?;
    Ok(PreparedPointCloud {
        request,
        reader,
        header,
        layout,
        coordinates,
        resolved,
        scales,
        offsets,
        expected,
    })
}
struct Workspace(tempfile::TempDir);
impl Workspace {
    fn create(output: &Path) -> Result<Self, JobError> {
        let parent = output.parent().expect("resolved output parent");
        fs::create_dir_all(parent)
            .map_err(|e| JobError::io("create point-cloud output parent", parent, e))?;
        tempfile::Builder::new()
            .prefix(".point-work-")
            .tempdir_in(parent)
            .map(Self)
            .map_err(|e| JobError::io("create point-cloud workspace", parent, e))
    }
    fn cleanup(self) -> Result<(), CleanupDiagnostic> {
        self.cleanup_with(&mut SystemIo)
    }
    fn cleanup_with(self, operations: &mut impl ProducerIo) -> Result<(), CleanupDiagnostic> {
        let path = self.0.keep();
        operations.cleanup(&path).map_err(|e| CleanupDiagnostic {
            error: JobError::io("remove point-cloud workspace", &path, e),
            path,
        })
    }
}
fn normalize_failure(attempt: &Attempt, failure: JobFailure) -> JobFailure {
    let mut normalized = attempt.fail(failure.error.clone());
    for cause in std::iter::once(failure.error).chain(failure.secondary) {
        if !normalized.error.same_cause(&cause)
            && !normalized
                .secondary
                .iter()
                .any(|prior| prior.same_cause(&cause))
        {
            normalized.secondary.push(cause);
        }
    }
    normalized.retained_paths = failure.retained_paths;
    normalized.recovery = failure.recovery;
    normalized
}
fn cleanup_failure(mut failure: JobFailure, workspace: Workspace) -> JobFailure {
    if let Err(d) = workspace.cleanup() {
        failure.retained_paths.push(d.path);
        failure.secondary.push(d.error);
    }
    failure
}
fn codec_error(error: CodecError, output: &Path) -> JobError {
    match error {
        CodecError::Io(e) => JobError::io("serialize point-cloud archive", output, e),
        CodecError::SourceIo { path, source } => {
            JobError::io("read accepted point-cloud member", &path, source)
        }
        CodecError::Zip(e) => JobError::io(
            "serialize point-cloud archive",
            output,
            std::io::Error::other(e),
        ),
        other => JobError::new(JobErrorKind::InvalidState, other.to_string()),
    }
}
// Private I/O seam: exercises physical producer writes and archive serialization faults.
trait ProducerIo {
    fn cleanup(&mut self, path: &Path) -> std::io::Result<()> {
        fs::remove_dir_all(path)
    }
    fn spool(&mut self, path: &Path) -> Result<Box<dyn Write>, Error> {
        Ok(Box::new(BufWriter::new(File::create(path)?)))
    }
    fn member(&mut self, path: &Path, bytes: &[u8]) -> Result<(), Error> {
        fs::write(path, bytes)?;
        Ok(())
    }
    fn archive(
        &mut self,
        members: &[archive3tz::Member<'_>],
        writer: &mut File,
        checkpoint: &mut impl FnMut(u64) -> Result<(), JobError>,
    ) -> Result<(), WriteFailure<JobError>> {
        archive3tz::serialize(members, writer, checkpoint)
    }
}
struct SystemIo;
impl ProducerIo for SystemIo {}
pub fn point_cloud_to_archive(
    request: PointCloudRequest,
    control: &RunControl,
) -> Result<PointCloudResult, JobFailure> {
    point_cloud_with_io(request, control, &mut SystemIo)
}
fn point_cloud_with_io(
    request: PointCloudRequest,
    control: &RunControl,
    operations: &mut impl ProducerIo,
) -> Result<PointCloudResult, JobFailure> {
    let attempt = control.begin()?;
    let prepared = prepare(request, &attempt).map_err(|e| attempt.fail(e))?;
    let output = prepared.request.output.clone();
    let policy = prepared.request.policy;
    let workspace = Workspace::create(&output).map_err(|e| attempt.fail(e))?;
    let completed = match convert(prepared, workspace.0.path(), &attempt, operations) {
        Ok(v) => v,
        Err(Error::Job(e)) => {
            return Err(cleanup_failure(normalize_failure(&attempt, e), workspace))
        }
        Err(e) => return Err(cleanup_failure(attempt.fail(job_error(e)), workspace)),
    };
    let paths: Vec<_> = completed
        .members
        .iter()
        .map(|name| workspace.0.path().join(name))
        .collect();
    let selected = match completed
        .members
        .iter()
        .zip(&paths)
        .map(|(name, path)| {
            let m = fs::symlink_metadata(path)
                .map_err(|e| JobError::io("inspect point-cloud member", path, e))?;
            if !m.is_file() {
                return Err(JobError::new(
                    JobErrorKind::InvalidState,
                    "accepted point-cloud member must be regular file",
                ));
            }
            Ok(archive3tz::Member {
                name,
                source: path,
                size: m.len(),
                modified: m.modified().ok(),
            })
        })
        .collect::<Result<Vec<_>, JobError>>()
    {
        Ok(v) => v,
        Err(e) => return Err(cleanup_failure(attempt.fail(e), workspace)),
    };
    let mut staging = match Staging::create(&output, &attempt) {
        Ok(v) => v,
        Err(e) => return Err(cleanup_failure(e, workspace)),
    };
    let total = selected.iter().map(|m| m.size).sum();
    if let Err(e) = operations.archive(&selected, staging.writer(), &mut |done| {
        attempt.check()?;
        attempt.emit(&RunEvent::Progress {
            phase: "point_archive",
            done,
            total: Some(total),
        })
    }) {
        let e = match e {
            WriteFailure::Checkpoint(e) => e,
            WriteFailure::Codec(e) => codec_error(e, &output),
        };
        return Err(cleanup_failure(staging.fail(e), workspace));
    }
    if let Err(d) = workspace.cleanup_with(operations) {
        let mut failure = staging.fail(d.error);
        failure.retained_paths.push(d.path);
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
    let published = staging.seal()?.publish(&output, policy)?;
    Ok(PointCloudResult {
        output: published.output,
        report: completed.report,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}
enum Coordinates {
    Local,
    Ecef(crs::Transform),
}

impl Coordinates {
    fn new(
        header: &las::Header,
        coordinates: &PointCloudCoordinates,
    ) -> Result<(Self, Option<String>), Error> {
        let (source, height) = match coordinates {
            PointCloudCoordinates::LocalMetres => return Ok((Self::Local, None)),
            PointCloudCoordinates::Horizontal {
                source,
                height_offset_metres,
            } => (source, *height_offset_metres),
        };
        if !height.is_finite() {
            return Err(Error::msg("height offset must be finite"));
        }
        let definition = match source {
            PointCloudCrs::Header => header_crs(header)?,
            PointCloudCrs::Definition(value) => value.clone(),
        };
        let transform = crs::Transform::new(&definition, height)?;
        Ok((Self::Ecef(transform), Some(definition)))
    }

    fn transform<'p>(
        &mut self,
        positions: &'p [[f64; 3]],
    ) -> Result<std::borrow::Cow<'p, [[f64; 3]]>, Error> {
        match self {
            Self::Local => Ok(positions.into()),
            Self::Ecef(transform) => transform.transform(positions).map(Into::into),
        }
    }
}

fn convert(
    prepared: PreparedPointCloud,
    output: &Path,
    attempt: &Attempt,
    operations: &mut impl ProducerIo,
) -> Result<CompletedPointCloud, Error> {
    std::fs::create_dir_all(output.join("scratch"))?;
    std::fs::create_dir(output.join("t"))?;
    let path = output.join("scratch/source.bin");
    let PreparedPointCloud {
        request,
        mut reader,
        header,
        layout,
        mut coordinates,
        resolved,
        scales,
        offsets,
        expected,
    } = prepared;
    let options = &request.options;
    let (origin, count) = {
        let mut points = las::PointDataBuilder::new().for_header(&header).build();
        let mut origin = None;
        let mut count = 0_u64;
        let mut file = operations.spool(&path)?;
        progress(attempt, "ingestion", 0, expected)?;
        loop {
            checkpoint(attempt)?;
            let n = reader
                .fill_points(options.chunk_points as u64, &mut points)
                .map_err(las_error)?;
            if n == 0 {
                break;
            }
            let raw_len = layout.record_len - RAW;
            let mut xyz = Vec::with_capacity(points.len());
            for raw in points.raw_bytes().chunks_exact(raw_len) {
                layout.validate(raw)?;
                let source: [f64; 3] = std::array::from_fn(|i| {
                    i32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap()) as f64 * scales[i]
                        + offsets[i]
                });
                if !source.iter().all(|v| v.is_finite()) {
                    return Err(Error::Data("nonfinite source coordinates".into()));
                }
                xyz.push(source);
            }
            checkpoint(attempt)?;
            let projected = coordinates.transform(&xyz)?;
            let origin = *origin.get_or_insert(projected[0]);
            for ((raw, source), point) in points
                .raw_bytes()
                .chunks_exact(raw_len)
                .zip(&xyz)
                .zip(projected.iter())
            {
                for i in 0..3 {
                    let relative = point[i] - origin[i];
                    if !relative.is_finite() {
                        return Err(Error::Data("nonfinite projected coordinates".into()));
                    }
                    file.write_all(&relative.to_le_bytes())?;
                }
                file.write_all(&count.to_le_bytes())?;
                for value in source {
                    file.write_all(&value.to_le_bytes())?;
                }
                file.write_all(raw)?;
                count += 1;
            }
            progress(attempt, "ingestion", count, expected)?;
        }
        file.flush()?;
        if count != expected {
            return Err(Error::Data("LAS point count differs from header".into()));
        }
        (
            origin.ok_or_else(|| Error::Data("empty point cloud".into()))?,
            count,
        )
    };
    drop(reader);
    let mut tree = tiles::Tree {
        attempt,
        members: Vec::new(),
        operations,
        layout: &layout,
        options,
        output,
        tiles: 0,
        max_rounding: 0.,
        total_points: count,
        leaf_points: 0,
    };
    progress(attempt, "tiling", 0, count)?;
    let mut root = tree.build(&path, [0.; 3], 0, None)?;
    for i in 0..3 {
        let value = root["transform"][12 + i].as_f64().unwrap() + origin[i];
        if !value.is_finite() {
            return Err(Error::Data("nonfinite tileset origin".into()));
        }
        root["transform"][12 + i] = value.into();
    }
    std::fs::remove_dir(output.join("scratch"))?;
    let error = tiles::tileset_error(&root);
    if !error.is_finite() {
        return Err(Error::Data(
            "tileset extent exceeds finite coordinate range".into(),
        ));
    }
    let mut manifest =
        serde_json::json!({"asset":{"version":"1.1"},"geometricError":error,"root":root});
    if !options.explicit {
        tree.members = crate::implicit::write_tileset_recorded(
            &mut manifest,
            output,
            crate::implicit::SubdivisionScheme::Octree,
            false,
            &mut || checkpoint(attempt),
        )?;
    }
    tree.operations.member(
        &output.join("tileset.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    checkpoint(attempt)?;
    let (source_crs, height_offset) = match &request.coordinates {
        PointCloudCoordinates::LocalMetres => ("local".to_owned(), None),
        PointCloudCoordinates::Horizontal {
            source,
            height_offset_metres,
        } => (
            match source {
                PointCloudCrs::Header => "header".to_owned(),
                PointCloudCrs::Definition(v) => v.clone(),
            },
            Some(*height_offset_metres),
        ),
    };
    let report = PointCloudReport {
        points: count, tiles: tree.tiles, source_crs, resolved_crs: resolved,
        source_scales: scales, source_offsets: offsets, height_offset,
        properties: layout.dimensions.iter().map(|d| d.name.clone()).collect(),
        max_position_rounding_metres: tree.max_rounding,
        sampling: "first source point per voxel; cell diagonal bounds source-to-sample distance".into(),
        max_points: options.max_points, chunk_points: options.chunk_points,
        encoder: if options.explicit { "rusty-tiles-native-las-v1" } else { "rusty-tiles-native-las-implicit-v2" }.into(),
        tiling: if options.explicit { "explicit" } else { "implicit" }.into(),
        coordinate_profile: "local metres or admitted strict horizontal CRS subset; explicit ellipsoidal metre offset".into(),
        scaled_extra_bytes_tolerance: "FLOAT64 multiply then add; absolute error <= 2*EPSILON*(abs(raw*scale)+abs(offset))+2*MIN_POSITIVE; scaled INT64/UINT64 unsupported".into(),
    };
    let serialized = serde_json::to_vec_pretty(&report)?;
    tree.operations
        .member(&output.join("conversion.json"), &serialized)?;
    tree.members.push("tileset.json".into());
    tree.members.push("conversion.json".into());
    tree.members.sort();
    Ok(CompletedPointCloud {
        members: tree.members,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Seek, SeekFrom};

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("source.las");
        let output = work.path().join("cloud.3tz");
        let mut writer = las::Writer::from_path(&input, las::Header::default()).unwrap();
        for x in [0., 1., 2., 3.] {
            writer
                .write_point(las::Point {
                    x,
                    y: x,
                    z: x,
                    ..Default::default()
                })
                .unwrap();
        }
        writer.close().unwrap();
        fs::write(&output, b"previous archive").unwrap();
        (work, input, output)
    }
    fn request(input: &Path, output: &Path) -> PointCloudRequest {
        PointCloudRequest::new(
            input,
            output,
            PointCloudCoordinates::LocalMetres,
            PointCloudOptions {
                explicit: true,
                max_points: 2,
                chunk_points: 2,
                ..Default::default()
            },
        )
        .with_policy(OutputPolicy::Replace)
    }
    fn assert_clean(work: &Path, input: &Path, output: &Path, failure: &JobFailure) {
        assert_eq!(failure.error.kind(), JobErrorKind::Io, "{failure:?}");
        assert!(failure.retained_paths.is_empty(), "{failure:?}");
        assert_eq!(fs::read(output).unwrap(), b"previous archive");
        let names: Vec<_> = fs::read_dir(work)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(names.len(), 2, "unexpected candidates {names:?}");
        assert!(input.exists());
    }
    struct PartialSpool {
        file: File,
        fired: bool,
    }
    impl Write for PartialSpool {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fired {
                return Err(io::Error::other("persistent spool fault"));
            }
            self.fired = true;
            self.file.write_all(&bytes[..bytes.len().min(3)])?;
            Err(io::Error::other("partial spool fault"))
        }
        fn flush(&mut self) -> io::Result<()> {
            self.file.flush()
        }
    }
    struct SpoolFault;
    impl ProducerIo for SpoolFault {
        fn spool(&mut self, path: &Path) -> Result<Box<dyn Write>, Error> {
            Ok(Box::new(PartialSpool {
                file: File::create(path)?,
                fired: false,
            }))
        }
    }
    #[test]
    fn physical_partial_spool_failure_preserves_previous_and_cleans_workspace() {
        let (work, input, output) = setup();
        let failure = point_cloud_with_io(
            request(&input, &output),
            &RunControl::default(),
            &mut SpoolFault,
        )
        .unwrap_err();
        assert_clean(work.path(), &input, &output, &failure);
        assert!(failure.error.message().contains("partial spool fault"));
    }
    struct MemberFault {
        target: &'static str,
        reached: bool,
    }
    impl ProducerIo for MemberFault {
        fn member(&mut self, path: &Path, bytes: &[u8]) -> Result<(), Error> {
            if path.to_string_lossy().ends_with(self.target) {
                self.reached = true;
                let mut file = File::create(path)?;
                file.write_all(&bytes[..bytes.len().min(17)])?;
                return Err(io::Error::other("physical partial generated member fault").into());
            }
            fs::write(path, bytes)?;
            Ok(())
        }
    }
    #[test]
    fn physical_partial_leaf_manifest_and_report_failures_clean_every_candidate() {
        for target in ["0.glb", "tileset.json", "conversion.json"] {
            let (work, input, output) = setup();
            let mut fault = MemberFault {
                target,
                reached: false,
            };
            let failure =
                point_cloud_with_io(request(&input, &output), &RunControl::default(), &mut fault)
                    .unwrap_err();
            assert!(fault.reached);
            assert_clean(work.path(), &input, &output, &failure);
        }
    }
    struct ArchiveFault {
        fault: archive3tz::test_faults::Fault,
        reached: bool,
    }
    impl ProducerIo for ArchiveFault {
        fn archive(
            &mut self,
            members: &[archive3tz::Member<'_>],
            writer: &mut File,
            checkpoint: &mut impl FnMut(u64) -> Result<(), JobError>,
        ) -> Result<(), WriteFailure<JobError>> {
            let mut fault = archive3tz::test_faults::FaultWriter::new(writer, self.fault);
            let result = archive3tz::serialize(members, &mut fault, checkpoint);
            self.reached = fault.fired;
            result
        }
    }
    #[test]
    fn physical_archive_finalization_and_flush_failures_clean_both_candidates() {
        use archive3tz::test_faults::Fault;
        for target in [Fault::Finalize, Fault::Flush] {
            let (work, input, output) = setup();
            let mut fault = ArchiveFault {
                fault: target,
                reached: false,
            };
            let failure =
                point_cloud_with_io(request(&input, &output), &RunControl::default(), &mut fault)
                    .unwrap_err();
            assert!(fault.reached);
            assert_clean(work.path(), &input, &output, &failure);
        }
    }
    struct PartialArchive<'a> {
        file: &'a mut File,
        fired: bool,
    }
    impl Write for PartialArchive<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fired {
                return Err(io::Error::other("persistent archive write fault"));
            }
            if bytes.starts_with(b"glTF") {
                self.file.write_all(&bytes[..bytes.len().min(17)])?;
                self.fired = true;
                return Err(io::Error::other("partial archive payload fault"));
            }
            self.file.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.file.flush()
        }
    }
    impl Seek for PartialArchive<'_> {
        fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
            self.file.seek(p)
        }
    }
    struct PayloadFault {
        reached: bool,
    }
    impl ProducerIo for PayloadFault {
        fn archive(
            &mut self,
            members: &[archive3tz::Member<'_>],
            writer: &mut File,
            checkpoint: &mut impl FnMut(u64) -> Result<(), JobError>,
        ) -> Result<(), WriteFailure<JobError>> {
            let mut fault = PartialArchive {
                file: writer,
                fired: false,
            };
            let result = archive3tz::serialize(members, &mut fault, checkpoint);
            self.reached = fault.fired;
            result
        }
    }
    #[test]
    fn physical_partial_archive_payload_failure_preserves_previous_output() {
        let (work, input, output) = setup();
        let mut fault = PayloadFault { reached: false };
        let failure =
            point_cloud_with_io(request(&input, &output), &RunControl::default(), &mut fault)
                .unwrap_err();
        assert!(fault.reached);
        assert_clean(work.path(), &input, &output, &failure);
    }
    struct CleanupFault {
        reached: bool,
    }
    impl ProducerIo for CleanupFault {
        fn cleanup(&mut self, path: &Path) -> io::Result<()> {
            self.reached = true;
            // Real filesystem refusal: an accepted workspace is nonempty.
            fs::remove_dir(path)
        }
    }
    #[test]
    fn physical_cleanup_failure_retains_workspace_and_aborts_before_publication() {
        let (work, input, output) = setup();
        let mut fault = CleanupFault { reached: false };
        let failure =
            point_cloud_with_io(request(&input, &output), &RunControl::default(), &mut fault)
                .unwrap_err();
        assert!(fault.reached);
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
        assert_eq!(fs::read(&output).unwrap(), b"previous archive");
        assert_eq!(failure.retained_paths.len(), 1);
        let retained = &failure.retained_paths[0];
        assert!(retained.join("tileset.json").is_file());
        assert!(retained.join("conversion.json").is_file());
        assert_eq!(fs::read_dir(work.path()).unwrap().count(), 3);
        // The keep() transfer prevents a hidden Drop cleanup retry.
        assert!(retained.exists());
        fs::remove_dir_all(retained).unwrap();
    }
    #[test]
    fn malformed_header_boundaries_refuse_before_staging() {
        for (offset, bytes) in [
            (94, 0u16.to_le_bytes().to_vec()),
            (94, 226u16.to_le_bytes().to_vec()),
            (94, u16::MAX.to_le_bytes().to_vec()),
            (96, 226u32.to_le_bytes().to_vec()),
            (96, u32::MAX.to_le_bytes().to_vec()),
        ] {
            let (work, input, output) = setup();
            let mut file = fs::OpenOptions::new().write(true).open(&input).unwrap();
            file.seek(SeekFrom::Start(offset)).unwrap();
            file.write_all(&bytes).unwrap();
            drop(file);
            let failure = point_cloud_to_archive(request(&input, &output), &RunControl::default())
                .unwrap_err();
            assert_eq!(
                failure.error.kind(),
                JobErrorKind::InvalidInput,
                "{failure:?}"
            );
            assert_eq!(fs::read(&output).unwrap(), b"previous archive");
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
        }
    }
    #[test]
    fn unsupported_literal_header_version_has_typed_profile_refusal() {
        let (work, input, output) = setup();
        let mut file = fs::OpenOptions::new().write(true).open(&input).unwrap();
        file.seek(SeekFrom::Start(24)).unwrap();
        file.write_all(&[1, 5]).unwrap();
        drop(file);
        let failure =
            point_cloud_to_archive(request(&input, &output), &RunControl::default()).unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::Unsupported);
        assert_eq!(fs::read(&output).unwrap(), b"previous archive");
        assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
    }
    #[test]
    fn duplicate_relevant_geokeys_refuse_actual_source_before_staging() {
        for key in [1024u16, 2048, 3072, 4096] {
            let work = tempfile::tempdir().unwrap();
            let input = work.path().join("source.las");
            let output = work.path().join("cloud.3tz");
            let words = [1u16, 1, 0, 2, key, 0, 1, 0, key, 0, 1, 4326];
            let mut builder = las::Builder::from(las::Header::default());
            builder.vlrs.push(las::Vlr {
                user_id: "LASF_Projection".into(),
                record_id: 34735,
                description: String::new(),
                data: words.into_iter().flat_map(u16::to_le_bytes).collect(),
            });
            let mut writer =
                las::Writer::from_path(&input, builder.into_header().unwrap()).unwrap();
            writer
                .write_point(las::Point {
                    x: 1.,
                    y: 1.,
                    z: 1.,
                    ..Default::default()
                })
                .unwrap();
            writer.close().unwrap();
            fs::write(&output, b"previous archive").unwrap();
            let request = PointCloudRequest::new(
                &input,
                &output,
                PointCloudCoordinates::Horizontal {
                    source: PointCloudCrs::Header,
                    height_offset_metres: 0.,
                },
                PointCloudOptions::default(),
            )
            .with_policy(OutputPolicy::Replace);
            let failure = point_cloud_to_archive(request, &RunControl::default()).unwrap_err();
            assert_eq!(
                failure.error.kind(),
                JobErrorKind::InvalidInput,
                "{failure:?}"
            );
            assert!(failure.error.message().contains("duplicate LAS CRS GeoKey"));
            assert_eq!(fs::read(&output).unwrap(), b"previous archive");
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
        }
    }
}
