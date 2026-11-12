use super::*;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn record(pid: u32, start: Option<&str>, host: Option<&str>) -> OwnerRecord {
    OwnerRecord {
        format: OWNER_FORMAT.into(),
        version: OWNER_VERSION,
        pid,
        process_start: start.map(Into::into),
        host: host.map(Into::into),
        command: "rusty-tiles validate big.3tz".into(),
        rusty_tiles: "0.4.1".into(),
        created_unix_seconds: 1_700_000_000,
    }
}

/// A folder named `name` in `root` holding `bytes` of data and, when given, `owner`.
fn folder(root: &Path, name: &str, bytes: usize, owner: Option<&OwnerRecord>) -> PathBuf {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    fs::write(path.join("run-0.spill"), vec![7u8; bytes]).unwrap();
    if let Some(owner) = owner {
        write_owner(&path, owner).unwrap();
    }
    path
}

/// PID 100 runs with start `s100`; PID 200 runs with start `other`; every other PID is gone.
fn presence(pid: u32) -> Presence {
    match pid {
        100 => Presence::Present {
            start: Some("s100".into()),
        },
        200 => Presence::Present {
            start: Some("other".into()),
        },
        300 => Presence::Present { start: None },
        _ => Presence::Absent,
    }
}

fn host() -> Option<String> {
    process::host()
}

#[test]
fn names_match_only_rusty_tiles_patterns() {
    for name in [
        "rusty-tiles-spill-Ab12Cd",
        ".tiles-mesh-000000",
        ".tiles-h16-zzzzzz",
        ".mesh-work-AAAAAA",
        ".point-work-a1b2c3",
        ".vector-work-QWERTY",
    ] {
        assert!(Kind::of_name(name).is_some(), "{name}");
    }
    for name in [
        "rusty-tiles-spill-",
        "rusty-tiles-spill-Ab12C",
        "rusty-tiles-spill-Ab12Cde",
        "rusty-tiles-spill-Ab12C-",
        "rusty-tiles-spill-Ab12C.",
        "rusty-tiles-spill-Ab12Cé",
        ".tiles-dir-AAAAAA",
        ".tiles-previous-AAAAAA",
        ".tiles-stage-AAAAAA",
        "tiles-mesh-AAAAAA",
        "scratch",
        ".tmpAAAAAA",
    ] {
        assert_eq!(Kind::of_name(name), None, "{name}");
    }
    assert_eq!(Kind::of_name("rusty-tiles-spill-x1y2z3"), Some(Kind::Spill));
}

#[test]
fn owner_record_round_trips_and_has_the_documented_fields() {
    let dir = tempfile::tempdir().unwrap();
    let mine = OwnerRecord::current();
    write_owner(dir.path(), &mine).unwrap();
    assert_eq!(read_owner(dir.path()), Ok(mine.clone()));
    assert!(!dir.path().join(OWNER_PARTIAL).exists());
    assert_eq!(mine.pid, std::process::id());
    assert_eq!(mine.rusty_tiles, env!("CARGO_PKG_VERSION"));
    assert!(mine.command.len() <= COMMAND_MAX_BYTES);
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join(OWNER_FILE)).unwrap()).unwrap();
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "command",
            "createdUnixSeconds",
            "format",
            "host",
            "pid",
            "processStart",
            "rustyTiles",
            "version"
        ]
    );
    assert_eq!(value["format"], OWNER_FORMAT);
    assert_eq!(value["version"], 1);
    // The current process is live by its own record.
    assert_eq!(
        judge(&mine, host().as_deref(), process::query),
        if mine.process_start.is_some() {
            (State::Live, Reason::OwnerRunning)
        } else {
            (State::Unverified, Reason::StartUnverifiable)
        }
    );
}

#[test]
fn malformed_missing_and_foreign_records_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read_owner(dir.path()), Err(RecordStatus::Missing));
    let path = dir.path().join(OWNER_FILE);
    fs::write(&path, b"{not json").unwrap();
    assert_eq!(read_owner(dir.path()), Err(RecordStatus::Invalid));
    let mut other = serde_json::to_value(record(1, None, None)).unwrap();
    other["format"] = "someone-else".into();
    fs::write(&path, other.to_string()).unwrap();
    assert_eq!(read_owner(dir.path()), Err(RecordStatus::Invalid));
    let mut newer = serde_json::to_value(record(1, None, None)).unwrap();
    newer["version"] = 2.into();
    fs::write(&path, newer.to_string()).unwrap();
    assert_eq!(read_owner(dir.path()), Err(RecordStatus::Unsupported));
    // Unknown fields from a later minor addition are tolerated.
    let mut extended = serde_json::to_value(record(1, None, None)).unwrap();
    extended["extra"] = true.into();
    fs::write(&path, extended.to_string()).unwrap();
    assert_eq!(read_owner(dir.path()), Ok(record(1, None, None)));
    fs::write(&path, vec![b' '; OWNER_MAX_BYTES as usize + 1]).unwrap();
    assert_eq!(read_owner(dir.path()), Err(RecordStatus::Invalid));
    #[cfg(unix)]
    {
        let elsewhere = tempfile::tempdir().unwrap();
        let target = elsewhere.path().join("record.json");
        write_owner(elsewhere.path(), &record(1, None, None)).unwrap();
        fs::rename(elsewhere.path().join(OWNER_FILE), &target).unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert_eq!(read_owner(dir.path()), Err(RecordStatus::Invalid));
    }
}

