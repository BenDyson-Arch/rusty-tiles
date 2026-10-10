//! Installed-binary preview runs with no Python, no repository cwd and an
//! empty executable `PATH`, and serves only the selected files.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const MANIFESTS: [(&str, &str); 5] = [
    ("point-cloud", "tileset.json"),
    ("mesh", "tileset.json"),
    ("annotations", "tileset.json"),
    ("imagery", "tilejson.json"),
    ("terrain", "tileset.json"),
];

struct Server {
    child: Child,
    address: String,
    ready: Value,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn preview() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
    command.args(["preview", "--json"]).env("PATH", "");
    command
}

fn serve(cesium: &Path, layers: &[(&str, &Path)], cwd: &Path) -> Server {
    let mut command = preview();
    command.args(["--port", "0", "--cesium"]).arg(cesium);
    for (name, path) in layers {
        command.arg(format!("--{name}")).arg(path);
    }
    let mut child = command
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send(line);
    });
    let mut server = Server {
        child,
        address: String::new(),
        ready: Value::Null,
    };
    let line = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("preview startup timed out");
    server.ready = serde_json::from_str(&line).unwrap();
    assert_eq!(server.ready["ok"], true, "{}", server.ready);
    server.address = server.ready["url"]
        .as_str()
        .unwrap()
        .strip_prefix("http://")
        .unwrap()
        .trim_end_matches('/')
        .to_owned();
    server
}

