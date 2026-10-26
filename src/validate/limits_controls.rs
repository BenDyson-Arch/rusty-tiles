//! VS-C controls: the limit surface. Report-level derivation at the budget crossovers (K1),
//! at-cap pairs and overrides below the floor (K2, K3), opt-in caps (K4), determinism (K6),
//! the typed scratch refusal through the public facade (K8), the cancellation bound (K9) and
//! the explicit scratch directory (K10).
use super::*;
use crate::archive3tz::FORCED;
use crate::{CancellationHandle, Observer, RunControl, RunEvent};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// POINTS at the origin, `count` of them, with `pad` zero bytes (a multiple of 4) after the
/// accessor data in the binary chunk.
fn points(count: usize, pad: usize) -> Vec<u8> {
    let mut bin = vec![0u8; 12 * count + pad];
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let doc = json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "buffers":[{"byteLength":bin.len()}],
        "bufferViews":[{"buffer":0,"byteLength":12 * count}],
        "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":count,
            "min":[0,0,0],"max":[0,0,0]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}]
    });
    let mut json = serde_json::to_vec(&doc).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut out = b"glTF".to_vec();
    out.extend(2u32.to_le_bytes());
    out.extend((28 + json.len() as u32 + bin.len() as u32).to_le_bytes());
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    out
}
fn tile(half: f64, error: f64) -> Value {
    json!({"boundingVolume":{"box":[0,0,0,half,0,0,0,half,0,0,0,half]},"geometricError":error})
}
fn tileset(root: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"asset":{"version":"1.1"},"geometricError":1000,"root":root}))
        .unwrap()
}
type Files = Vec<(&'static str, Vec<u8>)>;
fn package(files: &Files) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let data = work.path().join("data");
    for (name, bytes) in files {
        let file = data.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    let output = work.path().join("test.3tz");
    crate::package::package(
        crate::package::PackageRequest::directory(&data, &output),
        &RunControl::default(),
    )
    .unwrap();
    (work, output)
}
/// S read by the independent `zip` crate: every central-directory entry's stored size.
fn stored(path: &Path) -> u64 {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| zip.by_index_raw(i).unwrap().compressed_size())
        .sum()
}
/// N and the central-directory length from the (comment-free, non-ZIP64) end record.
fn directory(path: &Path) -> (u64, u64) {
    let bytes = std::fs::read(path).unwrap();
    let end = &bytes[bytes.len() - 22..];
    assert_eq!(&end[..4], b"PK\x05\x06");
    let entries = u16::from_le_bytes([end[10], end[11]]);
    let size = u32::from_le_bytes(end[12..16].try_into().unwrap());
    (u64::from(entries), u64::from(size))
}
/// One tile with one payload: the base archive of the controls.
fn small() -> Files {
    let mut root = tile(10., 0.);
    root["content"] = json!({"uri":"p.glb"});
    vec![("tileset.json", tileset(root)), ("p.glb", points(3, 0))]
}
/// An archive whose S is exactly `target`: a padded binary chunk for the bulk, tileset
/// whitespace for the remainder.
fn sized(target: u64) -> (tempfile::TempDir, PathBuf) {
    let build = |pad: usize, spaces: usize| {
        let mut files = small();
        files[0].1.extend(std::iter::repeat_n(b' ', spaces));
        files[1].1 = points(3, pad);
        package(&files)
    };
    let base = stored(&build(0, 0).1);
    let mut pad = (target.saturating_sub(base + 4096) / 4 * 4) as usize;
    loop {
        let measured = stored(&build(pad, 0).1);
        if measured <= target {
            let archive = build(pad, (target - measured) as usize);
            assert_eq!(stored(&archive.1), target);
            return archive;
        }
        pad -= 4096;
    }
}
fn plan(overrides: &[(LimitKey, u64)]) -> Budgets {
    let mut plan = LimitPlan::default();
    for &(key, value) in overrides {
        plan.set(key, value);
    }
    Budgets::Plan(plan)
}
fn run(path: &Path, overrides: &[(LimitKey, u64)]) -> Result<Inspection, Error> {
    inspect_facts(ValidationRequest::new(path), plan(overrides))
}
/// The limit members of a report as JSON: `storedBytes`, `limits`, `limitDerivation`.
fn limit_members(report: &ValidationReport) -> String {
    let value = report.summary_json().unwrap();
    json!([
        value["storedBytes"],
        value["limits"],
        value["limitDerivation"]
    ])
    .to_string()
}
fn refused(result: Result<Inspection, Error>) -> (LimitExceeded, String) {
    match result {
        Err(error) => (*error.exceeded().unwrap(), error.to_string()),
        Ok(_) => panic!("expected a limit refusal"),
    }
}