#[test]
fn liveness_needs_the_pid_and_its_start_time() {
    let local = Some("here");
    let verdict = |owner: OwnerRecord| judge(&owner, local, presence);
    assert_eq!(
        verdict(record(100, Some("s100"), Some("here"))),
        (State::Live, Reason::OwnerRunning)
    );
    assert_eq!(
        verdict(record(100, Some("s100"), None)),
        (State::Live, Reason::OwnerRunning)
    );
    // PID 100 was reused by a process that started later.
    assert_eq!(
        verdict(record(100, Some("s099"), Some("here"))),
        (State::Stale, Reason::PidReused)
    );
    assert_eq!(
        verdict(record(4242, Some("s4242"), Some("here"))),
        (State::Stale, Reason::OwnerExited)
    );
    // An owner on another host cannot be checked from here.
    assert_eq!(
        verdict(record(4242, Some("s4242"), Some("there"))),
        (State::Unverified, Reason::OtherHost)
    );
    // A PID that exists without a comparable start time is never proved gone.
    assert_eq!(
        verdict(record(300, Some("s300"), Some("here"))),
        (State::Unverified, Reason::StartUnverifiable)
    );
    assert_eq!(
        verdict(record(100, None, Some("here"))),
        (State::Unverified, Reason::StartUnverifiable)
    );
    assert_eq!(
        verdict(record(4242, None, Some("here"))),
        (State::Stale, Reason::OwnerExited)
    );
}

#[test]
fn create_writes_a_record_only_outside_other_scratch_folders() {
    let root = tempfile::tempdir().unwrap();
    let outer = create(root.path(), Kind::MeshWork).unwrap();
    let name = outer.path().file_name().unwrap().to_str().unwrap();
    assert_eq!(Kind::of_name(name), Some(Kind::MeshWork));
    assert_eq!(read_owner(outer.path()).unwrap().pid, std::process::id());
    let inner = create(outer.path(), Kind::MeshForest).unwrap();
    let spill = create(&inner.path().join("."), Kind::Spill).unwrap();
    assert_eq!(read_owner(inner.path()), Err(RecordStatus::Missing));
    assert_eq!(read_owner(spill.path()), Err(RecordStatus::Missing));
    let entries: Vec<_> = fs::read_dir(outer.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries.len(), 2, "{entries:?}");
    drop((spill, inner));
    let path = outer.path().to_owned();
    drop(outer);
    assert!(!path.exists());
}

fn by_name(inventory: &[Folder]) -> Vec<(String, State, Reason)> {
    inventory
        .iter()
        .map(|folder| {
            (
                folder
                    .location
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                folder.state,
                folder.reason,
            )
        })
        .collect()
}

/// A root with one folder per case, plus foreign entries that must never be touched.
struct Fixture {
    root: tempfile::TempDir,
    outside: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let here = host();
    let here = here.as_deref();
    let r = root.path();
    folder(
        r,
        "rusty-tiles-spill-LIVE00",
        10,
        Some(&record(100, Some("s100"), here)),
    );
    folder(
        r,
        "rusty-tiles-spill-GONE00",
        1000,
        Some(&record(4242, Some("s1"), here)),
    );
    folder(
        r,
        ".tiles-mesh-REUSED",
        200,
        Some(&record(200, Some("s200"), here)),
    );
    folder(
        r,
        ".point-work-ELSEWH",
        30,
        Some(&record(4242, Some("s1"), Some("other-host-name"))),
    );
    folder(r, ".vector-work-NOREC0", 5, None);
    // Foreign names that look close: never listed.
    folder(
        r,
        "rusty-tiles-spill-TOOLONG",
        7,
        Some(&record(4242, None, here)),
    );
    folder(r, ".tiles-dir-AAAAAA", 7, Some(&record(4242, None, here)));
    folder(r, "keep-me", 7, None);
    fs::write(r.join("rusty-tiles-spill-FILE00"), b"a file, not a folder").unwrap();
    // A stale folder holding a symlink out of itself; the target must survive.
    fs::write(outside.path().join("precious.bin"), vec![1u8; 4096]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(
            outside.path(),
            r.join("rusty-tiles-spill-GONE00").join("escape"),
        )
        .unwrap();
        symlink(
            outside.path().join("precious.bin"),
            r.join("rusty-tiles-spill-GONE00").join("escape.bin"),
        )
        .unwrap();
        // A symlink named like a scratch folder, pointing at a stale-looking folder.
        let decoy = folder(
            outside.path(),
            "rusty-tiles-spill-DECOY0",
            64,
            Some(&record(4242, Some("s1"), here)),
        );
        symlink(&decoy, r.join("rusty-tiles-spill-LINK00")).unwrap();
    }
    Fixture { root, outside }
}

