use super::*;
use crate::Observer;
use std::{
    error::Error as _,
    fs::{self, File, FileTimes, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct Watch<F>(F);
impl<F> Observer for Watch<F>
where
    F: Fn(&RunEvent<'_>) -> Result<(), JobError> + Send + Sync,
{
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        (self.0)(event)
    }
}
fn ready(event: &RunEvent<'_>) -> bool {
    matches!(
        event,
        RunEvent::Progress {
            phase: "vector_compression_ready",
            ..
        }
    )
}
// Deliberately hand-framed, valid raw core-four content with no selected views.
// Exact identity makes file-lifecycle assertions independent of native encoding.
fn raw_glb() -> Vec<u8> {
    let json = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],"bufferViews":[],"accessors":[],"meshes":[],"nodes":[],"scenes":[{"nodes":[]}],"scene":0,"extras":{"opaque":1e+02}}"#;
    let json_len = (json.len() + 3) & !3;
    let total = 12 + 8 + json_len + 8 + 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_len as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(json);
    out.resize(20 + json_len, b' ');
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&[1, 2, 3, 4]);
    out
}
fn source(work: &Path) -> (PathBuf, Vec<u8>) {
    let path = work.join("content.glb");
    let bytes = raw_glb();
    fs::write(&path, &bytes).unwrap();
    (path, bytes)
}
fn no_stage(work: &Path) {
    assert!(fs::read_dir(work).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tiles-stage-")));
}
fn refuse<T: std::fmt::Debug>(result: Result<T, JobFailure>) -> JobFailure {
    result.unwrap_err()
}

#[test]
fn raw_identity_replaces_one_named_entry_and_freezes_receipt() {
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    let control = RunControl::default();
    let result = compress_vector_file(VectorCompressionRequest::new(&path), &control).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(result.output, fs::canonicalize(&path).unwrap());
    assert_eq!(result.report.before_bytes, bytes.len());
    assert_eq!(result.report.after_bytes, bytes.len());
    let json: serde_json::Value = serde_json::from_str(result.report_json()).unwrap();
    assert_eq!(json["beforeBytes"], bytes.len());
    assert!(result.cleanup_diagnostics.is_empty());
    assert!(!control.cancellation_handle().cancel());
    no_stage(work.path());
    assert_eq!(
        refuse(compress_vector_file(
            VectorCompressionRequest::new(path),
            &control
        ))
        .error
        .kind(),
        JobErrorKind::InvalidState
    );
}

#[test]
fn source_limit_and_capture_work_refuse_before_observers_or_staging() {
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    for working in [false, true] {
        let seen = Arc::new(AtomicUsize::new(0));
        let watch = seen.clone();
        let control = RunControl::new(Some(Arc::new(Watch(move |_: &RunEvent<'_>| {
            watch.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }))));
        let mut limits = CompressionLimits::default();
        if working {
            limits.working_bytes = 1;
        } else {
            limits.source_bytes = bytes.len() - 1;
        }
        let failure = refuse(compress_vector_file(
            VectorCompressionRequest::new(&path).with_limits(limits),
            &control,
        ));
        assert_eq!(failure.error.kind(), JobErrorKind::ResourceLimit);
        assert_eq!(seen.load(Ordering::SeqCst), 0);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        no_stage(work.path());
    }
}

#[test]
fn ready_callback_byte_mutation_with_restored_mtime_is_conflict() {
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let watched = path.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |event: &RunEvent<'_>| {
        if ready(event) {
            let mut file = OpenOptions::new().write(true).open(&watched).unwrap();
            file.seek(SeekFrom::End(-1)).unwrap();
            file.write_all(&[99]).unwrap();
            file.set_times(FileTimes::new().set_modified(modified))
                .unwrap();
        }
        Ok(())
    }))));
    let failure = refuse(compress_vector_file(
        VectorCompressionRequest::new(&path),
        &control,
    ));
    assert_eq!(failure.error.kind(), JobErrorKind::Conflict);
    let mut changed = bytes;
    *changed.last_mut().unwrap() = 99;
    assert_eq!(fs::read(&path).unwrap(), changed);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    no_stage(work.path());
}

