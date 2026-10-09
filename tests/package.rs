use rusty_tiles::{
    package::{package, PackageMember, PackageRequest},
    JobError, JobErrorKind, Observer, OutputPolicy, RunControl, RunEvent,
};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

struct Action<F>(F);
impl<F> Observer for Action<F>
where
    F: Fn(&RunEvent<'_>) -> Result<(), JobError> + Send + Sync,
{
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        (self.0)(event)
    }
}
fn control(
    action: impl Fn(&RunEvent<'_>) -> Result<(), JobError> + Send + Sync + 'static,
) -> RunControl {
    RunControl::new(Some(Arc::new(Action(action))))
}
fn source(root: &Path) -> PathBuf {
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("tileset.json"),
        b"opaque manifest bytes, deliberately not JSON",
    )
    .unwrap();
    source
}
fn names(path: &Path) -> Vec<String> {
    let mut names = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}
fn u16le(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().unwrap())
}
fn u32le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

// Independent container oracle: ZIP reader for contents, then hand-parse every
// indexed local header/data/CRC; do not call the production 3TZ validator.
fn inspect(path: &Path, expected: &BTreeMap<String, Vec<u8>>) {
    let bytes = fs::read(path).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    assert_eq!(zip.by_index(0).unwrap().name(), "tileset.json");
    let entries = zip.len();
    assert_eq!(zip.by_index(entries - 1).unwrap().name(), "@3dtilesIndex1@");
    assert_eq!(zip.len(), expected.len() + 1);
    for (name, content) in expected {
        let mut entry = zip.by_name(name).unwrap();
        assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
        let mut actual = Vec::new();
        entry.read_to_end(&mut actual).unwrap();
        assert_eq!(&actual, content);
    }
    let mut index = Vec::new();
    let index_offset = {
        let mut entry = zip.by_name("@3dtilesIndex1@").unwrap();
        assert!(entry.comment().is_empty());
        let offset = entry.header_start() as usize;
        entry.read_to_end(&mut index).unwrap();
        offset
    };
    let index_header = &bytes[index_offset..index_offset + 30];
    assert_eq!(&index_header[..4], b"PK\x03\x04");
    assert_eq!(u16le(&index_header[6..8]) & 8, 0);
    assert_eq!(u16le(&index_header[8..10]), 0);
    assert_eq!(u32le(&index_header[14..18]), crc32fast::hash(&index));
    assert_eq!(u32le(&index_header[18..22]) as usize, index.len());
    assert_eq!(u32le(&index_header[22..26]) as usize, index.len());
    let index_name_length = u16le(&index_header[26..28]) as usize;
    assert_eq!(
        &bytes[index_offset + 30..index_offset + 30 + index_name_length],
        b"@3dtilesIndex1@"
    );
    assert_eq!(index.len(), expected.len() * 24);
    let mut found = HashSet::new();
    let mut previous = None;
    for record in index.as_chunks::<24>().0 {
        let key = (
            u64::from_le_bytes(record[..8].try_into().unwrap()),
            u64::from_le_bytes(record[8..16].try_into().unwrap()),
        );
        assert!(previous.is_none_or(|p| p < key));
        previous = Some(key);
        let offset = u64::from_le_bytes(record[16..].try_into().unwrap()) as usize;
        let header = &bytes[offset..offset + 30];
        assert_eq!(&header[..4], b"PK\x03\x04");
        assert_eq!(
            u16le(&header[6..8]) & 8,
            0,
            "local sizes/CRC must be in the header"
        );
        assert_eq!(u16le(&header[8..10]), 0);
        let name_len = u16le(&header[26..28]) as usize;
        let extra_len = u16le(&header[28..30]) as usize;
        let name = std::str::from_utf8(&bytes[offset + 30..offset + 30 + name_len]).unwrap();
        assert_eq!(&md5::compute(name.as_bytes()).0, &record[..16]);
        assert!(found.insert(name.to_owned()));
        let content = expected.get(name).unwrap();
        let size = u32le(&header[18..22]) as usize;
        assert_eq!(size, content.len());
        assert_eq!(u32le(&header[22..26]) as usize, size);
        assert_eq!(u32le(&header[14..18]), crc32fast::hash(content));
        let start = offset + 30 + name_len + extra_len;
        assert_eq!(&bytes[start..start + size], content);
    }
    assert_eq!(found, expected.keys().cloned().collect());
}