#[test]
fn limit_keys_round_trip_and_class_a_keys_are_refused() {
    for key in LimitKey::ALL {
        assert_eq!(key.as_str().parse::<LimitKey>().unwrap(), key);
    }
    assert_eq!(LimitKey::ALL.iter().filter(|key| key.is_cap()).count(), 4);
    for fixed in [
        "jsonBytes",
        "memberBytes",
        "liveDocumentBytes",
        "hierarchyDepth",
        "",
    ] {
        assert!(fixed.parse::<LimitKey>().is_err(), "{fixed}");
    }
}

#[test]
fn derivation_is_explicit_scaled_or_floor_with_ties_at_the_floor() {
    let (limits, derivation, _) = LimitPlan::default().resolve(0);
    assert_eq!(derivation.model, 1);
    assert_eq!(derivation.budgets.len(), 7);
    assert!(derivation
        .budgets
        .values()
        .all(|rule| rule.source == Derivation::Floor));
    assert_eq!(
        (
            limits.source_archive_bytes,
            limits.archive_entries,
            limits.central_directory_bytes,
            limits.archive_stored_bytes
        ),
        (None, None, None, None)
    );
    // The tie ⌊S/8⌋ = floor stays "floor"; one rule step above is "scaled".
    let at = |stored: u64| LimitPlan::default().resolve(stored);
    let visits = |stored: u64| at(stored).1.budgets["hierarchyVisits"];
    assert_eq!(visits(8 << 18).source, Derivation::Floor);
    assert_eq!(visits(8 << 18).per_stored_byte, [1, 8]);
    assert_eq!(visits((8 << 18) + 8).source, Derivation::Scaled);
    assert_eq!(at((8 << 18) + 8).0.hierarchy_visits, (1 << 18) + 1);
    // An explicit value replaces the rule, below the floor and above κ·S alike, and keeps
    // the model's rule in the derivation.
    let mut explicit = LimitPlan::default();
    explicit.set(LimitKey::HierarchyVisits, 5);
    explicit.set(LimitKey::ArchiveEntries, 7);
    let (limits, derivation, sources) = explicit.resolve(1 << 40);
    assert_eq!(limits.hierarchy_visits, 5);
    assert_eq!(limits.archive_entries, Some(7));
    assert_eq!(
        derivation.budgets["hierarchyVisits"],
        BudgetRule {
            floor: 1 << 18,
            per_stored_byte: [1, 8],
            source: Derivation::Explicit
        }
    );
    assert_eq!(sources.of(LimitKey::ArchiveEntries), Derivation::Cap);
    assert_eq!(limits.references, (1u64 << 40) / 8);
    assert_eq!(derivation.budgets["references"].source, Derivation::Scaled);
    // Class A never moves.
    assert_eq!(limits.json_bytes, ValidationLimits::default().json_bytes);
    assert_eq!(
        limits.live_document_bytes,
        ValidationLimits::default().live_document_bytes
    );
}

