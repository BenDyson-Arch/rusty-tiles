//! Local, read-only preview of explicitly selected outputs and a Cesium IIFE runtime.
use crate::Error;
use futures_util::TryStreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{json, Map, Value};
use std::convert::Infallible;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::File;
use tokio::net::TcpListener;
use tokio_util::io::ReaderStream;

type Body = BoxBody<Bytes, io::Error>;

pub const MANIFESTS: [(&str, &str); 5] = [
    ("point-cloud", "tileset.json"),
    ("mesh", "tileset.json"),
    ("annotations", "tileset.json"),
    ("imagery", "tilejson.json"),
    ("terrain", "tileset.json"),
];

pub struct Preview {
    routes: Vec<(String, PathBuf)>,
    config: Vec<u8>,
}

impl Preview {
    pub fn new(cesium: &Path, layers: &[(String, PathBuf)]) -> Result<Self, Error> {
        let cesium = selected_root("cesium", cesium, "Cesium.js")?;
        if layers.is_empty() {
            return Err(Error::Data(
                "select at least one of --point-cloud, --mesh, --annotations, --imagery, --terrain"
                    .into(),
            ));
        }
        let mut routes = vec![("/cesium/".into(), cesium)];
        let mut config = Map::new();
        for (name, path) in layers {
            let manifest = MANIFESTS
                .iter()
                .find(|(layer, _)| *layer == name)
                .ok_or_else(|| Error::Data(format!("unknown preview layer: {name}")))?
                .1;
            if config.contains_key(name) {
                return Err(Error::Data(format!("duplicate preview layer: {name}")));
            }
            routes.push((format!("/{name}/"), selected_root(name, path, manifest)?));
            config.insert(name.clone(), json!(format!("/{name}/{manifest}")));
        }
        Ok(Self {
            routes,
            config: serde_json::to_vec(&config)?,
        })
    }

    fn file(&self, path: &str) -> Option<PathBuf> {
        let (prefix, root) = self
            .routes
            .iter()
            .find(|(prefix, _)| path.starts_with(prefix.as_str()))?;
        exact_file(root, &segments(&path[prefix.len()..])?)
    }

    async fn respond(&self, request: Request<Incoming>, local: SocketAddr) -> Response<Body> {
        if !host_allowed(&request, local) {
            return full(421, "text/plain", Bytes::new());
        }
        if !matches!(*request.method(), Method::GET | Method::HEAD) {
            let mut response = full(405, "text/plain", "Method not allowed");
            response
                .headers_mut()
                .insert("Allow", "GET, HEAD".parse().unwrap());
            return response;
        }
        let Ok(path) = percent_encoding::percent_decode_str(request.uri().path()).decode_utf8()
        else {
            return full(400, "text/plain", Bytes::new());
        };
        match path.as_ref() {
            "/" | "/index.html" => full(
                200,
                "text/html; charset=utf-8",
                Bytes::from_static(include_bytes!("../preview/index.html")),
            ),
            "/config.json" => full(200, "application/json", self.config.clone()),
            path => {
                if let Some(target) = self.file(path) {
                    if let Ok(file) = File::open(&target).await {
                        if let Ok(metadata) = file.metadata().await {
                            // The body owns the file; Hyper polls it only as the
                            // socket can accept bytes. HEAD retains its length
                            // and Hyper suppresses the body without reading it.
                            let body = StreamBody::new(ReaderStream::new(file).map_ok(Frame::data))
                                .boxed();
                            return response(200, mime(&target), metadata.len(), body);
                        }
                    }
                }
                full(404, "text/plain", Bytes::new())
            }
        }
    }