#[test]
fn opaque_bytes_receipt_local_headers_and_index_are_independently_checked() {
    let work = tempfile::tempdir().unwrap();
    let root = source(work.path());
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/雪.bin"), vec![0x9b; 200_000]).unwrap();
    fs::write(root.join("conversion.json"), b"original report bytes\n").unwrap();
    let expected = [
        ("tileset.json", fs::read(root.join("tileset.json")).unwrap()),
        (
            "nested/雪.bin",
            fs::read(root.join("nested/雪.bin")).unwrap(),
        ),
        (
            "conversion.json",
            fs::read(root.join("conversion.json")).unwrap(),
        ),
    ]
    .into_iter()
    .map(|(n, b)| (n.to_owned(), b))
    .collect::<BTreeMap<_, _>>();
    let events = Arc::new(Mutex::new(Vec::new()));
    let capture = events.clone();
    let run = control(move |event| {
        if let RunEvent::Progress { phase, done, total } = event {
            capture
                .lock()
                .unwrap()
                .push((phase.to_string(), *done, *total));
        }
        Ok(())
    });
    let output = work.path().join("new/output.3tz");
    let result = package(
        PackageRequest::directory(root.join("tileset.json"), &output),
        &run,
    )
    .unwrap();
    assert_eq!(result.output, output);
    assert_eq!(result.receipt.member_count, 3);
    assert_eq!(
        result.receipt.source_bytes,
        expected.values().map(|b| b.len() as u64).sum::<u64>()
    );
    assert_eq!(
        result.receipt.archive_bytes,
        fs::metadata(&output).unwrap().len()
    );
    assert!(result.cleanup_diagnostics.is_empty());
    inspect(&output, &expected);
    let events = events.lock().unwrap();
    assert_eq!(events.last().unwrap().0, "ready_to_publish");
    assert!(events.windows(2).all(|pair| pair[0].1 <= pair[1].1));
    assert_eq!(names(output.parent().unwrap()), vec!["output.3tz"]);
}

#[test]
fn named_order_is_deterministic_and_alternative_archive_extension_is_supported() {
    let work = tempfile::tempdir().unwrap();
    let root = source(work.path());
    let payload = root.join("payload");
    fs::write(&payload, b"data").unwrap();
    let mut members = vec![
        PackageMember::new("payload", payload),
        PackageMember::new("tileset.json", root.join("tileset.json")),
    ];
    let a = work.path().join("a.3tz");
    let b = work.path().join("b.3dtiles.zip");
    package(
        PackageRequest::members(members.clone(), &a),
        &RunControl::default(),
    )
    .unwrap();
    members.reverse();
    package(PackageRequest::members(members, &b), &RunControl::default()).unwrap();
    assert_eq!(fs::read(a).unwrap(), fs::read(b).unwrap());
}

