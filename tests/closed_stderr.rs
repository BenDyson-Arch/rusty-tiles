//! A closed or broken stderr (`rusty-tiles ... 2>&1 | head -1`) must not change
//! the conversion's result: Rust ignores SIGPIPE, so a plain `eprintln!` would
//! panic and exit 101 after the archive was already written.
#![cfg(unix)]
use serde_json::Value;
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::Path;
use std::process::{Output, Stdio};

mod support;

/// A pipe whose read end is already dropped: every stderr write gets EPIPE.
fn broken_pipe() -> Stdio {
    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    drop(unsafe { OwnedFd::from_raw_fd(fds[0]) });
    Stdio::from(unsafe { OwnedFd::from_raw_fd(fds[1]) })
}

fn point_cloud(input: &Path) {
    let mut builder = las::Builder::from((1, 4));
    builder.point_format = las::point::Format::new(0).unwrap();
    let mut writer = las::Writer::from_path(input, builder.into_header().unwrap()).unwrap();
    for i in 0..8 {
        writer
            .write_point(las::Point {
                x: i as f64,
                y: (i * 2) as f64,
                z: 1.,
                ..Default::default()
            })
            .unwrap();
    }
    writer.close().unwrap();
}

fn run(args: &[&str]) -> (Output, tempfile::TempDir) {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("in.las");
    point_cloud(&input);
    let output = support::rusty_tiles()
        .args(args)
        .args(["point-cloud", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(work.path().join("out.3tz"))
        .args(["--sourceCrs", "local"])
        .stdin(Stdio::null())
        .stderr(broken_pipe())
        .output()
        .unwrap();
    assert!(work.path().join("out.3tz").is_file(), "{output:?}");
    (output, work)
}

#[test]
fn json_progress_json_survives_a_broken_stderr() {
    let (output, _work) = run(&["--json", "--progress", "json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true, "{result}");
}

#[test]
fn human_summary_survives_a_broken_stderr() {
    let (output, _work) = run(&[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test]
fn failure_exit_code_is_not_replaced_by_a_broken_stderr() {
    let work = tempfile::tempdir().unwrap();
    let output = support::rusty_tiles()
        .args(["--json", "--progress", "json", "point-cloud", "-i"])
        .arg(work.path().join("missing.las"))
        .arg("-o")
        .arg(work.path().join("out.3tz"))
        .args(["--sourceCrs", "local"])
        .stderr(broken_pipe())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["exitCode"], 1, "{result}");
}

/// A pipe whose read end is already dropped, as stdout.
#[test]
fn broken_stdout_keeps_the_conversion_status() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("in.las");
    point_cloud(&input);
    let output = support::rusty_tiles()
        .args(["--json", "point-cloud", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(work.path().join("out.3tz"))
        .args(["--sourceCrs", "local"])
        .stdout(broken_pipe())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(work.path().join("out.3tz").is_file());

    let missing = support::rusty_tiles()
        .args(["--json", "point-cloud", "-i"])
        .arg(work.path().join("missing.las"))
        .arg("-o")
        .arg(work.path().join("other.3tz"))
        .args(["--sourceCrs", "local"])
        .stdout(broken_pipe())
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1), "{missing:?}");
}

/// ENOSPC on stdout loses the result, which must not exit 0.
#[cfg(target_os = "linux")]
#[test]
fn full_stdout_is_a_failure() {
    let Ok(full) = std::fs::OpenOptions::new().write(true).open("/dev/full") else {
        eprintln!("skipped: /dev/full is not available");
        return;
    };
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("in.las");
    point_cloud(&input);
    let output = support::rusty_tiles()
        .args(["--json", "point-cloud", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(work.path().join("out.3tz"))
        .args(["--sourceCrs", "local"])
        .stdout(Stdio::from(full))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("writing to stdout"),
        "{output:?}"
    );

    // The doctor report is printed straight to stdout.
    let doctor = support::rusty_tiles()
        .args(["--json", "doctor", "--command", "point-cloud"])
        .stdout(Stdio::from(
            std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        ))
        .output()
        .unwrap();
    assert_eq!(doctor.status.code(), Some(1), "{doctor:?}");
}
