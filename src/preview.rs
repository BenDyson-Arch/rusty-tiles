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
use std::path::{Component, Path, PathBuf};
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

    async fn respond(&self, request: Request<Incoming>) -> Response<Body> {
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

    /// Each accepted socket owns an independent HTTP/1 task. Idle or persistent
    /// clients yield the executor instead of occupying shared reader workers.
    pub fn serve(self, host: &str, port: u16, json_output: bool) -> Result<(), Error> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()?;
        runtime.block_on(async move {
            let listener = TcpListener::bind((host, port))
                .await
                .map_err(|error| Error::Environment(format!("cannot bind preview: {error}")))?;
            let url = format!("http://{}/", listener.local_addr()?);
            if json_output {
                let config: Value = serde_json::from_slice(&self.config)?;
                println!("{}", json!({"ok":true,"url":url,"layers":config}));
            } else {
                println!("Preview: {url}");
            }
            std::io::stdout().flush()?;
            let preview = Arc::new(self);
            loop {
                let (stream, _) = listener.accept().await?;
                let preview = Arc::clone(&preview);
                tokio::spawn(async move {
                    let service = service_fn(|request| async {
                        Ok::<_, Infallible>(preview.respond(request).await)
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