#[test]
fn invalid_inventory_is_pure_and_missing_inputs_are_io() {
    let work = tempfile::tempdir().unwrap();
    let absent = work.path().join("missing-source");
    for name in [
        "",
        "/absolute",
        "../escape",
        "a/./b",
        "a//b",
        "a\\b",
        "nul\0name",
        "C:relative",
        "@3dtilesIndex1@",
        "nested.3tz/tile",
        "a.3dtiles.zip",
    ] {
        let output = work.path().join("never-created/out.3tz");
        let members = vec![
            PackageMember::new("tileset.json", &absent),
            PackageMember::new(name, &absent),
        ];
        let failure = package(
            PackageRequest::members(members, &output),
            &RunControl::default(),
        )
        .unwrap_err();
        assert_eq!(
            failure.error.kind(),
            JobErrorKind::InvalidRequest,
            "{name:?}"
        );
        assert!(!output.parent().unwrap().exists());
    }
    for members in [
        vec![],
        vec![
            PackageMember::new("tileset.json", &absent),
            PackageMember::new("tileset.json", &absent),
        ],
    ] {
        assert_eq!(
            package(
                PackageRequest::members(members, work.path().join("no/out.3tz")),
                &RunControl::default()
            )
            .unwrap_err()
            .error
            .kind(),
            JobErrorKind::InvalidRequest
        );
    }
    let long_name = "x".repeat(usize::from(u16::MAX) + 1);
    let members = vec![
        PackageMember::new("tileset.json", &absent),
        PackageMember::new(long_name, &absent),
    ];
    assert_eq!(
        package(
            PackageRequest::members(members, work.path().join("no/out.3tz")),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidRequest
    );
    let output = work.path().join("no/out.3tz");
    assert_eq!(
        package(
            PackageRequest::directory(&absent, &output),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::Io
    );
    assert!(!output.parent().unwrap().exists());
    assert_eq!(
        package(
            PackageRequest::directory(absent, work.path().join("invalid.zip")),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidRequest
    );
}

#[test]
fn source_tree_overlap_reserved_index_and_hardlink_alias_are_rejected_read_only() {
    let work = tempfile::tempdir().unwrap();
    let root = source(work.path());
    assert_eq!(
        package(
            PackageRequest::directory(&root, root.join("new/out.3tz")),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidRequest
    );
    assert!(!root.join("new").exists());
    let alias = work.path().join("alias.3tz");
    fs::hard_link(root.join("tileset.json"), &alias).unwrap();
    let before = fs::read(&alias).unwrap();
    let members = vec![PackageMember::new(
        "tileset.json",
        root.join("tileset.json"),
    )];
    assert_eq!(
        package(
            PackageRequest::members(members, &alias).with_policy(OutputPolicy::Replace),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidRequest
    );
    assert_eq!(fs::read(&alias).unwrap(), before);
    fs::write(root.join("@3dtilesIndex1@"), b"stale extracted index").unwrap();
    let output = work.path().join("new/out.3tz");
    assert_eq!(
        package(
            PackageRequest::directory(root, &output),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidRequest
    );
    assert!(!output.parent().unwrap().exists());
}

#[cfg(unix)]
#[test]
fn sparse_oversized_member_rejected_before_output_parent_creation() {
    let work = tempfile::tempdir().unwrap();
    let root = source(work.path());
    let huge = root.join("huge.bin");
    fs::File::create(&huge)
        .unwrap()
        .set_len(u64::from(u32::MAX))
        .unwrap();
    let output = work.path().join("new/out.3tz");
    assert_eq!(
        package(
            PackageRequest::directory(root, &output),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidInput
    );
    assert!(!output.parent().unwrap().exists());
}

#[cfg(unix)]
#[test]
fn symlink_roots_members_traversal_and_nonregular_entries_are_rejected() {
    use std::os::unix::fs::symlink;
    let work = tempfile::tempdir().unwrap();
    let root = source(work.path());
    let link = work.path().join("link");
    symlink(&root, &link).unwrap();
    for input in [&link, &link.join("tileset.json")] {
        let out = work.path().join("never/out.3tz");
        assert_eq!(
            package(
                PackageRequest::directory(input, &out),
                &RunControl::default()
            )
            .unwrap_err()
            .error
            .kind(),
            JobErrorKind::InvalidInput
        );
        assert!(!out.parent().unwrap().exists());
    }
    let leaf = root.join("linked.bin");
    symlink(root.join("tileset.json"), &leaf).unwrap();
    let out = work.path().join("never/out.3tz");
    assert_eq!(
        package(
            PackageRequest::directory(&root, &out),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidInput
    );
    let members = vec![PackageMember::new("tileset.json", leaf.clone())];
    assert_eq!(
        package(
            PackageRequest::members(members, &out),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidInput
    );
    fs::remove_file(leaf).unwrap();
    let directory_link = root.join("linked-dir");
    symlink(work.path(), &directory_link).unwrap();
    assert_eq!(
        package(
            PackageRequest::directory(&root, &out),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidInput
    );
    fs::remove_file(directory_link).unwrap();
    let socket = std::os::unix::net::UnixListener::bind(root.join("nonregular.sock")).unwrap();
    assert_eq!(
        package(
            PackageRequest::directory(&root, &out),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidInput
    );
    drop(socket);
    fs::remove_file(root.join("nonregular.sock")).unwrap();
    assert!(!out.parent().unwrap().exists());
}

// APFS rejects this raw-byte filename during creation. Keep the fixture on Linux;
// production UTF-8 validation still applies wherever such filenames can exist.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_names_are_rejected_before_output_parent_creation() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let work = tempfile::tempdir().unwrap();
    let root = source(work.path());
    let out = work.path().join("never/out.3tz");
    fs::write(root.join(OsString::from_vec(vec![0xff])), b"bytes").unwrap();
    assert_eq!(
        package(
            PackageRequest::directory(root, &out),
            &RunControl::default()
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidInput
    );
    assert!(!out.parent().unwrap().exists());
}

#[test]
fn late_observer_failure_and_source_change_preserve_previous_output() {
    let work = tempfile::tempdir().unwrap();
    let root = source(work.path());
    let output = work.path().join("out.3tz");
    fs::write(&output, b"previous").unwrap();
    let run = control(|event| {
        if matches!(
            event,
            RunEvent::Progress {
                phase: "ready_to_publish",
                ..
            }
        ) {
            Err(JobError::new(
                JobErrorKind::ObserverFailure,
                "injected final observer fault",
            ))
        } else {
            Ok(())
        }
    });
    assert_eq!(
        package(
            PackageRequest::directory(&root, &output).with_policy(OutputPolicy::Replace),
            &run
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::ObserverFailure
    );
    assert_eq!(fs::read(&output).unwrap(), b"previous");
    assert_eq!(names(work.path()), vec!["out.3tz", "source"]);
    let manifest = root.join("tileset.json");
    let changed = Arc::new(AtomicBool::new(false));
    let flag = changed.clone();
    let run = control(move |_| {
        if !flag.swap(true, Ordering::SeqCst) {
            // Detect a size change without relying on filesystem timestamp precision.
            fs::write(&manifest, vec![0xa5; 4096]).unwrap();
        }
        Ok(())
    });
    assert_eq!(
        package(
            PackageRequest::directory(root, &output).with_policy(OutputPolicy::Replace),
            &run
        )
        .unwrap_err()
        .error
        .kind(),
        JobErrorKind::InvalidInput
    );
    assert_eq!(fs::read(&output).unwrap(), b"previous");
    assert_eq!(names(work.path()), vec!["out.3tz", "source"]);
}

#[test]
fn source_read_failure_retains_the_source_path_and_previous_destination() {
    let work = tempfile::tempdir().unwrap();
    let root = source(work.path());
    let manifest = root.join("tileset.json");
    let expected_path = fs::canonicalize(&manifest).unwrap();
    let output = work.path().join("out.3tz");
    fs::write(&output, b"previous").unwrap();
    let removed = AtomicBool::new(false);
    // Deliberately delete after read-only resolution to exercise a real read failure.
    let run = control(move |_| {
        if !removed.swap(true, Ordering::SeqCst) {
            fs::remove_file(&manifest).unwrap();
        }
        Ok(())
    });
    let error = package(
        PackageRequest::directory(root, &output).with_policy(OutputPolicy::Replace),
        &run,
    )
    .unwrap_err();
    assert_eq!(error.error.kind(), JobErrorKind::Io);
    assert_eq!(error.error.path(), Some(expected_path.as_path()));
    assert_eq!(fs::read(&output).unwrap(), b"previous");
    assert_eq!(names(work.path()), vec!["out.3tz", "source"]);
}

#[test]
fn competing_output_is_preserved_and_cancelled_run_cannot_be_reused() {
    let work = tempfile::tempdir().unwrap();
    let root = source(work.path());
    let output = work.path().join("out.3tz");
    let raced = output.clone();
    let run = control(move |event| {
        if matches!(
            event,
            RunEvent::Progress {
                phase: "ready_to_publish",
                ..
            }
        ) {
            fs::write(&raced, b"competitor").unwrap();
        }
        Ok(())
    });
    assert_eq!(
        package(PackageRequest::directory(&root, &output), &run)
            .unwrap_err()
            .error
            .kind(),
        JobErrorKind::Conflict
    );
    assert_eq!(fs::read(output).unwrap(), b"competitor");
    let run = RunControl::default();
    assert!(run.cancellation_handle().cancel());
    let output = work.path().join("never/out.3tz");
    assert_eq!(
        package(PackageRequest::directory(&root, &output), &run)
            .unwrap_err()
            .error
            .kind(),
        JobErrorKind::Cancelled
    );
    assert!(!output.parent().unwrap().exists());
    assert_eq!(
        package(PackageRequest::directory(root, output), &run)
            .unwrap_err()
            .error
            .kind(),
        JobErrorKind::InvalidState
    );
}

#[test]
fn explicit_replace_commits_an_independently_readable_archive() {
    let work = tempfile::tempdir().unwrap();
    let root = source(work.path());
    let output = work.path().join("out.3tz");
    fs::write(&output, b"previous").unwrap();
    let expected = BTreeMap::from([(
        "tileset.json".to_owned(),
        fs::read(root.join("tileset.json")).unwrap(),
    )]);
    package(
        PackageRequest::directory(root, &output).with_policy(OutputPolicy::Replace),
        &RunControl::default(),
    )
    .unwrap();
    inspect(&output, &expected);
}
