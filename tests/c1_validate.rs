//! Independent, checked byte fixtures through public Rust and CLI entry points.
use rusty_tiles::validate::{inspect, ValidationRequest};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, process::Command};

#[test]
fn independent_corpus_rust_cli_parity_and_read_only_inputs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c1");
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let path = root.join(case["path"].as_str().unwrap());
        let before = fs::read(&path).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&before)),
            case["sha256"].as_str().unwrap(),
            "fixture changed: {name}"
        );
        let inspected = inspect(ValidationRequest::new(&path));
        match case["expectedKind"].as_str() {
            None => {
                assert!(inspected.is_ok(), "{name}: {inspected:?}");
            }
            Some(expected) => {
                let failure = inspected.as_ref().expect_err(name);
                assert_eq!(failure.category().0, expected, "{name}: {failure:?}");
            }
        }
        let cli = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
            .arg("validate")
            .arg(&path)
            .arg("--json")
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&cli.stdout).unwrap_or_else(|e| {
            panic!(
                "{name}: {e}; stderr {}",
                String::from_utf8_lossy(&cli.stderr)
            )
        });
        assert_eq!(
            cli.status.code(),
            Some(case["exitCode"].as_i64().unwrap() as i32),
            "{name}: {report}"
        );
        if let Some(expected) = case["expectedKind"].as_str() {
            assert_eq!(report["error"]["code"], expected, "{name}: {report}");
        } else {
            assert_eq!(report["ok"], true, "{name}: {report}");
            assert_eq!(
                serde_json::to_value(inspected.unwrap()).unwrap(),
                report,
                "complete report differs: {name}"
            );
        }
        assert_eq!(fs::read(path).unwrap(), before, "validation altered {name}");
    }
}

#[cfg(unix)]
#[test]
fn fifo_input_is_rejected_without_waiting_for_a_writer() {
    use std::{
        os::unix::ffi::OsStrExt,
        process::Stdio,
        time::{Duration, Instant},
    };
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("input.3tz");
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // No writer is ever opened: a blocking File::open would wait forever.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    let mut child = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .arg("validate")
        .arg(&path)
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("FIFO admission blocked before rejecting a nonregular file");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["error"]["code"], "unsupported");
    // The bounded child above prevents a regressed implementation hanging this
    // test before exercising the same admission directly through the Rust API.
    let failure = inspect(ValidationRequest::new(&path)).unwrap_err();
    assert_eq!(failure.category().0, "unsupported");
}
