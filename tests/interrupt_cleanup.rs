//! SIGINT and SIGTERM cancel a running `mesh` through the ordinary cancellation path:
//! scratch and spill directories are removed, nothing is published, and the process
//! exits with the shell's conventional `128 + signal` status.
#![cfg(unix)]

use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
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

/// Kills and reaps the child if the test unwinds before it has been waited for.
struct KillOnDrop(Option<Child>);

impl KillOnDrop {
    fn child(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }
    fn finish(mut self) -> Child {
        self.0.take().unwrap()
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct Interrupted {
    code: Option<i32>,
    stderr: String,
}

/// Spawn the CLI with `args`, wait until `ready` says the run is under way, send it
/// `signal` and return how it ended. A run that ends on its own before the signal is
/// sent is a failure of the fixture, not of cancellation, and is reported as such.
fn interrupt(
    args: &[&std::ffi::OsStr],
    signal: i32,
    ready: &mut dyn FnMut(&mut Child, &mpsc::Receiver<String>) -> bool,
) -> Interrupted {
    let mut guard = KillOnDrop(Some(
        Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    // Stderr is drained on its own thread so a full pipe never stalls the child; each
    // line is also forwarded so a test can wait for the run's first progress event.
    let stderr = guard.child().stderr.take().unwrap();
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
    let started = Instant::now();
    while !ready(guard.child(), &receiver) {
        if let Some(status) = guard.child().try_wait().unwrap() {
            panic!("the run ended ({status}) before it could be signalled: use a larger fixture");
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "the run never became ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = guard.child().id() as libc::pid_t;
    assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
    let status = guard.finish().wait().unwrap();
    let stderr = reader.join().unwrap();
    assert!(
        !status.success(),
        "the run finished before the signal took effect: use a larger fixture\n{stderr}"
    );
    Interrupted {
        code: status.code(),
        stderr,
    }
}

/// `mesh` with a scratch directory, signalled once its sort is spilling.
fn interrupted_mesh(signal: i32) -> (Interrupted, Vec<String>, Vec<String>) {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("grid.glb");
    grid_glb(&input, 700);
    let scratch = work.path().join("scratch");
    let out_dir = work.path().join("out");
    fs::create_dir(&scratch).unwrap();
    fs::create_dir(&out_dir).unwrap();
    let output = out_dir.join("grid.3tz");
    let result = interrupt(
        &[
            "mesh".as_ref(),
            "-i".as_ref(),
            input.as_os_str(),
            "-o".as_ref(),
            output.as_os_str(),
            "--jobs".as_ref(),
            "2".as_ref(),
            "--scratch-dir".as_ref(),
            scratch.as_os_str(),
        ],
        signal,
        // The sort's spill directory exists only while the run is deep in its capture.
        &mut |_, _| {
            names(&scratch)
                .iter()
                .any(|name| name.starts_with("rusty-tiles-spill-"))
        },
    );
    (result, names(&scratch), names(&out_dir))
}

/// A packaged archive whose one payload is a 24 MB point cloud: `validate` hashes it in a
/// phase that checks for cancellation, then refuses it on a budget, so a run that is not
/// cancelled still ends with a different status than the one asserted.
fn point_cloud_archive(work: &Path) -> PathBuf {
    const POINTS: u32 = 2_000_000;
    let data = work.join("data");
    fs::create_dir_all(&data).unwrap();
    let bin: Vec<u8> = (0..POINTS * 3)
        .flat_map(|i| (((i % 1000) * 7) as f32 / 10.0).to_le_bytes())
        .collect();
    let doc = serde_json::json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],
        "buffers":[{"byteLength":bin.len()}],
        "bufferViews":[{"buffer":0,"byteLength":bin.len()}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":POINTS,"type":"VEC3",
                      "min":[0,0,0],"max":[99.9,99.9,99.9]}]
    });
    let mut text = serde_json::to_vec(&doc).unwrap();
    text.resize(text.len().next_multiple_of(4), b' ');
    let mut glb = b"glTF".to_vec();
    for word in [
        2u32,
        (28 + text.len() + bin.len()) as u32,
        text.len() as u32,
    ] {
        glb.extend(word.to_le_bytes());
    }
    glb.extend(b"JSON");
    glb.extend(text);
    glb.extend((bin.len() as u32).to_le_bytes());
    glb.extend(b"BIN\0");
    glb.extend(bin);
    fs::write(data.join("a.glb"), glb).unwrap();
    let tileset = serde_json::json!({"asset":{"version":"1.1"},"geometricError":0,"root":{
        "boundingVolume":{"box":[0,0,0,2000,0,0,0,2000,0,0,0,2000]},
        "geometricError":0,"content":{"uri":"a.glb"}}});
    fs::write(
        data.join("tileset.json"),
        serde_json::to_vec(&tileset).unwrap(),
    )
    .unwrap();
    let output = work.join("points.3tz");
    rusty_tiles::package::package(
        rusty_tiles::package::PackageRequest::directory(&data, &output),
        &rusty_tiles::RunControl::default(),
    )
    .unwrap();
    output
}

#[test]
fn sigterm_cancels_mesh_cleans_scratch_and_exits_143() {
    let (run, scratch, out) = interrupted_mesh(libc::SIGTERM);
    assert_eq!(run.code, Some(143), "{}", run.stderr);
    assert!(run.stderr.contains("interrupted"), "{}", run.stderr);
    assert!(run.stderr.contains("cancelled"), "{}", run.stderr);
    assert!(scratch.is_empty(), "scratch left behind: {scratch:?}");
    assert!(out.is_empty(), "output directory not clean: {out:?}");
}

#[test]
fn sigint_cancels_mesh_cleans_scratch_and_exits_130() {
    let (run, scratch, out) = interrupted_mesh(libc::SIGINT);
    assert_eq!(run.code, Some(130), "{}", run.stderr);
    assert!(scratch.is_empty(), "scratch left behind: {scratch:?}");
    assert!(out.is_empty(), "output directory not clean: {out:?}");
}

/// The validator reports cancellation as its own failure type; it still exits 143.
#[test]
fn sigterm_cancels_validate_and_exits_143() {
    let work = tempfile::tempdir().unwrap();
    let archive = point_cloud_archive(work.path());
    let run = interrupt(
        &[
            "--progress".as_ref(),
            "json".as_ref(),
            "validate".as_ref(),
            archive.as_os_str(),
        ],
        libc::SIGTERM,
        // Hashing the 24 MB payload takes far longer than the signal does to arrive.
        &mut |_, lines| {
            std::iter::from_fn(|| lines.try_recv().ok()).any(|line| line.contains("validate.hash"))
        },
    );
    assert_eq!(run.code, Some(143), "{}", run.stderr);
    assert!(run.stderr.contains("cancelled"), "{}", run.stderr);
}