#[test]
fn ready_callback_failure_retains_cause_and_cleans_complete_candidate() {
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    let cause = JobError::new(JobErrorKind::InvalidState, "observer stopped publication");
    let watched = cause.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |event: &RunEvent<'_>| {
        if ready(event) {
            Err(watched.clone())
        } else {
            Ok(())
        }
    }))));
    let failure = refuse(compress_vector_file(
        VectorCompressionRequest::new(&path),
        &control,
    ));
    assert_eq!(failure.error.kind(), JobErrorKind::ObserverFailure);
    let original = failure
        .error
        .source()
        .unwrap()
        .downcast_ref::<JobError>()
        .unwrap();
    assert!(original.same_cause(&cause));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    no_stage(work.path());
}

#[test]
fn codec_transport_keeps_checkpoint_identity_and_typed_format_causes() {
    let cause = JobError::new(JobErrorKind::Cancelled, "cancelled checkpoint");
    assert!(authored_codec_error(CodecError::Checkpoint(cause.clone())).same_cause(&cause));
    for error in [
        FormatError::InvalidInput("invalid token".into()),
        FormatError::Unsupported("external URI".into()),
    ] {
        let mapped = authored_codec_error(CodecError::Format(error));
        assert_eq!(mapped.kind(), JobErrorKind::InvalidState);
        assert!(mapped.source().unwrap().is::<FormatError>());
    }
    let limit = authored_codec_error(CodecError::Format(FormatError::ResourceLimit(
        "cardinality".into(),
    )));
    assert_eq!(limit.kind(), JobErrorKind::ResourceLimit);
    assert!(limit.source().unwrap().is::<FormatError>());
    let native = authored_codec_error(CodecError::EncodingFailure("attributes"));
    assert_eq!(native.kind(), JobErrorKind::InvalidState);
    assert!(native.source().unwrap().is::<CodecError<JobError>>());
    assert_eq!(
        external_codec_error(CodecError::Format(FormatError::Unsupported(
            "valid unsupported".into()
        )))
        .kind(),
        JobErrorKind::Unsupported
    );
}

#[cfg(unix)]
#[test]
fn replacement_preserves_portable_permissions_and_old_hardlink_reader() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let alias = work.path().join("alias.glb");
    fs::hard_link(&path, &alias).unwrap();
    let old_inode = fs::metadata(&path).unwrap().ino();
    let mut old_reader = File::open(&path).unwrap();
    let watched_dir = work.path().to_path_buf();
    let watched_bytes = bytes.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |event: &RunEvent<'_>| {
        if ready(event) {
            let candidate = fs::read_dir(&watched_dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".tiles-stage-")
                })
                .unwrap();
            assert_eq!(
                fs::metadata(&candidate).unwrap().permissions().mode() & 0o777,
                0o640
            );
            assert_eq!(fs::read(candidate).unwrap(), watched_bytes);
        }
        Ok(())
    }))));
    compress_vector_file(VectorCompressionRequest::new(&path), &control).unwrap();
    assert_ne!(fs::metadata(&path).unwrap().ino(), old_inode);
    assert_eq!(fs::metadata(&alias).unwrap().ino(), old_inode);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let mut read = Vec::new();
    old_reader.read_to_end(&mut read).unwrap();
    assert_eq!(read, bytes);
    assert_eq!(fs::read(alias).unwrap(), bytes);
    no_stage(work.path());
}

#[cfg(unix)]
#[test]
fn symlink_leaf_refuses_before_observers_and_permission_change_conflicts() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    let alias = work.path().join("symlink.glb");
    symlink(&path, &alias).unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    let watched = seen.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |_: &RunEvent<'_>| {
        watched.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }))));
    assert_eq!(
        refuse(compress_vector_file(
            VectorCompressionRequest::new(alias),
            &control
        ))
        .error
        .kind(),
        JobErrorKind::Unsupported
    );
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let watched = path.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |event: &RunEvent<'_>| {
        if ready(event) {
            fs::set_permissions(&watched, fs::Permissions::from_mode(0o660)).unwrap();
        }
        Ok(())
    }))));
    assert_eq!(
        refuse(compress_vector_file(
            VectorCompressionRequest::new(&path),
            &control
        ))
        .error
        .kind(),
        JobErrorKind::Conflict
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o660
    );
    no_stage(work.path());
}

#[cfg(windows)]
#[test]
fn readonly_windows_source_is_unsupported_without_observation_or_stage() {
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    let watched = seen.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |_: &RunEvent<'_>| {
        watched.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }))));
    assert_eq!(
        refuse(compress_vector_file(
            VectorCompressionRequest::new(&path),
            &control
        ))
        .error
        .kind(),
        JobErrorKind::Unsupported
    );
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    no_stage(work.path());
    // Test-owned cleanup after the assertion; the operation never clears flags.
    let mut cleanup = fs::metadata(&path).unwrap().permissions();
    cleanup.set_readonly(false);
    fs::set_permissions(&path, cleanup).unwrap();
}