/// K1: at report level, `storedBytes` is S and each budget is `max(floor, ⌊S·p/q⌋)` with its
/// source, one below the tie, at the tie and at the first scaled S.
#[test]
fn report_derivation_at_the_crossovers() {
    for (key, floor, p, q) in [
        (LimitKey::TotalPayloadElements, 16_000_000u64, 16u64, 1u64),
        (LimitKey::HierarchyVisits, 1 << 18, 1, 8),
        (LimitKey::ContainmentWorkUnits, 1 << 29, 128, 1),
        (LimitKey::References, 1 << 20, 1, 8),
    ] {
        let tie = floor * q / p;
        let scaled = (floor + 1) * q / p + u64::from((floor + 1) * q % p != 0);
        for (stored_bytes, value, source) in [
            (tie - 1, floor, "floor"),
            (tie, floor, "floor"),
            (scaled, scaled * p / q, "scaled"),
        ] {
            let (_work, path) = sized(stored_bytes);
            let report = inspect(ValidationRequest::new(&path)).unwrap();
            let summary = report.summary_json().unwrap();
            assert_eq!(summary["storedBytes"], stored_bytes, "{key}");
            assert_eq!(
                summary["limits"][key.as_str()],
                value,
                "{key} at {stored_bytes}"
            );
            assert_eq!(
                summary["limitDerivation"]["budgets"][key.as_str()],
                json!({"floor": floor, "perStoredByte": [p, q], "source": source}),
                "{key} at {stored_bytes}"
            );
            assert!(summary["limits"]["archiveEntries"].is_null());
            assert_eq!(summary["limits"]["liveDocumentBytes"], 268_435_456);
        }
    }
}

/// K2, K3: each validator-charged budget passes at exactly its use and is refused one below
/// it (below the floor too), with the structured cause and the `explicit` suffix; the C2
/// budgets become `notInspected` instead.
#[test]
fn explicit_budgets_pass_at_use_and_refuse_one_below() {
    let (_work, path) = package(&small());
    let used = run(&path, &[]).unwrap();
    let [visits, references, bytes_read] = used.counters;
    for (key, value) in [
        (LimitKey::HierarchyVisits, visits),
        (LimitKey::References, references),
        (LimitKey::TotalBytesRead, bytes_read),
        (LimitKey::TotalPayloadElements, used.elements),
    ] {
        assert!(value > 0, "{key}");
        let at = run(&path, &[(key, value)]).unwrap();
        assert_eq!(at.report.limits.get(key), Some(value));
        assert_eq!(
            at.report.limit_derivation.budgets[key.as_str()].source,
            Derivation::Explicit
        );
        let (limit, message) = refused(run(&path, &[(key, value - 1)]));
        assert_eq!(
            limit,
            LimitExceeded {
                limit: key,
                value: value - 1,
                observed: value,
                derivation: Derivation::Explicit
            }
        );
        assert!(
            message.ends_with(&format!(" (limit {key}={}, explicit)", value - 1)),
            "{message}"
        );
    }
    // 0 refuses at the first charge.
    assert_eq!(
        refused(run(&path, &[(LimitKey::TotalBytesRead, 0)]))
            .0
            .limit,
        LimitKey::TotalBytesRead
    );
    let contained = |inspection: &Inspection| {
        inspection
            .report
            .checks
            .iter()
            .any(|check| check == "decodedContentContainment")
    };
    assert!(contained(&used) && used.units > 0);
    let at = run(&path, &[(LimitKey::ContainmentWorkUnits, used.units)]).unwrap();
    assert!(contained(&at));
    // (This payload materializes no decoded bytes, so `containmentDecodedBytes` 0 still fits.)
    for key in [
        LimitKey::ContainmentWorkUnits,
        LimitKey::ContainmentBytesRead,
    ] {
        let value = if key == LimitKey::ContainmentWorkUnits {
            used.units - 1
        } else {
            0
        };
        let below = run(&path, &[(key, value)]).unwrap();
        assert!(!contained(&below), "{key}");
        assert!(below
            .report
            .not_inspected
            .contains(&"decodedContentBounds.workLimit".to_owned()));
    }
}

