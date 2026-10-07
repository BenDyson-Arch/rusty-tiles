//! Installed-binary preview smoke test runs with no Python or repository cwd.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
            .args(["preview", "--json", "--port", "0", "--cesium"])
            .arg(&runtime)
            .arg("--mesh")
            .arg(&tiles)
            .env("PATH", "")
            .current_dir(root.path())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(server.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["ok"], true);
    let address = ready["url"]
        .as_str()
        .unwrap()
        .strip_prefix("http://")
        .unwrap()
        .trim_end_matches('/');
    assert!(address.starts_with("127.0.0.1:"));
    for (path, status, expected) in [
        ("/", "200", "/cesium/Cesium.js"),
        ("/config.json", "200", "/mesh/tileset.json"),
        ("/mesh/tileset.json", "200", "fixture"),
        ("/mesh/", "404", ""),
        ("/mesh/%2e%2e/private.txt", "404", ""),
        ("/terrain/layer.json", "404", ""),
    ] {
        let mut socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        write!(
            socket,
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status}")),
            "{response}"
        );
        assert!(response.contains("Cache-Control: no-cache"), "{response}");
        assert!(response.contains(expected), "{response}");
        assert!(!response.contains("not selected"));
    }
}