#[test]
fn list_reports_live_stale_and_unverified_folders_and_ignores_foreign_entries() {
    let fixture = fixture();
    let roots = [(fixture.root.path().to_owned(), true)];
    let now = SystemTime::now();
    let inventory = scan(&roots, OWNERLESS_STALE_AFTER, now, &presence);
    assert_eq!(
        by_name(&inventory.folders),
        [
            (
                ".point-work-ELSEWH".into(),
                State::Unverified,
                Reason::OtherHost
            ),
            (".tiles-mesh-REUSED".into(), State::Stale, Reason::PidReused),
            (
                ".vector-work-NOREC0".into(),
                State::Unverified,
                Reason::NoOwnerRecord
            ),
            (
                "rusty-tiles-spill-GONE00".into(),
                State::Stale,
                Reason::OwnerExited
            ),
            (
                "rusty-tiles-spill-LIVE00".into(),
                State::Live,
                Reason::OwnerRunning
            ),
        ]
    );
    let gone = &inventory.folders[3];
    // The data file and the record; symlink targets are not counted.
    let record_len = fs::metadata(
        fixture
            .root
            .path()
            .join("rusty-tiles-spill-GONE00")
            .join(OWNER_FILE),
    )
    .unwrap()
    .len();
    assert_eq!(gone.bytes, 1000 + record_len);
    assert!(gone.size_complete);
    assert_eq!(gone.owner_record, RecordStatus::Present);
    assert_eq!(gone.owner.as_ref().unwrap().pid, 4242);
    assert_eq!(inventory.folders[2].owner_record, RecordStatus::Missing);
    assert_eq!(inventory.stale().0, 2);
    // A day later the ownerless folder is stale too.
    let later = scan(&roots, OWNERLESS_STALE_AFTER, now + DAY + DAY, &presence);
    assert_eq!(later.folders[2].state, State::Stale);
    assert_eq!(later.stale().0, 3);
    // A missing root is reported, not an error.
    let missing = scan(
        &[(fixture.root.path().join("absent"), true)],
        DAY,
        now,
        &presence,
    );
    assert!(!missing.roots[0].exists && missing.roots[0].error.is_none());
    assert!(missing.folders.is_empty());
}

#[test]
fn clean_removes_only_stale_folders_and_never_follows_symlinks() {
    let fixture = fixture();
    let r = fixture.root.path();
    let roots = [(r.to_owned(), true)];
    let now = SystemTime::now;
    let dry = clean_with(&roots, None, true, now, &presence);
    assert!(dry.dry_run);
    assert_eq!(dry.removed.len(), 2);
    assert!(r.join("rusty-tiles-spill-GONE00").exists() && r.join(".tiles-mesh-REUSED").exists());
    let report = clean_with(&roots, None, false, now, &presence);
    assert_eq!(dry.bytes_freed, report.bytes_freed);
    let mut removed: Vec<_> = report.removed.iter().map(|f| f.location.clone()).collect();
    removed.sort();
    assert_eq!(
        removed,
        [
            r.join(".tiles-mesh-REUSED"),
            r.join("rusty-tiles-spill-GONE00")
        ]
    );
    assert!(report.failed.is_empty());
    assert!(report.bytes_freed >= 1200);
    for name in [
        "rusty-tiles-spill-LIVE00",
        ".point-work-ELSEWH",
        ".vector-work-NOREC0",
        "rusty-tiles-spill-TOOLONG",
        ".tiles-dir-AAAAAA",
        "keep-me",
        "rusty-tiles-spill-FILE00",
    ] {
        assert!(r.join(name).exists(), "{name} was removed");
    }
    assert!(!r.join("rusty-tiles-spill-GONE00").exists());
    assert_eq!(
        fs::read(fixture.outside.path().join("precious.bin"))
            .unwrap()
            .len(),
        4096
    );
    #[cfg(unix)]
    {
        assert!(fs::symlink_metadata(r.join("rusty-tiles-spill-LINK00")).is_ok());
        assert!(fixture
            .outside
            .path()
            .join("rusty-tiles-spill-DECOY0")
            .join(OWNER_FILE)
            .exists());
    }
    let kept = by_name(&report.kept);
    assert!(kept.contains(&(
        "rusty-tiles-spill-LIVE00".into(),
        State::Live,
        Reason::OwnerRunning
    )));
}