struct Response {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn request(address: &str, path: &str, method: &str) -> Response {
    let mut socket = TcpStream::connect(address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        socket,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    socket.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(raw[..split].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap();
    let headers = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();
    Response {
        status: status.parse().unwrap(),
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

fn get(address: &str, path: &str) -> Response {
    request(address, path, "GET")
}

fn send_persistent(socket: &mut TcpStream, address: &str, path: &str, method: &str) {
    write!(
        socket,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: keep-alive\r\n\r\n"
    )
    .unwrap();
}

/// Read exactly one response, retaining buffered bytes and the open connection.
/// EOF cannot stand in for response framing when another request follows.
fn persistent_response(socket: &mut BufReader<TcpStream>, head: bool) -> Response {
    let mut line = String::new();
    socket.read_line(&mut line).unwrap();
    assert!(line.starts_with("HTTP/1.1 "), "{line:?}");
    let status = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut headers = HashMap::new();
    loop {
        line.clear();
        assert!(socket.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.trim_end().split_once(':').unwrap();
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers["content-length"].parse::<usize>().unwrap();
    let mut body = vec![0; if head { 0 } else { length }];
    socket.read_exact(&mut body).unwrap();
    Response {
        status,
        headers,
        body,
    }
}

/// An invented Cesium runtime and one directory per layer with its manifest.
fn fixture(root: &Path) -> (PathBuf, Vec<(&'static str, PathBuf)>) {
    let cesium = root.join("runtime");
    std::fs::create_dir(&cesium).unwrap();
    std::fs::write(cesium.join("Cesium.js"), "// invented runtime").unwrap();
    std::fs::write(cesium.join("worker.wasm"), b"\0asm").unwrap();
    let layers = MANIFESTS
        .iter()
        .map(|(name, manifest)| {
            let path = root.join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join(manifest), "{\"fixture\":true}").unwrap();
            (*name, path)
        })
        .collect();
    (cesium, layers)
}

fn layer<'a>(layers: &'a [(&str, PathBuf)], name: &str) -> &'a Path {
    &layers.iter().find(|(layer, _)| *layer == name).unwrap().1
}

#[test]
fn embedded_preview_runs_without_python_and_serves_only_selected_files() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    let tiles = root.path().join("tiles");
    std::fs::create_dir(&runtime).unwrap();
    std::fs::create_dir(&tiles).unwrap();
    std::fs::write(runtime.join("Cesium.js"), "// invented runtime").unwrap();
    std::fs::write(tiles.join("tileset.json"), "{\"fixture\":true}").unwrap();
    std::fs::write(root.path().join("private.txt"), "not selected").unwrap();
    let server = serve(&runtime, &[("mesh", &tiles)], root.path());
    let address = server.address.as_str();
    assert!(address.starts_with("127.0.0.1:"));
    for (path, status, expected) in [
        ("/", 200, "/cesium/Cesium.js"),
        ("/config.json", 200, "/mesh/tileset.json"),
        ("/mesh/tileset.json", 200, "fixture"),
        ("/mesh/", 404, ""),
        ("/mesh/%2e%2e/private.txt", 404, ""),
        ("/terrain/tileset.json", 404, ""),
    ] {
        let response = get(address, path);
        let body = String::from_utf8_lossy(&response.body);
        assert_eq!(response.status, status, "{path}");
        assert_eq!(response.headers["cache-control"], "no-cache");
        assert!(body.contains(expected), "{body}");
        assert!(!body.contains("not selected"));
    }
}

#[test]
fn all_five_routes_serve_mime_types_head_and_no_cache() {
    let root = tempfile::tempdir().unwrap();
    let (cesium, layers) = fixture(root.path());
    std::fs::write(layer(&layers, "mesh").join("tile.glb"), b"glTF").unwrap();
    std::fs::write(layer(&layers, "terrain").join("0.glb"), b"terrain").unwrap();
    let selected: Vec<_> = layers
        .iter()
        .map(|(name, path)| (*name, path.as_path()))
        .collect();
    let server = serve(&cesium, &selected, root.path());
    let address = server.address.as_str();
    assert!(address.starts_with("127.0.0.1:"));
    let response = get(address, "/config.json?cache=fixture");
    assert_eq!(response.status, 200);
    let config: Value = serde_json::from_slice(&response.body).unwrap();
    let expected: serde_json::Map<String, Value> = MANIFESTS
        .iter()
        .map(|(name, manifest)| (name.to_string(), json!(format!("/{name}/{manifest}"))))
        .collect();
    assert_eq!(config, Value::Object(expected));
    assert_eq!(config, server.ready["layers"]);
    for (path, content_type) in [
        ("/", "text/html; charset=utf-8"),
        ("/index.html", "text/html; charset=utf-8"),
        ("/config.json", "application/json"),
        ("/cesium/Cesium.js", "text/javascript"),
        ("/cesium/worker.wasm", "application/wasm"),
        ("/mesh/tile.glb", "model/gltf-binary"),
        ("/terrain/0.glb", "model/gltf-binary"),
    ] {
        let response = get(address, path);
        assert_eq!(response.status, 200, "{path}");
        assert_eq!(response.headers["cache-control"], "no-cache", "{path}");
        assert_eq!(response.headers["content-type"], content_type, "{path}");
        let head = request(address, path, "HEAD");
        assert_eq!(head.status, 200, "{path}");
        assert!(head.body.is_empty(), "{path}");
        assert_eq!(
            head.headers["content-length"].parse::<usize>().unwrap(),
            response.body.len(),
            "{path}"
        );
    }
    let page = String::from_utf8(get(address, "/").body).unwrap();
    assert!(page.contains("/cesium/Cesium.js"));
    for path in config.as_object().unwrap().values() {
        let body: Value =
            serde_json::from_slice(&get(address, path.as_str().unwrap()).body).unwrap();
        assert_eq!(body, json!({"fixture":true}));
    }
    assert_eq!(request(address, "/config.json", "POST").status, 405);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..16)
            .map(|_| {
                scope.spawn(|| {
                    (0..4)
                        .map(|_| get(address, "/mesh/tile.glb"))
                        .all(|r| r.status == 200 && r.body == b"glTF")
                })
            })
            .collect();
        assert!(workers.into_iter().all(|worker| worker.join().unwrap()));
    });
}

#[test]
fn persistent_bursts_and_reuse_progress_with_idle_and_partial_connections() {
    const CLIENTS: usize = 16;
    let root = tempfile::tempdir().unwrap();
    let (cesium, layers) = fixture(root.path());
    let mesh = layer(&layers, "mesh");
    // Larger than a small HTTP/file buffer, with literal independently known bytes.
    let payload: Vec<u8> = (0..196_608).map(|i| (i % 251) as u8).collect();
    std::fs::write(mesh.join("streamed.glb"), &payload).unwrap();
    std::fs::File::create(mesh.join("backpressure.glb"))
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    let server = serve(&cesium, &[("mesh", mesh)], root.path());
    let address = server.address.as_str();
    let connect = || {
        let socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        socket.set_nodelay(true).unwrap();
        socket
    };
    // Neither an idle socket nor incomplete headers own an OS worker needed by
    // another connection. Keep these open until every active response completes.
    let idle: Vec<_> = (0..CLIENTS).map(|_| connect()).collect();
    let mut partial: Vec<_> = (0..CLIENTS).map(|_| connect()).collect();
    for socket in &mut partial {
        socket
            .write_all(b"GET /config.json HTTP/1.1\r\nHost:")
            .unwrap();
    }
    let mut blocked: Vec<_> = (0..4).map(|_| connect()).collect();
    for socket in &mut blocked {
        send_persistent(socket, address, "/mesh/backpressure.glb", "GET");
    }
    for socket in &blocked {
        // Observe that each stream started, then leave its large body unread.
        // These receivers stay open while all active clients finish and reuse.
        assert!(socket.peek(&mut [0; 1]).unwrap() > 0);
    }
    let active: Vec<_> = (0..CLIENTS).map(|_| connect()).collect();
    let start = std::sync::Barrier::new(CLIENTS);
    let completed = std::thread::scope(|scope| {
        let workers: Vec<_> = active
            .into_iter()
            .map(|socket| {
                let payload = &payload;
                let start = &start;
                scope.spawn(move || {
                    let mut socket = BufReader::new(socket);
                    start.wait();
                    // All clients issue the first request together. Later requests
                    // reuse that same socket while peers may already be idle.
                    send_persistent(socket.get_mut(), address, "/config.json", "GET");
                    let config = persistent_response(&mut socket, false);
                    assert_eq!(config.status, 200);
                    assert_eq!(config.headers["cache-control"], "no-cache");
                    assert_eq!(
                        serde_json::from_slice::<Value>(&config.body).unwrap(),
                        json!({"mesh":"/mesh/tileset.json"})
                    );
                    // Pipeline a streamed GET, HEAD and another GET. The reader
                    // must honor framing and order without waiting for EOF.
                    for method in ["GET", "HEAD", "GET"] {
                        send_persistent(socket.get_mut(), address, "/mesh/streamed.glb", method);
                    }
                    for method in ["GET", "HEAD", "GET"] {
                        let response = persistent_response(&mut socket, method == "HEAD");
                        assert_eq!(response.status, 200);
                        assert_eq!(response.headers["content-type"], "model/gltf-binary");
                        assert_eq!(response.headers["cache-control"], "no-cache");
                        assert_eq!(
                            response.headers["content-length"].parse::<usize>().unwrap(),
                            payload.len()
                        );
                        if method == "HEAD" {
                            assert!(response.body.is_empty());
                        } else {
                            assert_eq!(response.body, *payload);
                        }
                    }
                    // Return ownership rather than closing early and accidentally
                    // freeing a shared reader for a peer's pending response.
                    socket
                })
            })
            .collect();
        let results: Vec<_> = workers.into_iter().map(|worker| worker.join()).collect();
        results
            .into_iter()
            .map(|result| result.unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(completed.len(), CLIENTS);
    assert_eq!(idle.len(), CLIENTS);
    assert_eq!(partial.len(), CLIENTS);
    assert_eq!(blocked.len(), 4);
}

#[cfg(target_os = "linux")]
#[test]
fn stopped_single_cpu_keepalive_burst_completes_without_releasing_any_client() {
    struct Resume(libc::pid_t);
    impl Drop for Resume {
        fn drop(&mut self) {
            // Even setup failure must resume before unwinding to assertions.
            unsafe { libc::kill(self.0, libc::SIGCONT) };
        }
    }
    let root = tempfile::tempdir().unwrap();
    let (cesium, layers) = fixture(root.path());
    let server = serve(&cesium, &[("mesh", layer(&layers, "mesh"))], root.path());
    let address: std::net::SocketAddr = server.address.parse().unwrap();
    let pid = server.child.id() as libc::pid_t;
    let mut allowed: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::sched_getaffinity(0, std::mem::size_of_val(&allowed), &mut allowed) },
        0
    );
    let cpu = (0..libc::CPU_SETSIZE as usize)
        .find(|cpu| unsafe { libc::CPU_ISSET(*cpu, &allowed) })
        .unwrap();
    let mut selected: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    unsafe { libc::CPU_SET(cpu, &mut selected) };
    // Establish the old transport's initially idle connection readers. This is
    // a scheduling control, not a retry or an acceptance deadline extension.
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0);
    let resume = Resume(pid);
    let setup = (|| -> std::io::Result<Vec<BufReader<TcpStream>>> {
        let mut status = 0;
        if unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED) } != pid {
            return Err(std::io::Error::last_os_error());
        }
        // Restrict existing readers/acceptor as well as the main thread; new
        // tasks/threads inherit this mask. One accept burst can now queue all
        // sockets before the old idle reader workers are scheduled.
        for entry in std::fs::read_dir(format!("/proc/{pid}/task"))? {
            let entry = entry?;
            let tid = entry
                .file_name()
                .to_string_lossy()
                .parse::<libc::pid_t>()
                .map_err(std::io::Error::other)?;
            if unsafe { libc::sched_setaffinity(tid, std::mem::size_of_val(&selected), &selected) }
                != 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        let mut clients = Vec::new();
        for _ in 0..6 {
            let mut socket = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
            socket.set_read_timeout(Some(Duration::from_secs(10)))?;
            socket.set_write_timeout(Some(Duration::from_secs(10)))?;
            write!(
                socket,
                "GET /config.json HTTP/1.1\r\nHost: {address}\r\nConnection: keep-alive\r\n\r\n"
            )?;
            clients.push(BufReader::new(socket));
        }
        Ok(clients)
    })();
    drop(resume); // SIGCONT precedes all assertions and response reads.
    let mut clients = setup.unwrap();
    for client in &mut clients {
        let response = persistent_response(client, false);
        assert_eq!(response.status, 200);
        assert_eq!(
            serde_json::from_slice::<Value>(&response.body).unwrap(),
            json!({"mesh":"/mesh/tileset.json"})
        );
    }
    // Keeping the entire vector alive prevents early-close release from hiding
    // the two stranded sockets observed on the frozen tiny_http artifact.
    assert_eq!(clients.len(), 6);
}

