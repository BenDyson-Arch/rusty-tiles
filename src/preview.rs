//! Local, read-only preview of explicitly selected outputs and a Cesium IIFE runtime.
use crate::Error;
use serde_json::{json, Map, Value};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tiny_http::{Header, Method, Request, Response, Server};

pub const MANIFESTS: [(&str, &str); 5] = [
    ("point-cloud", "tileset.json"),
    ("mesh", "tileset.json"),
    ("annotations", "tileset.json"),
    ("imagery", "tilejson.json"),
    ("terrain", "layer.json"),
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
        for (prefix, root) in &self.routes {
            if let Some(relative) = path.strip_prefix(prefix) {
                // Reject traversal on all platforms, including encoded separators,
                // drive prefixes and backslashes. Canonicalization also fences symlinks.
                if relative.contains('\\')
                    || relative.contains(':')
                    || Path::new(relative)
                        .components()
                        .any(|part| !matches!(part, Component::Normal(_)))
                {
                    return None;
                }
                let target = root.join(relative).canonicalize().ok()?;
                return (target.starts_with(root) && target.is_file()).then_some(target);
            }
        }
        None
    }

    fn respond(&self, request: Request) {
        if !matches!(request.method(), Method::Get | Method::Head) {
            let _ = request.respond(
                headers(
                    Response::from_string("Method not allowed").with_status_code(405),
                    "text/plain",
                )
                .with_header(header("Allow", "GET, HEAD")),
            );
            return;
        }
        let encoded = request.url().split(['?', '#']).next().unwrap_or("");
        let Ok(path) = percent_encoding::percent_decode_str(encoded).decode_utf8() else {
            let _ = request.respond(headers(Response::empty(400), "text/plain"));
            return;
        };
        match path.as_ref() {
            "/" | "/index.html" => {
                let _ = request.respond(headers(
                    Response::from_data(include_bytes!("../preview/index.html").as_slice()),
                    "text/html; charset=utf-8",
                ));
            }
            "/config.json" => {
                let _ = request.respond(headers(
                    Response::from_data(self.config.as_slice()),
                    "application/json",
                ));
            }
            path => {
                if let Some(target) = self.file(path) {
                    if let Ok(file) = File::open(&target) {
                        let _ = request.respond(headers(Response::from_file(file), mime(&target)));
                        return;
                    }
                }
                let _ = request.respond(headers(Response::empty(404), "text/plain"));
            }
        }
    }

    /// Bind only after every directory selection has been validated. Four
    /// file-streaming workers avoid loading tile payloads into memory.
    pub fn serve(self, host: &str, port: u16, json_output: bool) -> Result<(), Error> {
        let server = Server::http((host, port))
            .map_err(|error| Error::Environment(format!("cannot bind preview: {error}")))?;
        let address = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| Error::Environment("preview has no TCP address".into()))?;
        let url = format!("http://{address}/");
        if json_output {
            let config: Value = serde_json::from_slice(&self.config)?;
            println!("{}", json!({"ok":true,"url":url,"layers":config}));
        } else {
            println!("Preview: {url}");
        }
        std::io::stdout().flush()?;
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for request in server.incoming_requests() {
                        self.respond(request);
                    }
                });
            }
        });
        Ok(())
    }
}

fn selected_root(name: &str, path: &Path, manifest: &str) -> Result<PathBuf, Error> {
    let root = path.canonicalize().map_err(|_| {
        Error::Data(format!(
            "--{name} directory does not exist: {}",
            path.display()
        ))
    })?;
    let file = root.join(manifest).canonicalize().ok();
    if !root.is_dir() || !file.is_some_and(|file| file.starts_with(&root) && file.is_file()) {
        return Err(Error::Data(format!(
            "--{name} must contain {manifest} within its selected directory"
        )));
    }
    Ok(root)
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).expect("static HTTP header")
}
fn headers<R: Read>(response: Response<R>, content_type: &str) -> Response<R> {
    response
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-cache"))
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
        "terrain" => "application/vnd.quantized-mesh",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
