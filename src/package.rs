//! Opaque 3TZ packaging: validate a request, resolve a regular-file inventory,
//! copy bounded payload chunks, then seal and publish one completed archive.
//!
//! Sources must remain unchanged for the operation. Size/mtime checks detect
//! some changes; this operation does not provide a filesystem snapshot. Payload
//! memory is bounded while inventory and index memory scale with member count.
use crate::{
    archive3tz::{self, CodecError, WriteFailure},
    runtime::{
        CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl, RunEvent,
        Staging,
    },
};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug)]
pub struct PackageMember {
    name: String,
    source: PathBuf,
}
impl PackageMember {
    pub fn new(name: impl Into<String>, source: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn source(&self) -> &Path {
        &self.source
    }
}

#[derive(Clone, Debug)]
enum Source {
    Directory(PathBuf),
    Members(Vec<PackageMember>),
}

#[derive(Clone, Debug)]
pub struct PackageRequest {
    source: Source,
    output: PathBuf,
    policy: OutputPolicy,
}
impl PackageRequest {
    pub fn directory(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            source: Source::Directory(input.into()),
            output: output.into(),
            policy: OutputPolicy::CreateNew,
        }
    }
    pub fn members(members: Vec<PackageMember>, output: impl Into<PathBuf>) -> Self {
        Self {
            source: Source::Members(members),
            output: output.into(),
            policy: OutputPolicy::CreateNew,
        }
    }
    pub fn with_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageReceipt {
    pub member_count: u64,
    pub source_bytes: u64,
    pub archive_bytes: u64,
}
#[derive(Debug)]
pub struct PackageResult {
    pub output: PathBuf,
    pub receipt: PackageReceipt,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}

struct Validated(PackageRequest);
struct ResolvedMember {
    name: String,
    source: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}
struct Resolved {
    request: PackageRequest,
    members: Vec<ResolvedMember>,
    source_bytes: u64,
}

fn invalid(kind: JobErrorKind, message: impl Into<String>) -> JobError {
    JobError::new(kind, message)
}
fn validate_name(name: &str) -> Result<(), JobError> {
    let drive = name.as_bytes().get(1) == Some(&b':') && name.as_bytes()[0].is_ascii_alphabetic();
    if name.is_empty()
        || name == archive3tz::TZ_INDEX_NAME
        || name.len() > u16::MAX as usize
        || name.contains(".3tz")
        || name.contains(".3dtiles.zip")
        || name.contains(['\\', '\0'])
        || name.starts_with('/')
        || drive
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(invalid(
            JobErrorKind::InvalidRequest,
            format!("invalid archive member name: {name:?}"),
        ));
    }
    Ok(())
}
fn validate(request: PackageRequest) -> Result<Validated, JobError> {
    if request.output.as_os_str().is_empty() || request.output.file_name().is_none() {
        return Err(invalid(
            JobErrorKind::InvalidRequest,
            "output must name an archive file",
        ));
    }
    if request
        .output
        .extension()
        .is_none_or(|extension| extension != "3tz")
        && !request
            .output
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".3dtiles.zip"))
    {
        return Err(invalid(
            JobErrorKind::InvalidRequest,
            "output must use .3tz or .3dtiles.zip",
        ));
    }
    match &request.source {
        Source::Directory(input) if input.as_os_str().is_empty() => {
            return Err(invalid(
                JobErrorKind::InvalidRequest,
                "input must name a directory or tileset.json",
            ))
        }
        Source::Members(members) => {
            let mut names = HashSet::new();
            for member in members {
                validate_name(&member.name)?;
                if !names.insert(member.name.clone()) {
                    return Err(invalid(
                        JobErrorKind::InvalidRequest,
                        format!("duplicate archive member: {:?}", member.name),
                    ));
                }
                if member.source.as_os_str().is_empty() {
                    return Err(invalid(
                        JobErrorKind::InvalidRequest,
                        "member source path must not be empty",
                    ));
                }
            }
            if !names.contains("tileset.json") {
                return Err(invalid(
                    JobErrorKind::InvalidRequest,
                    "package inventory requires exactly one tileset.json",
                ));
            }
        }
        _ => {}
    }
    Ok(Validated(request))
}