#[cfg(unix)]
#[test]
fn only_selected_roots_reject_listing_traversal_and_symlink_escape() {
    let root = tempfile::tempdir().unwrap();
    let (cesium, layers) = fixture(root.path());
    let mesh = layer(&layers, "mesh");
    let secret = root.path().join("private.txt");
    std::fs::write(&secret, "not selected").unwrap();
    std::os::unix::fs::symlink(&secret, mesh.join("escape.txt")).unwrap();
    std::os::unix::fs::symlink(root.path(), mesh.join("external")).unwrap();
    std::fs::write(mesh.join("valid file.bin"), b"selected").unwrap();
    let server = serve(&cesium, &[("mesh", mesh)], root.path());
    let address = server.address.as_str();
    for path in [
        "/mesh/",
        "/cesium/",
        "/terrain/tileset.json",
        "/data/private.txt",
        "/private.txt",
        "/mesh/../private.txt",
        "/mesh/%2e%2e/private.txt",
        "/mesh/%2e%2e%2fprivate.txt",
        "/mesh/%2fetc/passwd",
        "/mesh/%5c..%5cprivate.txt",
        "/mesh/escape.txt",
        "/mesh/external/private.txt",
        "/mesh/%00private.txt",
        &format!("/mesh/{}", secret.display()),
        &format!("/mesh/%2f{}", secret.display()),
    ] {
        let response = get(address, path);
        assert_eq!(response.status, 404, "{path}");
        assert_eq!(response.headers["cache-control"], "no-cache", "{path}");
        assert!(!String::from_utf8_lossy(&response.body).contains("not selected"));
    }
    assert_eq!(get(address, "/mesh/valid%20file.bin?x=1").body, b"selected");
}

