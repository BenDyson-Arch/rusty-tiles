//! A stop request (SIGTERM) ends each long phase within a few seconds, through the
//! ordinary cancellation path: exit 143, no spill or work folder left, nothing published.
//!
//! Each test holds a small run inside one phase with the debug-build pacing hook
//! (`RUSTY_TILES_TEST_PACE`, see `src/test_hooks.rs`): the named site's work unit is slowed
//! so that, without the phase's own stop check, the remaining work would take far longer
//! than the asserted bound. The signal is sent once the site announces itself. The
//! backstop test stands in for a call that cannot be interrupted
//! (`RUSTY_TILES_TEST_STALL`).
#![cfg(unix)]

mod support;

use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// The asserted bound from signal to exit: a checked phase stops within one paced work
/// unit (at most 1 s here) plus cleanup, with headroom for a loaded CI host. Each fixture
/// paces enough work that, without the phase's check, the run outlasts this bound.
const BOUND: Duration = Duration::from_secs(10);

/// Kills and reaps the child if the test unwinds before waiting for it.
struct KillOnDrop(Option<Child>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct Stopped {
    code: Option<i32>,
    after: Duration,
    stderr: String,
}

/// Run the CLI with `args` and `env`, send SIGTERM once `site` announces itself, and
/// return how and how fast the process ended.
fn stop_in(args: &[OsString], env: &[(&str, &str)], site: &str) -> Stopped {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
    command
        .args(args)
        .envs(env.iter().copied())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut guard = KillOnDrop(Some(command.spawn().unwrap()));
    let child = guard.0.as_mut().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (lines, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut all = String::new();
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            all.push_str(&line);
            let _ = lines.send(std::mem::take(&mut line));
        }
        let mut rest = String::new();
        let _ = reader.read_to_string(&mut rest);
        all + &rest
    });
    let marker = format!("rusty-tiles test: in {site}");
    let started = Instant::now();
    loop {
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(line) if line.contains(&marker) => break,
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let status = guard.0.as_mut().unwrap().wait().unwrap();
                guard.0 = None;
                panic!(
                    "the run ended ({status}) before reaching {site}:\n{}",
                    reader.join().unwrap()
                );
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(300),
            "the run never reached {site}"
        );
    }
    let pid = guard.0.as_ref().unwrap().id() as libc::pid_t;
    let sent = Instant::now();
    assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
    // A run that ignores the stop is killed after a minute (KillOnDrop) and fails here.
    let status = loop {
        if let Some(status) = guard.0.as_mut().unwrap().try_wait().unwrap() {
            guard.0 = None;
            break status;
        }
        assert!(
            sent.elapsed() < Duration::from_secs(60),
            "{site}: still running 60 s after SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let after = sent.elapsed();
    Stopped {
        code: status.code(),
        after,
        stderr: reader.join().unwrap(),
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn assert_stopped(run: &Stopped, scratch: &Path, out: &Path) {
    assert_eq!(run.code, Some(143), "{}", run.stderr);
    assert!(
        run.after < BOUND,
        "stopped after {:?} (bound {BOUND:?}):\n{}",
        run.after,
        run.stderr
    );
    assert!(run.stderr.contains("cancelled"), "{}", run.stderr);
    assert!(
        names(scratch).is_empty(),
        "scratch left: {:?}",
        names(scratch)
    );
    assert!(names(out).is_empty(), "output directory: {:?}", names(out));
}

/// A work root with an empty `scratch` and `out` directory.
struct Work {
    root: tempfile::TempDir,
}

impl Work {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for name in ["scratch", "out"] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        Self { root }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
    fn scratch(&self) -> PathBuf {
        self.path("scratch")
    }
    fn out(&self) -> PathBuf {
        self.path("out")
    }
}

fn os(args: &[&dyn AsRef<OsStr>]) -> Vec<OsString> {
    args.iter().map(|arg| arg.as_ref().to_owned()).collect()
}

/// A packaged archive of `members` small files plus `big` bytes in one more member.
fn archive(work: &Work, members: usize, big: usize) -> PathBuf {
    let data = work.path("data");
    fs::create_dir_all(data.join("m")).unwrap();
    let tileset = serde_json::json!({"asset":{"version":"1.1"},"geometricError":0,
        "root":{"boundingVolume":{"box":[0,0,0,1,0,0,0,1,0,0,0,1]},"geometricError":0}});
    fs::write(data.join("tileset.json"), tileset.to_string()).unwrap();
    for i in 0..members {
        fs::write(data.join("m").join(format!("{i}.bin")), i.to_le_bytes()).unwrap();
    }
    if big > 0 {
        let bytes: Vec<u8> = (0..big).map(|i| (i * 31 % 251) as u8).collect();
        fs::write(data.join("big.bin"), bytes).unwrap();
    }
    let output = work.path("input.3tz");
    rusty_tiles::package::package(
        rusty_tiles::package::PackageRequest::directory(&data, &output),
        &rusty_tiles::RunControl::default(),
    )
    .unwrap();
    fs::remove_dir_all(&data).unwrap();
    output
}

/// Archive admission (central-directory scan, catalog sort, index pass) checks every 1024
/// entries; it used to run to completion before the first progress event.
#[test]
fn validate_admission_stops() {
    let work = Work::new();
    let input = archive(&work, 20_000, 0);
    let run = stop_in(
        &os(&[&"validate", &input, &"--scratch-dir", &work.scratch()]),
        &[("RUSTY_TILES_TEST_PACE", "validate.admission")],
        "validate.admission",
    );
    assert_stopped(&run, &work.scratch(), &work.out());
}

/// Hashing one large member checks every 64 KiB block; it used to check per member.
#[test]
fn validate_hash_of_one_large_member_stops() {
    let work = Work::new();
    let input = archive(&work, 0, 40 << 20);
    let run = stop_in(
        &os(&[&"validate", &input, &"--scratch-dir", &work.scratch()]),
        &[
            ("RUSTY_TILES_TEST_PACE", "validate.hash-block"),
            ("RUSTY_TILES_TEST_PACE_US", "30000"),
            // Past the small members: inside the 640-block member.
            ("RUSTY_TILES_TEST_PACE_AFTER", "16"),
        ],
        "validate.hash-block",
    );
    assert_stopped(&run, &work.scratch(), &work.out());
}

/// `points` GeoJSON point features on a small lattice.
fn geojson(path: &Path, points: usize) {
    let features: Vec<_> = (0..points)
        .map(|i| {
            let (x, y) = ((i % 300) as f64 * 1e-4, (i / 300) as f64 * 1e-4);
            serde_json::json!({"type":"Feature","properties":{"id":i},
                "geometry":{"type":"Point","coordinates":[x, y, 1.0]}})
        })
        .collect();
    let doc = serde_json::json!({"type":"FeatureCollection","features":features});
    fs::write(path, doc.to_string()).unwrap();
}

/// A vector run over `points` features (or the existing `points.geojson`), signalled in
/// `site`. Enough features that the paced SQLite work left after the signal outlasts the
/// bound when nothing interrupts it.
fn vector_run(work: &Work, points: usize, site: &str, pace_us: &str, extra: &[&str]) -> Stopped {
    let input = work.path("points.geojson");
    if !input.exists() {
        geojson(&input, points);
    }
    let output = work.out().join("points.3tz");
    let mut args = os(&[&"vector", &"-i", &input, &"-o", &output]);
    args.extend(extra.iter().map(OsString::from));
    stop_in(
        &args,
        &[
            ("RUSTY_TILES_TEST_PACE", site),
            ("RUSTY_TILES_TEST_PACE_US", pace_us),
            ("TMPDIR", work.scratch().to_str().unwrap()),
        ],
        site,
    )
}

/// GeoJSON ingestion spools every feature into SQLite with no attempt in scope; it now
/// checks every 1024 features.
#[test]
fn vector_geojson_ingestion_stops() {
    let work = Work::new();
    let run = vector_run(&work, 30_000, "vector.geojson-feature", "1000", &[]);
    assert_stopped(&run, &work.scratch(), &work.out());
}

/// Partition statements and per-row fingerprint updates on the feature spool, after
/// admission (whose per-feature loop already checked).
#[test]
fn vector_spool_statements_stop() {
    let work = Work::new();
    let run = vector_run(&work, 30_000, "sqlite.spool-build", "400000", &[]);
    assert_stopped(&run, &work.scratch(), &work.out());
}

/// Tile encoding: prefix totals, OFFSET sorts and aggregation scans on worker connections.
/// Not sensitive on its own with this fixture (per-encode checks also catch the stop);
/// `vector_spool_statements_stop` shows the shared interrupt handler is what stops a
/// long statement.
#[test]
fn vector_encoding_statements_stop() {
    let work = Work::new();
    let run = vector_run(&work, 200_000, "sqlite.encode", "500000", &[]);
    assert_stopped(&run, &work.scratch(), &work.out());
}

/// Restoring a reused subtree copies every previous payload; it now checks per tile.
#[test]
fn vector_reuse_restore_stops() {
    let work = Work::new();
    let input = work.path("points.geojson");
    geojson(&input, 30_000);
    let previous = work.path("previous.3tz");
    let status = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args([OsStr::new("vector"), "-i".as_ref(), input.as_os_str()])
        .args(["-o".as_ref(), previous.as_os_str()])
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
    let run = vector_run(
        &work,
        30_000,
        "vector.reuse",
        "200000",
        &["--reuse-tileset", previous.to_str().unwrap()],
    );
    assert_stopped(&run, &work.scratch(), &work.out());
}

/// Mesh delivery stages each leaf payload (read, hash, link) of a document; it now checks
/// per payload rather than per 256 leaves.
#[test]
fn mesh_payload_staging_stops() {
    let work = Work::new();
    let input = work.path("grid.glb");
    fs::write(&input, support::textured_grid_glb(200)).unwrap();
    let output = work.out().join("grid.3tz");
    let run = stop_in(
        &os(&[
            &"mesh",
            &"-i",
            &input,
            &"-o",
            &output,
            &"--jobs",
            &"2",
            &"--scratch-dir",
            &work.scratch(),
        ]),
        &[
            ("RUSTY_TILES_TEST_PACE", "mesh.stage-payload"),
            ("RUSTY_TILES_TEST_PACE_US", "1000000"),
        ],
        "mesh.stage-payload",
    );
    assert_stopped(&run, &work.scratch(), &work.out());
}

/// A KTX2 page encode cannot be interrupted (minutes at the ladder's highest efforts). The
/// CLI's backstop removes the run's temporary paths and exits with the signal's status once
/// the stop has not finished in time; here the encode never returns.
#[test]
fn backstop_ends_a_run_blocked_in_a_texture_encode() {
    let work = Work::new();
    let input = work.path("grid.glb");
    fs::write(&input, support::textured_grid_glb(16)).unwrap();
    let output = work.out().join("grid.3tz");
    let run = stop_in(
        &os(&[
            &"mesh",
            &"-i",
            &input,
            &"-o",
            &output,
            &"--jobs",
            &"2",
            &"--scratch-dir",
            &work.scratch(),
        ]),
        &[
            ("RUSTY_TILES_TEST_STALL", "ktx2.encode"),
            ("RUSTY_TILES_TEST_BACKSTOP_MS", "1500"),
        ],
        "ktx2.encode",
    );
    assert_eq!(run.code, Some(143), "{}", run.stderr);
    assert!(run.after < BOUND, "{:?}\n{}", run.after, run.stderr);
    assert!(run.stderr.contains("did not stop within"), "{}", run.stderr);
    assert!(
        names(&work.scratch()).is_empty(),
        "{:?}",
        names(&work.scratch())
    );
    assert!(names(&work.out()).is_empty(), "{:?}", names(&work.out()));
}