/// K4: the four caps are unset by default, pass at the archive's exact value and refuse one
/// below it with the `cap` suffix and the observed quantity.
#[test]
fn opt_in_caps_pass_at_their_value_and_refuse_one_below() {
    let (_work, path) = package(&small());
    let default = inspect(ValidationRequest::new(&path)).unwrap();
    let summary = default.summary_json().unwrap();
    for key in LimitKey::ALL.into_iter().filter(|key| key.is_cap()) {
        assert!(summary["limits"][key.as_str()].is_null(), "{key}");
    }
    let (entries, directory_bytes) = directory(&path);
    let exact = [
        (
            LimitKey::SourceArchiveBytes,
            std::fs::metadata(&path).unwrap().len(),
        ),
        (LimitKey::ArchiveEntries, entries),
        (LimitKey::CentralDirectoryBytes, directory_bytes),
        (LimitKey::ArchiveStoredBytes, stored(&path)),
    ];
    assert_eq!(default.stored_bytes, exact[3].1);
    for (key, value) in exact {
        let at = run(&path, &[(key, value)]).unwrap();
        assert_eq!(at.report.limits.get(key), Some(value));
        let (limit, message) = refused(run(&path, &[(key, value - 1)]));
        assert_eq!(
            limit,
            LimitExceeded {
                limit: key,
                value: value - 1,
                observed: value,
                derivation: Derivation::Cap
            }
        );
        assert!(
            message.ends_with(&format!(" (limit {key}={}, cap)", value - 1)),
            "{message}"
        );
    }
}

/// K6: `storedBytes`, `limits` and `limitDerivation` do not depend on the catalog profile,
/// the scratch directory or the order of the overrides.
#[test]
fn limit_members_are_independent_of_profile_scratch_and_override_order() {
    let (_work, path) = sized(2_200_000);
    let first = [
        (LimitKey::References, 3_000_000),
        (LimitKey::ArchiveEntries, 99),
    ];
    let second = [first[1], first[0]];
    let scratch = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let mut seen = BTreeSet::new();
    for forced in [false, true] {
        for dir in &scratch {
            for overrides in [&first, &second] {
                FORCED.with(|f| f.set(Some(forced)));
                let mut request = ValidationRequest::new(&path).scratch_dir(dir.path());
                for &(key, value) in overrides {
                    request = request.max_volume(key, value);
                }
                let report = inspect(request).unwrap();
                FORCED.with(|f| f.set(None));
                assert_eq!(report.scratch.is_some(), forced);
                seen.insert(limit_members(&report));
            }
        }
    }
    assert_eq!(seen.len(), 1, "{seen:?}");
    let members: Value = serde_json::from_str(seen.first().unwrap()).unwrap();
    assert_eq!(members[0], 2_200_000);
    assert_eq!(members[1]["hierarchyVisits"], 275_000);
    assert_eq!(members[1]["references"], 3_000_000);
    assert_eq!(members[1]["archiveEntries"], 99);
}

fn memory_backed(dir: &Path) -> bool {
    crate::spill::inspect_volume(dir).is_ok_and(|volume| volume.memory_backed)
}