#[cfg(unix)]
#[test]
fn invalid_selections_and_symlinked_manifest_are_data_errors() {
    let root = tempfile::tempdir().unwrap();
    let (cesium, layers) = fixture(root.path());
    let empty = root.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let missing = root.path().join("missing");
    let cases: [&[&std::ffi::OsStr]; 3] = [
        &[],
        &["--mesh".as_ref(), empty.as_os_str()],
        &["--terrain".as_ref(), missing.as_os_str()],
    ];
    let run = |extra: &[&std::ffi::OsStr]| {
        preview()
            .arg("--cesium")
            .arg(&cesium)
            .args(extra)
            .output()
            .unwrap()
    };
    for extra in cases {
        let result = run(extra);
        assert_eq!(result.status.code(), Some(3), "{extra:?}");
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["error"]["code"], "data", "{extra:?}");
    }
    // A manifest symlinked from another root is not a selected manifest.
    std::os::unix::fs::symlink(
        layer(&layers, "mesh").join("tileset.json"),
        empty.join("tileset.json"),
    )
    .unwrap();
    let result = run(&["--mesh".as_ref(), empty.as_os_str()]);
    assert_eq!(result.status.code(), Some(3));
}

#[test]
fn duplicate_layers_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (cesium, layers) = fixture(root.path());
    let mesh = layer(&layers, "mesh");
    // The CLI takes each layer flag once.
    let result = preview()
        .arg("--cesium")
        .arg(&cesium)
        .arg("--mesh")
        .arg(mesh)
        .arg("--mesh")
        .arg(mesh)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["error"]["code"], "usage");
    // The library rejects a repeated layer name as data.
    let error = rusty_tiles::preview::Preview::new(
        &cesium,
        &[("mesh".into(), mesh.into()), ("mesh".into(), mesh.into())],
    )
    .err()
    .unwrap();
    assert_eq!(error.category(), ("data", 3));
    assert!(error.to_string().contains("duplicate preview layer: mesh"));
}

#[test]
fn json_startup_line_reports_url_and_layers() {
    let root = tempfile::tempdir().unwrap();
    let (cesium, layers) = fixture(root.path());
    let server = serve(
        &cesium,
        &[
            ("terrain", layer(&layers, "terrain")),
            ("imagery", layer(&layers, "imagery")),
        ],
        root.path(),
    );
    let mut keys: Vec<_> = server.ready.as_object().unwrap().keys().collect();
    keys.sort_unstable();
    assert_eq!(keys, ["layers", "ok", "url"]);
    assert_eq!(server.ready["ok"], true);
    assert_eq!(
        server.ready["url"],
        format!("http://{}/", server.address).as_str()
    );
    assert_eq!(
        server.ready["layers"],
        json!({"imagery":"/imagery/tilejson.json","terrain":"/terrain/tileset.json"})
    );
}