    /// Serve until the process is signalled. Each accepted socket owns an
    /// independent HTTP/1 task; idle or persistent clients yield the executor
    /// instead of occupying shared reader workers. A loopback `address` answers
    /// only loopback `Host` names (DNS-rebinding defence); any other address
    /// is the caller's explicit opt-in to serve every host that can reach it.
    pub fn serve(self, address: SocketAddr, json_output: bool) -> Result<(), Error> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()?;
        runtime.block_on(async move {
            let listener = TcpListener::bind(address)
                .await
                .map_err(|error| Error::Environment(format!("cannot bind preview: {error}")))?;
            let local = listener.local_addr()?;
            let loopback = is_loopback(local);
            let url = format!("http://{local}/");
            if !loopback {
                let _ = writeln!(
                    io::stderr().lock(),
                    "warning: preview listens on non-loopback {local}; any host that can reach it can read the selected files"
                );
            }
            // A reader that closed stdout does not stop the server; any other
            // failure to announce the URL is a startup error.
            let announced = if json_output {
                let config: Value = serde_json::from_slice(&self.config)?;
                writeln!(
                    io::stdout().lock(),
                    "{}",
                    json!({"ok":true,"url":url,"layers":config,"loopback":loopback})
                )
            } else {
                writeln!(io::stdout().lock(), "Preview: {url}")
            }
            .and_then(|()| io::stdout().flush());
            match announced {
                Err(error) if error.kind() != io::ErrorKind::BrokenPipe => return Err(error.into()),
                _ => {}
            }
            let preview = Arc::new(self);
            loop {
                let stream = match listener.accept().await {
                    Ok((stream, _)) => stream,
                    // A client that aborts before acceptance ends only itself.
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::ConnectionAborted
                                | io::ErrorKind::ConnectionReset
                                | io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue
                    }
                    Err(error) => return Err(error.into()),
                };
                let preview = Arc::clone(&preview);
                tokio::spawn(async move {
                    let service = service_fn(|request| async {
                        Ok::<_, Infallible>(preview.respond(request, local).await)
                    });
                    // A disconnected or malformed client ends its own task.
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        })
    }
}

/// IPv4-mapped IPv6 loopback (`::ffff:127.0.0.1`) is loopback too.
fn is_loopback(address: SocketAddr) -> bool {
    address.ip().to_canonical().is_loopback()
}

/// A non-loopback listener accepts any authority. A loopback listener accepts
/// only `localhost`, `127.0.0.1`, `[::1]` or its own IP, with its own port
/// (which may be omitted only for port 80). An absolute-form request target's
/// authority takes precedence over `Host`, as in HTTP/1.1.
fn host_allowed<B>(request: &Request<B>, local: SocketAddr) -> bool {
    if !is_loopback(local) {
        return true;
    }
    let authority = match request.uri().authority() {
        Some(authority) => authority.clone(),
        None => match request
            .headers()
            .get(hyper::header::HOST)
            .and_then(|host| host.to_str().ok())
            .and_then(|host| host.parse::<hyper::http::uri::Authority>().ok())
        {
            Some(authority) => authority,
            None => return false,
        },
    };
    let port_matches = match authority.port_u16() {
        Some(port) => port == local.port(),
        None => local.port() == 80,
    };
    let host = authority.host().to_ascii_lowercase();
    let own = match local.ip() {
        std::net::IpAddr::V4(ip) => ip.to_string(),
        std::net::IpAddr::V6(ip) => format!("[{ip}]"),
    };
    port_matches && ["localhost", "127.0.0.1", "[::1]", own.as_str()].contains(&host.as_str())
}

fn selected_root(name: &str, path: &Path, manifest: &str) -> Result<PathBuf, Error> {
    let root = path.canonicalize().map_err(|_| {
        Error::Data(format!(
            "--{name} directory does not exist: {}",
            path.display()
        ))
    })?;
    if !root.is_dir() || exact_file(&root, &[manifest]).is_none() {
        return Err(Error::Data(format!(
            "--{name} must contain {manifest} within its selected directory"
        )));
    }
    Ok(root)
}

/// Lexical request-path admission, decided before any filesystem access so
/// every platform agrees: one or more `/`-separated segments, each non-empty
/// (no trailing slash or `//`), not `.` or `..`, and free of backslashes, `:` (Windows
/// drives and streams) and control characters including NUL.
fn segments(relative: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = relative.split('/').collect();
    parts
        .iter()
        .all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && !part
                    .chars()
                    .any(|c| c == '\\' || c == ':' || c.is_control())
        })
        .then_some(parts)
}

/// The regular file named exactly by `segments` below the canonical `root`.
/// The canonical target must sit below `root` under the requested names,
/// compared component by component as bytes. A platform may resolve an alias
/// to an existing file: a case or Unicode-normalization variant on macOS or
/// Windows, a Windows short name or trailing dot/space, or any symlink. Its
/// canonical names then differ from the request, so it is refused everywhere.
fn exact_file(root: &Path, segments: &[&str]) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    path.extend(segments);
    let target = path.canonicalize().ok()?;
    let exact = target
        .strip_prefix(root)
        .ok()?
        .components()
        .map(|component| component.as_os_str())
        .eq(segments.iter().map(std::ffi::OsStr::new));
    (exact && target.is_file()).then_some(target)
}

fn full(status: u16, content_type: &'static str, bytes: impl Into<Bytes>) -> Response<Body> {
    let bytes = bytes.into();
    let length = bytes.len() as u64;
    let body = Full::new(bytes).map_err(|never| match never {}).boxed();
    response(status, content_type, length, body)
}