#[test]
fn older_than_filters_every_folder_and_sets_the_ownerless_threshold() {
    let fixture = fixture();
    let r = fixture.root.path();
    let roots = [(r.to_owned(), true)];
    // Nothing is a week old yet.
    let report = clean_with(&roots, Some(7 * DAY), false, SystemTime::now, &presence);
    assert!(report.removed.is_empty());
    assert_eq!(report.older_than_seconds, Some(7 * DAY.as_secs()));
    // Two hours on, a one-hour threshold also takes the ownerless folder.
    let later = || SystemTime::now() + Duration::from_secs(2 * 60 * 60);
    let report = clean_with(
        &roots,
        Some(Duration::from_secs(3600)),
        false,
        later,
        &presence,
    );
    let mut removed: Vec<_> = by_name(&report.removed).into_iter().map(|f| f.0).collect();
    removed.sort();
    assert_eq!(
        removed,
        [
            ".tiles-mesh-REUSED",
            ".vector-work-NOREC0",
            "rusty-tiles-spill-GONE00"
        ]
    );
    assert!(r.join("rusty-tiles-spill-LIVE00").exists());
}

#[test]
fn clean_rechecks_the_owner_before_removing() {
    let root = tempfile::tempdir().unwrap();
    let path = folder(
        root.path(),
        "rusty-tiles-spill-RACE00",
        10,
        Some(&record(4242, Some("s1"), host().as_deref())),
    );
    let calls = std::cell::Cell::new(0);
    // Gone at the scan, running again (a resumed owner) at the recheck.
    let flipping = |pid: u32| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            Presence::Absent
        } else {
            Presence::Present {
                start: Some(if pid == 4242 { "s1" } else { "x" }.into()),
            }
        }
    };
    let report = clean_with(
        &[(root.path().to_owned(), true)],
        None,
        false,
        SystemTime::now,
        &flipping,
    );
    assert!(report.removed.is_empty());
    assert_eq!(by_name(&report.kept)[0].1, State::Live);
    assert!(path.exists());
}

#[test]
fn reports_serialise_with_the_documented_keys() {
    let fixture = fixture();
    let roots = [(fixture.root.path().to_owned(), true)];
    let inventory = scan(&roots, OWNERLESS_STALE_AFTER, SystemTime::now(), &presence);
    let value = serde_json::to_value(&inventory).unwrap();
    let keys = |value: &serde_json::Value| {
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    assert_eq!(
        keys(&value),
        ["folders", "ownerlessStaleAfterSeconds", "roots"]
    );
    assert_eq!(
        keys(&value["roots"][0]),
        ["error", "exists", "explicit", "path"]
    );
    let folder = &value["folders"][3];
    assert_eq!(
        keys(folder),
        [
            "bytes",
            "kind",
            "modifiedSecondsAgo",
            "owner",
            "ownerRecord",
            "path",
            "reason",
            "sizeComplete",
            "state"
        ]
    );
    assert_eq!(folder["kind"], "spill");
    assert_eq!(folder["state"], "stale");
    assert_eq!(folder["reason"], "owner_exited");
    assert_eq!(folder["ownerRecord"], "present");
    assert_eq!(value["folders"][2]["owner"], serde_json::Value::Null);
    assert_eq!(value["folders"][2]["ownerRecord"], "missing");
    let report = clean_with(&roots, None, true, SystemTime::now, &presence);
    assert_eq!(
        keys(&serde_json::to_value(&report).unwrap()),
        [
            "bytesFreed",
            "dryRun",
            "failed",
            "kept",
            "olderThanSeconds",
            "ownerlessStaleAfterSeconds",
            "removed",
            "roots"
        ]
    );
}

#[test]
fn durations_and_sizes_parse_and_print() {
    assert_eq!(parse_duration("90"), Ok(Duration::from_secs(90)));
    assert_eq!(parse_duration("90s"), Ok(Duration::from_secs(90)));
    assert_eq!(parse_duration("30m"), Ok(Duration::from_secs(1800)));
    assert_eq!(parse_duration("24h"), Ok(DAY));
    assert_eq!(parse_duration("7d"), Ok(7 * DAY));
    assert_eq!(parse_duration("2w"), Ok(14 * DAY));
    assert_eq!(parse_duration("0"), Ok(Duration::ZERO));
    for bad in ["", "h", "1.5h", "-1d", "1y", "1 d", "99999999999999999999w"] {
        assert!(parse_duration(bad).is_err(), "{bad}");
    }
    assert_eq!(human_bytes(0), "0 B");
    assert_eq!(human_bytes(1023), "1023 B");
    assert_eq!(human_bytes(1536), "1.5 KiB");
    assert_eq!(human_bytes(154_510_000_000), "143.9 GiB");
}