/// K8 (b): the public `validate_3tz` facade, in a child process whose every scratch
/// candidate is tmpfs, refuses with the typed `environment` refusal (exit class 4).
#[test]
fn public_validate_3tz_refuses_memory_backed_scratch_as_environment() {
    let shm = Path::new("/dev/shm");
    if !memory_backed(shm) {
        eprintln!("skipped: /dev/shm is not tmpfs here");
        return;
    }
    let (_work, path) = package(&small());
    let ram = tempfile::tempdir_in(shm).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "validate::limits_controls::facade_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("RUSTY_TILES_FORCED_SPILL", "1")
        .env("TMPDIR", ram.path())
        .env("XDG_CACHE_HOME", ram.path().join("cache"))
        .env("RUSTY_TILES_FACADE_INPUT", &path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FACADE environment 4 MemoryBackedOnly"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Nothing was left in RAM.
    assert!(std::fs::read_dir(ram.path())
        .unwrap()
        .all(|entry| entry.unwrap().file_name() == "cache"));
}

/// The child of the K8 facade control.
#[test]
#[ignore = "child process of public_validate_3tz_refuses_memory_backed_scratch_as_environment"]
fn facade_child() {
    let Some(path) = std::env::var_os("RUSTY_TILES_FACADE_INPUT") else {
        return;
    };
    match crate::validate_3tz(Path::new(&path)) {
        Err(error @ crate::Error::Scratch(ScratchRefusal::MemoryBackedOnly { .. })) => {
            let (category, code) = error.category();
            println!("FACADE {category} {code} MemoryBackedOnly {error}");
        }
        other => println!("FACADE unexpected {other:?}"),
    }
}

/// K8 (c), K10: an explicit scratch directory is used exactly, a memory-backed one is labelled,
/// and a missing one is an invalid request before the input is opened.
#[test]
fn explicit_scratch_directory_is_used_and_labelled() {
    let (_work, path) = package(&small());
    let dir = tempfile::tempdir().unwrap();
    let observer = Arc::new(Listing {
        dir: dir.path().to_owned(),
        seen: Mutex::new(Vec::new()),
    });
    FORCED.with(|f| f.set(Some(true)));
    let report = inspect_with(
        ValidationRequest::new(&path).scratch_dir(dir.path()),
        &RunControl::new(Some(observer.clone())),
    );
    FORCED.with(|f| f.set(None));
    let report = report.unwrap();
    assert_eq!(
        report.scratch.map(|scratch| scratch.memory_backed),
        Some(memory_backed(dir.path()))
    );
    // The spilled rows live as long as the report.
    drop(report);
    // While the rows were sorted, the only entry of the directory was the spill directory.
    let seen = observer.seen.lock().unwrap();
    assert!(!seen.is_empty());
    assert!(
        seen.iter()
            .all(|names| names.len() == 1 && names[0].starts_with("rusty-tiles-spill-")),
        "{seen:?}"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let shm = Path::new("/dev/shm");
    if memory_backed(shm) {
        let ram = tempfile::tempdir_in(shm).unwrap();
        FORCED.with(|f| f.set(Some(true)));
        let report = inspect(ValidationRequest::new(&path).scratch_dir(ram.path()));
        FORCED.with(|f| f.set(None));
        assert_eq!(
            report.unwrap().scratch.map(|scratch| scratch.memory_backed),
            Some(true)
        );
    }
    let missing = dir.path().join("missing");
    let refused =
        inspect(ValidationRequest::new(dir.path().join("absent.3tz")).scratch_dir(&missing));
    assert!(
        matches!(&refused, Err(Error::InvalidRequest(_))),
        "{refused:?}"
    );
    assert_eq!(refused.unwrap_err().category(), ("usage", 2));
}

struct Listing {
    dir: PathBuf,
    seen: Mutex<Vec<Vec<String>>>,
}
impl Observer for Listing {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), crate::JobError> {
        if let RunEvent::Progress {
            phase: "validate.rows",
            ..
        } = event
        {
            let names = std::fs::read_dir(&self.dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            self.seen.lock().unwrap().push(names);
        }
        Ok(())
    }
}

/// Cancels at the event of `phase` with `done`.
struct CancelAt {
    phase: &'static str,
    done: u64,
    handle: OnceLock<CancellationHandle>,
    events: Mutex<Vec<(String, u64, Option<u64>)>>,
}
impl Observer for CancelAt {
    fn observe(&self, event: &RunEvent<'_>) -> Result<(), crate::JobError> {
        if let RunEvent::Progress { phase, done, total } = *event {
            self.events
                .lock()
                .unwrap()
                .push((phase.to_owned(), done, total));
            if phase == self.phase && done == self.done {
                self.handle.get().unwrap().cancel();
            }
        }
        Ok(())
    }
}
fn cancelled_at(path: &Path, phase: &'static str, done: u64) -> Option<(&'static str, u64)> {
    let observer = Arc::new(CancelAt {
        phase,
        done,
        handle: OnceLock::new(),
        events: Mutex::new(Vec::new()),
    });
    let control = RunControl::new(Some(observer.clone()));
    let _ = observer.handle.set(control.cancellation_handle());
    STOPPED.with(|stopped| stopped.set(None));
    let result = inspect_with(ValidationRequest::new(path), &control);
    assert!(
        matches!(result, Err(Error::Cancelled)),
        "{phase} {done}: {:?}",
        result.map(|report| report.tiles)
    );
    assert_eq!(result.unwrap_err().category(), ("cancelled", 1));
    STOPPED.with(std::cell::Cell::get)
}

/// A 6-level constant-availability quadtree subtree: 1,365 implicit tiles in one task.
fn quadtree() -> (tempfile::TempDir, PathBuf) {
    let mut root = tile(100., 10.);
    root["refine"] = json!("REPLACE");
    root["implicitTiling"] = json!({"subdivisionScheme":"QUADTREE","subtreeLevels":6,
        "availableLevels":6,"subtrees":{"uri":"s/{level}/{x}/{y}.subtree"}});
    let subtree = {
        let mut text =
            br#"{"tileAvailability":{"constant":1},"childSubtreeAvailability":{"constant":0}}"#
                .to_vec();
        while !text.len().is_multiple_of(8) {
            text.push(b' ');
        }
        let mut out = b"subt".to_vec();
        out.extend(1u32.to_le_bytes());
        out.extend((text.len() as u64).to_le_bytes());
        out.extend(0u64.to_le_bytes());
        out.extend(text);
        out
    };
    package(&vec![
        ("tileset.json", tileset(root)),
        ("s/0/0/0.subtree", subtree),
    ])
}

/// The I1 tile reservation is a structured `hierarchyVisits` refusal whose `observed` is the
/// visits before the subtree plus its whole reservation (A-I1-1).
#[test]
fn implicit_reservation_refusals_name_the_validation_budget() {
    let (_work, path) = quadtree();
    let used = run(&path, &[]).unwrap().counters[0];
    assert!(used > 1365);
    assert!(run(&path, &[(LimitKey::HierarchyVisits, used)]).is_ok());
    // The tasks after the subtree are charged one at a time; below them the subtree's whole
    // reservation is refused, by exactly one visit at the first such value.
    let (limit, message) = (1..8)
        .map(|below| refused(run(&path, &[(LimitKey::HierarchyVisits, used - below)])))
        .find(|(_, message)| message.starts_with("implicit: "))
        .unwrap();
    assert_eq!(
        (limit.limit, limit.observed, limit.derivation),
        (
            LimitKey::HierarchyVisits,
            limit.value + 1,
            Derivation::Explicit
        )
    );
    assert!(
        message.starts_with(&format!(
            "implicit: subtree tile availability exceeds remaining hierarchy visits (limit hierarchyVisits={}, explicit)",
            limit.value
        )),
        "{message}"
    );
}

/// The phase and step at which a cancellation requested between events, at `phase` step
/// `done`, stopped the run.
fn cancelled_between(path: &Path, phase: &'static str, done: u64) -> Option<(&'static str, u64)> {
    let control = RunControl::default();
    CANCEL_AT
        .with(|cancel| *cancel.borrow_mut() = Some((phase, done, control.cancellation_handle())));
    STOPPED.with(|stopped| stopped.set(None));
    let result = inspect_with(ValidationRequest::new(path), &control);
    CANCEL_AT.with(|cancel| *cancel.borrow_mut() = None);
    assert!(matches!(result, Err(Error::Cancelled)), "{phase} {done}");
    STOPPED.with(std::cell::Cell::get)
}

/// The progress events of an uncancelled run of `path`.
fn events(path: &Path) -> Vec<(String, u64, Option<u64>)> {
    let observer = Arc::new(CancelAt {
        phase: "",
        done: u64::MAX,
        handle: OnceLock::new(),
        events: Mutex::new(Vec::new()),
    });
    let control = RunControl::new(Some(observer.clone()));
    inspect_with(ValidationRequest::new(path), &control).unwrap();
    let events = observer.events.lock().unwrap().clone();
    events
}

/// K9: a cancellation is honoured at the next charge site. From an event it stops at once
/// (the event is itself a check); between events it stops at the very step it was requested
/// before (the next hashed member, traversal task, implicit tile, phase-2 stream entry or the
/// end of the row sort).
#[test]
fn cancellation_stops_within_one_check_interval() {
    // 1 + 2,100 explicit tiles: more than two progress intervals of traversal tasks.
    let mut root = tile(100., 10.);
    root["children"] = (0..2100)
        .map(|i| tile(1. + f64::from(i) / 4096., 0.))
        .collect();
    root["children"][2099]["content"] = json!({"uri":"p.glb"});
    let (_work, explicit) = package(&vec![
        ("tileset.json", tileset(root)),
        ("p.glb", points(1, 0)),
    ]);
    for (phase, done) in [
        ("validate.hash", 0),
        ("validate.traverse", 0),
        ("validate.traverse", 1024),
        ("validate.traverse", 2048),
        ("validate.stream", 0),
        ("validate.stream", 1024),
        ("validate.stream", 2048),
        ("validate.rows", 0),
    ] {
        assert_eq!(
            cancelled_at(&explicit, phase, done),
            Some((phase, done)),
            "{phase} {done}"
        );
    }
    for (phase, done) in [
        ("validate.hash", 2),
        ("validate.traverse", 1),
        ("validate.traverse", 1500),
        ("validate.traverse", 2101),
        ("validate.stream", 1),
        ("validate.stream", 1500),
        ("validate.stream", 2102),
        ("validate.rows", 1),
    ] {
        assert_eq!(
            cancelled_between(&explicit, phase, done),
            Some((phase, done)),
            "{phase} {done}"
        );
    }
    // Phase 2 (VS-B-4) streams after traversal, one step per logged tile and reference:
    // 2,101 tiles and the one content reference.
    let phases: Vec<_> = events(&explicit)
        .into_iter()
        .filter(|(phase, done, _)| phase == "validate.stream" && *done == 0)
        .collect();
    assert_eq!(phases, [("validate.stream".into(), 0, Some(2102))]);
    // Implicit tiles: 1,365 in one task.
    let (_work, implicit) = quadtree();
    assert!(inspect(ValidationRequest::new(&implicit)).is_ok());
    assert_eq!(
        cancelled_at(&implicit, "validate.traverse", 1024),
        Some(("validate.traverse", 1024))
    );
    assert_eq!(
        cancelled_between(&implicit, "validate.traverse", 1300),
        Some(("validate.traverse", 1300))
    );
}

/// K9 for the memo (VS-B EM-5.4): the second placement of a shared document is replayed
/// inside one traversal task, so the replay is checked before each event. A cancellation
/// requested at a replayed event stops the run there, not at the next traversal task, and a
/// long replay re-emits its phase's event every 1,024 events.
#[test]
fn cancellation_stops_a_memo_replay_at_its_next_event() {
    let mut shared = tile(50., 5.);
    shared["children"] = (0..2100)
        .map(|i| tile(1. + f64::from(i) / 4096., 0.))
        .collect();
    let mut top = tile(100., 10.);
    top["children"] = json!([
        {"boundingVolume":{"box":[0,0,0,60,0,0,0,60,0,0,0,60]},"geometricError":9,
            "content":{"uri":"d.json"}},
        {"boundingVolume":{"box":[0,0,0,60,0,0,0,60,0,0,0,60]},"geometricError":8,
            "content":{"uri":"d.json"}}
    ]);
    let (_work, path) = package(&vec![
        ("tileset.json", tileset(top)),
        ("d.json", tileset(shared)),
    ]);
    CHECKS.with(|checks| checks.set(0));
    let all = events(&path);
    // Each of the shared document's 2,101 tiles has a C2 tile event and a
    // delivery-kind event; cancellation checks cover both bounded operations.
    let replayed = CHECKS.with(std::cell::Cell::get);
    assert_eq!(replayed, 4202);
    // Four heartbeats have the placing task's unchanged traversal count.
    let traverse: Vec<_> = all
        .iter()
        .filter(|(phase, _, _)| phase == "validate.traverse")
        .map(|(_, done, _)| *done)
        .collect();
    let placing = *traverse.last().unwrap();
    assert_eq!(traverse.iter().filter(|&&done| done == placing).count(), 4);
    // Each heartbeat lands on a multiple of 1,024 replayed checks.
    struct Heartbeats(Mutex<Vec<(u64, u64)>>);
    impl Observer for Heartbeats {
        fn observe(&self, event: &RunEvent<'_>) -> Result<(), crate::JobError> {
            if let RunEvent::Progress {
                phase: "validate.traverse",
                done,
                ..
            } = *event
            {
                let checks = CHECKS.with(std::cell::Cell::get);
                self.0.lock().unwrap().push((done, checks));
            }
            Ok(())
        }
    }
    let observer = Arc::new(Heartbeats(Mutex::new(Vec::new())));
    CHECKS.with(|checks| checks.set(0));
    inspect_with(
        ValidationRequest::new(&path),
        &RunControl::new(Some(observer.clone())),
    )
    .unwrap();
    let beats: Vec<_> = observer.0.lock().unwrap().clone();
    let beats: Vec<_> = beats
        .iter()
        .filter(|(done, _)| *done == placing)
        .map(|(_, checks)| *checks)
        .collect();
    assert_eq!(beats, [1024, 2048, 3072, 4096]);
    for at in [1, 1024, 1500, 2101, 3072, 4202] {
        let control = RunControl::default();
        CANCEL_AT_CHECK
            .with(|cancel| *cancel.borrow_mut() = Some((at, control.cancellation_handle())));
        CHECKS.with(|checks| checks.set(0));
        STOPPED.with(|stopped| stopped.set(None));
        let result = inspect_with(ValidationRequest::new(&path), &control);
        CANCEL_AT_CHECK.with(|cancel| *cancel.borrow_mut() = None);
        assert!(matches!(result, Err(Error::Cancelled)), "{at}");
        assert_eq!(CHECKS.with(std::cell::Cell::get), at);
        assert_eq!(
            STOPPED.with(std::cell::Cell::get),
            Some(("validate.traverse", placing)),
            "{at}"
        );
    }
}

/// An observer failure aborts the validation with its own category.
#[test]
fn observer_failure_aborts_validation() {
    struct Failing;
    impl Observer for Failing {
        fn observe(&self, _: &RunEvent<'_>) -> Result<(), crate::JobError> {
            Err(crate::JobError::new(
                crate::JobErrorKind::ObserverFailure,
                "observer refused",
            ))
        }
    }
    let (_work, path) = package(&small());
    let result = inspect_with(
        ValidationRequest::new(&path),
        &RunControl::new(Some(Arc::new(Failing))),
    );
    assert!(matches!(&result, Err(Error::Observer(message)) if message == "observer refused"));
    assert_eq!(result.unwrap_err().category(), ("observer_failure", 1));
    // A control is claimed once.
    let control = RunControl::default();
    inspect_with(ValidationRequest::new(&path), &control).unwrap();
    assert!(matches!(
        inspect_with(ValidationRequest::new(&path), &control),
        Err(Error::InvalidState(_))
    ));
}
