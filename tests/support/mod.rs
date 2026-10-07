//! Helpers shared by the native CLI acceptance tests.
#![allow(dead_code)]
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

/// The built binary with an empty executable `PATH`, so no helper can run.
pub fn rusty_tiles() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
    command.env("PATH", "");
    command
}

/// Run the binary under `umask 022`. The shell is named by absolute path
/// and execs the binary, so `PATH` stays empty.
#[cfg(unix)]
pub fn with_umask_022<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Output {
    Command::new("/bin/sh")
        .arg("-c")
        .arg("umask 022 && exec \"$0\" \"$@\"")
        .arg(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(args)
        .env("PATH", "")
        .output()
        .unwrap()
}

/// Permission bits of a published file or directory.
#[cfg(unix)]
pub fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// An existing output survives a run without `--force` and a failed forced
/// run. Only a successful forced run replaces it.
pub fn force_replaces_only_successful_output(
    command: &str,
    source: &Path,
    options: &[&str],
    directory: bool,
) {
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join(command);
    let previous = if directory {
        fs::create_dir(&out).unwrap();
        out.join("previous")
    } else {
        out.clone()
    };
    fs::write(&previous, b"original").unwrap();
    let run = |input: &Path, extra: &[&str]| {
        rusty_tiles()
            .args([command, "-i"])
            .arg(input)
            .arg("-o")
            .arg(&out)
            .args(options)
            .args(extra)
            .output()
            .unwrap()
    };
    let rejected = run(source, &[]);
    assert_eq!(rejected.status.code(), Some(5), "{command}");
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("--force"),
        "{command}: {}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    let bad = root.path().join("bad");
    fs::write(&bad, b"invalid data").unwrap();
    assert!(!run(&bad, &["--force"]).status.success(), "{command}");
    assert_eq!(fs::read(&previous).unwrap(), b"original", "{command}");
    let success = run(source, &["-f"]);
    assert!(
        success.status.success(),
        "{command}: {}",
        String::from_utf8_lossy(&success.stderr)
    );
    if directory {
        assert!(!previous.exists(), "{command}");
        assert!(fs::read_dir(&out).unwrap().next().is_some(), "{command}");
    } else {
        assert_ne!(fs::read(&out).unwrap(), b"original", "{command}");
    }
    // Only the output and the bad source remain: no work or backup directory.
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2, "{command}");
}