fn response(status: u16, content_type: &'static str, length: u64, body: Body) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Content-Length", length)
        .header("Cache-Control", "no-cache")
        .body(body)
        .expect("static HTTP response headers")
}
fn mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" | "gltf" => "application/json",
        "html" => "text/html; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "wasm" => "application/wasm",
        "glb" => "model/gltf-binary",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Preview) {
        let root = tempfile::tempdir().unwrap();
        let cesium = root.path().join("runtime");
        let mesh = root.path().join("mesh");
        let other = root.path().join("terrain");
        for directory in [&cesium, &mesh, &other, &mesh.join("nested")] {
            std::fs::create_dir(directory).unwrap();
        }
        std::fs::write(cesium.join("Cesium.js"), "//").unwrap();
        std::fs::write(mesh.join("tileset.json"), "{}").unwrap();
        std::fs::write(mesh.join("nested/tile.glb"), "glTF").unwrap();
        std::fs::write(other.join("tileset.json"), "{}").unwrap();
        std::fs::write(root.path().join("private.txt"), "secret").unwrap();
        let preview = Preview::new(&cesium, &[("mesh".into(), mesh)]).unwrap();
        (root, preview)
    }

    #[test]
    fn fence_admits_only_regular_files_inside_selected_roots() {
        let (root, preview) = fixture();
        let mesh = root.path().join("mesh").canonicalize().unwrap();
        assert_eq!(
            preview.file("/mesh/nested/tile.glb"),
            Some(mesh.join("nested/tile.glb"))
        );
        assert!(preview.file("/cesium/Cesium.js").is_some());
        for path in [
            "/mesh/",
            "/mesh/nested",
            "/mesh/../private.txt",
            "/mesh/nested/../../private.txt",
            "/mesh/./tileset.json",
            "/mesh//etc/passwd",
            "/mesh/..\\private.txt",
            "/mesh/C:/private.txt",
            "/mesh/missing.json",
            "/terrain/tileset.json",
            "/cesium/../mesh/tileset.json",
            "/private.txt",
            "/mesh",
        ] {
            assert_eq!(preview.file(path), None, "{path}");
        }
        let absolute = format!("/mesh/{}", root.path().join("private.txt").display());
        assert_eq!(preview.file(&absolute), None);
    }

    #[cfg(unix)]
    #[test]
    fn fence_refuses_every_symlink_below_the_root() {
        let (root, preview) = fixture();
        let mesh = root.path().join("mesh");
        std::os::unix::fs::symlink(root.path().join("private.txt"), mesh.join("out.txt")).unwrap();
        std::os::unix::fs::symlink(root.path().join("terrain"), mesh.join("sibling")).unwrap();
        std::os::unix::fs::symlink(mesh.join("nested/tile.glb"), mesh.join("alias.glb")).unwrap();
        std::os::unix::fs::symlink("/dev/null", mesh.join("device")).unwrap();
        assert_eq!(preview.file("/mesh/out.txt"), None);
        assert_eq!(preview.file("/mesh/sibling/tileset.json"), None);
        assert_eq!(preview.file("/mesh/device"), None);
        // Even an alias of an inner file names a different canonical path.
        assert_eq!(preview.file("/mesh/alias.glb"), None);
        std::os::unix::fs::symlink(mesh.join("nested"), mesh.join("linked")).unwrap();
        assert_eq!(preview.file("/mesh/linked/tile.glb"), None);
        assert!(preview.file("/mesh/nested/tile.glb").is_some());
    }

    #[test]
    fn lexical_admission_rules_need_no_filesystem() {
        assert_eq!(segments("a/b.glb"), Some(vec!["a", "b.glb"]));
        assert_eq!(segments("%2e%2e"), Some(vec!["%2e%2e"])); // a literal name after one decode
        for (rule, relative) in [
            ("empty path", ""),
            ("trailing slash", "tileset.json/"),
            ("trailing slash on a directory", "nested/"),
            ("empty segment", "nested//tile.glb"),
            ("leading slash (absolute)", "/etc/passwd"),
            ("dot segment", "./tileset.json"),
            ("inner dot segment", "nested/./tile.glb"),
            ("parent segment", "../private.txt"),
            ("inner parent segment", "nested/../tileset.json"),
            ("trailing parent segment", "tileset.json/.."),
            ("backslash separator", "..\\private.txt"),
            ("backslash in a name", "nested\\tile.glb"),
            ("Windows drive", "C:/private.txt"),
            ("Windows drive-relative", "C:private.txt"),
            ("alternate data stream", "tileset.json:stream"),
            ("UNC-like prefix", "\\\\server\\share"),
            ("NUL", "tileset.json\0.png"),
            ("control character", "tile\u{1}.glb"),
            ("DEL", "tile\u{7f}.glb"),
        ] {
            assert_eq!(segments(relative), None, "{rule}: {relative:?}");
        }
    }

    #[test]
    fn names_must_match_the_canonical_names_exactly() {
        let (root, preview) = fixture();
        let mesh = root.path().join("mesh");
        std::fs::write(mesh.join("Tile.glb"), "glTF").unwrap();
        // NFC "café" stored; NFD spelling requested.
        std::fs::write(mesh.join("caf\u{e9}.bin"), "nfc").unwrap();
        assert!(preview.file("/mesh/Tile.glb").is_some());
        assert!(
            preview.file("/mesh/caf%C3%A9.bin").is_none(),
            "undecoded request text"
        );
        assert!(preview.file("/mesh/caf\u{e9}.bin").is_some());
        for (rule, path) in [
            // A case-insensitive filesystem (APFS, NTFS) resolves these to the
            // stored file; its canonical name differs, so all platforms refuse.
            ("case variant of a file", "/mesh/tile.glb"),
            ("case variant of a directory", "/mesh/NESTED/tile.glb"),
            ("case variant of the manifest", "/mesh/TILESET.JSON"),
            // macOS file systems match normalization variants.
            ("NFD variant of an NFC name", "/mesh/cafe\u{301}.bin"),
            // Windows strips trailing dots and spaces from names.
            ("trailing dot", "/mesh/tileset.json."),
            ("trailing space", "/mesh/tileset.json "),
            // Windows 8.3 short-name form.
            ("short name", "/mesh/TILESE~1.JSO"),
        ] {
            assert_eq!(preview.file(path), None, "{rule}: {path:?}");
        }
    }

    #[test]
    fn selected_roots_are_canonical_directories_with_an_inner_manifest() {
        let root = tempfile::tempdir().unwrap();
        let mesh = root.path().join("mesh");
        std::fs::create_dir_all(mesh.join("tileset.json")).unwrap();
        let error = selected_root("mesh", &mesh, "tileset.json").unwrap_err();
        assert!(error.to_string().contains("must contain tileset.json"));
        let file = root.path().join("file");
        std::fs::write(&file, "{}").unwrap();
        assert!(selected_root("mesh", &file, "tileset.json").is_err());
        let spelled = root.path().join("mesh/../mesh");
        std::fs::remove_dir(mesh.join("tileset.json")).unwrap();
        std::fs::write(mesh.join("tileset.json"), "{}").unwrap();
        assert_eq!(
            selected_root("mesh", &spelled, "tileset.json").unwrap(),
            mesh.canonicalize().unwrap()
        );
    }

    #[test]
    fn loopback_listeners_answer_only_loopback_authorities() {
        let request = |uri: &str, host: Option<&str>| {
            let mut builder = Request::builder().uri(uri);
            if let Some(host) = host {
                builder = builder.header("Host", host);
            }
            builder.body(()).unwrap()
        };
        let v4: SocketAddr = "127.0.0.1:9227".parse().unwrap();
        let v6: SocketAddr = "[::1]:9227".parse().unwrap();
        for host in [
            "127.0.0.1:9227",
            "localhost:9227",
            "LOCALHOST:9227",
            "[::1]:9227",
        ] {
            assert!(host_allowed(&request("/", Some(host)), v4), "{host}");
            assert!(host_allowed(&request("/", Some(host)), v6), "{host}");
        }
        for host in [
            "evil.example:9227",
            "127.0.0.1:80",
            "127.0.0.1",
            "localhost:9228",
            "127.0.0.2:9227",
            "localhost.:9227",
            "",
        ] {
            assert!(!host_allowed(&request("/", Some(host)), v4), "{host:?}");
        }
        assert!(!host_allowed(&request("/", None), v4));
        // The absolute-form authority is authoritative over Host.
        assert!(!host_allowed(
            &request("http://evil.example:9227/", Some("127.0.0.1:9227")),
            v4
        ));
        let port_80: SocketAddr = "127.0.0.1:80".parse().unwrap();
        assert!(host_allowed(&request("/", Some("localhost")), port_80));
        let mapped: SocketAddr = "[::ffff:127.0.0.1]:9227".parse().unwrap();
        assert!(is_loopback(mapped));
        assert!(!host_allowed(
            &request("/", Some("evil.example:9227")),
            mapped
        ));
        assert!(host_allowed(&request("/", Some("localhost:9227")), mapped));
        assert!(host_allowed(
            &request("/", Some("[::ffff:127.0.0.1]:9227")),
            mapped
        ));
        let open: SocketAddr = "0.0.0.0:9227".parse().unwrap();
        assert!(host_allowed(&request("/", Some("evil.example")), open));
        assert!(host_allowed(&request("/", None), open));
    }
}