fn metadata(path: &Path) -> Result<fs::Metadata, JobError> {
    fs::symlink_metadata(path).map_err(|error| JobError::io("inspect source", path, error))
}
fn regular(path: &Path) -> Result<fs::Metadata, JobError> {
    let value = metadata(path)?;
    if !value.is_file() {
        return Err(invalid(
            JobErrorKind::InvalidInput,
            format!(
                "source must be a regular file, without symlinks: {}",
                path.display()
            ),
        ));
    }
    Ok(value)
}

fn resolve(
    Validated(mut request): Validated,
    mut checkpoint: impl FnMut() -> Result<(), JobError>,
) -> Result<Resolved, JobError> {
    checkpoint()?;
    let output = crate::output_path::resolve(&request.output)?;
    let output_metadata = match fs::symlink_metadata(&request.output) {
        Ok(value) => Some(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(JobError::io("inspect output", &request.output, error)),
    };
    let selected = match &request.source {
        Source::Members(members) => members.clone(),
        Source::Directory(input) => {
            let input_metadata = metadata(input)?;
            let root = if input_metadata.is_dir() {
                input.clone()
            } else if input_metadata.is_file()
                && input.file_name().is_some_and(|name| name == "tileset.json")
            {
                input
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."))
                    .into()
            } else {
                return Err(invalid(
                    JobErrorKind::InvalidInput,
                    "input must be a directory or its exact tileset.json regular file",
                ));
            };
            if !metadata(&root)?.is_dir() {
                return Err(invalid(
                    JobErrorKind::InvalidInput,
                    "source root must be a directory, without symlinks",
                ));
            }
            let canonical_root = fs::canonicalize(&root)
                .map_err(|error| JobError::io("resolve source root", &root, error))?;
            if output.starts_with(&canonical_root) {
                return Err(invalid(
                    JobErrorKind::InvalidRequest,
                    "output must be outside the selected source tree",
                ));
            }
            let mut selected = Vec::new();
            for entry in walkdir::WalkDir::new(&root).follow_links(false) {
                checkpoint()?;
                let entry = entry.map_err(|error| {
                    let path = error.path().unwrap_or(&root).to_path_buf();
                    JobError::io(
                        "walk source tree",
                        &path,
                        error
                            .into_io_error()
                            .unwrap_or_else(|| std::io::Error::other("source traversal failed")),
                    )
                })?;
                if entry.file_type().is_dir() {
                    continue;
                }
                if !entry.file_type().is_file() {
                    return Err(invalid(
                        JobErrorKind::InvalidInput,
                        format!(
                            "source tree contains a symlink or nonregular entry: {}",
                            entry.path().display()
                        ),
                    ));
                }
                let relative = entry
                    .path()
                    .strip_prefix(&root)
                    .expect("walker entries are below root");
                let name = relative.to_str().ok_or_else(|| {
                    invalid(
                        JobErrorKind::InvalidInput,
                        "source member name must be UTF-8",
                    )
                })?;
                #[cfg(windows)]
                let name = name.replace('\\', "/");
                selected.push(PackageMember::new(name, entry.path()));
            }
            if !selected.iter().any(|member| member.name == "tileset.json") {
                return Err(invalid(
                    JobErrorKind::InvalidInput,
                    "package source is missing exact tileset.json",
                ));
            }
            selected
        }
    };
    let mut members = Vec::with_capacity(selected.len());
    let mut names = HashSet::new();
    let mut source_bytes = 0u64;
    for member in selected {
        checkpoint()?;
        validate_name(&member.name)?;
        if !names.insert(member.name.clone()) {
            return Err(invalid(
                JobErrorKind::InvalidInput,
                "source inventory contains duplicate names",
            ));
        }
        let metadata = regular(&member.source)?;
        if metadata.len() >= u64::from(u32::MAX) {
            return Err(invalid(
                JobErrorKind::InvalidInput,
                "3TZ member sizes must fit their 32-bit local headers",
            ));
        }
        let canonical_source = fs::canonicalize(&member.source)
            .map_err(|error| JobError::io("resolve source", &member.source, error))?;
        if output == canonical_source
            || output.starts_with(&canonical_source)
            || (output_metadata.is_some()
                && same_file::is_same_file(&request.output, &canonical_source).map_err(
                    |error| JobError::io("compare output/source identity", &request.output, error),
                )?)
        {
            return Err(invalid(
                JobErrorKind::InvalidRequest,
                "output overlaps or aliases a selected source member",
            ));
        }
        source_bytes = source_bytes.checked_add(metadata.len()).ok_or_else(|| {
            invalid(
                JobErrorKind::InvalidInput,
                "source inventory byte count exceeds u64",
            )
        })?;
        members.push(ResolvedMember {
            name: member.name,
            source: canonical_source,
            size: metadata.len(),
            modified: metadata.modified().ok(),
        });
    }
    if (members.len() as u64)
        .checked_mul(24)
        .is_none_or(|bytes| bytes >= u64::from(u32::MAX))
    {
        return Err(invalid(
            JobErrorKind::InvalidInput,
            "3TZ index size must fit its 32-bit local header",
        ));
    }
    if output_metadata.is_some_and(|metadata| !metadata.is_file()) {
        return Err(invalid(
            JobErrorKind::InvalidInput,
            "output must be a regular file, without symlinks",
        ));
    }
    // Bind side effects and the returned identity to the location checked above.
    // An observer may change process CWD after resolution; retaining a relative
    // request path would redirect staging/publication and invalidate overlap checks.
    request.output = output;
    Ok(Resolved {
        request,
        members,
        source_bytes,
    })
}

fn codec_error(error: CodecError, output: &Path) -> JobError {
    match error {
        CodecError::Io(error) => JobError::io("serialize archive", output, error),
        CodecError::SourceIo { path, source } => {
            JobError::io("read selected source", &path, source)
        }
        CodecError::Zip(error) => {
            JobError::io("serialize ZIP", output, std::io::Error::other(error))
        }
        CodecError::SourceChanged(path) => invalid(
            JobErrorKind::InvalidInput,
            format!("source changed while packaging: {}", path.display()),
        ),
        CodecError::MissingManifest => {
            invalid(JobErrorKind::InvalidInput, "package missing tileset.json")
        }
        CodecError::Invalid(message) => invalid(JobErrorKind::InvalidInput, message),
    }
}

pub fn package(request: PackageRequest, control: &RunControl) -> Result<PackageResult, JobFailure> {
    package_with_encoder(request, control, |members, writer, checkpoint| {
        archive3tz::serialize(members, writer, checkpoint)
    })
}

// Private producer seam permits storage/finalization faults in tests without
// public configuration or fault flags in normal execution.
fn package_with_encoder(
    request: PackageRequest,
    control: &RunControl,
    encode: impl FnOnce(
        &[archive3tz::Member<'_>],
        &mut fs::File,
        &mut dyn FnMut(u64) -> Result<(), JobError>,
    ) -> Result<(), WriteFailure<JobError>>,
) -> Result<PackageResult, JobFailure> {
    let attempt = control.begin()?;
    attempt.check().map_err(|error| attempt.fail(error))?;
    let resolved = validate(request)
        .and_then(|validated| resolve(validated, || attempt.check()))
        .map_err(|error| attempt.fail(error))?;
    let progress = |phase, done| RunEvent::Progress {
        phase,
        done,
        total: Some(resolved.source_bytes),
    };
    attempt
        .emit(&progress("encoding", 0))
        .map_err(|error| attempt.fail(error))?;
    let mut staging = Staging::create(&resolved.request.output, &attempt)?;
    let members: Vec<_> = resolved
        .members
        .iter()
        .map(|member| archive3tz::Member {
            name: &member.name,
            source: &member.source,
            size: member.size,
            modified: member.modified,
        })
        .collect();
    let mut checkpoint = |done| {
        attempt.check()?;
        attempt.emit(&progress("encoding", done))
    };
    let encoded = encode(&members, staging.writer(), &mut checkpoint);
    if let Err(error) = encoded {
        let error = match error {
            WriteFailure::Codec(error) => codec_error(error, &resolved.request.output),
            WriteFailure::Checkpoint(error) => error,
        };
        return Err(staging.fail(error));
    }
    let archive_bytes = match staging.writer().metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return Err(staging.fail(JobError::io(
                "inspect encoded archive",
                &resolved.request.output,
                error,
            )))
        }
    };
    if let Err(error) = attempt
        .emit(&progress("ready_to_publish", resolved.source_bytes))
        .and_then(|_| attempt.close_events())
    {
        return Err(staging.fail(error));
    }
    let receipt = PackageReceipt {
        member_count: resolved.members.len() as u64,
        source_bytes: resolved.source_bytes,
        archive_bytes,
    };
    let sealed = staging.seal()?;
    let published = sealed.publish(&resolved.request.output, resolved.request.policy)?;
    Ok(PackageResult {
        output: published.output,
        receipt,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive3tz::test_faults::{Fault, FaultWriter};
    #[test]
    fn codec_payload_finalize_and_flush_faults_abort_and_clean_before_replace() {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("source");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("tileset.json"), vec![0xaa; 8192]).unwrap();
        let output = work.path().join("output.3tz");
        fs::write(&output, b"previous").unwrap();
        for fault in [Fault::Payload, Fault::Finalize, Fault::Flush] {
            let request =
                PackageRequest::directory(&input, &output).with_policy(OutputPolicy::Replace);
            let control = RunControl::default();
            let failure = package_with_encoder(request, &control, |members, file, checkpoint| {
                let mut writer = FaultWriter::new(file, fault);
                let result = archive3tz::serialize(members, &mut writer, checkpoint);
                assert!(writer.fired, "{fault:?} injection not reached");
                result
            })
            .unwrap_err();
            assert_eq!(
                failure.error.kind(),
                JobErrorKind::Io,
                "{fault:?}: {failure:?}"
            );
            assert_eq!(fs::read(&output).unwrap(), b"previous");
            assert!(failure.retained_paths.is_empty());
            assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
            assert!(
                !control.cancellation_handle().cancel(),
                "producer failure must remain terminal"
            );
        }
    }
}

#[cfg(test)]
mod reproducibility_tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn packing_is_independent_of_caller_entry_order() {
        let work = tempfile::tempdir().unwrap();
        let manifest = work.path().join("tileset.json");
        let content = work.path().join("tile.glb");
        fs::write(&manifest, b"{}").unwrap();
        fs::write(&content, b"same payload").unwrap();
        let mut members = vec![
            PackageMember::new("tile.glb", content),
            PackageMember::new("tileset.json", manifest),
        ];
        let a = work.path().join("a.3tz");
        let b = work.path().join("b.3tz");
        let first = package(
            PackageRequest::members(members.clone(), &a),
            &RunControl::default(),
        )
        .unwrap();
        members.reverse();
        let reversed =
            package(PackageRequest::members(members, &b), &RunControl::default()).unwrap();
        assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
        assert_eq!(first.receipt, reversed.receipt);
        assert_eq!(first.receipt.member_count, 2);
        assert_eq!(first.receipt.source_bytes, 14);
        archive3tz::validate_3tz(&a).unwrap();
        // Independently read copied bytes instead of inferring preservation
        // from equality between two outputs of the same encoder.
        let mut zip = zip::ZipArchive::new(fs::File::open(&a).unwrap()).unwrap();
        assert_eq!(zip.len(), 3);
        for (name, expected) in [
            ("tileset.json", b"{}".as_slice()),
            ("tile.glb", b"same payload".as_slice()),
        ] {
            let mut bytes = Vec::new();
            zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, expected);
        }
    }
}
