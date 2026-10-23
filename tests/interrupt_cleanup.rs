//! SIGINT and SIGTERM cancel a running `mesh` through the ordinary cancellation path:
//! scratch and spill directories are removed, nothing is published, and the process
//! exits with the shell's conventional `128 + signal` status.
#![cfg(unix)]

use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// An indexed `n` x `n` grid of quads as a GLB: large enough that the debug-built CLI is
/// still capturing when the signal arrives.
fn grid_glb(path: &Path, n: u32) {
    let side = n + 1;
    let mut positions = Vec::new();
    for y in 0..side {
        for x in 0..side {
            let z = ((x * 7 + y * 13) % 5) as f32 * 0.01;
            for v in [x as f32, y as f32, z] {
                positions.extend(v.to_le_bytes());
            }
        }
    }
    let mut indices = Vec::new();
    for y in 0..n {
        for x in 0..n {
            let a = y * side + x;
            for i in [a, a + 1, a + side, a + 1, a + side + 1, a + side] {
                indices.extend(i.to_le_bytes());
            }
        }
    }
    let count = (n * n * 6) as usize;
    let mut bin = positions.clone();
    bin.extend(&indices);
    let doc = serde_json::json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
        "buffers":[{"byteLength":bin.len()}],
        "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":positions.len()},
            {"buffer":0,"byteOffset":positions.len(),"byteLength":indices.len()}],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":side * side,"type":"VEC3",
             "min":[0,0,0],"max":[n,n,0.04]},
            {"bufferView":1,"componentType":5125,"count":count,"type":"SCALAR"}]
    });
    let mut text = serde_json::to_vec(&doc).unwrap();
    text.resize(text.len().next_multiple_of(4), b' ');
    bin.resize(bin.len().next_multiple_of(4), 0);
    let mut bytes = b"glTF".to_vec();
    for word in [
        2u32,
        (28 + text.len() + bin.len()) as u32,
        text.len() as u32,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(b"JSON");
    bytes.extend(text);
    bytes.extend((bin.len() as u32).to_le_bytes());
    bytes.extend(b"BIN\0");
    bytes.extend(bin);
    fs::write(path, bytes).unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Run `mesh` with a scratch directory, signal it once it is spilling, and
/// return its status, stderr and the parent-directory listings afterwards.
fn interrupted_mesh(signal: i32) -> (Option<i32>, String, Vec<String>, Vec<String>) {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("grid.glb");
    grid_glb(&input, 700);
    let scratch = work.path().join("scratch");
    let out_dir = work.path().join("out");
    fs::create_dir(&scratch).unwrap();
    fs::create_dir(&out_dir).unwrap();
    let output = out_dir.join("grid.3tz");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["mesh", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .args(["--jobs", "2", "--scratch-dir"])
        .arg(&scratch)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    // The sort's spill directory exists only while the run is deep in its capture.
    while !names(&scratch)
        .iter()
        .any(|name| name.starts_with("rusty-tiles-spill-"))
    {
        assert!(
            child.try_wait().unwrap().is_none(),
            "mesh finished before it spilled"
        );
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "no spill directory appeared"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let pid = child.id() as libc::pid_t;
    assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
    let result = child.wait_with_output().unwrap();
    let status = {
        use std::os::unix::process::ExitStatusExt;
        // A process killed by the signal instead of exiting reports its signal.
        result.status.code().or(result.status.signal().map(|s| -s))
    };
    (
        status,
        String::from_utf8_lossy(&result.stderr).into_owned(),
        names(&scratch),
        names(&out_dir),
    )
}

#[test]
fn sigterm_cancels_mesh_cleans_scratch_and_exits_143() {
    let (status, stderr, scratch, out) = interrupted_mesh(libc::SIGTERM);
    assert_eq!(status, Some(143), "{stderr}");
    assert!(stderr.contains("interrupted"), "{stderr}");
    assert!(stderr.contains("cancelled"), "{stderr}");
    assert!(scratch.is_empty(), "scratch left behind: {scratch:?}");
    assert!(out.is_empty(), "output directory not clean: {out:?}");
}

#[test]
fn sigint_cancels_mesh_cleans_scratch_and_exits_130() {
    let (status, stderr, scratch, out) = interrupted_mesh(libc::SIGINT);
    assert_eq!(status, Some(130), "{stderr}");
    assert!(scratch.is_empty(), "scratch left behind: {scratch:?}");
    assert!(out.is_empty(), "output directory not clean: {out:?}");
}