#[test]
fn ready_callback_renamed_source_and_same_byte_replacement_is_conflict() {
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let previous = work.path().join("previous.glb");
    let watched = path.clone();
    let moved = previous.clone();
    let replacement = bytes.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |event: &RunEvent<'_>| {
        if ready(event) {
            fs::rename(&watched, &moved).unwrap();
            fs::write(&watched, &replacement).unwrap();
            OpenOptions::new()
                .write(true)
                .open(&watched)
                .unwrap()
                .set_times(FileTimes::new().set_modified(modified))
                .unwrap();
        }
        Ok(())
    }))));
    assert_eq!(
        refuse(compress_vector_file(
            VectorCompressionRequest::new(&path),
            &control
        ))
        .error
        .kind(),
        JobErrorKind::Conflict
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read(&previous).unwrap(), bytes);
    no_stage(work.path());
}

#[test]
fn ready_callback_delete_or_length_change_refuses_without_candidate_install() {
    for mutation in [0, 1, 2] {
        let work = tempfile::tempdir().unwrap();
        let (path, bytes) = source(work.path());
        let watched = path.clone();
        let mut mutated = bytes.clone();
        if mutation == 1 {
            mutated.pop();
        }
        if mutation == 2 {
            mutated.push(99);
        }
        let changed = mutated.clone();
        let control = RunControl::new(Some(Arc::new(Watch(move |event: &RunEvent<'_>| {
            if ready(event) {
                if mutation == 0 {
                    fs::remove_file(&watched).unwrap();
                } else {
                    fs::write(&watched, &changed).unwrap();
                }
            }
            Ok(())
        }))));
        assert_eq!(
            refuse(compress_vector_file(
                VectorCompressionRequest::new(&path),
                &control
            ))
            .error
            .kind(),
            JobErrorKind::Conflict
        );
        if mutation == 0 {
            assert!(!path.exists());
        } else {
            assert_eq!(fs::read(&path).unwrap(), mutated);
        }
        no_stage(work.path());
    }
}

#[test]
fn cancellation_at_ready_cleans_candidate_and_preserves_primary_cause() {
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    let cell = Arc::new(std::sync::OnceLock::new());
    let watched = cell.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |event: &RunEvent<'_>| {
        if ready(event) {
            let cancel: &crate::CancellationHandle = watched.get().unwrap();
            assert!(cancel.cancel());
        }
        Ok(())
    }))));
    assert!(cell.set(control.cancellation_handle()).is_ok());
    assert_eq!(
        refuse(compress_vector_file(
            VectorCompressionRequest::new(&path),
            &control
        ))
        .error
        .kind(),
        JobErrorKind::Cancelled
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    no_stage(work.path());
}

#[test]
fn malformed_source_retains_format_cause_and_original_bytes() {
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("invalid.glb");
    fs::write(&path, b"not glb").unwrap();
    let failure = refuse(compress_vector_file(
        VectorCompressionRequest::new(&path),
        &RunControl::default(),
    ));
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidInput);
    assert!(failure.error.source().unwrap().is::<FormatError>());
    assert_eq!(fs::read(&path).unwrap(), b"not glb");
    no_stage(work.path());
}

#[test]
fn invalid_limits_consume_control_and_never_read_or_observe_source() {
    let limits = CompressionLimits {
        source_bytes: 0,
        ..CompressionLimits::default()
    };
    let seen = Arc::new(AtomicUsize::new(0));
    let watched = seen.clone();
    let control = RunControl::new(Some(Arc::new(Watch(move |_: &RunEvent<'_>| {
        watched.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }))));
    let request = VectorCompressionRequest::new("definitely-absent.glb").with_limits(limits);
    // Codec's invalid configuration is distinct from a source I/O boundary.
    assert_eq!(
        refuse(compress_vector_file(request.clone(), &control))
            .error
            .kind(),
        JobErrorKind::InvalidRequest
    );
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    assert_eq!(
        refuse(compress_vector_file(request, &control)).error.kind(),
        JobErrorKind::InvalidState
    );
}

#[test]
#[ignore = "changes process CWD; coordinator runs this case alone in a fresh process"]
fn callback_cwd_change_cannot_redirect_relative_source_or_publication() {
    struct Restore(PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).unwrap();
        }
    }
    let restore = Restore(std::env::current_dir().unwrap());
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let (path, bytes) = source(first.path());
    let (other, mut other_bytes) = source(second.path());
    *other_bytes.last_mut().unwrap() = 44;
    fs::write(&other, &other_bytes).unwrap();
    std::env::set_current_dir(first.path()).unwrap();
    let second_dir = second.path().to_path_buf();
    let control = RunControl::new(Some(Arc::new(Watch(move |event: &RunEvent<'_>| {
        if matches!(
            event,
            RunEvent::Progress {
                phase: "vector_compression_captured",
                ..
            }
        ) {
            std::env::set_current_dir(&second_dir).unwrap();
        }
        Ok(())
    }))));
    let result =
        compress_vector_file(VectorCompressionRequest::new("content.glb"), &control).unwrap();
    assert_eq!(result.output, fs::canonicalize(&path).unwrap());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read(&other).unwrap(), other_bytes);
    no_stage(first.path());
    no_stage(second.path());
    drop(restore);
}

#[test]
fn maximum_finite_receipt_fits_fixed_reserved_report_owner() {
    let report = VectorCompressionReport {
        before_bytes: usize::MAX,
        after_bytes: usize::MAX,
        views: usize::MAX,
        raw_views: usize::MAX,
        compressed_views: usize::MAX,
        existing_views: usize::MAX,
        accessors: usize::MAX,
        logical_bytes: usize::MAX,
        estimated_peak_bytes: usize::MAX,
    };
    let serialized = serde_json::to_vec(&report).unwrap();
    assert!(serialized.len() < REPORT_BYTES);
}

#[test]
fn actual_owned_source_path_and_header_ledger_is_recorded_without_maximum_reservation() {
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    let limits = CompressionLimits::default();
    let control = RunControl::default();
    let attempt = control.begin().unwrap();
    let captured = source::Captured::capture(
        path,
        limits.source_bytes,
        limits.working_bytes,
        fixed_storage().unwrap(),
        &attempt,
    )
    .unwrap();
    assert_eq!(captured.bytes.capacity(), bytes.len());
    let storage = captured.persistent_storage().unwrap();
    assert!(storage >= captured.bytes.capacity());
    assert!(storage < limits.source_bytes);
    println!(
        "{}",
        serde_json::json!({
            "record": "vector_operation_requested_owners",
            "source_len": captured.bytes.len(), "source_capacity": captured.bytes.capacity(),
            "spelling_capacity": captured.spelling.capacity(), "canonical_capacity": captured.canonical.capacity(),
            "parent_capacity": captured.parent.capacity(), "parent_bytes": captured.parent.as_os_str().as_encoded_bytes().len(),
            "parent_components": captured.parent.components().count(),
            "staging_construction_reserve": source::staging_storage(&captured.parent).unwrap(),
            "retained_including_path_phase_reserves": storage,
            "fixed_storage": fixed_storage().unwrap(),
            "captured_header": size_of::<source::Captured>(), "encoded_header": size_of::<Encoded>(),
            "result_header": size_of::<VectorCompressionResult>(), "report_header": size_of::<VectorCompressionReport>(),
            "limits_header": size_of::<CompressionLimits>(), "staging_header": size_of::<Staging<'static>>(),
            "metadata_header": size_of::<fs::Metadata>(), "identity_header": size_of::<same_file::Handle>(),
            "path_header": size_of::<PathBuf>(), "component_header": size_of::<std::path::Component<'_>>(),
            "comparison_bytes": CHUNK_BYTES, "report_reserved_bytes": REPORT_BYTES
        })
    );
}

#[test]
fn transferred_request_path_capacity_is_refused_or_retained_in_phase_peak() {
    let work = tempfile::tempdir().unwrap();
    let (path, bytes) = source(work.path());
    let mut padded = PathBuf::with_capacity(2_097_152);
    padded.push(&path);
    let path_capacity = padded.capacity();
    let limits = CompressionLimits {
        working_bytes: path_capacity - 1,
        ..CompressionLimits::default()
    };
    let mut refused_path = PathBuf::with_capacity(path_capacity);
    refused_path.push(&path);
    let failure = refuse(compress_vector_file(
        VectorCompressionRequest::new(refused_path).with_limits(limits),
        &RunControl::default(),
    ));
    assert_eq!(failure.error.kind(), JobErrorKind::ResourceLimit);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let result = compress_vector_file(
        VectorCompressionRequest::new(padded),
        &RunControl::default(),
    )
    .unwrap();
    assert!(result.report.estimated_peak_bytes >= path_capacity);
    no_stage(work.path());
}
